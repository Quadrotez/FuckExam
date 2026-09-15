use std::{
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tauri::Manager;
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    graphics_capture_picker::GraphicsCapturePicker,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};

use crate::streaming::StreamHub;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub node_id: u32,
    pub width: i32,
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
    pub hub: Arc<StreamHub>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            hub: Arc::new(StreamHub::default()),
        }
    }
}

pub struct RecorderInner {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    window: WindowInfo,
}

impl Drop for RecorderInner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn output_dir() -> Result<PathBuf, String> {
    let dir = dirs::video_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("FuckExam");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    Ok(dir)
}

fn stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

struct Capture {
    stop: Arc<AtomicBool>,
    hub: Arc<StreamHub>,
    node_id: u32,
    output: PathBuf,
    width: u32,
    height: u32,
    ffmpeg: PathBuf,
    encoder: Option<Child>,
    last_jpeg: Option<Vec<u8>>,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = (i32, i32, Arc<AtomicBool>, Arc<StreamHub>, PathBuf, PathBuf);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            stop: ctx.flags.2.clone(),
            hub: ctx.flags.3.clone(),
            node_id: 1,
            output: ctx.flags.4.clone(),
            width: ctx.flags.0 as u32,
            height: ctx.flags.1 as u32,
            ffmpeg: ctx.flags.5.clone(),
            encoder: None,
            last_jpeg: None,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.stop.load(Ordering::Relaxed) {
            control.stop();
            return Ok(());
        }

        let mut buffer = frame.buffer()?;
        let mut rows = Vec::new();
        let pixels = buffer.as_nopadding_buffer(&mut rows);
        let mut data = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in pixels {
            data.extend_from_slice(row);
        }

        if self.encoder.is_none() {
            self.encoder = Some(spawn_ffmpeg(
                self.width,
                self.height,
                &self.output,
                &self.ffmpeg,
            )?);
        }
        if let Some(child) = self.encoder.as_mut() {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(&data)?;
            }
        }

        if let Some((w, h, jpeg)) = crate::streaming::encode_stream_frame(
            &data,
            self.width as usize,
            self.height as usize,
            self.width as usize * 4,
        ) {
            if self.last_jpeg.as_ref() != Some(&jpeg) {
                self.last_jpeg = Some(jpeg.clone());
                self.hub.push_video(self.node_id, w, h, jpeg);
            }
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.stop.store(true, Ordering::Relaxed);
        self.hub.announce_end(self.node_id);
        Ok(())
    }
}

fn spawn_ffmpeg(
    width: u32,
    height: u32,
    output: &PathBuf,
    executable: &PathBuf,
) -> Result<Child, String> {
    let command = if executable.exists() {
        executable.clone()
    } else {
        PathBuf::from("ffmpeg")
    };
    Command::new(command)
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgra",
            "-video_size",
            &format!("{width}x{height}"),
            "-r",
            "30",
            "-i",
            "pipe:0",
            "-vf",
            "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-tune",
            "zerolatency",
            "-movflags",
            "+faststart",
        ])
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))
}

pub async fn start_recording(
    state: &Recorder,
    app: Option<&tauri::AppHandle>,
) -> Result<String, String> {
    if state.inner.lock().map_err(|_| "lock")?.is_some() {
        return Err("запись уже идёт".into());
    }

    let item = GraphicsCapturePicker::pick_item()
        .map_err(|e| format!("Windows Graphics Capture: {e}"))?
        .ok_or_else(|| "окно не выбрано".to_string())?;
    let size = item.size().map_err(|e| format!("capture item size: {e}"))?;
    let dir = output_dir()?;
    let output = dir.join(format!("fuckexam_win1_{}.mp4", stamp()));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let hub = state.hub.clone();
    let output_thread = output.clone();
    let ffmpeg = app
        .and_then(|a| a.path().resource_dir().ok())
        .map(|p| p.join("ffmpeg.exe"))
        .unwrap_or_else(|| PathBuf::from("ffmpeg.exe"));
    let settings = Settings::new(
        item,
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        (size.0, size.1, stop, hub, output, ffmpeg),
    );

    let thread = std::thread::spawn(move || {
        if let Err(e) = Capture::start(settings) {
            eprintln!("[fuckexam] Windows capture: {e}");
            thread_stop.store(true, Ordering::Relaxed);
        }
    });
    let info = WindowInfo {
        node_id: 1,
        width: size.0,
        height: size.1,
        output: output_thread.display().to_string(),
    };
    *state.inner.lock().map_err(|_| "lock")? = Some(RecorderInner {
        stop,
        thread: Some(thread),
        window: info,
    });
    Ok("запись идёт (1 окно)".into())
}

pub fn stop_recording(state: &Recorder) -> Result<String, String> {
    let mut guard = state.inner.lock().map_err(|_| "lock")?;
    let mut inner = guard.take().ok_or("запись не запущена")?;
    inner.stop.store(true, Ordering::Relaxed);
    if let Some(thread) = inner.thread.take() {
        let _ = thread.join();
    }
    Ok(format!("запись остановлена: {}", inner.window.output))
}

pub fn status(state: &Recorder) -> RecordingSnapshot {
    match state.inner.lock() {
        Ok(guard) => guard
            .as_ref()
            .map(|inner| RecordingSnapshot {
                running: !inner.stop.load(Ordering::Relaxed),
                windows: vec![inner.window.clone()],
                error: None,
            })
            .unwrap_or_default(),
        Err(_) => RecordingSnapshot {
            error: Some("lock".into()),
            ..Default::default()
        },
    }
}

pub fn run_cli_record() {}
