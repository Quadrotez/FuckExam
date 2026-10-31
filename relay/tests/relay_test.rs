use std::{sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

struct TestRelay {
    url: String,
    relay: Arc<fuck_exam_relay::Relay>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn start_relay(relay: Arc<fuck_exam_relay::Relay>) -> TestRelay {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn({
        let relay = relay.clone();
        async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let relay = relay.clone();
                tokio::spawn(async move {
                    let _ = fuck_exam_relay::handle_conn(relay, stream).await;
                });
            }
        }
    });
    TestRelay {
        url: format!("ws://127.0.0.1:{port}"),
        relay,
        server,
    }
}

async fn connect(url: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.expect("connect");
    ws
}

async fn next_json(ws: &mut Ws) -> serde_json::Value {
    let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timeout")
        .expect("stream closed")
        .expect("ws error");
    match msg {
        Message::Text(t) => serde_json::from_str(&t).expect("json"),
        other => panic!("expected text, got {other:?}"),
    }
}

async fn recorder_join(url: &str) -> (Ws, String) {
    let mut ws = connect(url).await;
    ws.send(Message::Text(
        r#"{"type":"hello","role":"recorder"}"#.into(),
    ))
    .await
    .unwrap();
    let msg = next_json(&mut ws).await;
    assert_eq!(msg["type"], "room");
    let room = msg["room"].as_str().expect("room code").to_string();
    (ws, room)
}

async fn viewer_join(url: &str, room: &str) -> Ws {
    let mut ws = connect(url).await;
    ws.send(Message::Text(
        format!(r#"{{"type":"hello","role":"viewer","room":"{room}"}}"#).into(),
    ))
    .await
    .unwrap();
    ws
}

#[tokio::test]
async fn recorder_gets_server_generated_room_and_viewer_joins() {
    let relay = Arc::new(fuck_exam_relay::Relay::new());
    let t = start_relay(relay).await;

    let (mut recorder, room) = recorder_join(&t.url).await;
    assert_eq!(room.len(), 6);
    assert!(room.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
    assert!(!room.contains(['O', 'I', '0', '1']), "код не должен путаться: {room}");

    // Зритель вводит код в любом регистре.
    let mut viewer = viewer_join(&t.url, &room.to_lowercase()).await;
    let welcome = next_json(&mut viewer).await;
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["room"], room);

    recorder
        .send(Message::Text(
            r#"{"type":"streams","streams":[{"id":149,"w":640,"h":480}]}"#.into(),
        ))
        .await
        .unwrap();

    let bin = vec![0xA7u8, 0x01, 0, 0, 1, 2, 3];
    recorder.send(Message::Binary(bin.clone().into())).await.unwrap();

    for _ in 0..2 {
        match tokio::time::timeout(Duration::from_secs(2), viewer.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(t) => assert!(t.contains("\"type\":\"streams\"")),
            Message::Binary(b) => assert_eq!(&b[..], bin.as_slice()),
            other => panic!("unexpected: {other:?}"),
        }
    }

    viewer
        .send(Message::Text(
            r#"{"type":"chat","from":"Профессор","text":"привет"}"#.into(),
        ))
        .await
        .unwrap();

    let got = next_json(&mut recorder).await;
    assert_eq!(got["type"], "chat");
    assert_eq!(got["text"], "привет");
    assert_eq!(got["from"], "Профессор");

    recorder
        .send(Message::Text(r#"{"type":"stream-end","id":149}"#.into()))
        .await
        .unwrap();
    let mut end_found = false;
    for _ in 0..4 {
        let got = next_json(&mut viewer).await;
        if got["type"] == "stream-end" {
            end_found = true;
            break;
        }
    }
    assert!(end_found, "viewer должен получить stream-end");

    recorder.send(Message::Close(None)).await.unwrap();
    let ended = next_json(&mut viewer).await;
    assert_eq!(ended["type"], "streams");
    assert!(ended["streams"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn ping_room_is_not_a_real_room() {
    let relay = Arc::new(fuck_exam_relay::Relay::new());
    let t = start_relay(relay.clone()).await;

    let mut viewer = viewer_join(&t.url, "__ping__").await;
    let msg = next_json(&mut viewer).await;
    assert_eq!(msg["type"], "welcome");
    assert!(msg["streams"].as_array().unwrap().is_empty());
    assert_eq!(relay.room_count(), 0, "пинг не должен создавать комнату");
}

#[tokio::test]
async fn viewer_cannot_invent_room() {
    let relay = Arc::new(fuck_exam_relay::Relay::new());
    let t = start_relay(relay.clone()).await;

    let mut viewer = viewer_join(&t.url, "ABCDEF").await;
    let msg = next_json(&mut viewer).await;
    assert_eq!(msg["type"], "error");
    assert_eq!(msg["error"], "комната не найдена");
    assert_eq!(relay.room_count(), 0, "комната не должна создаваться зрителем");

    // Пустая комната тоже не создаётся.
    let mut viewer = viewer_join(&t.url, "").await;
    let msg = next_json(&mut viewer).await;
    assert_eq!(msg["type"], "error");
    assert_eq!(relay.room_count(), 0);
}

#[tokio::test]
async fn photo_and_markdown_chat_are_broadcast_and_kept_in_history() {
    let relay = Arc::new(fuck_exam_relay::Relay::new());
    let t = start_relay(relay).await;

    let (mut recorder, room) = recorder_join(&t.url).await;
    let mut viewer = viewer_join(&t.url, &room).await;
    let welcome = next_json(&mut viewer).await;
    assert_eq!(welcome["type"], "welcome");

    viewer
        .send(Message::Text(
            r#"{"type":"chat","from":"Ana","text":"**важно**","markdown":true,"image":{"name":"photo.jpg","data":"/9j/2Q=="}}"#.into(),
        ))
        .await
        .unwrap();
    let payload = next_json(&mut recorder).await;
    assert_eq!(payload["type"], "chat");
    assert_eq!(payload["markdown"], true);
    assert_eq!(payload["image"]["name"], "photo.jpg");
    assert_eq!(payload["image"]["data"], "/9j/2Q==");

    // Зритель получает эхо своего сообщения.
    let echo = next_json(&mut viewer).await;
    assert_eq!(echo["markdown"], true);
    assert_eq!(echo["image"]["data"], "/9j/2Q==");

    recorder
        .send(Message::Text(
            r#"{"type":"chat","from":"Транслятор","text":"**ответ**","markdown":true,"image":{"name":"reply.jpg","data":"/9j/2Q=="}}"#.into(),
        ))
        .await
        .unwrap();
    let reply = next_json(&mut viewer).await;
    assert_eq!(reply["type"], "chat");
    assert_eq!(reply["from"], "Транслятор");
    assert_eq!(reply["markdown"], true);
    assert_eq!(reply["image"]["name"], "reply.jpg");

    // Переподключение получает историю.
    let mut reconnect = viewer_join(&t.url, &room).await;
    let welcome = next_json(&mut reconnect).await;
    assert_eq!(welcome["history"][0]["markdown"], true);
    assert_eq!(welcome["history"][0]["image"]["name"], "photo.jpg");
    assert_eq!(welcome["history"][1]["from"], "Транслятор");
    assert_eq!(welcome["history"][1]["image"]["name"], "reply.jpg");
}

#[tokio::test]
async fn two_rooms_isolated() {
    let relay = Arc::new(fuck_exam_relay::Relay::new());
    let t = start_relay(relay).await;

    let (_rec_a, room_a) = recorder_join(&t.url).await;
    let (_rec_b, room_b) = recorder_join(&t.url).await;
    assert_ne!(room_a, room_b);

    let mut v1 = viewer_join(&t.url, &room_a).await;
    assert_eq!(next_json(&mut v1).await["type"], "welcome");
    let mut v2 = viewer_join(&t.url, &room_b).await;
    assert_eq!(next_json(&mut v2).await["type"], "welcome");

    v1.send(Message::Text(r#"{"type":"chat","from":"a","text":"xA"}"#.into()))
        .await
        .unwrap();

    let res = tokio::time::timeout(Duration::from_millis(300), v2.next()).await;
    assert!(res.is_err(), "v2 не должен получать чат из roomA");
}

#[tokio::test]
async fn empty_room_expires_and_busy_room_gets_hard_limit() {
    let relay = Arc::new(fuck_exam_relay::Relay::with_ttls(
        Duration::ZERO,
        Duration::from_secs(3600),
    ));
    let t = start_relay(relay.clone()).await;

    let (mut recorder, _room) = recorder_join(&t.url).await;
    assert_eq!(relay.room_count(), 1);
    relay.prune_expired();
    assert_eq!(relay.room_count(), 1, "комната с записывающим не удаляется по простою");

    recorder.send(Message::Close(None)).await.unwrap();
    // Даём серверу обработать закрытие соединения.
    tokio::time::sleep(Duration::from_millis(100)).await;
    relay.prune_expired();
    assert_eq!(relay.room_count(), 0, "пустая комната удаляется");

    // Жёсткий предел: комната истекает, даже если зритель на связи.
    let hard = Arc::new(fuck_exam_relay::Relay::with_ttls(
        Duration::from_secs(3600),
        Duration::ZERO,
    ));
    let t2 = start_relay(hard.clone()).await;
    let (_rec, room) = recorder_join(&t2.url).await;
    let mut viewer = viewer_join(&t2.url, &room).await;
    assert_eq!(next_json(&mut viewer).await["type"], "welcome");

    hard.prune_expired();
    let msg = next_json(&mut viewer).await;
    assert_eq!(msg["type"], "error");
    assert_eq!(hard.room_count(), 0);
}
