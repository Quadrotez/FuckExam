use std::{
    os::fd::OwnedFd,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions, Stream,
};
use ashpd::desktop::Session;
use enumflags2::BitFlags;
use serde::Serialize;

use crate::window_capture::WindowCapture;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub node_id: u32,
    #[serde(rename = "width")]
    pub width: i32,
    #[serde(rename = "height")]
    pub height: i32,
    pub output: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSnapshot {
    pub running: bool,
    pub windows: Vec<WindowInfo>,
    pub error: Option<String>,
}

pub struct Recorder {
    pub inner: Mutex<Option<RecorderInner>>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

pub struct RecorderInner {
    #[allow(dead_code)]
    proxy: Screencast,
    #[allow(dead_code)]
    session: Session<Screencast>,
    windows: Vec<WindowCapture>,
}

impl Drop for RecorderInner {
    fn drop(&mut self) {
        for w in &mut self.windows {
            w.finish();
        }
    }
}

pub struct Picked {
    proxy: Screencast,
    session: Session<Screencast>,
    streams: Vec<Stream>,
    fds: Vec<OwnedFd>,
}

async fn pick_windows() -> Result<Picked, String> {
    let proxy = Screencast::new().await.map_err(|e| format!("portal: {e}"))?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| format!("create_session: {e}"))?;

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(BitFlags::from_flag(SourceType::Window))
                .set_multiple(true),
        )
        .await
        .map_err(|e| format!("select_sources: {e}"))?;

    let streams = proxy
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(|e| format!("start: {e}"))?
        .response()
        .map_err(|e| format!("start response: {e}"))?
        .streams()
        .to_vec();

    if streams.is_empty() {
        return Err("окна не выбраны".into());
    }

    let mut fds = Vec::new();
    for _ in &streams {
        fds.push(
            proxy
                .open_pipe_wire_remote(&session, Default::default())
                .await
                .map_err(|e| format!("open_pipe_wire_remote: {e}"))?,
        );
    }

    Ok(Picked {
        proxy,
        session,
        streams,
        fds,
    })
}

fn output_dir() -> Result<PathBuf, String> {
    let vid = dirs::video_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")));
    let dir = vid.join("FuckExam");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    Ok(dir)
}

fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

pub async fn start_recording(state: &Recorder) -> Result<String, String> {
    {
        let guard = state.inner.lock().map_err(|_| "lock")?;
        if guard.is_some() {
            return Err("запись уже идёт".into());
        }
    }

    let picked = pick_windows().await?;
    let dir = output_dir()?;
    let stamp = stamp();

    let mut windows = Vec::new();
    for (s, fd) in picked.streams.iter().zip(picked.fds) {
        let node = s.pipe_wire_node_id();
        let out = dir.join(format!("fuckexam_win{node}_{stamp}.mp4"));
        let cap = WindowCapture::spawn(fd, node, &out)?;
        windows.push(cap);
    }

    let window_count = windows.len();

    let inner = RecorderInner {
        proxy: picked.proxy,
        session: picked.session,
        windows,
    };

    let mut guard = state.inner.lock().map_err(|_| "lock")?;
    *guard = Some(inner);

    Ok(if window_count == 1 {
        "запись идёт (1 окно)".to_string()
    } else {
        format!("запись идёт ({} окон)", window_count)
    })
}

pub fn stop_recording(state: &Recorder) -> Result<String, String> {
    let mut guard = state.inner.lock().map_err(|_| "lock")?;
    let mut inner = guard.take().ok_or("запись не запущена")?;
    drop(guard);

    let mut outs = Vec::new();
    for w in &mut inner.windows {
        w.finish();
        outs.push(w.output.clone());
    }

    drop(inner);

    if outs.is_empty() {
        return Err("файлы не найдены".into());
    }

    Ok(format!("запись остановлена: {}", outs.join(", ")))
}

pub fn status(state: &Recorder) -> RecordingSnapshot {
    match state.inner.lock() {
        Ok(guard) => match &*guard {
            Some(inner) => RecordingSnapshot {
                running: true,
                windows: inner
                    .windows
                    .iter()
                    .map(|w| {
                        let (width, height) = *w.size.lock().unwrap();
                        WindowInfo {
                            node_id: w.node_id,
                            width: width as i32,
                            height: height as i32,
                            output: w.output.clone(),
                        }
                    })
                    .collect(),
                error: None,
            },
            None => RecordingSnapshot::default(),
        },
        Err(_) => RecordingSnapshot {
            running: false,
            windows: vec![],
            error: Some("lock".into()),
        },
    }
}

const DEFAULT_SECS: u64 = 12;

fn cli_duration() -> u64 {
    std::env::var("FEX_DUR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SECS)
}

pub fn run_cli_record() {
    if let Ok(node_str) = std::env::var("FEX_SELFTEST") {
        use std::os::fd::{FromRawFd, OwnedFd};
        let node_id: u32 = node_str.parse().expect("FEX_SELFTEST=pipe_wire_node_id");
        let raw = unsafe {
            libc::open(
                b"/dev/null\0".as_ptr() as *const _,
                libc::O_RDONLY,
            )
        };
        if raw < 0 {
            eprintln!("open /dev/null failed");
            return;
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let dir = output_dir().expect("output dir");
        let stamp = stamp();
        let out = dir.join(format!("fuckexam_selftest_{node_id}_{stamp}.mp4"));
        let mut cap = crate::window_capture::WindowCapture::spawn(fd, node_id, &out)
            .expect("spawn");
        let secs = cli_duration();
        println!("SELFTEST: capture node {node_id} for {secs}s -> {}", out.display());
        std::thread::sleep(std::time::Duration::from_secs(secs));
        cap.finish();
        println!("DONE: {}", out.display());
        return;
    }

    let secs = cli_duration();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let rec = Recorder::default();
        match start_recording(&rec).await {
            Ok(msg) => println!("START: {msg}"),
            Err(e) => {
                eprintln!("START ERROR: {e}");
                return;
            }
        }
        for w in status(&rec).windows {
            println!("NODE {} {}x{} -> {}", w.node_id, w.width, w.height, w.output);
        }
        println!("RECORDING for {secs}s…");
        tokio::time::sleep(Duration::from_secs(secs)).await;
        match stop_recording(&rec) {
            Ok(msg) => println!("STOP: {msg}"),
            Err(e) => eprintln!("STOP ERROR: {e}"),
        }
    });
}