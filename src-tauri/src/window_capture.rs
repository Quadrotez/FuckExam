use std::{
    io::Write,
    os::fd::OwnedFd,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

#[derive(Default)]
struct FrameState {
    w: u32,
    h: u32,
    bytes: Vec<u8>,
    ready: bool,
}

pub struct WindowCapture {
    pub node_id: u32,
    pub output: String,
    pub size: Arc<Mutex<(u32, u32)>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WindowCapture {
    pub fn spawn(fd: OwnedFd, node_id: u32, out: &Path) -> Result<WindowCapture, String> {
        let output = out.display().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let size = Arc::new(Mutex::new((0u32, 0u32)));
        let stop2 = stop.clone();
        let size2 = size.clone();
        let out2 = out.to_path_buf();
        let thread = std::thread::spawn(move || {
            if let Err(e) = run(fd, node_id, &out2, stop2, size2) {
                eprintln!("[fuckexam] window {node_id}: {e}");
            }
        });
        Ok(WindowCapture {
            node_id,
            output,
            size,
            stop,
            thread: Some(thread),
        })
    }

    pub fn finish(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        self.finish();
    }
}

fn run(
    fd: OwnedFd,
    node_id: u32,
    out: &Path,
    stop: Arc<AtomicBool>,
    size: Arc<Mutex<(u32, u32)>>,
) -> Result<(), String> {
    let _fd = fd;
    gst::init().map_err(|e| format!("gst init: {e}"))?;

    let pipeline = gst::Pipeline::new();
    let src = gst::ElementFactory::make("pipewiresrc")
        .name("src")
        .build()
        .map_err(|e| format!("pipewiresrc: {e}"))?;
    src.set_property("path", node_id.to_string());
    src.set_property("do-timestamp", true);

    let convert = gst::ElementFactory::make("videoconvert")
        .name("convert")
        .build()
        .map_err(|e| format!("videoconvert: {e}"))?;

    let sink = gst::ElementFactory::make("appsink")
        .name("sink")
        .build()
        .map_err(|e| format!("appsink: {e}"))?;
    sink.set_property(
        "caps",
        gst::Caps::builder("video/x-raw").field("format", "BGRA").build(),
    );
    sink.set_property("sync", false);
    sink.set_property("drop", false);
    sink.set_property("max-buffers", 8u32);

    let _ = pipeline.add_many(&[&src, &convert, &sink]);
    gst::Element::link_many(&[&src, &convert, &sink])
        .map_err(|e| format!("link: {e}"))?;

    let error_log: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let error_log2 = error_log.clone();
    pipeline
        .bus()
        .expect("bus")
        .set_sync_handler(move |_bus, msg| {
            if let gst::MessageView::Error(err) = msg.view() {
                *error_log2.lock().unwrap() = Some(err.error().to_string());
            } else if let gst::MessageView::Eos(_) = msg.view() {
                *error_log2.lock().unwrap() = Some("EOS".into());
            }
            gst::BusSyncReply::Drop
        });

    pipeline.set_state(gst::State::Playing).map_err(|e| format!("play: {e}"))?;

    if std::env::var_os("FEX_DEBUG").is_some() {
        eprintln!("[fuckexam] window {node_id}: gst pipeline playing");
    }

    let appsink = sink
        .clone()
        .dynamic_cast::<gst_app::AppSink>()
        .map_err(|_| "appsink downcast")?;

    let frame = Arc::new(Mutex::new(FrameState::default()));
    let writer_stop = stop.clone();
    let writer_size = size.clone();
    let writer_frame = frame.clone();
    let writer_node = node_id;
    let writer_out = out.to_path_buf();
    std::thread::spawn(move || {
        writer_loop(writer_node, writer_out, writer_frame, writer_size, writer_stop);
    });

    while !stop.load(Ordering::Relaxed) {
        match appsink.try_pull_sample(Some(gst::ClockTime::from_mseconds(2))) {
            Some(sample) => {
                let Some(buf) = sample.buffer() else {
                    continue;
                };
                let Some(caps) = sample.caps() else {
                    continue;
                };
                let info = match gst_video::VideoInfo::from_caps(caps) {
                    Ok(info) => info,
                    Err(e) => {
                        if std::env::var_os("FEX_DEBUG").is_some() {
                            eprintln!("[fuckexam] window {node_id}: caps: {e}");
                        }
                        continue;
                    }
                };
                let w = info.width();
                let h = info.height();
                let stride = info.stride().first().copied().unwrap_or((w * 4) as i32) as usize;
                let need = stride * h as usize;
                if need == 0 {
                    continue;
                }

                let map = buf.map_readable().map_err(|_| "buffer map failed")?;
                let src_slice = map.as_slice();
                let n = need.min(src_slice.len());

                let mut f = frame.lock().unwrap();
                f.w = w;
                f.h = h;
                if f.bytes.len() < n {
                    f.bytes.resize(n, 0);
                }
                f.bytes[..n].copy_from_slice(&src_slice[..n]);
                f.ready = true;
            }
            None => {}
        }
    }

    let _ = pipeline.set_state(gst::State::Null);

    if let Some(log) = error_log.lock().unwrap().clone() {
        eprintln!("[fuckexam] window {node_id}: stderr: {log}");
    }
    Ok(())
}

fn writer_loop(
    node_id: u32,
    out: std::path::PathBuf,
    frame: Arc<Mutex<FrameState>>,
    size: Arc<Mutex<(u32, u32)>>,
    stop: Arc<AtomicBool>,
) {
    let mut child: Option<Child> = None;
    let mut next = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
            continue;
        }
        next = now + Duration::from_millis(33);

        let f = frame.lock().unwrap();
        if !f.ready {
            continue;
        }

        {
            let mut sz = size.lock().unwrap();
            *sz = (f.w, f.h);
        }

        if child.is_none() {
            match spawn_ffmpeg(f.w, f.h, &out) {
                Ok(c) => child = Some(c),
                Err(e) => {
                    eprintln!("[fuckexam] window {node_id}: ffmpeg: {e}");
                    break;
                }
            }
        }

        if let Some(c) = child.as_mut() {
            if let Some(stdin) = c.stdin.as_mut() {
                if let Err(e) = stdin.write_all(&f.bytes) {
                    eprintln!("[fuckexam] window {node_id}: write: {e}");
                    break;
                }
            }
        }
    }

    if let Some(mut c) = child {
        drop(c.stdin.take());
        let _ = c.wait();
    }
}

fn spawn_ffmpeg(w: u32, h: u32, out: &Path) -> Result<Child, String> {
    Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgra",
            "-video_size",
            &format!("{w}x{h}"),
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
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))
}