use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clipline_capture::ffmpeg;
use clipline_capture::ffmpeg_encoder::FfmpegVideoEncoder;
use clipline_capture::probe::{
    rank_encoders, EncoderApi, EncoderBackend, EncoderCandidate, EncoderCapability,
    EncoderPreference,
};
use clipline_capture::{CaptureEngine, Encoder, Recorder, ReplayStorageConfig};
use clipline_storage::sessions::{session_label, SessionTracker};
use clipline_storage::{enforce_quota, storage_status, StorageStatus};

use crate::macos_capture::{ScreenCaptureKitCapture, ScreenCaptureKitConfig};

use crate::video_encoder::encoder_label;
pub use crate::video_encoder::{codec_id, EncoderOption, VideoEncoder};
pub use clipline_capture::probe::Codec;

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
    WindowHandle { hwnd: isize, title: String },
    DisplayRegion(CaptureRegion),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    #[default]
    Auto,
    Wgc,
    DesktopDuplication,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioChannelMode {
    #[default]
    Mono,
    Stereo,
}

pub fn available_encoder_options() -> Vec<EncoderOption> {
    let mut seen = std::collections::BTreeSet::new();
    let mut options = Vec::new();
    for cap in macos_encoder_capabilities() {
        for &codec in &cap.codecs {
            let Some(encoder) = VideoEncoder::from_parts(cap.backend, codec) else {
                continue;
            };
            if !seen.insert(encoder.id()) {
                continue;
            }
            let candidate = EncoderCandidate {
                api: cap.api,
                backend: cap.backend,
                codec,
            };
            options.push(EncoderOption {
                id: encoder.id().to_string(),
                name: encoder_label(candidate),
                codec: codec_id(codec).to_string(),
            });
        }
    }
    options
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioOptions {
    pub output_enabled: bool,
    pub output_device_id: Option<String>,
    pub output_volume: f64,
    pub split_output_by_process: bool,
    pub mic_enabled: bool,
    pub mic_device_id: Option<String>,
    pub mic_volume: f64,
    pub mic_channels: AudioChannelMode,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            output_enabled: false,
            output_device_id: None,
            output_volume: 1.0,
            split_output_by_process: false,
            mic_enabled: false,
            mic_device_id: None,
            mic_volume: 1.0,
            mic_channels: AudioChannelMode::Mono,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ReplayStorageOptions {
    #[default]
    Memory,
    Disk {
        dir: PathBuf,
        quota_bytes: u64,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordingMode {
    FullSession,
    #[default]
    ReplaysOnly,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum OutputResolution {
    #[default]
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "480p")]
    P480,
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Status {
        recording: bool,
        segments: usize,
        buffered_s: f64,
        buffered_mb: f64,
        #[serde(default)]
        full_session: bool,
        #[serde(default)]
        encoder: String,
    },
    Saved {
        path: String,
        seconds: f64,
        markers: usize,
        #[serde(default)]
        full_session: bool,
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

#[derive(Clone, Debug)]
pub struct ActiveGame {
    pub id: String,
    pub name: String,
}

pub struct ServiceOptions {
    pub capture_source: CaptureSource,
    pub capture_backend: CaptureBackend,
    pub active_game_plugin_id: Option<String>,
    pub active_game: Option<ActiveGame>,
    pub media_dir: PathBuf,
    pub lol_url: Option<String>,
    pub replay_window_s: f64,
    pub buffer_bytes: usize,
    pub replay_storage: ReplayStorageOptions,
    pub disk_quota_bytes: Option<u64>,
    pub recording_mode: RecordingMode,
    pub fps: u32,
    pub bitrate_bps: u32,
    pub video_encoder: VideoEncoder,
    pub output_resolution: OutputResolution,
    pub decodable_codecs: Vec<Codec>,
    pub audio: AudioOptions,
}

pub const DEFAULT_DISK_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;

impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            capture_source: CaptureSource::PrimaryMonitor,
            capture_backend: CaptureBackend::Auto,
            active_game_plugin_id: None,
            active_game: None,
            media_dir: default_clips_dir(),
            lol_url: None,
            replay_window_s: 60.0,
            buffer_bytes: 220 * 1024 * 1024,
            replay_storage: ReplayStorageOptions::Memory,
            disk_quota_bytes: Some(DEFAULT_DISK_QUOTA_BYTES),
            recording_mode: RecordingMode::ReplaysOnly,
            fps: 60,
            bitrate_bps: 12_000_000,
            video_encoder: VideoEncoder::Auto,
            output_resolution: OutputResolution::Source,
            decodable_codecs: vec![Codec::H264],
            audio: AudioOptions::default(),
        }
    }
}

type MacRecorder = Recorder<ScreenCaptureKitCapture, FfmpegVideoEncoder>;

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
        .expect("spawn macOS recorder thread");
    (cmd_tx, event_rx)
}

pub fn ensure_recording_available() -> Result<(), String> {
    let has_videotoolbox = ffmpeg::locate().is_some()
        && macos_encoder_capabilities().iter().any(|cap| {
            cap.api == EncoderApi::Ffmpeg
                && cap.backend == EncoderBackend::VideoToolbox
                && cap.codecs.contains(&Codec::H264)
        });
    if has_videotoolbox {
        Ok(())
    } else {
        Err("macOS recording requires FFmpeg with h264_videotoolbox".into())
    }
}

fn run(opts: ServiceOptions, cmd_rx: Receiver<Cmd>, events: &Sender<Event>) -> Result<(), String> {
    let _ = (
        &opts.capture_backend,
        &opts.active_game_plugin_id,
        &opts.lol_url,
    );
    if let Some(active_game) = &opts.active_game {
        let _ = (&active_game.id, &active_game.name);
    }
    reject_unsupported_options(&opts, events)?;

    let capture_config = ScreenCaptureKitConfig {
        fps: opts.fps,
        max_height: max_height(opts.output_resolution),
    };
    let capture = ScreenCaptureKitCapture::new(capture_config).map_err(|e| format!("init: {e}"))?;
    let stream = capture.stream_info();
    let (encoder, active) = build_encoder(&opts, stream.width, stream.height, events)?;
    let encoder_status = encoder_label(active);
    let replay_storage = replay_storage_config(&opts)?;
    let mut rec = Recorder::new_with_replay_storage(capture, encoder, replay_storage)
        .map_err(|e| format!("replay cache: {e}"))?;
    let clips_root = clips_dir(&opts.media_dir)?;
    let session = SessionTracker::new(local_session_label(false));
    let mut last_status = Instant::now();

    send_recording_status(events, &rec, &encoder_status);

    loop {
        match rec.step() {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => return Err(format!("recording: {e}")),
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            last_status = Instant::now();
            send_recording_status(events, &rec, &encoder_status);
        }

        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => match cmd {
                    Cmd::Save => {
                        let session_dir = clips_root.join(session.current());
                        if let Err(e) = std::fs::create_dir_all(&session_dir) {
                            let _ = events.send(Event::Error {
                                message: format!("create session folder {session_dir:?}: {e}"),
                            });
                            continue;
                        }
                        let path = unique_media_path(&session_dir, "clip");
                        match save(&rec, &path, opts.replay_window_s) {
                            Ok((_, seconds)) => {
                                emit_saved_clip(events, &clips_root, &path, seconds, &opts);
                            }
                            Err(e) => {
                                let _ = events.send(Event::Error { message: e });
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    }
                    Cmd::Stop { announce } => {
                        finish_recorder(&mut rec, events)?;
                        if announce {
                            send_stopped(events);
                        }
                        return Ok(());
                    }
                },
                Err(TryRecvError::Disconnected) => {
                    finish_recorder(&mut rec, events)?;
                    send_stopped(events);
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
    }

    finish_recorder(&mut rec, events)?;
    send_stopped(events);
    Ok(())
}

fn reject_unsupported_options(opts: &ServiceOptions, events: &Sender<Event>) -> Result<(), String> {
    match &opts.capture_source {
        CaptureSource::PrimaryMonitor => {}
        CaptureSource::WindowTitle(_) | CaptureSource::WindowHandle { .. } => {
            let _ = events.send(Event::Error {
                message: "macOS window capture is not implemented in this slice".into(),
            });
            return Err("macOS window capture is not implemented in this slice".into());
        }
        CaptureSource::DisplayRegion(_) => {
            let _ = events.send(Event::Error {
                message: "macOS region capture is not implemented in this slice".into(),
            });
            return Err("macOS region capture is not implemented in this slice".into());
        }
    }

    if opts.recording_mode == RecordingMode::FullSession {
        return Err("macOS full-session recording is not implemented in this slice".into());
    }
    if opts.audio.output_enabled || opts.audio.split_output_by_process || opts.audio.mic_enabled {
        return Err("macOS audio capture is not implemented in this slice".into());
    }
    Ok(())
}

fn build_encoder(
    opts: &ServiceOptions,
    width: u32,
    height: u32,
    events: &Sender<Event>,
) -> Result<(FfmpegVideoEncoder, EncoderCandidate), String> {
    let preference = opts.video_encoder.preference();
    let explicit_target = match preference {
        EncoderPreference::Explicit { backend, codec } => Some((backend, codec)),
        EncoderPreference::Auto => None,
    };
    let candidates = rank_encoders(
        macos_encoder_capabilities(),
        &opts.decodable_codecs,
        preference,
    );
    if candidates.is_empty() {
        return Err("init: no usable macOS H.264 VideoToolbox encoder found".into());
    }
    let ffmpeg = ffmpeg::locate().ok_or_else(|| "init: ffmpeg not located".to_string())?;
    let mut last_err = String::new();
    for candidate in candidates {
        match FfmpegVideoEncoder::new(
            &ffmpeg,
            candidate.backend,
            candidate.codec,
            width,
            height,
            opts.fps,
            opts.bitrate_bps,
        ) {
            Ok(enc) => {
                if let Some((backend, codec)) = explicit_target {
                    if candidate.backend != backend || candidate.codec != codec {
                        warn_user(
                            events,
                            format!(
                                "{:?} encoder unavailable on macOS; using {} instead",
                                opts.video_encoder,
                                encoder_label(candidate)
                            ),
                        );
                    }
                }
                return Ok((enc, candidate));
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let _ = events.send(Event::Error {
        message: format!("macOS VideoToolbox encoder failed: {last_err}"),
    });
    Err(format!(
        "init: macOS VideoToolbox encoder failed: {last_err}"
    ))
}

fn macos_encoder_capabilities() -> &'static [EncoderCapability] {
    use std::sync::OnceLock;

    static CAPS: OnceLock<Vec<EncoderCapability>> = OnceLock::new();
    CAPS.get_or_init(|| {
        ffmpeg::probe()
            .into_iter()
            .filter_map(|mut cap| {
                if cap.api != EncoderApi::Ffmpeg || cap.backend != EncoderBackend::VideoToolbox {
                    return None;
                }
                cap.codecs.retain(|codec| *codec == Codec::H264);
                (!cap.codecs.is_empty()).then_some(cap)
            })
            .collect()
    })
}

fn replay_storage_config(opts: &ServiceOptions) -> Result<ReplayStorageConfig, String> {
    match &opts.replay_storage {
        ReplayStorageOptions::Memory => Ok(ReplayStorageConfig::Memory {
            max_bytes: opts.buffer_bytes,
        }),
        ReplayStorageOptions::Disk { dir, quota_bytes } => {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("create replay cache folder {dir:?}: {e}"))?;
            Ok(ReplayStorageConfig::Disk {
                max_bytes: usize::try_from(*quota_bytes).unwrap_or(usize::MAX),
                dir: dir.clone(),
            })
        }
    }
}

fn finish_recorder(rec: &mut MacRecorder, events: &Sender<Event>) -> Result<(), String> {
    match rec.finish_stream() {
        Ok(()) => Ok(()),
        Err(e) => {
            let message = format!("finish: {e}");
            warn_user(events, message.clone());
            Err(message)
        }
    }
}

fn send_stopped(events: &Sender<Event>) {
    let _ = events.send(Event::Status {
        recording: false,
        segments: 0,
        buffered_s: 0.0,
        buffered_mb: 0.0,
        full_session: false,
        encoder: String::new(),
    });
}

fn send_recording_status(events: &Sender<Event>, rec: &MacRecorder, encoder_status: &str) {
    let _ = events.send(Event::Status {
        recording: true,
        segments: rec.ring_len(),
        buffered_s: rec.buffered_span_s(),
        buffered_mb: rec.ring_bytes() as f64 / (1024.0 * 1024.0),
        full_session: false,
        encoder: encoder_status.to_string(),
    });
}

fn warn_user(events: &Sender<Event>, message: String) {
    let _ = events.send(Event::Error { message });
}

fn emit_saved_clip(
    events: &Sender<Event>,
    clips_dir: &Path,
    path: &Path,
    seconds: f64,
    opts: &ServiceOptions,
) {
    let report = enforce_quota(clips_dir, opts.disk_quota_bytes, Some(path)).unwrap_or_else(|_| {
        let status = storage_status(clips_dir, opts.disk_quota_bytes).unwrap_or(StorageStatus {
            clip_count: 0,
            total_bytes: 0,
            quota_bytes: opts.disk_quota_bytes,
        });
        clipline_storage::GcReport {
            deleted_clips: 0,
            freed_bytes: 0,
            status,
        }
    });
    let _ = events.send(Event::Saved {
        path: path.display().to_string(),
        seconds,
        markers: 0,
        full_session: false,
        gc_deleted: report.deleted_clips,
        gc_freed_bytes: report.freed_bytes,
        storage_total_bytes: report.status.total_bytes,
        storage_quota_bytes: report.status.quota_bytes,
        storage_over_quota: report.status.is_over_quota(),
    });
}

fn save(
    rec: &Recorder<impl CaptureEngine, impl Encoder>,
    path: &Path,
    window_s: f64,
) -> Result<(f64, f64), String> {
    let saved_from = rec
        .save_window_bounds(window_s, None)
        .map(|(start, _)| start);
    let file = std::fs::File::create(path).map_err(|e| format!("create {path:?}: {e}"))?;
    let (_, end) = rec
        .save_replay(file, window_s, None)
        .map_err(|e| format!("save: {e}"))?;
    Ok((end, end - saved_from.unwrap_or(end)))
}

fn unique_media_path(session_dir: &Path, prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for attempt in 0u32..1024 {
        let name = if attempt == 0 {
            format!("{prefix}_{stamp}.mp4")
        } else {
            format!("{prefix}_{stamp}_{attempt}.mp4")
        };
        let candidate = session_dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    let fallback = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    session_dir.join(format!("{prefix}_{fallback}.mp4"))
}

fn max_height(resolution: OutputResolution) -> Option<u32> {
    match resolution {
        OutputResolution::Source => None,
        OutputResolution::P1440 => Some(1440),
        OutputResolution::P1080 => Some(1080),
        OutputResolution::P720 => Some(720),
        OutputResolution::P480 => Some(480),
    }
}

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

pub fn default_clips_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Movies").join("Clipline"))
        .unwrap_or_else(|| std::env::temp_dir().join("Clipline"))
}

pub fn clips_dir(root: &Path) -> Result<PathBuf, String> {
    if root.as_os_str().is_empty() {
        return Err("media folder is required".into());
    }
    std::fs::create_dir_all(root).map_err(|e| format!("create {root:?}: {e}"))?;
    Ok(root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_test_utils::TestDir;

    #[test]
    fn clips_dir_creates_configured_root() {
        let dir = TestDir::new("clipline-service-macos", "configured-root");
        let root = dir.path().join("media");

        let resolved = clips_dir(&root).unwrap();

        assert_eq!(resolved, root);
        assert!(root.is_dir());
    }
}
