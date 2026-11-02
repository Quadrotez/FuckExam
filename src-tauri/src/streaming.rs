use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite::Message;

const MAX_HISTORY: usize = 100;
const MAX_HISTORY_BYTES: usize = 2 * 1024 * 1024;
const MAX_CHAT_TEXT_CHARS: usize = 4000;
const MAX_CHAT_IMAGE_BYTES: usize = 512 * 1024;
const DEFAULT_STREAM_FPS: u32 = 24;
const DEFAULT_JPEG_QUALITY: u8 = 75;
const DEFAULT_STREAM_MAX_WIDTH: usize = 1280;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatImage {
    pub name: String,
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMsg {
    pub from: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub markdown: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ChatImage>,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn chat_message(
    from: Option<String>,
    text: Option<String>,
    markdown: Option<bool>,
    image: Option<ChatImage>,
) -> Option<ChatMsg> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let text = text.unwrap_or_default();
    if text.len() > MAX_CHAT_TEXT_CHARS * 4 || text.chars().count() > MAX_CHAT_TEXT_CHARS {
        return None;
    }
    let image = match image {
        Some(image) => {
            if image.name.len() > 480
                || image.name.chars().count() > 120
                || image.data.len() > 4 * MAX_CHAT_IMAGE_BYTES.div_ceil(3)
            {
                return None;
            }
            let bytes = STANDARD.decode(&image.data).ok()?;
            if bytes.len() > MAX_CHAT_IMAGE_BYTES
                || bytes.len() < 4
                || bytes[..2] != [0xff, 0xd8]
                || bytes[bytes.len() - 2..] != [0xff, 0xd9]
            {
                return None;
            }
            Some(image)
        }
        None => None,
    };
    if text.trim().is_empty() && image.is_none() {
        return None;
    }
    let from = from.unwrap_or_else(|| "зритель".into());
    let from = from.chars().take(80).collect::<String>();
    Some(ChatMsg {
        from: if from.is_empty() {
            "зритель".into()
        } else {
            from
        },
        text,
        markdown: markdown.unwrap_or(false),
        image,
    })
}

fn chat_history_size(history: &[ChatMsg]) -> usize {
    history
        .iter()
        .map(|msg| {
            msg.from.len()
                + msg.text.len()
                + msg
                    .image
                    .as_ref()
                    .map_or(0, |i| i.name.len() + i.data.len())
                + 128
        })
        .sum()
}

fn trim_chat_history(history: &mut Vec<ChatMsg>) {
    while history.len() > MAX_HISTORY || chat_history_size(history) > MAX_HISTORY_BYTES {
        history.remove(0);
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamConfig {
    pub fps: u32,
    pub quality: u8,
    pub max_width: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            fps: DEFAULT_STREAM_FPS,
            quality: DEFAULT_JPEG_QUALITY,
            max_width: DEFAULT_STREAM_MAX_WIDTH,
        }
    }
}

impl StreamConfig {
    /// Заменяет нулевые/выходящие за границы поля значениями из env или дефолтом.
    pub fn normalized(self) -> Self {
        let env_fps = std::env::var("FEX_STREAM_FPS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok());
        let env_quality = std::env::var("FEX_JPEG_QUALITY")
            .ok()
            .and_then(|v| v.parse::<u8>().ok());
        let env_max_width = std::env::var("FEX_STREAM_MAX_W")
            .ok()
            .and_then(|v| v.parse::<usize>().ok());
        StreamConfig {
            fps: if (1..=60).contains(&self.fps) {
                self.fps
            } else {
                env_fps.unwrap_or(DEFAULT_STREAM_FPS).clamp(1, 60)
            },
            quality: if (1..=100).contains(&self.quality) {
                self.quality
            } else {
                env_quality.unwrap_or(DEFAULT_JPEG_QUALITY).clamp(1, 100)
            },
            max_width: if self.max_width >= 64 {
                self.max_width
            } else {
                env_max_width.unwrap_or(DEFAULT_STREAM_MAX_WIDTH).max(64)
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamEntry {
    pub id: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug)]
pub enum ClientEvent {
    Welcome {
        streams: Vec<StreamEntry>,
        history: Vec<ChatMsg>,
    },
    Streams(Vec<StreamEntry>),
    StreamEnd(u32),
    Chat(ChatMsg),
    Video {
        node: u32,
        jpeg: Vec<u8>,
    },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    pub active: bool,
    pub mode: String,
    pub addresses: Vec<String>,
    pub web: Vec<String>,
    pub relay: Option<String>,
    pub room: Option<String>,
    pub viewers: usize,
    pub streams: Vec<StreamEntry>,
    pub frames: u64,
}

pub struct Streaming(pub Arc<StreamHub>);

impl Default for Streaming {
    fn default() -> Self {
        Self(Arc::new(StreamHub::default()))
    }
}

struct Client {
    id: usize,
    tx: mpsc::Sender<ClientEvent>,
    recorder_link: bool,
}

struct HubInner {
    clients: Vec<Client>,
    next_id: usize,
    streams: HashMap<u32, StreamEntry>,
    history: Vec<ChatMsg>,
    viewers: usize,
    frames: u64,
}

#[derive(Default)]
struct ModeState {
    active: bool,
    mode: String,
    addresses: Vec<String>,
    web: Vec<String>,
    relay: Option<String>,
    room: Option<String>,
}

pub struct StreamHub {
    inner: Mutex<HubInner>,
    app: Mutex<Option<AppHandle>>,
    mode: Mutex<ModeState>,
    config: Mutex<StreamConfig>,
    stop: Arc<AtomicBool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Default for StreamHub {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HubInner {
                clients: Vec::new(),
                next_id: 1,
                streams: HashMap::new(),
                history: Vec::new(),
                viewers: 0,
                frames: 0,
            }),
            app: Mutex::new(None),
            mode: Mutex::new(ModeState::default()),
            config: Mutex::new(StreamConfig::default()),
            stop: Arc::new(AtomicBool::new(false)),
            task: Mutex::new(None),
        }
    }
}

fn event_to_message(ev: &ClientEvent) -> Message {
    match ev {
        ClientEvent::Video { node, jpeg } => {
            let mut v = Vec::with_capacity(4 + jpeg.len());
            v.extend_from_slice(&node.to_le_bytes());
            v.extend_from_slice(jpeg);
            Message::Binary(v.into())
        }
        ClientEvent::Streams(list) => Message::Text(
            json!({ "type": "streams", "streams": list })
                .to_string()
                .into(),
        ),
        ClientEvent::StreamEnd(id) => {
            Message::Text(json!({ "type": "stream-end", "id": id }).to_string().into())
        }
        ClientEvent::Chat(m) => Message::Text(
            json!({ "type": "chat", "from": m.from, "text": m.text, "markdown": m.markdown, "image": m.image })
                .to_string()
                .into(),
        ),
        ClientEvent::Welcome { streams, history } => Message::Text(
            json!({ "type": "welcome", "streams": streams, "history": history })
                .to_string()
                .into(),
        ),
    }
}

#[derive(Deserialize)]
struct Hello {
    #[serde(rename = "type")]
    _kind: String,
    role: String,
}

#[derive(Deserialize)]
struct Incoming {
    #[serde(rename = "type")]
    kind: String,
    from: Option<String>,
    text: Option<String>,
    #[serde(default)]
    markdown: Option<bool>,
    #[serde(default)]
    image: Option<ChatImage>,
}

pub fn encode_jpeg(bgra: &[u8], w: usize, h: usize, stride: usize, quality: u8) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || bgra.len() < stride * h {
        return None;
    }
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        let row = &bgra[y * stride..y * stride + w * 4];
        let out = &mut rgb[y * w * 3..(y + 1) * w * 3];
        for x in 0..w {
            let i = x * 4;
            let o = x * 3;
            out[o] = row[i + 2];
            out[o + 1] = row[i + 1];
            out[o + 2] = row[i];
        }
    }
    let mut out = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.encode(&rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
        .ok()?;
    Some(out)
}

/// Кадр для стрима: при большом размере уменьшает до max_dim по большей стороне
/// (WebKit быстрее декодирует маленькие JPEG, а зрители смотрят в окне ~500px).
/// Возвращает (w, h, jpeg) реальных размеров JPEG.
pub fn encode_stream_frame(
    bgra: &[u8],
    w: usize,
    h: usize,
    stride: usize,
    config: &StreamConfig,
) -> Option<(u32, u32, Vec<u8>)> {
    let max_dim = if config.max_width >= 64 {
        config.max_width
    } else {
        1920
    };
    let quality = if (1..=100).contains(&config.quality) {
        config.quality
    } else {
        75
    };
    if w <= max_dim || h == 0 {
        let jpeg = encode_jpeg(bgra, w, h, stride, quality)?;
        return Some((w as u32, h as u32, jpeg));
    }
    let scale = (max_dim as f64) / (w.max(h) as f64);
    let w2 = ((w as f64) * scale).round().clamp(1.0, u32::MAX as f64) as usize;
    let h2 = ((h as f64) * scale).round().clamp(1.0, u32::MAX as f64) as usize;
    let stride2 = w2 * 4;
    let mut scaled = vec![0u8; stride2 * h2];
    for y2 in 0..h2 {
        let sy = (((y2 as f64 + 0.5) / h2 as f64) * h as f64) as usize;
        let sy = sy.min(h - 1);
        for x2 in 0..w2 {
            let sx = (((x2 as f64 + 0.5) / w2 as f64) * w as f64) as usize;
            let sx = sx.min(w - 1);
            let si = sy * stride + sx * 4;
            let oi = y2 * stride2 + x2 * 4;
            scaled[oi..oi + 4].copy_from_slice(&bgra[si..si + 4]);
        }
    }
    let jpeg = encode_jpeg(&scaled, w2, h2, stride2, quality)?;
    Some((w2 as u32, h2 as u32, jpeg))
}

pub fn local_addresses() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for iface in if_addrs::get_if_addrs().map_err(|e| format!("get interfaces: {e}"))? {
        if let if_addrs::IfAddr::V4(addr) = iface.addr {
            let ip = addr.ip;
            if !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local() {
                out.push(ip.to_string());
            }
        }
    }
    Ok(out)
}

/// Проверка доступности relay: подключаемся как зритель в тестовую комнату
/// и ждём welcome. Позволяет узнать «ворк/не ворк» до запуска трансляции.
pub async fn ping_relay(url: String) -> Result<String, String> {
    let url = if url.starts_with("ws://") || url.starts_with("wss://") {
        url
    } else {
        format!("ws://{url}")
    };
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("не удалось подключиться: {e}"))?;
    let hello = json!({"type":"hello","role":"viewer","room":"__ping__"}).to_string();
    ws.send(Message::Text(hello.into()))
        .await
        .map_err(|e| format!("не удалось отправить hello: {e}"))?;

    match tokio::time::timeout(Duration::from_secs(3), ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) if t.contains("\"type\":\"welcome\"") => {
            let _ = ws.send(Message::Close(None)).await;
            Ok(format!("relay доступен ({url})"))
        }
        Ok(Some(Ok(Message::Text(t)))) if t.contains("\"type\":\"error\"") => {
            let _ = ws.send(Message::Close(None)).await;
            Err(format!("relay вернул ошибку: {t}"))
        }
        Ok(_) => {
            let _ = ws.send(Message::Close(None)).await;
            Err("неожиданный ответ от relay".into())
        }
        Err(_) => {
            let _ = ws.send(Message::Close(None)).await;
            Err("таймаут: relay не отвечает (проверьте адрес, порт, фаервол)".into())
        }
    }
}

impl StreamHub {
    /// Локальный режим: HTTP+WS-сервер уже поднят (`web::WebServer`), здесь
    /// только помечаем трансляцию активной и публикуем адреса для зрителей.
    pub async fn arm_local(
        self: &Arc<Self>,
        app: AppHandle,
        port: u16,
        config: StreamConfig,
    ) -> Result<StreamStatus, String> {
        self.set_config(config);
        if self.is_active() {
            return Ok(self.status());
        }
        *self.app.lock().unwrap() = Some(app);
        let mut ips = local_addresses().unwrap_or_default();
        ips.sort();
        *self.mode.lock().unwrap() = ModeState {
            active: true,
            mode: "local".into(),
            addresses: ips.iter().map(|ip| format!("ws://{ip}:{port}")).collect(),
            web: ips.iter().map(|ip| format!("http://{ip}:{port}/")).collect(),
            relay: None,
            room: None,
        };
        Ok(self.status())
    }

    pub async fn arm_relay(
        self: &Arc<Self>,
        app: AppHandle,
        url: String,
        config: StreamConfig,
    ) -> Result<StreamStatus, String> {
        self.set_config(config);
        if self.mode.lock().unwrap().active {
            return Ok(self.status());
        }
        *self.app.lock().unwrap() = Some(app);
        *self.mode.lock().unwrap() = ModeState {
            active: true,
            mode: "relay".into(),
            addresses: Vec::new(),
            web: Vec::new(),
            relay: Some(url.clone()),
            room: None,
        };
        self.stop.store(false, Ordering::Relaxed);
        let task = tokio::spawn(relay_client(self.clone(), url));
        *self.task.lock().unwrap() = Some(task);
        Ok(self.status())
    }
    pub fn set_config(&self, config: StreamConfig) {
        *self.config.lock().unwrap() = config.normalized();
    }

    pub fn config(&self) -> StreamConfig {
        *self.config.lock().unwrap()
    }

    pub fn is_active(&self) -> bool {
        self.mode.lock().unwrap().active
    }

    pub fn status(&self) -> StreamStatus {
        let (mode, addresses, web, relay, room, active) = {
            let m = self.mode.lock().unwrap();
            (
                m.mode.clone(),
                m.addresses.clone(),
                m.web.clone(),
                m.relay.clone(),
                m.room.clone(),
                m.active,
            )
        };
        let (streams, viewers) = {
            let i = self.inner.lock().unwrap();
            (i.streams.values().cloned().collect(), i.viewers)
        };
        let frames = self.inner.lock().unwrap().frames;
        StreamStatus {
            active,
            mode,
            addresses,
            web,
            relay,
            room,
            viewers,
            streams,
            frames,
        }
    }

    /// Останавливает трансляцию, но оставляет веб-сервер поднятым:
    /// иначе из «скрытого» режима приложение было бы не достать.
    pub fn deactivate(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.task.lock().unwrap().take() {
            t.abort();
        }
        *self.mode.lock().unwrap() = ModeState::default();
    }

    pub fn register(&self, recorder_link: bool) -> (usize, mpsc::Receiver<ClientEvent>) {
        let (tx, rx) = mpsc::channel(32);
        let mut i = self.inner.lock().unwrap();
        let id = i.next_id;
        i.next_id += 1;
        if !recorder_link {
            i.viewers += 1;
        }
        i.clients.push(Client {
            id,
            tx,
            recorder_link,
        });
        (id, rx)
    }

    pub fn unregister(&self, id: usize) {
        let mut i = self.inner.lock().unwrap();
        if let Some(pos) = i.clients.iter().position(|c| c.id == id) {
            if !i.clients[pos].recorder_link {
                i.viewers = i.viewers.saturating_sub(1);
            }
            i.clients.remove(pos);
        }
    }

    pub fn push_video(&self, node: u32, w: u32, h: u32, jpeg: Vec<u8>) {
        let mut i = self.inner.lock().unwrap();
        i.frames = i.frames.saturating_add(1);
        let is_new = !i.streams.contains_key(&node);
        i.streams.insert(node, StreamEntry { id: node, w, h });
        if is_new {
            let list = i.streams.values().cloned().collect::<Vec<_>>();
            for c in &i.clients {
                let _ = c.tx.try_send(ClientEvent::Streams(list.clone()));
            }
        }
        let ev = ClientEvent::Video { node, jpeg };
        for c in &i.clients {
            if c.tx.try_send(ev.clone()).is_err() && std::env::var_os("FEX_DEBUG").is_some() {
                eprintln!("[fuckexam] hub: drop frame for client {}", c.id);
            }
        }
    }

    pub fn announce_end(&self, node: u32) {
        let mut i = self.inner.lock().unwrap();
        if i.streams.remove(&node).is_some() {
            for c in &i.clients {
                let _ = c.tx.try_send(ClientEvent::StreamEnd(node));
            }
        }
    }

    pub async fn send_chat(
        &self,
        text: String,
        markdown: bool,
        image: Option<ChatImage>,
    ) -> Result<(), String> {
        let msg = chat_message(
            Some("транслятор".into()),
            Some(text),
            Some(markdown),
            image,
        )
        .ok_or_else(|| "сообщение или фото не прошло проверку".to_string())?;

        let relay_mode = {
            let mode = self.mode.lock().unwrap();
            if !mode.active {
                return Err("сначала запустите трансляцию".into());
            }
            match mode.mode.as_str() {
                "local" => false,
                "relay" => true,
                _ => return Err("режим трансляции не готов".into()),
            }
        };

        if relay_mode {
            let tx = self
                .inner
                .lock()
                .unwrap()
                .clients
                .iter()
                .find(|client| client.recorder_link)
                .map(|client| client.tx.clone())
                .ok_or_else(|| "подключение к relay ещё не готово".to_string())?;
            tx.send(ClientEvent::Chat(msg.clone()))
                .await
                .map_err(|_| "соединение с relay закрыто".to_string())?;
        }

        // В relay-режиме это локальное отображение исходящего сообщения; в relay
        // сервер отправляется только через recorder_link, чтобы не зациклить эхо.
        self.emit_chat(msg);
        Ok(())
    }

    pub fn chat_history(&self) -> Vec<ChatMsg> {
        self.inner.lock().unwrap().history.clone()
    }

    pub fn welcome_snapshot(&self) -> (Vec<StreamEntry>, Vec<ChatMsg>) {
        let i = self.inner.lock().unwrap();
        (
            i.streams.values().cloned().collect(),
            i.history.clone(),
        )
    }

    pub fn set_room(&self, room: String) {
        let changed = {
            let mut mode = self.mode.lock().unwrap();
            if mode.room.as_deref() == Some(room.as_str()) {
                false
            } else {
                mode.room = Some(room.clone());
                true
            }
        };
        if changed {
            if let Some(app) = self.app.lock().unwrap().clone() {
                let _ = app.emit("stream-room", &room);
            }
        }
    }

    pub fn emit_error(&self, message: String) {
        if let Some(app) = self.app.lock().unwrap().clone() {
            let _ = app.emit("stream-error", &message);
        }
    }

    pub fn emit_chat(&self, msg: ChatMsg) {
        {
            let mut i = self.inner.lock().unwrap();
            i.history.push(msg.clone());
            trim_chat_history(&mut i.history);
            for c in &i.clients {
                if !c.recorder_link {
                    let _ = c.tx.try_send(ClientEvent::Chat(msg.clone()));
                }
            }
        }
        if let Some(app) = self.app.lock().unwrap().clone() {
            let _ = app.emit("stream-chat", &msg);
        }
    }
}

pub(crate) async fn run_viewer<S>(
    hub: Arc<StreamHub>,
    ws: tokio_tungstenite::WebSocketStream<S>,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut sink, mut rstream) = ws.split();

    let hello = match rstream.next().await {
        Some(Ok(Message::Text(t))) => serde_json::from_str::<Hello>(&t).ok(),
        _ => None,
    };
    match hello {
        Some(h) if h.role == "viewer" => {}
        _ => {
            let _ = sink
                .send(Message::Text(
                    json!({"type":"error","error":"role must be viewer"})
                        .to_string()
                        .into(),
                ))
                .await;
            return Err("bad hello".into());
        }
    }

    let (id, rx) = hub.register(false);
    if std::env::var_os("FEX_DEBUG").is_some() {
        eprintln!("[fuckexam] hub: viewer registered (id={id})");
    }
    let (streams, history) = hub.welcome_snapshot();
    if sink
        .send(event_to_message(&ClientEvent::Welcome { streams, history }))
        .await
        .is_err()
    {
        hub.unregister(id);
        return Ok(());
    }

    let mut rx = rx;
    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Some(ev) => {
                        if sink.send(event_to_message(&ev)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            msg = rstream.next() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(incoming) = serde_json::from_str::<Incoming>(&t) {
                            if incoming.kind == "chat" {
                                if let Some(msg) = chat_message(incoming.from, incoming.text, incoming.markdown, incoming.image) {
                                    hub.emit_chat(msg);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
    hub.unregister(id);
    Ok(())
}

async fn relay_client(hub: Arc<StreamHub>, url: String) {
    let (conn, _resp) = match tokio_tungstenite::connect_async(&url).await {
        Ok(x) => x,
        Err(e) => {
            hub.emit_error(format!("relay: {e}"));
            hub.deactivate();
            return;
        }
    };
    let (mut sink, mut rstream) = conn.split();

    // Комнату придумывает сервер: клиент её не передаёт.
    let hello = json!({"type":"hello","role":"recorder"}).to_string();
    if sink.send(Message::Text(hello.into())).await.is_err() {
        hub.deactivate();
        return;
    }

    let (id, rx) = hub.register(true);
    let mut rx = rx;
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(ev) => {
                    if sink.send(event_to_message(&ev)).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            msg = rstream.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&t) else {
                        continue;
                    };
                    match value.get("type").and_then(|x| x.as_str()) {
                        Some("room") => {
                            if let Some(room) = value.get("room").and_then(|x| x.as_str()) {
                                hub.set_room(room.to_string());
                            }
                        }
                        Some("error") => {
                            let error = value
                                .get("error")
                                .and_then(|x| x.as_str())
                                .unwrap_or("relay отклонил подключение");
                            hub.emit_error(error.to_string());
                            break;
                        }
                        Some("chat") => {
                            if let Ok(incoming) = serde_json::from_value::<Incoming>(value) {
                                if let Some(msg) = chat_message(incoming.from, incoming.text, incoming.markdown, incoming.image) {
                                    hub.emit_chat(msg);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(_))) => break,
                Some(Err(_)) => break,
                _ => {}
            },
        }
    }
    hub.unregister(id);
    hub.deactivate();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Поднимает настоящий веб-сервер (HTTP+WS) на свободном порту.
    async fn spawn_server(hub: Arc<StreamHub>) -> (u16, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let server = crate::web::WebServer::new(hub, crate::settings::SettingsStore::ephemeral());
        let stop_task = stop.clone();
        tokio::spawn(async move { crate::web::serve(server, listener, stop_task).await });
        (port, stop)
    }

    #[test]
    fn jpeg_encode_bgra() {
        let w = 16usize;
        let h = 12usize;
        let stride = w * 4 + 64;
        let mut buf = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x * 4;
                // BGRA: красный пиксель
                buf[i] = 0u8; // B
                buf[i + 1] = 0; // G
                buf[i + 2] = 200; // R
                buf[i + 3] = 255; // A
            }
        }
        let jpeg = encode_jpeg(&buf, w, h, stride, 75).expect("jpeg");
        assert!(jpeg.len() > 100, "jpeg should be non-trivial");
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "SOI marker");
        assert!(jpeg.ends_with(&[0xFF, 0xD9]), "EOI marker");
    }

    #[test]
    fn jpeg_bad_dims() {
        assert!(encode_jpeg(&[], 0, 10, 40, 75).is_none());
    }

    #[test]
    fn room_status_is_filled_from_server() {
        let hub = StreamHub::default();
        assert!(hub.status().room.is_none());
        hub.set_room("ABC123".into());
        assert_eq!(hub.status().room.as_deref(), Some("ABC123"));
    }

    #[test]
    fn chat_message_keeps_legacy_text_and_accepts_bounded_jpeg() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let legacy = chat_message(Some("Ana".into()), Some("привет".into()), None, None).unwrap();
        assert!(!legacy.markdown);
        assert!(legacy.image.is_none());
        let legacy_json = serde_json::to_value(&legacy).unwrap();
        assert!(legacy_json.get("markdown").is_none());
        assert!(legacy_json.get("image").is_none());

        let image = ChatImage {
            name: "photo.jpg".into(),
            data: STANDARD.encode([0xff, 0xd8, 0xff, 0xd9]),
        };
        let msg = chat_message(
            Some("Ana".into()),
            Some("**важно**".into()),
            Some(true),
            Some(image),
        )
        .unwrap();
        assert!(msg.markdown);
        assert_eq!(msg.image.as_ref().unwrap().name, "photo.jpg");
        assert_eq!(
            serde_json::to_value(msg).unwrap()["image"]["data"],
            "/9j/2Q=="
        );
    }

    #[test]
    fn chat_message_rejects_invalid_photo_and_empty_content() {
        let invalid = ChatImage {
            name: "bad.jpg".into(),
            data: "bm90IGEgSlBFRw==".into(),
        };
        assert!(chat_message(None, Some("text".into()), None, Some(invalid)).is_none());
        assert!(chat_message(None, Some(" \n".into()), None, None).is_none());
        assert!(
            chat_message(None, Some("x".repeat(MAX_CHAT_TEXT_CHARS + 1)), None, None).is_none()
        );
    }

    #[tokio::test]
    async fn recorder_chat_is_sent_to_local_viewers() {
        let hub = Arc::new(StreamHub::default());
        *hub.mode.lock().unwrap() = ModeState {
            active: true,
            mode: "local".into(),
            ..ModeState::default()
        };
        let (_viewer_id, mut viewer_rx) = hub.register(false);

        hub.send_chat("**ответ**".into(), true, None)
            .await
            .unwrap();

        match viewer_rx.recv().await.unwrap() {
            ClientEvent::Chat(msg) => {
                assert_eq!(msg.from, "транслятор");
                assert_eq!(msg.text, "**ответ**");
                assert!(msg.markdown);
            }
            other => panic!("expected chat, got {other:?}"),
        }
        assert_eq!(hub.chat_history().len(), 1);
    }

    #[tokio::test]
    async fn recorder_chat_is_queued_for_relay_connection() {
        let hub = Arc::new(StreamHub::default());
        *hub.mode.lock().unwrap() = ModeState {
            active: true,
            mode: "relay".into(),
            ..ModeState::default()
        };
        let (_relay_id, mut relay_rx) = hub.register(true);

        hub.send_chat("ответ через relay".into(), false, None)
            .await
            .unwrap();

        match relay_rx.recv().await.unwrap() {
            ClientEvent::Chat(msg) => {
                assert_eq!(msg.from, "транслятор");
                assert_eq!(msg.text, "ответ через relay");
            }
            other => panic!("expected chat, got {other:?}"),
        }
        assert_eq!(hub.chat_history().len(), 1);
    }

    #[test]
    fn event_to_message_binary_prefix() {
        let m = event_to_message(&ClientEvent::Video {
            node: 149,
            jpeg: vec![0xAA, 0xBB],
        });
        match m {
            Message::Binary(b) => {
                assert_eq!(&b[..4], &[149u8, 0, 0, 0], "u32LE node id");
                assert_eq!(&b[4..], &[0xAA, 0xBB]);
            }
            _ => panic!("expected binary"),
        }
    }

    #[test]
    fn jpeg_1080p_speed_nonblack() {
        // диагностика: скорость и цветность кодирования реального размера кадра
        let (w, h) = (1920usize, 1080usize);
        let stride = w * 4 + 64;
        let mut buf = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x * 4;
                let r = ((x as f32 / w as f32) * 255.0) as u8;
                let g = ((y as f32 / h as f32) * 255.0) as u8;
                buf[i] = r; // B канал (мнемонично не важно — проверяем только яркость)
                buf[i + 1] = g;
                buf[i + 2] = ((u16::from(r) + u16::from(g)) / 2) as u8;
                buf[i + 3] = 255;
            }
        }
        let t0 = std::time::Instant::now();
        let jpeg = encode_jpeg(&buf, w, h, stride, 75).expect("jpeg 1080p");
        let ms = t0.elapsed().as_millis();
        eprintln!("encode 1920x1080 -> {} bytes in {ms} ms", jpeg.len());
        let path = std::path::Path::new("/tmp/fex_diag.jpg");
        std::fs::write(path, &jpeg).expect("write");
    }

    #[test]
    fn local_addresses_octet_order() {
        // эндпоинты, которые реально попадают в локальную сеть
        if let Ok(list) = local_addresses() {
            for ip in list {
                let octets = ip.split('.').collect::<Vec<_>>();
                assert!(
                    ip.parse::<std::net::Ipv4Addr>().is_ok(),
                    "не привильный IP: {ip}"
                );
                assert_eq!(octets.len(), 4);
                assert!(!ip.starts_with("1.0.0."), "байты перевёрнуты? {ip}");
            }
        }
    }

    #[test]
    fn stream_frame_scales_down() {
        let config = StreamConfig {
            fps: 24,
            quality: 75,
            max_width: 1280,
        };
        let (w, h) = (1920usize, 1080usize);
        let stride = w * 4;
        let buf = vec![128u8; stride * h];
        let r = encode_stream_frame(&buf, w, h, stride, &config).expect("scaled jpeg");
        assert_eq!(
            r.0 as usize, 1280,
            "должно ужаться до 1280 по большей стороне"
        );
        assert!(r.1 > 0 && r.2.len() > 100);
        std::fs::write("/tmp/fex_scaled.jpg", &r.2).ok();
    }

    #[test]
    fn stream_frame_no_scale_small() {
        let config = StreamConfig::default();
        let (w, h) = (640usize, 360usize);
        let stride = w * 4;
        let buf = vec![64u8; stride * h];
        let r = encode_stream_frame(&buf, w, h, stride, &config).expect("jpeg");
        assert_eq!(r.0 as usize, 640);
        assert_eq!(r.1 as usize, 360);
    }

    #[tokio::test]
    async fn local_server_serves_viewer() {
        use std::time::Duration;

        let hub = Arc::new(StreamHub::default());
        let (port, stop) = spawn_server(hub.clone()).await;

        let url = format!("ws://127.0.0.1:{port}/ws");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws.send(Message::Text(
            json!({"type":"hello","role":"viewer"}).to_string().into(),
        ))
        .await
        .unwrap();

        // welcome
        let first = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match &first {
            Message::Text(t) => assert!(t.contains("welcome")),
            other => panic!("expected welcome, got {other:?}"),
        }

        // второй клиент-наблюдатель для проверки рассылки
        let (_vid, mut vrx) = hub.register(false);

        hub.push_video(7, 100, 50, vec![0xDE; 64]);

        // ws-viewer должен получить streams + бинарный кадр
        let mut got_bin = false;
        for _ in 0..4 {
            let m = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let Message::Binary(b) = &m {
                assert_eq!(&b[..4], &[7u8, 0, 0, 0]);
                got_bin = true;
                break;
            }
        }
        assert!(got_bin, "viewer должен получить бинарный кадр");

        // чат от ws-viewer доходит наблюдателю
        ws.send(Message::Text(
            json!({
                "type":"chat",
                "from":"Ana",
                "text":"**preved**",
                "markdown":true,
                "image":{"name":"photo.jpg","data":"/9j/2Q=="}
            })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

        let mut got_chat = false;
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(2), vrx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                ClientEvent::Chat(m) => {
                    assert_eq!(m.from, "Ana");
                    assert_eq!(m.text, "**preved**");
                    assert!(m.markdown);
                    assert_eq!(m.image.as_ref().unwrap().name, "photo.jpg");
                    assert_eq!(m.image.as_ref().unwrap().data, "/9j/2Q==");
                    got_chat = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(got_chat, "наблюдатель должен получить чат");

        ws.send(Message::Close(None)).await.unwrap();
        stop.store(true, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn viewer_receives_real_jpeg() {
        use std::time::Duration;

        let hub = Arc::new(StreamHub::default());
        let (port, stop) = spawn_server(hub.clone()).await;

        let url = format!("ws://127.0.0.1:{port}/ws");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws.send(Message::Text(
            json!({"type":"hello","role":"viewer"}).to_string().into(),
        ))
        .await
        .unwrap();

        let welcome = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(welcome, Message::Text(t) if t.contains("welcome")));

        // точно как writer_loop: фрейм BGRA -> encode_jpeg -> hub.push_video
        let (w, h) = (640usize, 360usize);
        let stride = w * 4;
        let mut bg = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x * 4;
                let r = ((x as f32 / w as f32) * 255.0) as u8;
                let g = ((y as f32 / h as f32) * 255.0) as u8;
                bg[i] = r;
                bg[i + 1] = g;
                bg[i + 2] = 0;
                bg[i + 3] = 255;
            }
        }
        let jpeg = encode_jpeg(&bg, w, h, stride, 75).expect("jpeg");
        hub.push_video(3, w as u32, h as u32, jpeg);

        // viewer должен получить streams + бинарный кадр с настоящим jpeg
        let mut got_bin = false;
        for _ in 0..4 {
            let m = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let Message::Binary(b) = &m {
                assert_eq!(&b[..4], &[3u8, 0, 0, 0]);
                assert!(b.len() > 4);
                std::fs::write("/tmp/fex_viewer_frame.jpg", &b[4..]).ok();
                got_bin = true;
                break;
            }
        }
        assert!(got_bin, "viewer должен получить бинарный кадр");
        stop.store(true, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn ping_relay_ok() {
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");

        let fake = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut sink, mut rstream) = ws.split();
            // ждём hello-зрителя
            loop {
                match rstream.next().await {
                    Some(Ok(Message::Text(t))) if t.contains("viewer") => break,
                    _ => continue,
                }
            }
            // отвечаем welcome
            let _ = sink
                .send(Message::Text(
                    json!({"type":"welcome","streams":[],"history":[]})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = tokio::time::timeout(Duration::from_secs(2), rstream.next()).await;
        });

        let res = ping_relay(url).await;
        assert!(res.is_ok(), "ping должен пройти: {res:?}");
        assert!(res.unwrap().contains("relay доступен"));
        fake.abort();
    }

    #[tokio::test]
    async fn ping_relay_connection_refused() {
        // никто не слушает порт — должны получить понятную ошибку
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let res = ping_relay(format!("ws://{addr}")).await;
        assert!(
            res.is_err(),
            "ping к закрытому порту должен падать: {res:?}"
        );
    }

    #[tokio::test]
    async fn ping_relay_rejects_bad_url() {
        let res = ping_relay("не-адрес".into()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn relay_client_pushes_and_receives_chat() {
        let hub = Arc::new(StreamHub::default());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");

        let (hello_tx, hello_rx) = tokio::sync::oneshot::channel();
        let (got_bin_tx, mut got_bin_rx) = tokio::sync::mpsc::channel(4);

        let fake = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut sink, mut rstream) = ws.split();
            // ждём hello записывающего
            loop {
                match rstream.next().await {
                    Some(Ok(Message::Text(t))) if t.contains("recorder") => {
                        let _ = hello_tx.send(());
                        break;
                    }
                    _ => continue,
                }
            }
            // сервер выдаёт комнату и инжектит чат в сторону записывающего
            let _ = sink
                .send(Message::Text(
                    json!({"type":"room","room":"ROOM42"}).to_string().into(),
                ))
                .await;
            let _ = sink
                .send(Message::Text(
                    json!({"type":"chat","from":"Prof","text":"oi"})
                        .to_string()
                        .into(),
                ))
                .await;
            // слушаем бинарные кадры от записывающего
            while let Some(msg) = rstream.next().await {
                if matches!(msg, Ok(Message::Binary(_))) {
                    let _ = got_bin_tx.send(()).await;
                }
            }
        });

        // зритель на том же хабе, чтобы наблюдать чат
        let (_vid, mut vrx) = hub.register(false);

        let task = tokio::spawn(relay_client(hub.clone(), url));

        hello_rx.await.expect("recorder hello");

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(hub.status().room.as_deref(), Some("ROOM42"));

        hub.push_video(9, 320, 240, vec![0xAB; 128]);

        tokio::time::timeout(Duration::from_secs(2), got_bin_rx.recv())
            .await
            .expect("relay должен получить бинарный кадр")
            .expect("bin signal");

        let mut got_chat = false;
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(2), vrx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                ClientEvent::Chat(m) => {
                    assert_eq!(m.from, "Prof");
                    assert_eq!(m.text, "oi");
                    got_chat = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(got_chat, "записывающий должен получить чат от relay");

        task.abort();
        fake.abort();
    }
}
