//! Диагностика запуска: файловый лог и видимое окно ошибки на Windows.
//!
//! Смысл: `windows_subsystem = "windows"` не оставляет пользователю ни консоли,
//! ни сообщения, если приложение упало до создания окна. Лог — единственный
//! способ узнать, что произошло на чужой машине.
//!
//! На Android файловый лог не нужен: там всё видно в logcat.

#[cfg(not(target_os = "android"))]
use std::path::PathBuf;
#[cfg(not(target_os = "android"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "android"))]
use std::{
    fs::OpenOptions,
    io::Write,
    sync::OnceLock,
};

#[cfg(not(target_os = "android"))]
const MAX_LOG_BYTES: u64 = 1024 * 1024;

#[cfg(not(target_os = "android"))]
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Куда писать лог: рядом с данными приложения, без прав администратора.
#[cfg(not(target_os = "android"))]
pub fn log_path() -> PathBuf {
    LOG_PATH
        .get_or_init(|| {
            let base = app_data_dir().unwrap_or_else(std::env::temp_dir);
            let dir = base.join("logs");
            let _ = std::fs::create_dir_all(&dir);
            dir.join("app.log")
        })
        .clone()
}

#[cfg(not(target_os = "android"))]
fn app_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return Some(PathBuf::from(local).join("FuckExam"));
        }
    }
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(state).join("fuckexam"));
    }
    dirs::state_dir()
        .map(|p| p.join("fuckexam"))
        .or_else(|| dirs::data_local_dir().map(|p| p.join("fuckexam")))
}

#[cfg(not(target_os = "android"))]
fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(not(target_os = "android"))]
fn rotate_if_needed(path: &PathBuf) {
    let too_big = std::fs::metadata(path)
        .map(|m| m.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if too_big {
        let prev = path.with_extension("log.1");
        let _ = std::fs::rename(path, prev);
    }
}

#[cfg(not(target_os = "android"))]
pub fn log(message: &str) {
    let path = log_path();
    rotate_if_needed(&path);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {message}", timestamp());
    }
}

#[cfg(target_os = "android")]
pub fn log(_message: &str) {}

/// Пишет панику в лог и, на Windows, показывает окно.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = format!("PANIC: {info}");
        log(&text);
        if let Some(location) = info.location() {
            log(&format!(
                "PANIC at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            ));
        }
        #[cfg(windows)]
        {
            let tail = std::fs::read_to_string(log_path())
                .map(|s| {
                    let lines: Vec<&str> = s.lines().rev().take(12).collect();
                    let mut v: Vec<&str> = lines.into_iter().collect();
                    v.reverse();
                    v.join("\n")
                })
                .unwrap_or_default();
            message_box(&format!(
                "FuckExam не смог запуститься.\n\n{text}\n\nЛог: {}\n\n{tail}",
                log_path().display()
            ));
        }
        previous(info);
    }));
}

/// Фатальная ошибка запуска: лог + (на Windows) видимое окно.
pub fn report_fatal(message: &str) {
    log(message);
    eprintln!("[fuckexam] {message}");
    #[cfg(windows)]
    message_box(message);
}

/// Основная информация о среде — первое, что нужно в логе от чужой машины.
pub fn log_environment() {
    log(&format!(
        "start FuckExam v{} | os={} arch={} pid={}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id()
    ));

    #[cfg(not(target_os = "android"))]
    log(&format!("log file: {}", log_path().display()));

    #[cfg(windows)]
    {
        log(&format!("windows version: {}", windows_version()));
        log(&format!(
            "webview2: {}",
            webview2_version().unwrap_or_else(|| "НЕ НАЙДЕН".into())
        ));
        log(&format!("ffmpeg: {}", ffmpeg_status()));
        log(&format!(
            "localappdata={:?} temp={:?}",
            std::env::var_os("LOCALAPPDATA"),
            std::env::temp_dir()
        ));
    }
}

#[cfg(windows)]
fn windows_version() -> String {
    std::process::Command::new("cmd")
        .args(["/c", "ver"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "неизвестно".into())
}

/// WebView2: сначала реестр (как это делает сама Tauri), потом каталог установки.
#[cfg(windows)]
pub fn webview2_version() -> Option<String> {
    const CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    for root in ["HKLM", "HKCU"] {
        for prefix in ["SOFTWARE\\WOW6432Node\\", "SOFTWARE\\"] {
            let key = format!("{root}\\{prefix}{CLIENT}");
            let Ok(out) = std::process::Command::new("reg")
                .args(["query", &key, "/v", "pv"])
                .output()
            else {
                continue;
            };
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(version) = text.split("REG_SZ").nth(1) {
                let version = version.trim();
                if !version.is_empty() {
                    return Some(version.to_string());
                }
            }
        }
    }
    for base in [
        std::env::var_os("ProgramFiles(x86)"),
        std::env::var_os("ProgramFiles"),
    ]
    .into_iter()
    .flatten()
    {
        let dir = PathBuf::from(base).join("Microsoft/EdgeWebView/Application");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            if let Some(ver) = entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .max()
            {
                return Some(format!("{ver} (каталог)"));
            }
        }
    }
    None
}

#[cfg(windows)]
fn ffmpeg_status() -> String {
    let local = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("ffmpeg.exe")));
    if let Some(path) = local {
        if path.is_file() {
            return format!("рядом с приложением: {}", path.display());
        }
    }
    match std::process::Command::new("ffmpeg").arg("-version").output() {
        Ok(out) if out.status.success() => "в PATH".into(),
        _ => "НЕ НАЙДЕН (запись работать не будет)".into(),
    }
}

#[cfg(windows)]
fn message_box(text: &str) {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn MessageBoxW(
            hwnd: *mut core::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            u_type: u32,
        ) -> i32;
    }

    const MB_ICONERROR: u32 = 0x10;
    let wide: Vec<u16> = std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let caption: Vec<u16> = std::ffi::OsStr::new("FuckExam — ошибка")
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            caption.as_ptr(),
            MB_ICONERROR,
        );
    }
}
