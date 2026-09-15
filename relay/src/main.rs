#[tokio::main]
async fn main() {
    let port = std::env::var("RELAY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fuck_exam_relay::DEFAULT_PORT);
    let addr = format!("0.0.0.0:{port}");
    println!("FuckExam relay listening on {addr}");
    println!("recorder: ws://<ip>:{port} (relay-режим в приложении)");
    println!("viewer:   ws://<ip>:{port} с комнатой");
    if let Err(e) = fuck_exam_relay::serve(&addr).await {
        eprintln!("relay error: {e}");
        std::process::exit(1);
    }
}