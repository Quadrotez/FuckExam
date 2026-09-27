use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
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

pub const DEFAULT_PORT: u16 = 7444;

#[derive(Clone, Deserialize)]
struct Hello {
    #[serde(rename = "type")]
    _kind: String,
    role: String,
    room: String,
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

struct Room {
    recorder: Option<mpsc::Sender<Message>>,
    viewers: Vec<mpsc::Sender<Message>>,
    streams: Vec<StreamEntry>,
    history: Vec<ChatMsg>,
}

impl Room {
    fn new() -> Self {
        Self {
            recorder: None,
            viewers: Vec::new(),
            streams: Vec::new(),
            history: Vec::new(),
        }
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

pub struct Relay {
    rooms: Mutex<HashMap<String, Arc<Mutex<Room>>>>,
}

impl Relay {
    pub fn new() -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
        }
    }

    fn get_room(&self, name: &str) -> Arc<Mutex<Room>> {
        let mut rooms = self.rooms.lock().unwrap();
        rooms
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(Room::new())))
            .clone()
    }
}

pub async fn serve(addr: &str) -> Result<(), String> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;
    let relay = Arc::new(Relay::new());
    loop {
        let (stream, _peer) = listener.accept().await.map_err(|e| e.to_string())?;
        let relay = relay.clone();
        tokio::spawn(async move {
            let _ = handle_conn(relay, stream).await;
        });
    }
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

    if hello.room.is_empty() {
        let _ = sink
            .send(Message::Text(
                json!({"type":"error","error":"room required"})
                    .to_string()
                    .into(),
            ))
            .await;
        return Err("no room".into());
    }

    let role = hello.role.as_str();
    let is_recorder = role == "recorder";
    if !is_recorder && role != "viewer" {
        let _ = sink
            .send(Message::Text(
                json!({"type":"error","error":"unknown role"})
                    .to_string()
                    .into(),
            ))
            .await;
        return Err("bad role".into());
    }

    let room = relay.get_room(&hello.room);

    if is_recorder {
        {
            let mut r = room.lock().unwrap();
            r.recorder = Some(tx.clone());
        }
        println!("recorder joined room '{}'", hello.room);
    } else {
        {
            let mut r = room.lock().unwrap();
            r.viewers.push(tx.clone());
        }
        let (streams, history) = {
            let r = room.lock().unwrap();
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
                json!({ "type": "welcome", "streams": streams, "history": history })
                    .to_string()
                    .into(),
            ))
            .await
            .is_err()
        {
            return Ok(());
        }
        println!("viewer joined room '{}'", hello.room);
    }

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
                                _ => {}
                            }
                        } else if kind == "chat" {
                            if let Ok(incoming) = serde_json::from_value::<IncomingChat>(v) {
                                if let Some(chat) = validate_chat(incoming.from, incoming.text, incoming.markdown, incoming.image) {
                                    let mut payload = serde_json::to_value(&chat).unwrap_or_default();
                                    if let Some(obj) = payload.as_object_mut() {
                                        obj.insert("type".into(), json!("chat"));
                                    }
                                    let mut r = room.lock().unwrap();
                                    r.history.push(chat);
                                    trim_chat_history(&mut r.history);
                                    r.broadcast_to(&Message::Text(payload.to_string().into()), true, true);
                                }
                            }
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
                }
            }
        }
    }
    println!(
        "{} left room '{}'",
        if is_recorder { "recorder" } else { "viewer" },
        hello.room
    );
    Ok(())
}
