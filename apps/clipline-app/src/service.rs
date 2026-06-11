//! The replay-buffer service: a dedicated recorder thread (ddoc §3 — the
//! pipeline is a synchronous pull loop on its own thread) talking to the
//! shell over channels. No Tauri types in here.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clipline_capture::traits::{CaptureError, FrameData};
use clipline_capture::windows::{
    d3d11, find_window_by_title, MftConfig, MftH264Encoder, WasapiLoopback, WgcCapture,
};
use clipline_capture::{even_dimensions, PipelineError, Recorder};
use clipline_events::MarkerLog;

use crate::markers;

pub enum Cmd {
    Save,
    Stop,
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Status { recording: bool, segments: usize, buffered_s: f64, buffered_mb: f64 },
    Saved { path: String, seconds: f64, markers: usize },
    Error { message: String },
}

pub struct ServiceOptions {
    /// Capture a window matching this title substring; None = primary monitor.
    pub window_title: Option<String>,
    /// Override the League Live Client endpoint (mock servers).
    pub lol_url: Option<String>,
    /// Save Replay trailing window (s).
    pub replay_window_s: f64,
    /// Ring budget in bytes.
    pub buffer_bytes: usize,
    pub fps: u32,
    pub bitrate_bps: u32,
}

impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            window_title: None,
            lol_url: None,
            replay_window_s: 60.0,
            // ~2 min at 12 Mbps video + audio headroom.
            buffer_bytes: 220 * 1024 * 1024,
            fps: 60,
            bitrate_bps: 12_000_000,
        }
    }
}

pub fn spawn(opts: ServiceOptions) -> (Sender<Cmd>, Receiver<Event>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("clipline-recorder".into())
        .spawn(move || {
            if let Err(e) = run(opts, cmd_rx, &event_tx) {
                let _ = event_tx.send(Event::Error { message: e });
            }
            let _ = event_tx.send(Event::Status {
                recording: false,
                segments: 0,
                buffered_s: 0.0,
                buffered_mb: 0.0,
            });
        })
        .expect("spawn recorder thread");
    (cmd_tx, event_rx)
}

fn run(opts: ServiceOptions, cmd_rx: Receiver<Cmd>, events: &Sender<Event>) -> Result<(), String> {
    let init = |e: &dyn std::fmt::Display| format!("init: {e}");
    let (device, _ctx) = d3d11::create_device().map_err(|e| init(&e))?;
    let clock = WgcCapture::new_clock().map_err(|e| init(&e))?;
    // The wall-clock twin of the capture clock origin (both are QPC under
    // the hood; sampled together they describe one timeline — ddoc §5).
    let recording_t0 = Instant::now();
    let marker_rx = markers::spawn(opts.lol_url.clone(), recording_t0);
    let mut marker_log = MarkerLog::new();
    let mut cap = match &opts.window_title {
        Some(needle) => {
            let hwnd = find_window_by_title(needle)
                .ok_or_else(|| format!("no visible window matching {needle:?}"))?;
            WgcCapture::for_window_on(device.clone(), hwnd, clock).map_err(|e| init(&e))?
        }
        None => WgcCapture::primary_monitor_on(device.clone(), clock).map_err(|e| init(&e))?,
    };

    // First frame fixes the capture size; ultrawide scales to ≤2560.
    let first = cap
        .next_frame_timeout(Duration::from_secs(5))
        .map_err(|e| init(&e))?
        .ok_or("capture ended before the first frame")?;
    let FrameData::Gpu(tex) = &first.data else { return Err("expected a GPU frame".into()) };
    let (in_w, in_h) = d3d11::texture_size(tex);
    let scale = if in_w > 2560 { 2560.0 / in_w as f64 } else { 1.0 };
    let (enc_w, enc_h) = even_dimensions(
        (in_w as f64 * scale).round() as u32,
        (in_h as f64 * scale).round() as u32,
    );

    let cfg =
        MftConfig { width: enc_w, height: enc_h, fps: opts.fps, bitrate_bps: opts.bitrate_bps };
    let encoder = MftH264Encoder::new(&device, in_w, in_h, cfg).map_err(|e| init(&e))?;
    let audio = WasapiLoopback::start(clock).map_err(|e| init(&e))?;

    let mut rec =
        Recorder::new(cap, encoder, opts.buffer_bytes).with_audio(Box::new(audio));
    let clips_dir = clips_dir()?;
    let mut last_save_end: Option<f64> = None;
    let mut last_status = Instant::now();

    loop {
        match rec.step() {
            Ok(true) => {}
            Ok(false) => break,
            // Idle screen: WGC delivers nothing — keep serving commands.
            Err(PipelineError::Capture(CaptureError::Timeout(_))) => {}
            Err(e) => return Err(format!("recording: {e}")),
        }

        while let Ok(event) = marker_rx.try_recv() {
            marker_log.push(event);
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            last_status = Instant::now();
            let (mut span, mut first_pts) = (0.0f64, None::<f64>);
            for seg in rec.ring().segments() {
                first_pts.get_or_insert(seg.pts_start_s);
                span = seg.pts_end_s() - first_pts.unwrap();
            }
            let _ = events.send(Event::Status {
                recording: true,
                segments: rec.ring().len(),
                buffered_s: span,
                buffered_mb: rec.ring().bytes() as f64 / (1024.0 * 1024.0),
            });
        }

        loop {
            match cmd_rx.try_recv() {
                Ok(Cmd::Save) => {
                    let stamp =
                        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                    let path = clips_dir.join(format!("clip_{stamp}.mp4"));
                    match save(&rec, &path, opts.replay_window_s, last_save_end) {
                        Ok((end, seconds)) => {
                            last_save_end = Some(end);
                            // Markers inside the saved window ride along as
                            // a sidecar (ddoc §5) — only when there are any.
                            let clip = marker_log.clip_markers(end - seconds, end);
                            let markers = clip.markers.len();
                            if markers > 0 {
                                let sidecar = path.with_extension("markers.json");
                                if let Ok(json) = serde_json::to_string_pretty(&clip) {
                                    let _ = std::fs::write(sidecar, json);
                                }
                            }
                            let _ = events.send(Event::Saved {
                                path: path.display().to_string(),
                                seconds,
                                markers,
                            });
                        }
                        Err(e) => {
                            let _ = events.send(Event::Error { message: e });
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => {
                    let _ = rec.finish_stream();
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
    }
    rec.finish_stream().map_err(|e| format!("finish: {e}"))
}

fn save(
    rec: &Recorder<WgcCapture, MftH264Encoder>,
    path: &PathBuf,
    window_s: f64,
    exclude_before_s: Option<f64>,
) -> Result<(f64, f64), String> {
    let saved_from = {
        let segs = rec.ring().save_window(window_s, exclude_before_s);
        segs.first().map(|s| s.pts_start_s)
    };
    let file = std::fs::File::create(path).map_err(|e| format!("create {path:?}: {e}"))?;
    let (_, end) = rec
        .save_replay(file, window_s, exclude_before_s)
        .map_err(|e| format!("save: {e}"))?;
    Ok((end, end - saved_from.unwrap_or(end)))
}

fn clips_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE").ok_or("no USERPROFILE")?;
    let dir = PathBuf::from(home).join("Videos").join("Clipline");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {dir:?}: {e}"))?;
    Ok(dir)
}
