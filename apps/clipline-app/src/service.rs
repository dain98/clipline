//! The replay-buffer service: a dedicated recorder thread (ddoc §3 — the
//! pipeline is a synchronous pull loop on its own thread) talking to the
//! shell over channels. No Tauri types in here.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clipline_capture::probe::EncoderBackend;
use clipline_capture::traits::{AudioSource, CaptureError, FrameData};
use clipline_capture::windows::nv12::CropRect;
use clipline_capture::windows::wasapi::{WasapiChannelMode, WasapiMixedLoopback};
use clipline_capture::windows::{
    d3d11, find_window_by_title, MftConfig, MftH264Encoder, WasapiLoopback, WgcCapture,
};
use clipline_capture::{even_dimensions, PipelineError, Recorder, RelativeClock};
use clipline_events::{EventKind, MarkerLog};
use clipline_storage::sessions::{session_label, SessionTracker};
use clipline_storage::{enforce_quota, storage_status};

use crate::markers::{self, PollerMsg};

pub enum Cmd {
    Save,
    Stop { announce: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRegion {
    pub display_id: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    PrimaryMonitor,
    WindowTitle(String),
    DisplayRegion(CaptureRegion),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioChannelMode {
    #[default]
    Mono,
    Stereo,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    #[default]
    Auto,
    NvencH264,
    AmfH264,
    QuickSyncH264,
}

impl VideoEncoder {
    fn backend(self) -> Option<EncoderBackend> {
        match self {
            Self::Auto => None,
            Self::NvencH264 => Some(EncoderBackend::Nvenc),
            Self::AmfH264 => Some(EncoderBackend::Amf),
            Self::QuickSyncH264 => Some(EncoderBackend::QuickSync),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioOptions {
    pub output_enabled: bool,
    pub output_device_id: Option<String>,
    pub output_volume: f64,
    pub mic_enabled: bool,
    pub mic_device_id: Option<String>,
    pub mic_volume: f64,
    pub mic_channels: AudioChannelMode,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            output_enabled: true,
            output_device_id: None,
            output_volume: 1.0,
            mic_enabled: false,
            mic_device_id: None,
            mic_volume: 1.0,
            mic_channels: AudioChannelMode::Mono,
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Status {
        recording: bool,
        segments: usize,
        buffered_s: f64,
        buffered_mb: f64,
    },
    Saved {
        path: String,
        seconds: f64,
        markers: usize,
        gc_deleted: usize,
        gc_freed_bytes: u64,
        storage_total_bytes: u64,
        storage_quota_bytes: Option<u64>,
        storage_over_quota: bool,
    },
    Error {
        message: String,
    },
}

pub struct ServiceOptions {
    pub capture_source: CaptureSource,
    /// Root folder for saved media.
    pub media_dir: PathBuf,
    /// Override the League Live Client endpoint (mock servers).
    pub lol_url: Option<String>,
    /// Save Replay trailing window (s).
    pub replay_window_s: f64,
    /// Ring budget in bytes.
    pub buffer_bytes: usize,
    /// Saved clip disk quota. None disables save-time GC.
    pub disk_quota_bytes: Option<u64>,
    pub fps: u32,
    pub bitrate_bps: u32,
    pub video_encoder: VideoEncoder,
    pub audio: AudioOptions,
}

pub const DEFAULT_DISK_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;

impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            capture_source: CaptureSource::PrimaryMonitor,
            media_dir: default_clips_dir(),
            lol_url: None,
            replay_window_s: 60.0,
            // ~2 min at 12 Mbps video + audio headroom.
            buffer_bytes: 220 * 1024 * 1024,
            disk_quota_bytes: Some(DEFAULT_DISK_QUOTA_BYTES),
            fps: 60,
            bitrate_bps: 12_000_000,
            video_encoder: VideoEncoder::Auto,
            audio: AudioOptions::default(),
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
                send_stopped(&event_tx);
            }
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
    let (mut cap, crop) = match &opts.capture_source {
        CaptureSource::WindowTitle(needle) => {
            let hwnd = find_window_by_title(needle)
                .ok_or_else(|| format!("no visible window matching {needle:?}"))?;
            (
                WgcCapture::for_window_on(device.clone(), hwnd, clock).map_err(|e| init(&e))?,
                None,
            )
        }
        CaptureSource::PrimaryMonitor => (
            WgcCapture::primary_monitor_on(device.clone(), clock).map_err(|e| init(&e))?,
            None,
        ),
        CaptureSource::DisplayRegion(region) => {
            let display = clipline_capture::windows::display::display_handle_by_id(
                region.display_id.as_deref(),
            )
            .map_err(|e| init(&e))?;
            let crop = crop_for_region(region, &display.info)?;
            (
                WgcCapture::for_monitor_on(device.clone(), display.handle, clock)
                    .map_err(|e| init(&e))?,
                Some(crop),
            )
        }
    };

    // First frame fixes the capture size; ultrawide scales to ≤2560.
    let first = cap
        .next_frame_timeout(Duration::from_secs(5))
        .map_err(|e| init(&e))?
        .ok_or("capture ended before the first frame")?;
    let FrameData::Gpu(tex) = &first.data else {
        return Err("expected a GPU frame".into());
    };
    let (in_w, in_h) = d3d11::texture_size(tex);
    validate_crop_in_frame(crop, in_w, in_h)?;
    let (source_w, source_h) = crop
        .map(|crop| (crop.width, crop.height))
        .unwrap_or((in_w, in_h));
    let scale = if source_w > 2560 {
        2560.0 / source_w as f64
    } else {
        1.0
    };
    let (enc_w, enc_h) = even_dimensions(
        (source_w as f64 * scale).round() as u32,
        (source_h as f64 * scale).round() as u32,
    );

    let cfg = MftConfig {
        width: enc_w,
        height: enc_h,
        fps: opts.fps,
        bitrate_bps: opts.bitrate_bps,
        encoder_backend: opts.video_encoder.backend(),
    };
    let encoder = match MftH264Encoder::new_with_crop(&device, in_w, in_h, cfg, crop) {
        Ok(encoder) => encoder,
        Err(e) if opts.video_encoder != VideoEncoder::Auto => {
            warn_user(
                events,
                format!(
                    "{:?} encoder unavailable; using Automatic instead: {e}",
                    opts.video_encoder
                ),
            );
            MftH264Encoder::new_with_crop(
                &device,
                in_w,
                in_h,
                MftConfig {
                    encoder_backend: None,
                    ..cfg
                },
                crop,
            )
            .map_err(|e| init(&e))?
        }
        Err(e) => return Err(init(&e)),
    };

    let mut rec = Recorder::new(cap, encoder, opts.buffer_bytes);
    if let Some(audio) = audio_source_from_options(clock, &opts.audio, events) {
        rec = rec.with_audio(audio);
    }
    let (clips_dir, fell_back) = clips_dir_resolved(&opts.media_dir, default_clips_dir)?;
    if fell_back {
        warn_user(
            events,
            format!(
                "media folder {:?} is unavailable; saving to {:?} instead",
                opts.media_dir, clips_dir
            ),
        );
    }
    if is_within_temp(&clips_dir, &std::env::temp_dir()) {
        // Windows reclaims %TEMP% (Storage Sense, Disk Cleanup), so saving here
        // risks silently losing replays. Surface it loudly instead of failing.
        warn_user(
            events,
            format!(
                "saving recordings to a temporary folder {clips_dir:?} that the system may delete; choose a Media folder in Settings"
            ),
        );
    }
    // Saves land in a session folder: one per recorder run, with a dedicated
    // folder per detected match. Folders are created lazily at save time.
    let mut session = SessionTracker::new(local_session_label(false));
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

        while let Ok(msg) = marker_rx.try_recv() {
            match msg {
                PollerMsg::Event(event) => {
                    // GameEnd means the match is over even while the Live
                    // Client API lingers — stop attributing saves to it.
                    if event.kind == EventKind::GameEnd {
                        session.match_ended();
                    }
                    marker_log.push(event);
                }
                PollerMsg::MatchStarted => session.match_started(local_session_label(true)),
                PollerMsg::MatchEnded => session.match_ended(),
            }
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
                    let stamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let session_dir = clips_dir.join(session.current());
                    if let Err(e) = std::fs::create_dir_all(&session_dir) {
                        let _ = events.send(Event::Error {
                            message: format!("create session folder {session_dir:?}: {e}"),
                        });
                        continue;
                    }
                    let path = session_dir.join(format!("clip_{stamp}.mp4"));
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
                            let gc =
                                match enforce_quota(&clips_dir, opts.disk_quota_bytes, Some(&path))
                                {
                                    Ok(report) => report,
                                    Err(e) => {
                                        let _ = events.send(Event::Error {
                                            message: format!("storage cleanup: {e}"),
                                        });
                                        let status =
                                            storage_status(&clips_dir, opts.disk_quota_bytes)
                                                .unwrap_or(clipline_storage::StorageStatus {
                                                    clip_count: 0,
                                                    total_bytes: 0,
                                                    quota_bytes: opts.disk_quota_bytes,
                                                });
                                        clipline_storage::GcReport {
                                            deleted_clips: 0,
                                            freed_bytes: 0,
                                            status,
                                        }
                                    }
                                };
                            let _ = events.send(Event::Saved {
                                path: path.display().to_string(),
                                seconds,
                                markers,
                                gc_deleted: gc.deleted_clips,
                                gc_freed_bytes: gc.freed_bytes,
                                storage_total_bytes: gc.status.total_bytes,
                                storage_quota_bytes: gc.status.quota_bytes,
                                storage_over_quota: gc.status.is_over_quota(),
                            });
                        }
                        Err(e) => {
                            let _ = events.send(Event::Error { message: e });
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                Ok(Cmd::Stop { announce }) => {
                    let _ = rec.finish_stream();
                    if announce {
                        send_stopped(events);
                    }
                    return Ok(());
                }
                Err(TryRecvError::Disconnected) => {
                    let _ = rec.finish_stream();
                    send_stopped(events);
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
    }
    rec.finish_stream().map_err(|e| format!("finish: {e}"))?;
    send_stopped(events);
    Ok(())
}

fn audio_source_from_options(
    clock: RelativeClock,
    options: &AudioOptions,
    events: &Sender<Event>,
) -> Option<Box<dyn AudioSource>> {
    let mic_channels = match options.mic_channels {
        AudioChannelMode::Mono => WasapiChannelMode::Mono,
        AudioChannelMode::Stereo => WasapiChannelMode::Stereo,
    };
    match (options.output_enabled, options.mic_enabled) {
        (true, true) => match WasapiMixedLoopback::start(
            clock,
            Some((options.output_device_id.as_deref(), options.output_volume)),
            Some((
                options.mic_device_id.as_deref(),
                options.mic_volume,
                mic_channels,
            )),
        ) {
            Ok(audio) => Some(Box::new(audio)),
            Err(e) => {
                warn_user(
                    events,
                    format!("mixed audio unavailable; trying single-source fallback: {e}"),
                );
                audio_source_from_options(
                    clock,
                    &AudioOptions {
                        mic_enabled: false,
                        ..options.clone()
                    },
                    events,
                )
                .or_else(|| {
                    audio_source_from_options(
                        clock,
                        &AudioOptions {
                            output_enabled: false,
                            ..options.clone()
                        },
                        events,
                    )
                })
            }
        },
        (true, false) => match WasapiLoopback::start_output(
            clock,
            options.output_device_id.as_deref(),
            options.output_volume,
        ) {
            Ok(audio) => Some(Box::new(audio)),
            Err(e) => {
                warn_user(events, format!("output audio unavailable; continuing: {e}"));
                None
            }
        },
        (false, true) => match WasapiLoopback::start_microphone(
            clock,
            options.mic_device_id.as_deref(),
            options.mic_volume,
            mic_channels,
        ) {
            Ok(audio) => Some(Box::new(audio)),
            Err(e) => {
                warn_user(events, format!("microphone unavailable; continuing: {e}"));
                None
            }
        },
        (false, false) => None,
    }
}

fn warn_user(events: &Sender<Event>, message: String) {
    let _ = events.send(Event::Error { message });
}

fn send_stopped(events: &Sender<Event>) {
    let _ = events.send(Event::Status {
        recording: false,
        segments: 0,
        buffered_s: 0.0,
        buffered_mb: 0.0,
    });
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

fn crop_for_region(
    region: &CaptureRegion,
    display: &clipline_capture::windows::display::DisplayInfo,
) -> Result<CropRect, String> {
    if region.width < 2 || region.height < 2 {
        return Err("capture region must be at least 2x2 pixels".into());
    }
    let local_x = region.x - display.x;
    let local_y = region.y - display.y;
    if local_x < 0
        || local_y < 0
        || local_x as i64 + region.width as i64 > display.width as i64
        || local_y as i64 + region.height as i64 > display.height as i64
    {
        return Err(format!(
            "capture region must fit inside {} ({}x{} at {}, {})",
            display.name, display.width, display.height, display.x, display.y
        ));
    }
    Ok(CropRect {
        x: local_x as u32,
        y: local_y as u32,
        width: region.width,
        height: region.height,
    })
}

fn validate_crop_in_frame(crop: Option<CropRect>, in_w: u32, in_h: u32) -> Result<(), String> {
    let Some(crop) = crop else { return Ok(()) };
    if crop.x + crop.width > in_w || crop.y + crop.height > in_h {
        return Err(format!(
            "capture region {}x{} at {}, {} exceeds captured frame {}x{}",
            crop.width, crop.height, crop.x, crop.y, in_w, in_h
        ));
    }
    Ok(())
}

/// Session label from the local wall clock (folder names should match what
/// the user's file explorer shows, not UTC).
fn local_session_label(league_match: bool) -> String {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    session_label(
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        league_match,
    )
}

pub(crate) fn default_clips_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Videos")
        .join("Clipline")
}

pub(crate) fn clips_dir(media_dir: &Path) -> Result<PathBuf, String> {
    clips_dir_resolved(media_dir, default_clips_dir).map(|(dir, _)| dir)
}

/// Resolve the directory clips are actually written to. The configured folder
/// is used when it can be created; otherwise `fallback` is, so an unplugged
/// external drive degrades to the default folder instead of killing recording
/// and emptying the library. The bool is true when the fallback was taken, so
/// callers with a UI channel can warn the user.
pub(crate) fn clips_dir_resolved(
    media_dir: &Path,
    fallback: impl FnOnce() -> PathBuf,
) -> Result<(PathBuf, bool), String> {
    if std::fs::create_dir_all(media_dir).is_ok() {
        return Ok((media_dir.to_path_buf(), false));
    }
    let dir = fallback();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {dir:?}: {e}"))?;
    Ok((dir, true))
}

/// Whether `dir` lives under the system temp root. Both paths are canonicalized
/// when they exist so a symlinked or short-name temp root still matches.
fn is_within_temp(dir: &Path, temp_dir: &Path) -> bool {
    let normalize = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    normalize(dir).starts_with(normalize(temp_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "clipline-service-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn clips_dir_uses_configured_root_when_creatable() {
        let dir = TestDir::new("configured-root");
        let configured = dir.path().join("media");

        let (resolved, fell_back) =
            clips_dir_resolved(&configured, || panic!("must not fall back")).unwrap();

        assert!(!fell_back);
        assert_eq!(resolved, configured);
        assert!(configured.is_dir());
    }

    #[test]
    fn clips_dir_falls_back_when_configured_root_is_unusable() {
        let dir = TestDir::new("unusable-root");
        // A directory cannot be created under a regular file, so this stands in
        // for an unreachable root (e.g. an unplugged drive).
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        let unusable = blocker.join("clipline");
        let fallback = dir.path().join("fallback");

        let (resolved, fell_back) =
            clips_dir_resolved(&unusable, || fallback.clone()).unwrap();

        assert!(fell_back);
        assert_eq!(resolved, fallback);
        assert!(fallback.is_dir());
    }

    #[test]
    fn temp_guard_flags_clips_inside_temp_root() {
        let dir = TestDir::new("temp-guard");
        let temp_root = dir.path().join("temp");
        let inside = temp_root.join("Videos").join("Clipline");
        std::fs::create_dir_all(&inside).unwrap();

        assert!(is_within_temp(&inside, &temp_root));
    }

    #[test]
    fn temp_guard_allows_clips_outside_temp_root() {
        let dir = TestDir::new("temp-guard-outside");
        let temp_root = dir.path().join("temp");
        let outside = dir.path().join("media").join("Clipline");
        std::fs::create_dir_all(&temp_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        assert!(!is_within_temp(&outside, &temp_root));
    }
}
