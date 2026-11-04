//! Курсор мыши на KDE Wayland.
//!
//! Пока под курсором есть окно приложения, GDK через XWayland сообщает его
//! позицию. Как только все окна спрятаны, координата замирает на последнем
//! значении — знает её только композитор. Поэтому на время работы «Хайди»
//! в KWin подгружается крошечный скрипт: он слушает `workspace.cursorPosChanged`
//! и отправляет позицию нам по DBus.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Имя нашей службы на сессионной шине.
const SERVICE: &str = "ru.quadro.fuckexam.Hidey";
const PATH: &str = "/ru/quadro/fuckexam/Hidey";
const IFACE: &str = "ru.quadro.fuckexam.Hidey";
/// Под этим именем скрипт живёт в KWin.
const SCRIPT_NAME: &str = "fuckexam-hidey";

static ACTIVE: AtomicBool = AtomicBool::new(false);
static SERVING: AtomicBool = AtomicBool::new(false);
static X: AtomicI32 = AtomicI32::new(i32::MIN);
static Y: AtomicI32 = AtomicI32::new(i32::MIN);

/// Скрипт для KWin. Первый же вызов отдаёт текущую позицию, дальше KWin
/// дёргает нас только когда мышь действительно сдвинулась.
const SCRIPT: &str = r#"
function fexPushNow() {
    var p = workspace.cursorPos;
    callDBus("__SERVICE__", "__PATH__", "__IFACE__", "cursor", Math.round(p.x), Math.round(p.y));
}

fexPushNow();
workspace.cursorPosChanged.connect(fexPushNow);
"#;

struct Hidey;

#[zbus::interface(name = "ru.quadro.fuckexam.Hidey")]
impl Hidey {
    /// KWin присылает позицию курсора при каждом движении мыши.
    #[zbus(name = "cursor")]
    fn cursor(&self, x: i32, y: i32) {
        X.store(x, Ordering::Relaxed);
        Y.store(y, Ordering::Relaxed);
    }
}

/// Последняя позиция курсора, если скрипт в KWin уже работает.
pub fn cursor() -> Option<(f64, f64)> {
    if !ACTIVE.load(Ordering::Relaxed) {
        return None;
    }
    let (x, y) = (X.load(Ordering::Relaxed), Y.load(Ordering::Relaxed));
    if x == i32::MIN || y == i32::MIN {
        return None;
    }
    Some((x as f64, y as f64))
}

/// Включает слежение: поднимает службу и загружает скрипт в KWin.
pub fn start() {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = serve().await {
            crate::diag::log(&format!("hidey: служба курсора не поднялась: {e}"));
            return;
        }
        if let Err(e) = load_script().await {
            crate::diag::log(&format!("hidey: скрипт KWin не загрузился: {e}"));
            return;
        }
        ACTIVE.store(true, Ordering::Relaxed);
        crate::diag::log("hidey: курсор берём у KWin");
    });
}

/// Выключает слежение и снимает скрипт с KWin.
pub fn stop() {
    ACTIVE.store(false, Ordering::Relaxed);
    X.store(i32::MIN, Ordering::Relaxed);
    Y.store(i32::MIN, Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        let _ = unload_script().await;
        crate::diag::log("hidey: скрипт KWin снят");
    });
}

/// Снимает скрипт синхронно — на выходе из приложения асинхронщик уже мёртв.
pub fn stop_blocking() {
    if !ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = zbus::blocking::Connection::session().and_then(|conn| unload_with(&conn));
        let _ = tx.send(result);
    });
    // KWin отвечает мгновенно; если шина зависла, не держим выход из приложения.
    let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
}

async fn serve() -> zbus::Result<()> {
    if SERVING.load(Ordering::Relaxed) {
        return Ok(());
    }
    let conn = zbus::Connection::session().await?;
    let reply = conn
        .request_name_with_flags(SERVICE, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await?;
    if reply != zbus::fdo::RequestNameReply::PrimaryOwner {
        return Err(zbus::Error::NameTaken);
    }
    conn.object_server().at(PATH, Hidey).await?;
    SERVING.store(true, Ordering::Relaxed);
    // Соединение держим живым всё время работы приложения.
    tauri::async_runtime::spawn(async move {
        std::future::pending::<()>().await;
        drop(conn);
    });
    // Дать шине время раздать имя, иначе первый вызов скрипта уйдёт в пустоту.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    Ok(())
}

fn script_path() -> std::path::PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    base.join("fuckexam").join("hidey.js")
}

/// Пишет скрипт на диск: KWin умеет грузить только файлы.
fn write_script() -> std::io::Result<std::path::PathBuf> {
    let path = script_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let source = SCRIPT
        .replace("__SERVICE__", SERVICE)
        .replace("__PATH__", PATH)
        .replace("__IFACE__", IFACE);
    std::fs::write(&path, source)?;
    Ok(path)
}

async fn load_script() -> zbus::Result<()> {
    let path = write_script()
        .map_err(|e| zbus::Error::Failure(format!("не записался скрипт: {e}")))?;
    let path = path.to_string_lossy().to_string();
    let conn = zbus::Connection::session().await?;
    let proxy =
        zbus::Proxy::new(&conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting").await?;
    // Прошлый запуск мог оставить скрипт загруженным.
    let loaded: bool = proxy
        .call::<_, _, bool>("isScriptLoaded", &(SCRIPT_NAME,))
        .await
        .unwrap_or(false);
    if loaded {
        let _ = proxy
            .call::<_, _, bool>("unloadScript", &(SCRIPT_NAME,))
            .await;
    }
    proxy
        .call::<_, _, i32>("loadScript", &(path.as_str(), SCRIPT_NAME))
        .await?;
    proxy.call::<_, _, ()>("start", &()).await?;
    Ok(())
}

async fn unload_script() -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    let proxy =
        zbus::Proxy::new(&conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting").await?;
    let _ = proxy
        .call::<_, _, bool>("unloadScript", &(SCRIPT_NAME,))
        .await;
    Ok(())
}

fn unload_with(conn: &zbus::blocking::Connection) -> zbus::Result<()> {
    let proxy = zbus::blocking::Proxy::new(
        conn,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )?;
    let _ = proxy.call::<_, _, bool>("unloadScript", &(SCRIPT_NAME,));
    Ok(())
}
