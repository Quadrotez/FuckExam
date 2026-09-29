use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

async fn connect(url: &str) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.expect("connect");
    ws
}

#[tokio::test]
async fn relay_viewer_and_recorder() {
    let addr = "127.0.0.1:0";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // reuse listener: build server task from it
    let relay = std::sync::Arc::new(fuck_exam_relay::Relay::new());
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let relay = relay.clone();
            tokio::spawn(async move {
                let _ = fuck_exam_relay::handle_conn(relay, stream).await;
            });
        }
    });

    let url = format!("ws://127.0.0.1:{port}");

    let mut viewer = connect(&url).await;
    viewer
        .send(Message::Text(
            r#"{"type":"hello","role":"viewer","room":"abc123"}"#.into(),
        ))
        .await
        .unwrap();

    // welcome (empty, recorder ещё нет)
    match tokio::time::timeout(Duration::from_secs(2), viewer.next()).await.unwrap().unwrap() {
        Ok(Message::Text(t)) => assert!(t.contains("\"type\":\"welcome\"")),
        _ => panic!("expected welcome"),
    }

    // recorder подключается и шлёт streams + бинарный кадр
    let mut recorder = connect(&url).await;
    recorder
        .send(Message::Text(
            r#"{"type":"hello","role":"recorder","room":"abc123"}"#.into(),
        ))
        .await
        .unwrap();

    recorder
        .send(Message::Text(
            r#"{"type":"streams","streams":[{"id":149,"w":640,"h":480}]}"#.into(),
        ))
        .await
        .unwrap();

    let bin = vec![0xA7u8, 0x01, 0, 0, 1, 2, 3];
    recorder.send(Message::Binary(bin.clone().into())).await.unwrap();

    // viewer должен получить streams и бинарный кадр
    for _ in 0..2 {
        match tokio::time::timeout(Duration::from_secs(2), viewer.next())
            .await
            .unwrap()
            .unwrap()
        {
            Ok(Message::Text(t)) => assert!(t.contains("\"type\":\"streams\"")),
            Ok(Message::Binary(b)) => assert_eq!(&b[..], bin.as_slice()),
            other => panic!("unexpected: {other:?}"),
        }
    }

    // чат зрителя доходит до записывающего (у него в app.chat будет эвент, а тут — текстовое сообщение)
    viewer
        .send(Message::Text(
            r#"{"type":"chat","from":"Профессор","text":"привет"}"#.into(),
        ))
        .await
        .unwrap();

    let got = tokio::time::timeout(Duration::from_secs(2), recorder.next())
        .await
        .unwrap()
        .unwrap();
    match got {
        Ok(Message::Text(t)) => assert!(t.contains("\"text\":\"привет\"") && t.contains("\"from\":\"Профессор\"")),
        other => panic!("unexpected recorder msg: {other:?}"),
    }

    // stream-end доходит до зрителя (пропускаем эхо своего чата)
    recorder
        .send(Message::Text(r#"{"type":"stream-end","id":149}"#.into()))
        .await
        .unwrap();
    let mut end_found = false;
    for _ in 0..4 {
        let got = tokio::time::timeout(Duration::from_secs(2), viewer.next())
            .await
            .unwrap()
            .unwrap();
        match got {
            Ok(Message::Text(t)) => {
                if t.contains("\"stream-end\"") {
                    end_found = true;
                    break;
                }
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
    assert!(end_found, "viewer должен получить stream-end");

    viewer.send(Message::Close(None)).await.unwrap();
    recorder.send(Message::Close(None)).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), viewer.next()).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), recorder.next()).await;
    server.abort();
}

#[tokio::test]
async fn photo_and_markdown_chat_are_broadcast_and_kept_in_history() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let relay = std::sync::Arc::new(fuck_exam_relay::Relay::new());
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let relay = relay.clone();
            tokio::spawn(async move {
                let _ = fuck_exam_relay::handle_conn(relay, stream).await;
            });
        }
    });
    let url = format!("ws://127.0.0.1:{port}");

    let mut recorder = connect(&url).await;
    recorder.send(Message::Text(r#"{"type":"hello","role":"recorder","room":"photo-room"}"#.into())).await.unwrap();
    let mut viewer = connect(&url).await;
    viewer.send(Message::Text(r#"{"type":"hello","role":"viewer","room":"photo-room"}"#.into())).await.unwrap();
    let welcome = tokio::time::timeout(Duration::from_secs(2), viewer.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(welcome, Message::Text(t) if t.contains("welcome")));

    viewer.send(Message::Text(
        r#"{"type":"chat","from":"Ana","text":"**важно**","markdown":true,"image":{"name":"photo.jpg","data":"/9j/2Q=="}}"#.into(),
    )).await.unwrap();
    let forwarded = tokio::time::timeout(Duration::from_secs(2), recorder.next()).await.unwrap().unwrap().unwrap();
    let payload: serde_json::Value = match forwarded {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected forwarded chat, got {other:?}"),
    };
    assert_eq!(payload["type"], "chat");
    assert_eq!(payload["markdown"], true);
    assert_eq!(payload["image"]["name"], "photo.jpg");
    assert_eq!(payload["image"]["data"], "/9j/2Q==");

    // viewer receives its own message, with markdown and photo preserved
    let viewer_echo = tokio::time::timeout(Duration::from_secs(2), viewer.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let viewer_payload: serde_json::Value = match viewer_echo {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected viewer chat echo, got {other:?}"),
    };
    assert_eq!(viewer_payload["markdown"], true);
    assert_eq!(viewer_payload["image"]["data"], "/9j/2Q==");

    // recorder replies back into the room; it must reach the viewer, preserving both fields.
    recorder.send(Message::Text(
        r#"{"type":"chat","from":"Транслятор","text":"**ответ**","markdown":true,"image":{"name":"reply.jpg","data":"/9j/2Q=="}}"#.into(),
    )).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), viewer.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reply_payload: serde_json::Value = match reply {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected recorder reply, got {other:?}"),
    };
    assert_eq!(reply_payload["type"], "chat");
    assert_eq!(reply_payload["from"], "Транслятор");
    assert_eq!(reply_payload["markdown"], true);
    assert_eq!(reply_payload["image"]["name"], "reply.jpg");

    let mut reconnect = connect(&url).await;
    reconnect.send(Message::Text(r#"{"type":"hello","role":"viewer","room":"photo-room"}"#.into())).await.unwrap();
    let welcome = tokio::time::timeout(Duration::from_secs(2), reconnect.next()).await.unwrap().unwrap().unwrap();
    let history: serde_json::Value = match welcome {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected room history, got {other:?}"),
    };
    assert_eq!(history["history"][0]["markdown"], true);
    assert_eq!(history["history"][0]["image"]["name"], "photo.jpg");
    assert_eq!(history["history"][1]["from"], "Транслятор");
    assert_eq!(history["history"][1]["markdown"], true);
    assert_eq!(history["history"][1]["image"]["name"], "reply.jpg");

    let _ = viewer.send(Message::Close(None)).await;
    let _ = reconnect.send(Message::Close(None)).await;
    let _ = recorder.send(Message::Close(None)).await;
    server.abort();
}

#[tokio::test]
async fn two_rooms_isolated() {
    let addr = "127.0.0.1:0";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let relay = std::sync::Arc::new(fuck_exam_relay::Relay::new());
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let relay = relay.clone();
            tokio::spawn(async move {
                let _ = fuck_exam_relay::handle_conn(relay, stream).await;
            });
        }
    });

    let url = format!("ws://127.0.0.1:{port}");

    let mut v1 = connect(&url).await;
    v1.send(Message::Text(r#"{"type":"hello","role":"viewer","room":"roomA"}"#.into())).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), v1.next()).await.unwrap().unwrap();

    let mut v2 = connect(&url).await;
    v2.send(Message::Text(r#"{"type":"hello","role":"viewer","room":"roomB"}"#.into())).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), v2.next()).await.unwrap().unwrap();

    v1.send(Message::Text(r#"{"type":"chat","from":"a","text":"xA"}"#.into())).await.unwrap();

    // v2 живёт в другой комнате — ему ничего не должно прийти
    let res = tokio::time::timeout(Duration::from_millis(300), v2.next()).await;
    assert!(res.is_err(), "v2 не должен получать чат из roomA");

    v2.send(Message::Close(None)).await.unwrap();
    v1.send(Message::Close(None)).await.unwrap();
    server.abort();
}
