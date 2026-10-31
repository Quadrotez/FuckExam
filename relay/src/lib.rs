use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const MAX_HISTORY: usize = 100;
const MAX_HISTORY_BYTES: usize = 2 * 1024 * 1024;
const MAX_CHAT_TEXT_CHARS: usize = 4000;
const MAX_CHAT_IMAGE_BYTES: usize = 512 * 1024;

/// Сколько живёт комната, из которой ушли все участники.
const DEFAULT_EMPTY_ROOM_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Жёсткий предел жизни любой комнаты, даже с активными участниками.
const DEFAULT_MAX_ROOM_AGE: Duration = Duration::from_secs(24 * 60 * 60);

const ROOM_CODE_LEN: usize = 6;
/// Без похожих символов (0/O, 1/I), чтобы код можно было продиктовать голосом.
const ROOM_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MAX_ROOMS: usize = 1024;
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
const PING_ROOM: &str = "__ping__";

pub const DEFAULT_PORT: u16 = 7444;

#[derive(Clone, Deserialize)]
struct Hello {
    #[serde(rename = "type")]
    _kind: String,
    role: String,
    #[serde(default)]
    room: Option<String>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
struct ChatImage {
    name: String,
    data: String,
}

#[derive(Clone)]
struct StreamEntry {
    id: u32,
    w: u32,
    h: u32,
}

#[derive(Clone, serde::Serialize)]
struct ChatMsg {
    from: String,
    text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    markdown: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<ChatImage>,
}

#[derive(Deserialize)]
struct IncomingChat {
    from: Option<String>,
    text: Option<String>,
    markdown: Option<bool>,
    image: Option<ChatImage>,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn validate_chat(
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
                    .map_or(0, |image| image.name.len() + image.data.len())
                + 128
        })
        .sum()
}

fn trim_chat_history(history: &mut Vec<ChatMsg>) {
    while history.len() > MAX_HISTORY || chat_history_size(history) > MAX_HISTORY_BYTES {
        history.remove(0);
    }
}

/// Код комнаты не чувствителен к регистру и лишним пробелам.
fn normalize_room(code: &str) -> String {
    code.trim().to_ascii_uppercase()
}

struct Room {
    created: Instant,
    empty_since: Option<Instant>,
    recorder: Option<mpsc::Sender<Message>>,
    viewers: Vec<mpsc::Sender<Message>>,
    streams: Vec<StreamEntry>,
    history: Vec<ChatMsg>,
}

impl Room {
    fn new() -> Self {
        Self {
            created: Instant::now(),
            empty_since: None,
            recorder: None,
            viewers: Vec::new(),
            streams: Vec::new(),
            history: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.recorder.is_none() && self.viewers.is_empty()
    }

    fn touch(&mut self) {
        self.empty_since = None;
    }

    fn broadcast_viewers(&self, msg: &Message) {
        for v in &self.viewers {
            let _ = v.try_send(msg.clone());
        }
    }

    fn broadcast_to(&self, msg: &Message, include_recorder: bool, include_viewers: bool) {
        if include_viewers {
            self.broadcast_viewers(msg);
        }
        if include_recorder {
            if let Some(r) = &self.recorder {
                let _ = r.try_send(msg.clone());
            }
        }
    }
}

fn broadcast_chat(room: &Arc<Mutex<Room>>, value: serde_json::Value, include_recorder: bool) {
    let Ok(incoming) = serde_json::from_value::<IncomingChat>(value) else {
        return;
    };
    let Some(chat) = validate_chat(incoming.from, incoming.text, incoming.markdown, incoming.image)
    else {
        return;
    };
    let mut payload = serde_json::to_value(&chat).unwrap_or_default();
    if let Some(object) = payload.as_object_mut() {
        object.insert("type".into(), json!("chat"));
    }
    let mut room = room.lock().unwrap();
    room.history.push(chat);
    trim_chat_history(&mut room.history);
    room.broadcast_to(
        &Message::Text(payload.to_string().into()),
        include_recorder,
        true,
    );
}

pub struct Relay {
    rooms: Mutex<HashMap<String, Arc<Mutex<Room>>>>,
    counter: AtomicU64,
    empty_ttl: Duration,
    max_age: Duration,
}

impl Default for Relay {
    fn default() -> Self {
        Self::new()
    }
}

impl Relay {
    pub fn new() -> Self {
        Self::with_ttls(DEFAULT_EMPTY_ROOM_TTL, DEFAULT_MAX_ROOM_AGE)
    }

    /// Отдельные TTL нужны тестам и, при желании, окружению.
    pub fn with_ttls(empty_ttl: Duration, max_age: Duration) -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            empty_ttl,
            max_age,
        }
    }

    pub fn room_count(&self) -> usize {
        self.rooms.lock().unwrap().len()
    }

    fn make_code(&self) -> String {
        let ticks = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or_default();
        let mut x = ticks
            ^ (u64::from(std::process::id())).rotate_left(32)
            ^ self.counter.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut code = String::with_capacity(ROOM_CODE_LEN);
        for _ in 0..ROOM_CODE_LEN {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            code.push(ROOM_ALPHABET[(x % ROOM_ALPHABET.len() as u64) as usize] as char);
        }
        code
    }

    /// Комнату создаёт только сервер. Клиент не может придумать код.
    fn create_room(&self) -> Result<String, String> {
        if self.room_count() >= MAX_ROOMS {
            self.prune_expired();
            if self.room_count() >= MAX_ROOMS {
                return Err("слишком много активных комнат, попробуйте позже".into());
            }
        }
        let mut rooms = self.rooms.lock().unwrap();
        for _ in 0..64 {
            let code = self.make_code();
            if !rooms.contains_key(&code) {
                rooms.insert(code.clone(), Arc::new(Mutex::new(Room::new())));
                return Ok(code);
            }
        }
        Err("не удалось сгенерировать код комнаты".into())
    }

    fn join_room(&self, code: &str) -> Option<Arc<Mutex<Room>>> {
        self.rooms.lock().unwrap().get(code).cloned()
    }

    /// Убирает просроченные комнаты: пустые — по TTL простоя, любые — по жёсткому пределу.
    pub fn prune_expired(&self) {
        let now = Instant::now();
        let mut rooms = self.rooms.lock().unwrap();

        let hard: Vec<(String, Arc<Mutex<Room>>)> = rooms
            .iter()
            .filter(|(_, room)| {
                now.saturating_duration_since(room.lock().unwrap().created) >= self.max_age
            })
            .map(|(code, room)| (code.clone(), room.clone()))
            .collect();

        for (code, room) in &hard {
            {
                let mut r = room.lock().unwrap();
                r.broadcast_to(
                    &Message::Text(
                        json!({"type":"error","error":"комната истекла — создайте новую"})
                            .to_string()
                            .into(),
                    ),
                    true,
                    true,
                );
                r.viewers.clear();
                r.recorder = None;
                r.streams.clear();
            }
            println!("room '{code}' expired: max age reached");
        }

        rooms.retain(|code, room| {
            if now.saturating_duration_since(room.lock().unwrap().created) >= self.max_age {
                return false;
            }
            let r = room.lock().unwrap();
            let idle = r
                .empty_since
                .is_some_and(|since| now.saturating_duration_since(since) >= self.empty_ttl);
            let keep = !(r.is_empty() && idle);
            if !keep {
                println!("room '{code}' expired: empty for too long");
            }
            keep
        });
    }
}

pub async fn serve(addr: &str) -> Result<(), String> {
    serve_with(addr, Arc::new(Relay::new())).await
}

pub async fn serve_with(addr: &str, relay: Arc<Relay>) -> Result<(), String> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;

    {
        let relay = relay.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            loop {
                ticker.tick().await;
                relay.prune_expired();
            }
        });
    }

    loop {
        let (stream, _peer) = listener.accept().await.map_err(|e| e.to_string())?;
        let relay = relay.clone();
        tokio::spawn(async move {
            let _ = handle_conn(relay, stream).await;
        });
    }
}

async fn reject(
    sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    error: &str,
) -> Result<(), String> {
    let _ = sink
        .send(Message::Text(
            json!({ "type": "error", "error": error })
                .to_string()
                .into(),
        ))
        .await;
    let _ = sink.send(Message::Close(None)).await;
    Ok(())
}

pub async fn handle_conn(relay: Arc<Relay>, stream: TcpStream) -> Result<(), String> {
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| format!("accept: {e}"))?;
    let (mut sink, mut rstream) = ws.split();
    let (tx, mut rx) = mpsc::channel(16);

    let hello = match rstream.next().await {
        Some(Ok(Message::Text(t))) => serde_json::from_str::<Hello>(&t).map_err(|_| "bad hello")?,
        _ => return Err("no hello".into()),
    };

    let role = hello.role.as_str();
    let is_recorder = role == "recorder";
    if !is_recorder && role != "viewer" {
        return reject(&mut sink, "неизвестная роль").await;
    }

    // Пинг доступности relay: отвечаем welcome и закрываем, комнату не создаём.
    let is_ping = hello
        .room
        .as_deref()
        .map(|room| room.trim().eq_ignore_ascii_case(PING_ROOM))
        .unwrap_or(false);
    if !is_recorder && is_ping {
        let _ = sink
            .send(Message::Text(
                json!({ "type": "welcome", "streams": [], "history": [] })
                    .to_string()
                    .into(),
            ))
            .await;
        return Ok(());
    }

    let (code, room) = if is_recorder {
        let code = match relay.create_room() {
            Ok(code) => code,
            Err(e) => return reject(&mut sink, &e).await,
        };
        let room = relay.join_room(&code).expect("freshly created room");
        {
            let mut r = room.lock().unwrap();
            r.recorder = Some(tx.clone());
            r.touch();
        }
        if sink
            .send(Message::Text(
                json!({ "type": "room", "room": code }).to_string().into(),
            ))
            .await
            .is_err()
        {
            let mut r = room.lock().unwrap();
            r.recorder = None;
            r.empty_since = Some(Instant::now());
            return Ok(());
        }
        println!("recorder created room '{code}'");
        (code, room)
    } else {
        let requested = hello
            .room
            .as_deref()
            .map(normalize_room)
            .filter(|r| !r.is_empty());
        let Some(requested) = requested else {
            return reject(&mut sink, "комната не указана").await;
        };
        let Some(room) = relay.join_room(&requested) else {
            return reject(&mut sink, "комната не найдена").await;
        };
        let (streams, history) = {
            let mut r = room.lock().unwrap();
            r.viewers.push(tx.clone());
            r.touch();
            let streams = r
                .streams
                .iter()
                .map(|s| json!({"id": s.id, "w": s.w, "h": s.h}))
                .collect::<Vec<_>>();
            let history = r
                .history
                .iter()
                .map(|m| {
                    json!({
                        "from": m.from,
                        "text": m.text,
                        "markdown": m.markdown,
                        "image": m.image
                    })
                })
                .collect::<Vec<_>>();
            (streams, history)
        };

        if sink
            .send(Message::Text(
                json!({ "type": "welcome", "room": requested, "streams": streams, "history": history })
                    .to_string()
                    .into(),
            ))
            .await
            .is_err()
        {
            let mut r = room.lock().unwrap();
            r.viewers.retain(|v| !v.same_channel(&tx));
            r.empty_since = if r.is_empty() { Some(Instant::now()) } else { None };
            return Ok(());
        }
        println!("viewer joined room '{requested}'");
        (requested, room)
    };

    loop {
        tokio::select! {
            out = rx.recv() => {
                match out {
                    Some(msg) => {
                        if sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            inc = rstream.next() => {
                match inc {
                    Some(Ok(Message::Text(t))) => {
                        let parsed: Option<serde_json::Value> = serde_json::from_str(&t).ok();
                        let Some(v) = parsed else { continue; };
                        let Some(kind) = v.get("type").and_then(|x| x.as_str()) else {
                            continue;
                        };
                        if is_recorder {
                            match kind {
                                "streams" => {
                                    let mut r = room.lock().unwrap();
                                    r.streams = v.get("streams")
                                        .and_then(|x| x.as_array())
                                        .map(|arr| {
                                            arr.iter().filter_map(|s| {
                                                Some(StreamEntry {
                                                    id: s.get("id")?.as_u64()? as u32,
                                                    w: s.get("w")?.as_u64()? as u32,
                                                    h: s.get("h")?.as_u64()? as u32,
                                                })
                                            }).collect()
                                        })
                                        .unwrap_or_default();
                                    r.broadcast_to(&Message::Text(t.clone().into()), false, true);
                                }
                                "stream-end" => {
                                    let id = v.get("id").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                                    let mut r = room.lock().unwrap();
                                    r.streams.retain(|s| s.id != id);
                                    r.broadcast_to(&Message::Text(t.clone().into()), false, true);
                                }
                                "chat" => broadcast_chat(&room, v, false),
                                _ => {}
                            }
                        } else if kind == "chat" {
                            broadcast_chat(&room, v, true);
                        }
                    }
                    Some(Ok(Message::Binary(b))) => {
                        if !is_recorder {
                            continue;
                        }
                        let msg = Message::Binary(b);
                        room.lock().unwrap().broadcast_viewers(&msg);
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }

    {
        let mut r = room.lock().unwrap();
        r.viewers.retain(|v| !v.same_channel(&tx));
        if is_recorder {
            if let Some(rtx) = &r.recorder {
                if rtx.same_channel(&tx) {
                    r.recorder = None;
                    r.streams.clear();
                    r.broadcast_to(
                        &Message::Text(json!({ "type": "streams", "streams": [] }).to_string().into()),
                        false,
                        true,
                    );
                }
            }
        }
        r.empty_since = if r.is_empty() { Some(Instant::now()) } else { None };
    }
    println!(
        "{} left room '{code}'",
        if is_recorder { "recorder" } else { "viewer" }
    );
    Ok(())
}
