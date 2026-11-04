use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::Duration,
};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

use crate::{
    settings::{Settings, SettingsStore},
    streaming::{self, ChatImage, StreamConfig, StreamStatus, StreamHub},
};

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
const SHIM_JS: &str = include_str!("../web/tauri-shim.js");
const DESKTOP_INDEX: &str = include_str!("../../src/index.html");
const DESKTOP_JS: &str = include_str!("../../src/main.js");
const DESKTOP_CSS: &str = include_str!("../../src/styles.css");
const MARKED_JS: &str = include_str!("../../src/vendor/marked.umd.js");
const PURIFY_JS: &str = include_str!("../../src/vendor/purify.min.js");

const MAX_HEAD_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 256 * 1024;

struct Running {
    port: u16,
    stop: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

/// HTTP+WS-сервер на одном порту: `/` — веб-плеер для зрителей,
/// `/ws` — протокол трансляции, `/admin` и `/api/*` — управление,
/// доступное только с этой же машины (127.0.0.1).
pub fn ensure_running(app: &AppHandle) {
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if let Some(server) = app.try_state::<Arc<WebServer>>() {
            if server.port().is_some() {
                return;
            }
            let port = server.store.get().port;
            let server = server.inner().clone();
            tokio::spawn(async move {
                let _ = server.ensure(port).await;
            });
        }
    }
    #[cfg(not(desktop))]
    let _ = app;
}

pub struct WebServer {
    pub hub: Arc<StreamHub>,
    pub store: Arc<SettingsStore>,
    app: Mutex<Option<AppHandle>>,
    running: Mutex<Option<Running>>,
}

impl WebServer {
    pub fn new(hub: Arc<StreamHub>, store: Arc<SettingsStore>) -> Arc<Self> {
        Arc::new(Self {
            hub,
            store,
            app: Mutex::new(None),
            running: Mutex::new(None),
        })
    }

    pub fn set_app(&self, app: AppHandle) {
        *self.app.lock().unwrap() = Some(app);
    }

    pub fn app(&self) -> Option<AppHandle> {
        self.app.lock().unwrap().clone()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().unwrap().as_ref().map(|r| r.port)
    }

    pub fn stop(&self) {
        if let Some(running) = self.running.lock().unwrap().take() {
            running.stop.store(true, Ordering::Relaxed);
            running.task.abort();
        }
    }

    /// Поднимает сервер на порту. Старый сервер останавливается только после
    /// успешного bind, чтобы неудачная смена порта не оставляла приложение
    /// без веб-доступа.
    pub async fn ensure(self: &Arc<Self>, port: u16) -> Result<u16, String> {
        if self.port() == Some(port) {
            return Ok(port);
        }
        let listener = TcpListener::bind(("0.0.0.0", port))
            .await
            .map_err(|e| format!("порт {port}: {e}"))?;
        self.stop();
        let stop = Arc::new(AtomicBool::new(false));
        let server = self.clone();
        let stop_task = stop.clone();
        let task = tokio::spawn(async move { serve(server, listener, stop_task).await });
        *self.running.lock().unwrap() = Some(Running { port, stop, task });
        Ok(port)
    }

    pub fn restart_later(self: &Arc<Self>, port: u16) {
        let server = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server.ensure(port).await {
                eprintln!("[fuckexam] web: {e}");
            }
        });
    }

    fn effective_port(&self) -> u16 {
        self.port().unwrap_or_else(|| self.store.get().port)
    }
}

pub(crate) async fn serve(server: Arc<WebServer>, listener: TcpListener, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let (stream, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let server = server.clone();
        tokio::spawn(async move { handle_conn(server, stream, peer).await });
    }
}

/* ---------------- request parsing ---------------- */

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    raw: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn is_websocket(&self) -> bool {
        self.header("upgrade")
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
            && self
                .header("connection")
                .is_some_and(|value| value.to_ascii_lowercase().contains("upgrade"))
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn read_chunk(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<usize, String> {
    let mut tmp = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(15), stream.read(&mut tmp))
        .await
        .map_err(|_| "таймаут чтения запроса".to_string())?
        .map_err(|e| format!("чтение: {e}"))?;
    if n == 0 {
        return Err("соединение закрыто".into());
    }
    buf.extend_from_slice(&tmp[..n]);
    Ok(n)
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut raw = Vec::with_capacity(1024);
    let head_len = loop {
        if let Some(pos) = find_subslice(&raw, b"\r\n\r\n") {
            break pos + 4;
        }
        if raw.len() > MAX_HEAD_BYTES {
            return Err("запрос слишком большой".into());
        }
        read_chunk(stream, &mut raw).await?;
    };

    let head = String::from_utf8_lossy(&raw[..head_len]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let method = first.next().unwrap_or_default().to_string();
    let target = first.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }

    let body_len = headers
        .iter()
        .find(|(key, _)| key == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_BODY_BYTES);
    while raw.len() < head_len + body_len {
        read_chunk(stream, &mut raw).await?;
    }
    let body = raw[head_len..head_len + body_len].to_vec();

    Ok(Request {
        method,
        path,
        headers,
        body,
        raw,
    })
}

/// Отдаёт WebSocket-хендшейку уже прочитанный HTTP-заголовок, чтобы
/// tungstenite не ждал его повторно.
struct PrefixedStream {
    prefix: Vec<u8>,
    pos: usize,
    inner: TcpStream,
}

impl AsyncRead for PrefixedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.pos < this.prefix.len() {
            let n = (this.prefix.len() - this.pos).min(buf.remaining());
            buf.put_slice(&this.prefix[this.pos..this.pos + n]);
            this.pos += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

async fn handle_conn(server: Arc<WebServer>, mut stream: TcpStream, peer: SocketAddr) {
    let local = peer.ip().is_loopback();
    let mut request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(_) => return,
    };

    if request.is_websocket() {
        let admin_events = request.path == "/api/events";
        if admin_events && !local {
            let _ = write_response(
                &mut stream,
                Response::text(403, "управление доступно только с этого компьютера"),
            )
            .await;
            return;
        }
        // Для хендшейка нужен исходный HTTP-заголовок целиком.
        let raw = std::mem::take(&mut request.raw);
        let prefixed = PrefixedStream {
            prefix: raw,
            pos: 0,
            inner: stream,
        };
        if let Ok(ws) = tokio_tungstenite::accept_async(prefixed).await {
            if admin_events {
                let _ = streaming::run_admin_events(server.hub.clone(), ws).await;
            } else {
                let _ = streaming::run_viewer(server.hub.clone(), ws).await;
            }
        }
        return;
    }

    let response = route(&server, &request, local).await;
    let _ = write_response(&mut stream, response).await;
}

/// Десктопный `index.html` плюс шим, который подменяет Tauri IPC на `/api`.
fn admin_page() -> &'static str {
    use std::sync::OnceLock;
    static PAGE: OnceLock<String> = OnceLock::new();
    PAGE.get_or_init(|| {
        DESKTOP_INDEX.replacen(
            "<script src=\"main.js\"></script>",
            "<script src=\"tauri-shim.js\"></script>\n    <script src=\"main.js\"></script>",
            1,
        )
    })
}

/* ---------------- responses ---------------- */

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn file(content_type: &'static str, body: &str) -> Self {
        Self {
            status: 200,
            content_type,
            body: body.as_bytes().to_vec(),
        }
    }

    fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body: value.to_string().into_bytes(),
        }
    }
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

async fn write_response(stream: &mut TcpStream, response: Response) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        status_text(response.status),
        response.content_type,
        response.body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

/* ---------------- routing ---------------- */

async fn route(server: &Arc<WebServer>, request: &Request, local: bool) -> Response {
    let path = request.path.as_str();
    let method = request.method.as_str();

    if path == "/api" || path.starts_with("/api/") {
        if !local {
            return Response::json(
                403,
                json!({ "error": "управление доступно только с этого компьютера" }),
            );
        }
        return api(server, method, path, &request.body).await;
    }

    if path == "/admin" || path == "/admin.html" {
        if !local {
            return Response::text(403, "управление доступно только с этого компьютера");
        }
        return Response::file("text/html; charset=utf-8", admin_page());
    }
    if local {
        match path {
            "/tauri-shim.js" => return Response::file("text/javascript; charset=utf-8", SHIM_JS),
            "/main.js" => return Response::file("text/javascript; charset=utf-8", DESKTOP_JS),
            "/styles.css" => return Response::file("text/css; charset=utf-8", DESKTOP_CSS),
            _ => {}
        }
    }

    match (method, path) {
        ("GET" | "HEAD", "/") | ("GET" | "HEAD", "/index.html") => {
            Response::file("text/html; charset=utf-8", INDEX_HTML)
        }
        ("GET", "/app.js") => Response::file("text/javascript; charset=utf-8", APP_JS),
        ("GET", "/style.css") => Response::file("text/css; charset=utf-8", STYLE_CSS),
        ("GET", "/vendor/marked.umd.js") => {
            Response::file("text/javascript; charset=utf-8", MARKED_JS)
        }
        ("GET", "/vendor/purify.min.js") => {
            Response::file("text/javascript; charset=utf-8", PURIFY_JS)
        }
        ("GET", "/favicon.ico") => Response {
            status: 204,
            content_type: "image/x-icon",
            body: Vec::new(),
        },
        _ => Response::text(404, "не найдено"),
    }
}

/* ---------------- API ---------------- */

fn stream_config(settings: &Settings) -> StreamConfig {
    StreamConfig {
        fps: settings.fps,
        quality: settings.quality,
        max_width: settings.max_width,
    }
}

fn status_payload(server: &WebServer, stream: StreamStatus) -> Value {
    let settings = server.store.get();
    json!({
        "stream": stream,
        "settings": settings,
        "webPort": server.effective_port(),
    })
}

async fn api(
    server: &Arc<WebServer>,
    method: &str,
    path: &str,
    body: &[u8],
) -> Response {
    match (method, path) {
        ("POST", "/api/invoke") => api_invoke(server, body).await,
        ("GET", "/api/status") => {
            Response::json(200, status_payload(server, server.hub.status()))
        }
        ("GET", "/api/settings") => Response::json(200, json!(server.store.get())),
        ("POST", "/api/settings") => {
            let Ok(settings) = serde_json::from_slice::<Settings>(body) else {
                return Response::json(400, json!({ "error": "ожидался JSON с настройками" }));
            };
            let saved = server.store.set(settings);
            let current = server.port();
            if current.is_some_and(|port| port != saved.port) {
                server.restart_later(saved.port);
            }
            Response::json(200, json!(saved))
        }
        ("POST", "/api/stream/start") => {
            let Some(app) = server.app() else {
                return Response::json(503, json!({ "error": "приложение не готово" }));
            };
            let settings = server.store.get();
            let config = stream_config(&settings);
            let result = if settings.mode == "relay" {
                if settings.relay_url.is_empty() {
                    Err("адрес relay не указан в настройках".to_string())
                } else {
                    server
                        .hub
                        .arm_relay(app, settings.relay_url.clone(), config)
                        .await
                }
            } else {
                server.hub.arm_local(app, settings.port, config).await
            };
            match result {
                Ok(status) => Response::json(200, status_payload(server, status)),
                Err(e) => Response::json(409, json!({ "error": e })),
            }
        }
        ("POST", "/api/stream/stop") => {
            server.hub.deactivate();
            Response::json(200, status_payload(server, server.hub.status()))
        }
        ("POST", "/api/window/show") => match server.app() {
            Some(app) => {
                crate::ui::show(&app);
                Response::json(200, json!({ "ok": true }))
            }
            None => Response::json(503, json!({ "error": "приложение не готово" })),
        },
        ("POST", "/api/window/tray") => match server.app() {
            Some(app) => {
                crate::ui::hide_to_tray(&app);
                Response::json(200, json!({ "ok": true }))
            }
            None => Response::json(503, json!({ "error": "приложение не готово" })),
        },
        ("POST", "/api/window/hide") => match server.app() {
            Some(app) => {
                crate::ui::hide_completely(&app);
                Response::json(200, json!({ "ok": true }))
            }
            None => Response::json(503, json!({ "error": "приложение не готово" })),
        },
        ("POST", "/api/quit") => match server.app() {
            Some(app) => {
                app.exit(0);
                Response::json(200, json!({ "ok": true }))
            }
            None => Response::json(503, json!({ "error": "приложение не готово" })),
        },
        _ => Response::json(404, json!({ "error": "неизвестный метод" })),
    }
}

/* ---------------- desktop IPC over HTTP ---------------- */

#[derive(serde::Deserialize)]
struct InvokeCall {
    cmd: String,
    #[serde(default)]
    args: Value,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamOptions {
    fps: u32,
    quality: u8,
    max_width: usize,
}

fn arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("поле «{key}»: {e}"))
}

fn options_config(options: StreamOptions) -> StreamConfig {
    StreamConfig {
        fps: options.fps,
        quality: options.quality,
        max_width: options.max_width,
    }
}

fn web_info_value(server: &WebServer) -> Value {
    let settings = server.store.get();
    let port = server.effective_port();
    #[cfg(desktop)]
    let mut addresses = Vec::new();
    #[cfg(not(desktop))]
    let addresses: Vec<String> = Vec::new();
    #[cfg(desktop)]
    {
        let mut ips = streaming::local_addresses().unwrap_or_default();
        ips.sort();
        addresses.extend(ips.iter().map(|ip| format!("http://{ip}:{port}/")));
        addresses.push(format!("http://127.0.0.1:{port}/admin"));
    }
    json!({
        "running": server.port().is_some(),
        "port": port,
        "addresses": addresses,
        "closeToTray": settings.close_to_tray,
    })
}

async fn api_invoke(server: &Arc<WebServer>, body: &[u8]) -> Response {
    let Ok(call) = serde_json::from_slice::<InvokeCall>(body) else {
        return Response::json(400, json!({ "error": "ожидался JSON вида {cmd, args}" }));
    };
    match invoke_command(server, &call.cmd, call.args).await {
        Ok(result) => Response::json(200, json!({ "result": result })),
        Err(error) => Response::json(409, json!({ "error": error })),
    }
}

fn app_of(server: &WebServer) -> Result<AppHandle, String> {
    server.app().ok_or_else(|| "приложение не готово".to_string())
}

async fn invoke_command(
    server: &Arc<WebServer>,
    cmd: &str,
    args: Value,
) -> Result<Value, String> {
    match cmd {
        "platform_info" => Ok(json!({
            "os": std::env::consts::OS,
            "recorder": cfg!(any(target_os = "linux", target_os = "windows")),
            "relay": true,
            "web": cfg!(desktop),
            "desktop": cfg!(desktop),
        })),
        "web_info" => Ok(web_info_value(server)),
        "get_settings" => Ok(json!(server.store.get())),
        "save_settings" => {
            let settings: Settings = arg(&args, "settings")?;
            let previous = server.store.get();
            let saved = server.store.set(settings);
            if previous.port != saved.port && saved.web_on_launch && server.port().is_some() {
                server.restart_later(saved.port);
            }
            Ok(json!(saved))
        }
        "recording_status" => {
            let app = app_of(server)?;
            let recorder = app.state::<crate::capture::Recorder>();
            Ok(json!(crate::capture::status(&recorder)))
        }
        "start_recording" => {
            let app = app_of(server)?;
            let recorder = app.state::<crate::capture::Recorder>();
            let message = crate::capture::start_recording(&recorder, Some(&app)).await?;
            Ok(json!(message))
        }
        "stop_recording" => {
            let app = app_of(server)?;
            let recorder = app.state::<crate::capture::Recorder>();
            let message = crate::capture::stop_recording(&recorder)?;
            Ok(json!(message))
        }
        "stream_start" => {
            let app = app_of(server)?;
            let options: StreamOptions = arg(&args, "options")?;
            let settings = server.store.get();
            // Веб-сервер уже поднят: запрос пришёл через него. Смена порта
            // обрабатывается в save_settings (restart_later).
            let status = server
                .hub
                .arm_local(app, settings.port, options_config(options))
                .await?;
            Ok(json!(status))
        }
        "relay_connect" => {
            let app = app_of(server)?;
            let url: String = arg(&args, "url")?;
            let options: StreamOptions = arg(&args, "options")?;
            let status = server
                .hub
                .arm_relay(app, url, options_config(options))
                .await?;
            Ok(json!(status))
        }
        "stream_status" => Ok(json!(server.hub.status())),
        "stream_stop" => {
            server.hub.deactivate();
            Ok(json!(null))
        }
        "relay_ping" => {
            let url: String = arg(&args, "url")?;
            Ok(json!(streaming::ping_relay(url).await?))
        }
        "window_show" => {
            let app = app_of(server)?;
            crate::ui::show(&app);
            Ok(json!(null))
        }
        "window_to_tray" => {
            let app = app_of(server)?;
            crate::ui::hide_to_tray(&app);
            Ok(json!(null))
        }
        "window_hide_all" => {
            let app = app_of(server)?;
            crate::ui::hide_completely(&app);
            Ok(json!(null))
        }
        "quit" => {
            let app = app_of(server)?;
            app.exit(0);
            Ok(json!(null))
        }
        "chat_send" => {
            let text: String = arg(&args, "text")?;
            let markdown: bool = args
                .get("markdown")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let image: Option<ChatImage> = match args.get("image") {
                Some(Value::Null) | None => None,
                Some(value) => serde_json::from_value(value.clone())
                    .map_err(|e| format!("поле «image»: {e}"))?,
            };
            server.hub.send_chat(text, markdown, image).await?;
            Ok(json!(null))
        }
        "chat_history" => Ok(json!(server.hub.chat_history())),
        "open_chat_window" => {
            #[cfg(desktop)]
            {
                let app = app_of(server)?;
                crate::open_chat_window_impl(&app).await?;
                Ok(json!(null))
            }
            #[cfg(not(desktop))]
            {
                Err("отдельное окно чата недоступно на этой платформе".into())
            }
        }
        other => Err(format!("неизвестная команда: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str) -> Request {
        Request {
            method: "GET".into(),
            path: path.into(),
            headers: Vec::new(),
            body: Vec::new(),
            raw: Vec::new(),
        }
    }

    #[tokio::test]
    async fn api_is_loopback_only() {
        let hub = Arc::new(StreamHub::default());
        let server = WebServer::new(hub, SettingsStore::ephemeral());
        let response = route(&server, &get("/api/status"), false).await;
        assert_eq!(response.status, 403);
        let response = route(&server, &get("/api/status"), true).await;
        assert_eq!(response.status, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["webPort"], 7335);
    }

    #[tokio::test]
    async fn admin_page_is_loopback_only() {
        let hub = Arc::new(StreamHub::default());
        let server = WebServer::new(hub, SettingsStore::ephemeral());
        assert_eq!(route(&server, &get("/admin"), false).await.status, 403);
        assert_eq!(route(&server, &get("/admin"), true).await.status, 200);
        assert_eq!(route(&server, &get("/"), false).await.status, 200);
    }

    #[tokio::test]
    async fn settings_api_saves_and_clamps() {
        let hub = Arc::new(StreamHub::default());
        let server = WebServer::new(hub, SettingsStore::ephemeral());
        let body = json!({ "port": 7455, "fps": 0, "quality": 75, "maxWidth": 1280 })
            .to_string();
        let response = api(&server, "POST", "/api/settings", body.as_bytes()).await;
        assert_eq!(response.status, 200);
        let settings = server.store.get();
        assert_eq!(settings.port, 7455);
        assert_eq!(settings.fps, 1);
    }
}
