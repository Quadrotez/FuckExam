use std::{
    collections::HashMap,
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite::Message;

const MAX_HISTORY: usize = 100;
const DEFAULT_PORT: u16 = 7335;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMsg {
    pub from: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamEntry {
    pub id: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug)]
pub enum ClientEvent {
    Welcome { streams: Vec<StreamEntry>, history: Vec<ChatMsg> },
    Streams(Vec<StreamEntry>),
    StreamEnd(u32),
    Chat(ChatMsg),
    Video { node: u32, jpeg: Vec<u8> },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    pub active: bool,
    pub mode: String,
    pub addresses: Vec<String>,
    pub relay: Option<String>,
    pub room: Option<String>,
    pub viewers: usize,
    pub streams: Vec<StreamEntry>,
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
}

#[derive(Default)]
struct ModeState {
    active: bool,
    mode: String,
    addresses: Vec<String>,
    relay: Option<String>,
    room: Option<String>,
}

pub struct StreamHub {
    inner: Mutex<HubInner>,
    app: Mutex<Option<AppHandle>>,
    mode: Mutex<ModeState>,
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
            }),
            app: Mutex::new(None),
            mode: Mutex::new(ModeState::default()),
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
        ClientEvent::Streams(list) => {
            Message::Text(json!({ "type": "streams", "streams": list }).to_string().into())
        }
        ClientEvent::StreamEnd(id) => {
            Message::Text(json!({ "type": "stream-end", "id": id }).to_string().into())
        }
        ClientEvent::Chat(m) => Message::Text(
            json!({ "type": "chat", "from": m.from, "text": m.text }).to_string().into(),
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
}

pub fn encode_jpeg(bgra: &[u8], w: usize, h: usize, stride: usize) -> Option<Vec<u8>> {
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
    let quality = std::env::var("FEX_JPEG_QUALITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(85);
    let mut out = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.encode(&rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
        .ok()?;
    Some(out)
}

pub fn local_addresses() -> Result<Vec<String>, String> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Err("getifaddrs failed".into());
    }
    let mut out = Vec::new();
    let mut cur = list;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        if !ifa.ifa_addr.is_null() {
            let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
            if family == libc::AF_INET {
                let sin = ifa.ifa_addr as *const libc::sockaddr_in;
                let addr = unsafe { &*sin }.sin_addr;
if addr.s_addr != 0 {
                // s_addr уже в сетевом порядке; порядок байтов в памяти = октеты IP
                let b = addr.s_addr.to_ne_bytes();
                let ip = format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]);
                    if !ip.starts_with("127.")
                        && !ip.starts_with("0.")
                        && !ip.starts_with("169.254.")
                    {
                        out.push(ip);
                    }
                }
            }
        }
        cur = ifa.ifa_next;
    }
    unsafe { libc::freeifaddrs(list) };
    Ok(out)
}

fn gen_room() -> String {
    use std::io::Read;
    let mut buf = [0u8; 4];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if ok {
        format!("{:08x}", u32::from_le_bytes(buf))
    } else {
        format!("{:08x}", std::process::id())
    }
}

impl Streaming {
    pub async fn arm_local(&self, app: AppHandle, port: Option<u16>) -> Result<StreamStatus, String> {
        let port = port.unwrap_or_else(|| {
            std::env::var("FEX_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_PORT)
        });
        if self.0.mode.lock().unwrap().active {
            return Ok(self.0.status());
        }
        let addr = format!("0.0.0.0:{port}");
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("bind {addr}: {e}"))?;
        *self.0.app.lock().unwrap() = Some(app);
        let addresses = match local_addresses() {
            Ok(mut ips) => {
                ips.sort();
                ips.iter().map(|ip| format!("ws://{ip}:{port}")).collect()
            }
            Err(_) => Vec::new(),
        };
        *self.0.mode.lock().unwrap() = ModeState {
            active: true,
            mode: "local".into(),
            addresses: addresses.clone(),
            relay: None,
            room: None,
        };
        self.0.stop.store(false, Ordering::Relaxed);
        let hub = self.0.clone();
        let stop = self.0.stop.clone();
        let task = tokio::spawn(serve_local(hub, listener, stop));
        *self.0.task.lock().unwrap() = Some(task);
        Ok(self.0.status())
    }

    pub async fn arm_relay(
        &self,
        app: AppHandle,
        url: String,
        room: Option<String>,
    ) -> Result<StreamStatus, String> {
        if self.0.mode.lock().unwrap().active {
            return Ok(self.0.status());
        }
        let room = room.unwrap_or_else(gen_room);
        *self.0.app.lock().unwrap() = Some(app);
        *self.0.mode.lock().unwrap() = ModeState {
            active: true,
            mode: "relay".into(),
            addresses: Vec::new(),
            relay: Some(url.clone()),
            room: Some(room.clone()),
        };
        self.0.stop.store(false, Ordering::Relaxed);
        let hub = self.0.clone();
        let task = tokio::spawn(relay_client(hub, url, room));
        *self.0.task.lock().unwrap() = Some(task);
        Ok(self.0.status())
    }
}

impl StreamHub {
    pub fn status(&self) -> StreamStatus {
        let (mode, addresses, relay, room, active) = {
            let m = self.mode.lock().unwrap();
            (
                m.mode.clone(),
                m.addresses.clone(),
                m.relay.clone(),
                m.room.clone(),
                m.active,
            )
        };
        let (streams, viewers) = {
            let i = self.inner.lock().unwrap();
            (i.streams.values().cloned().collect(), i.viewers)
        };
        StreamStatus {
            active,
            mode,
            addresses,
            relay,
            room,
            viewers,
            streams,
        }
    }

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
            let _ = c.tx.try_send(ev.clone());
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

    pub fn emit_chat(&self, msg: ChatMsg) {
        {
            let mut i = self.inner.lock().unwrap();
            i.history.push(msg.clone());
            if i.history.len() > MAX_HISTORY {
                let excess = i.history.len() - MAX_HISTORY;
                i.history.drain(0..excess);
            }
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

async fn serve_local(hub: Arc<StreamHub>, listener: TcpListener, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let (stream, _peer) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => continue,
        };
        let hub2 = hub.clone();
        tokio::spawn(async move {
            if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
                let _ = run_viewer(hub2, ws).await;
            }
        });
    }
}

async fn run_viewer(
    hub: Arc<StreamHub>,
    ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> Result<(), String> {
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
                    json!({"type":"error","error":"role must be viewer"}).to_string().into(),
                ))
                .await;
            return Err("bad hello".into());
        }
    }

    let (id, rx) = hub.register(false);
    let (streams, history) = {
        let i = hub.inner.lock().unwrap();
        (
            i.streams.values().cloned().collect::<Vec<_>>(),
            i.history.clone(),
        )
    };
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
                                let from = incoming.from.unwrap_or_else(|| "зритель".into());
                                if let Some(text) = incoming.text {
                                    hub.emit_chat(ChatMsg { from, text });
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

async fn relay_client(hub: Arc<StreamHub>, url: String, room: String) {
    let (conn, _resp) = match tokio_tungstenite::connect_async(&url).await {
        Ok(x) => x,
        Err(e) => {
            if let Some(app) = hub.app.lock().unwrap().clone() {
                let _ = app.emit("stream-error", format!("relay: {e}"));
            }
            hub.deactivate();
            return;
        }
    };
    let (mut sink, mut rstream) = conn.split();

    let hello = json!({"type":"hello","role":"recorder","room":room}).to_string();
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
                    if let Ok(incoming) = serde_json::from_str::<Incoming>(&t) {
                        if incoming.kind == "chat" {
                            let from = incoming.from.unwrap_or_else(|| "зритель".into());
                            if let Some(text) = incoming.text {
                                hub.emit_chat(ChatMsg { from, text });
                            }
                        }
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
        let jpeg = encode_jpeg(&buf, w, h, stride).expect("jpeg");
        assert!(jpeg.len() > 100, "jpeg should be non-trivial");
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "SOI marker");
        assert!(jpeg.ends_with(&[0xFF, 0xD9]), "EOI marker");
    }

    #[test]
    fn jpeg_bad_dims() {
        assert!(encode_jpeg(&[], 0, 10, 40).is_none());
    }

    #[test]
    fn room_is_hex() {
        let r = gen_room();
        assert_eq!(r.len(), 8);
        assert!(r.chars().all(|c| c.is_ascii_hexdigit()));
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
    fn local_addresses_octet_order() {
        // эндпоинты, которые реально попадают в локальную сеть
        if let Ok(list) = local_addresses() {
            for ip in list {
                let octets = ip.split('.').collect::<Vec<_>>();
                assert!(
                    ip.parse::<std::net::Ipv4Addr>().is_ok(),
                    "неправильный IP: {ip}"
                );
                assert_eq!(octets.len(), 4);
                assert!(!ip.starts_with("1.0.0."), "байты перевёрнуты? {ip}");
            }
        }
    }

    #[tokio::test]
    async fn local_server_serves_viewer() {
        use std::time::Duration;

        let hub = Arc::new(StreamHub::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        tokio::spawn(serve_local(hub.clone(), listener, stop.clone()));

        let url = format!("ws://127.0.0.1:{port}");
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
            json!({"type":"chat","from":"Ana","text":"preved"}).to_string().into(),
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
                    assert_eq!(m.text, "preved");
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
    async fn relay_client_pushes_and_receives_chat() {
        use std::time::Duration;

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
            // инжектим чат в сторону записывающего
            let _ = sink
                .send(Message::Text(
                    json!({"type":"chat","from":"Prof","text":"oi"}).to_string().into(),
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

        let task = tokio::spawn(relay_client(hub.clone(), url, String::from("room42")));

        hello_rx.await.expect("recorder hello");

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