//! The replay-buffer service: a dedicated recorder thread (ddoc §3 — the
//! pipeline is a synchronous pull loop on its own thread) talking to the
//! shell over channels. No Tauri types in here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

use clipline_capture::ffmpeg;
use clipline_capture::ffmpeg_encoder::FfmpegVideoEncoder;
use clipline_capture::probe::{
    rank_encoders, EncoderApi, EncoderBackend, EncoderCandidate, EncoderCapability,
    EncoderPreference,
};
use clipline_capture::traits::{AudioSource, CaptureError, Encoder, FrameData};
use clipline_capture::windows::nv12::CropRect;
use clipline_capture::windows::wasapi::{WasapiChannelMode, WasapiMixedLoopback};
use clipline_capture::windows::{
    d3d11, find_window_by_title, mft_probe, window_from_raw_handle, ID3D11Device, MftConfig,
    MftH264Encoder, WasapiLoopback, WgcCapture,
};
use clipline_capture::{
    even_dimensions, PipelineError, Recorder, RelativeClock, ReplayStorageConfig,
};
use clipline_events::{EventKind, MarkerLog};
use clipline_storage::sessions::{session_label, SessionTracker};
use clipline_storage::{enforce_quota, recover_recording_files, storage_status, StorageStatus};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::markers::PollerMsg;

/// Re-exported so the app layer can name codecs without its own
/// clipline-capture import.
pub use clipline_capture::probe::Codec;

const LOW_REPLAY_CACHE_DISK_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
pub enum AudioChannelMode {
    #[default]
    Mono,
    Stereo,
}

/// The user's encoder choice. `Auto` follows the ddoc §4 merit order
/// restricted to player-decodable codecs; the explicit variants force a
/// (backend, codec) pair (still falling back through Auto if it can't open).
/// Legacy saved values (`auto`, `nvenc_h264`, `amf_h264`, `quick_sync_h264`)
/// still deserialize.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    #[default]
    Auto,
    NvencH264,
    NvencHevc,
    NvencAv1,
    AmfH264,
    AmfHevc,
    AmfAv1,
    QuickSyncH264,
    QuickSyncHevc,
    QuickSyncAv1,
    SvtAv1,
}

impl VideoEncoder {
    fn preference(self) -> EncoderPreference {
        let (backend, codec) = match self {
            Self::Auto => return EncoderPreference::Auto,
            Self::NvencH264 => (EncoderBackend::Nvenc, Codec::H264),
            Self::NvencHevc => (EncoderBackend::Nvenc, Codec::Hevc),
            Self::NvencAv1 => (EncoderBackend::Nvenc, Codec::Av1),
            Self::AmfH264 => (EncoderBackend::Amf, Codec::H264),
            Self::AmfHevc => (EncoderBackend::Amf, Codec::Hevc),
            Self::AmfAv1 => (EncoderBackend::Amf, Codec::Av1),
            Self::QuickSyncH264 => (EncoderBackend::QuickSync, Codec::H264),
            Self::QuickSyncHevc => (EncoderBackend::QuickSync, Codec::Hevc),
            Self::QuickSyncAv1 => (EncoderBackend::QuickSync, Codec::Av1),
            Self::SvtAv1 => (EncoderBackend::SvtAv1, Codec::Av1),
        };
        EncoderPreference::Explicit { backend, codec }
    }

    /// The settings/serde id (snake_case). Kept in lockstep with the
    /// `serde(rename_all = "snake_case")` derive by a test.
    fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::NvencH264 => "nvenc_h264",
            Self::NvencHevc => "nvenc_hevc",
            Self::NvencAv1 => "nvenc_av1",
            Self::AmfH264 => "amf_h264",
            Self::AmfHevc => "amf_hevc",
            Self::AmfAv1 => "amf_av1",
            Self::QuickSyncH264 => "quick_sync_h264",
            Self::QuickSyncHevc => "quick_sync_hevc",
            Self::QuickSyncAv1 => "quick_sync_av1",
            Self::SvtAv1 => "svt_av1",
        }
    }

    /// The explicit variant for a (backend, codec) pair, if Clipline exposes
    /// it as a user choice. `None` for combinations with no settings id
    /// (e.g. `MfSoftware`, or SvtAv1 paired with a non-AV1 codec).
    fn from_parts(backend: EncoderBackend, codec: Codec) -> Option<Self> {
        Some(match (backend, codec) {
            (EncoderBackend::Nvenc, Codec::H264) => Self::NvencH264,
            (EncoderBackend::Nvenc, Codec::Hevc) => Self::NvencHevc,
            (EncoderBackend::Nvenc, Codec::Av1) => Self::NvencAv1,
            (EncoderBackend::Amf, Codec::H264) => Self::AmfH264,
            (EncoderBackend::Amf, Codec::Hevc) => Self::AmfHevc,
            (EncoderBackend::Amf, Codec::Av1) => Self::AmfAv1,
            (EncoderBackend::QuickSync, Codec::H264) => Self::QuickSyncH264,
            (EncoderBackend::QuickSync, Codec::Hevc) => Self::QuickSyncHevc,
            (EncoderBackend::QuickSync, Codec::Av1) => Self::QuickSyncAv1,
            (EncoderBackend::SvtAv1, Codec::Av1) => Self::SvtAv1,
            _ => return None,
        })
    }
}

/// The settings id string for a codec, matching the frontend's decode-probe
/// keys ("h264"/"hevc"/"av1").
pub fn codec_id(codec: Codec) -> &'static str {
    match codec {
        Codec::Av1 => "av1",
        Codec::Hevc => "hevc",
        Codec::H264 => "h264",
    }
}

/// One selectable encoder for the Settings dropdown.
#[derive(serde::Serialize)]
pub struct EncoderOption {
    /// VideoEncoder settings id (e.g. "amf_hevc").
    pub id: String,
    /// Human label (e.g. "AMD AMF · HEVC").
    pub name: String,
    /// Codec key the frontend matches against its decode-capability probe.
    pub codec: String,
}

/// The encoders this machine can actually use, as Settings options. Dedupes
/// the same (backend, codec) offered by both MFT and FFmpeg, ordered by the
/// ddoc merit/preference order.
pub fn available_encoder_options() -> Vec<EncoderOption> {
    let mut seen = std::collections::BTreeSet::new();
    let mut options = Vec::new();
    for cap in encoder_capabilities() {
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

/// A short, human-readable label for the active encoder, shown in the
/// sidebar status (e.g. "AMD AMF · H.264" or "Software · AV1").
pub fn encoder_label(candidate: EncoderCandidate) -> String {
    let backend = match candidate.backend {
        EncoderBackend::Nvenc => "NVIDIA NVENC",
        EncoderBackend::Amf => "AMD AMF",
        EncoderBackend::QuickSync => "Intel Quick Sync",
        EncoderBackend::SvtAv1 => "Software",
        EncoderBackend::MfSoftware => "Software",
    };
    let codec = match candidate.codec {
        Codec::Av1 => "AV1",
        Codec::Hevc => "HEVC",
        Codec::H264 => "H.264",
    };
    format!("{backend} · {codec}")
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

impl OutputResolution {
    fn bounds(self) -> Option<(u32, u32)> {
        match self {
            Self::Source => None,
            Self::P1440 => Some((2560, 1440)),
            Self::P1080 => Some((1920, 1080)),
            Self::P720 => Some((1280, 720)),
            Self::P480 => Some((854, 480)),
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
        /// True while a full-session writer is active in addition to the replay ring.
        #[serde(default)]
        full_session: bool,
        /// Active encoder label (e.g. "AMD AMF · H.264"); empty when stopped.
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

pub struct ServiceOptions {
    pub capture_source: CaptureSource,
    /// Built-in game plugin id for the active capture target, if any.
    pub active_game_plugin_id: Option<String>,
    /// Root folder for saved media.
    pub media_dir: PathBuf,
    /// Override the League Live Client endpoint (mock servers).
    pub lol_url: Option<String>,
    /// Save Replay trailing window (s).
    pub replay_window_s: f64,
    /// Ring budget in bytes.
    pub buffer_bytes: usize,
    /// Where the rolling replay buffer stores encoded GOP segments.
    pub replay_storage: ReplayStorageOptions,
    /// Saved clip disk quota. None disables save-time GC.
    pub disk_quota_bytes: Option<u64>,
    pub recording_mode: RecordingMode,
    pub fps: u32,
    pub bitrate_bps: u32,
    pub video_encoder: VideoEncoder,
    pub output_resolution: OutputResolution,
    /// Codecs the in-app review player can decode. `Auto` is restricted to
    /// these so we never record a clip the user can't play back. The
    /// frontend reports the real set (canPlayType); H.264 is always safe.
    pub decodable_codecs: Vec<Codec>,
    pub audio: AudioOptions,
}

pub const DEFAULT_DISK_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;

impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            capture_source: CaptureSource::PrimaryMonitor,
            active_game_plugin_id: None,
            media_dir: default_clips_dir(),
            lol_url: None,
            replay_window_s: 60.0,
            // ~2 min at 12 Mbps video + audio headroom.
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
    let marker_rx = crate::game_plugins::spawn_event_source(
        opts.active_game_plugin_id.as_deref(),
        crate::game_plugins::GameEventSourceContext {
            lol_url: opts.lol_url.clone(),
            recording_t0,
        },
    );
    let mut marker_log = MarkerLog::new();
    let mut cap = match &opts.capture_source {
        CaptureSource::WindowTitle(needle) => {
            let hwnd = find_window_by_title(needle)
                .ok_or_else(|| format!("no visible window matching {needle:?}"))?;
            WgcCapture::for_window_client_on(device.clone(), hwnd, clock).map_err(|e| init(&e))?
        }
        CaptureSource::WindowHandle { hwnd, title } => {
            let hwnd = window_from_raw_handle(*hwnd)
                .ok_or_else(|| format!("game window {title:?} is no longer available"))?;
            WgcCapture::for_window_client_on(device.clone(), hwnd, clock).map_err(|e| init(&e))?
        }
        CaptureSource::PrimaryMonitor => {
            WgcCapture::primary_monitor_on(device.clone(), clock).map_err(|e| init(&e))?
        }
        CaptureSource::DisplayRegion(region) => {
            let display = clipline_capture::windows::display::display_handle_by_id(
                region.display_id.as_deref(),
            )
            .map_err(|e| init(&e))?;
            let crop = crop_for_region(region, &display.info)?;
            WgcCapture::for_monitor_region_on(device.clone(), display.handle, clock, crop)
                .map_err(|e| init(&e))?
        }
    };

    // First frame fixes the capture size; output resolution caps scale down
    // while preserving the captured aspect ratio.
    let first = cap
        .next_frame_timeout(Duration::from_secs(5))
        .map_err(|e| init(&e))?
        .ok_or("capture ended before the first frame")?;
    let FrameData::Gpu(tex) = &first.data else {
        return Err("expected a GPU frame".into());
    };
    let (in_w, in_h) = d3d11::texture_size(tex);
    let (enc_w, enc_h) = output_dimensions(in_w, in_h, opts.output_resolution);

    let (encoder, active) = build_encoder(&device, &opts, in_w, in_h, enc_w, enc_h, events)?;
    let encoder_status = encoder_label(active);

    let replay_cache_dir = prepare_replay_storage(&opts)?;
    let replay_storage = match &opts.replay_storage {
        ReplayStorageOptions::Memory => ReplayStorageConfig::Memory {
            max_bytes: opts.buffer_bytes,
        },
        ReplayStorageOptions::Disk { quota_bytes, .. } => ReplayStorageConfig::Disk {
            max_bytes: usize::try_from(*quota_bytes).unwrap_or(usize::MAX),
            dir: replay_cache_dir
                .clone()
                .ok_or_else(|| "disk replay cache was not prepared".to_string())?,
        },
    };
    let mut rec = Recorder::new_with_replay_storage(cap, encoder, replay_storage)
        .map_err(|e| format!("replay cache: {e}"))?;
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
    recover_abandoned_recordings(&clips_dir, events);
    // Saves land in a session folder: one per recorder run, with a dedicated
    // folder per detected match. Folders are created lazily at save time.
    let mut session = SessionTracker::new(local_session_label(false));
    let mut last_save_end: Option<f64> = None;
    let mut last_status = Instant::now();
    let mut full_session = begin_full_session_recording(
        &mut rec,
        &clips_dir,
        session.current(),
        opts.recording_mode,
        events,
    );
    send_recording_status(events, &rec, &full_session, &encoder_status);

    loop {
        match rec.step() {
            Ok(true) => {}
            Ok(false) => break,
            // Idle screen: WGC delivers nothing — keep serving commands.
            Err(PipelineError::Capture(CaptureError::Timeout(_))) => {}
            Err(e) => {
                let _ = shutdown_recorder(
                    &mut rec,
                    &mut full_session,
                    &marker_log,
                    &clips_dir,
                    &opts,
                    events,
                );
                return Err(format!("recording: {e}"));
            }
        }

        if let Some(marker_rx) = &marker_rx {
            while let Ok(msg) = marker_rx.try_recv() {
                match msg {
                    PollerMsg::Event(event) => {
                        // GameEnd means the match is over even while the Live
                        // Client API lingers; stop attributing saves to it.
                        if event.kind == EventKind::GameEnd {
                            session.match_ended();
                        }
                        marker_log.push(event);
                    }
                    PollerMsg::MatchStarted => session.match_started(local_session_label(true)),
                    PollerMsg::MatchEnded => session.match_ended(),
                }
            }
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            last_status = Instant::now();
            send_recording_status(events, &rec, &full_session, &encoder_status);
            if replay_cache_dir.is_some() {
                ensure_replay_cache_free_space(&opts)?;
            }
        }

        loop {
            match cmd_rx.try_recv() {
                Ok(Cmd::Save) => {
                    let session_dir = clips_dir.join(session.current());
                    if let Err(e) = std::fs::create_dir_all(&session_dir) {
                        let _ = events.send(Event::Error {
                            message: format!("create session folder {session_dir:?}: {e}"),
                        });
                        continue;
                    }
                    let path = unique_media_path(&session_dir, "clip");
                    match save(&rec, &path, opts.replay_window_s, last_save_end) {
                        Ok((end, seconds)) => {
                            last_save_end = Some(end);
                            // Markers inside the saved window ride along as
                            // a sidecar (ddoc §5) — only when there are any.
                            let markers = write_marker_sidecar(
                                events,
                                &marker_log,
                                &path,
                                end - seconds,
                                end,
                            );
                            emit_saved_clip(
                                events, &clips_dir, &path, seconds, markers, false, &opts,
                            );
                        }
                        Err(e) => {
                            let _ = events.send(Event::Error { message: e });
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                Ok(Cmd::Stop { announce }) => {
                    let _ = shutdown_recorder(
                        &mut rec,
                        &mut full_session,
                        &marker_log,
                        &clips_dir,
                        &opts,
                        events,
                    );
                    if announce {
                        send_stopped(events);
                    }
                    return Ok(());
                }
                Err(TryRecvError::Disconnected) => {
                    let _ = shutdown_recorder(
                        &mut rec,
                        &mut full_session,
                        &marker_log,
                        &clips_dir,
                        &opts,
                        events,
                    );
                    send_stopped(events);
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
    }
    if let Some(err) = shutdown_recorder(
        &mut rec,
        &mut full_session,
        &marker_log,
        &clips_dir,
        &opts,
        events,
    ) {
        return Err(err);
    }
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

/// Combined MFT + FFmpeg capabilities. Probing is hardware-stable per
/// process, so it is computed once and reused across recorder restarts
/// (the FFmpeg probe test-encodes, which is too slow to repeat per save).
fn encoder_capabilities() -> &'static [EncoderCapability] {
    use std::sync::OnceLock;
    static CAPS: OnceLock<Vec<EncoderCapability>> = OnceLock::new();
    CAPS.get_or_init(|| {
        let mut caps = mft_probe::enumerate().unwrap_or_default();
        caps.extend(ffmpeg::probe());
        caps
    })
}

fn output_dimensions(in_w: u32, in_h: u32, resolution: OutputResolution) -> (u32, u32) {
    let max_box = resolution.bounds().unwrap_or((2560, u32::MAX));
    let scale = (max_box.0 as f64 / in_w.max(1) as f64)
        .min(max_box.1 as f64 / in_h.max(1) as f64)
        .min(1.0);
    even_dimensions(
        (in_w as f64 * scale).round() as u32,
        (in_h as f64 * scale).round() as u32,
    )
}

/// Build the recorder's video encoder by walking the ranked candidate list
/// until one opens. Returns the boxed encoder and the candidate that won so
/// the caller can report it. Warns the user once if an explicit choice could
/// not be honored and Auto fallback was used instead.
#[allow(clippy::too_many_arguments)]
fn build_encoder(
    device: &ID3D11Device,
    opts: &ServiceOptions,
    in_w: u32,
    in_h: u32,
    enc_w: u32,
    enc_h: u32,
    events: &Sender<Event>,
) -> Result<(Box<dyn Encoder>, EncoderCandidate), String> {
    let preference = opts.video_encoder.preference();
    let candidates = rank_encoders(encoder_capabilities(), &opts.decodable_codecs, preference);
    if candidates.is_empty() {
        return Err("init: no usable video encoder found on this system".into());
    }

    let explicit_target = match preference {
        EncoderPreference::Explicit { backend, codec } => Some((backend, codec)),
        EncoderPreference::Auto => None,
    };
    let ffmpeg_path = ffmpeg::locate();
    let mut last_err = String::new();
    for candidate in &candidates {
        match open_candidate(
            *candidate,
            device,
            opts,
            in_w,
            in_h,
            enc_w,
            enc_h,
            &ffmpeg_path,
        ) {
            Ok(encoder) => {
                // If the user forced a specific encoder/codec and we ended up
                // on a different one — whether the choice failed to open or
                // was never offered (so it isn't even in `candidates`) — tell
                // them we downgraded.
                if let Some((backend, codec)) = explicit_target {
                    if candidate.backend != backend || candidate.codec != codec {
                        let reason = if last_err.is_empty() {
                            "not available on this system".to_string()
                        } else {
                            last_err.clone()
                        };
                        warn_user(
                            events,
                            format!(
                                "{:?} encoder unavailable ({reason}); using {} instead",
                                opts.video_encoder,
                                encoder_label(*candidate)
                            ),
                        );
                    }
                }
                return Ok((encoder, *candidate));
            }
            Err(e) => last_err = e,
        }
    }
    Err(format!(
        "init: no video encoder could be opened: {last_err}"
    ))
}

/// Construct one candidate encoder. MFT uses the zero-copy GPU H.264 path;
/// FFmpeg converts BGRA→NV12 on the GPU and pipes it. `MfSoftware` is modeled
/// by the probe but not yet instantiable, so it is skipped (the walk moves on).
#[allow(clippy::too_many_arguments)]
fn open_candidate(
    candidate: EncoderCandidate,
    device: &ID3D11Device,
    opts: &ServiceOptions,
    in_w: u32,
    in_h: u32,
    enc_w: u32,
    enc_h: u32,
    ffmpeg_path: &Option<PathBuf>,
) -> Result<Box<dyn Encoder>, String> {
    match candidate.api {
        EncoderApi::Mft => {
            if candidate.backend == EncoderBackend::MfSoftware {
                return Err("software H.264 MFT is not yet wired".into());
            }
            let cfg = MftConfig {
                width: enc_w,
                height: enc_h,
                fps: opts.fps,
                bitrate_bps: opts.bitrate_bps,
                encoder_backend: Some(candidate.backend),
            };
            MftH264Encoder::new(device, in_w, in_h, cfg)
                .map(|e| Box::new(e) as Box<dyn Encoder>)
                .map_err(|e| e.to_string())
        }
        EncoderApi::Ffmpeg => {
            let ffmpeg = ffmpeg_path
                .as_deref()
                .ok_or_else(|| "ffmpeg not located".to_string())?;
            FfmpegVideoEncoder::new_on(
                device,
                ffmpeg,
                candidate.backend,
                candidate.codec,
                in_w,
                in_h,
                None,
                enc_w,
                enc_h,
                opts.fps,
                opts.bitrate_bps,
            )
            .map(|e| Box::new(e) as Box<dyn Encoder>)
            .map_err(|e| e.to_string())
        }
    }
}

fn prepare_replay_storage(opts: &ServiceOptions) -> Result<Option<PathBuf>, String> {
    match &opts.replay_storage {
        ReplayStorageOptions::Memory => Ok(None),
        ReplayStorageOptions::Disk { dir, quota_bytes } => {
            if *quota_bytes < 256 * 1024 * 1024 {
                return Err("replay cache quota is too small".into());
            }
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("create replay cache folder {dir:?}: {e}"))?;
            ensure_replay_cache_free_space(opts)?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let run_dir = (0u32..1024)
                .find_map(|attempt| {
                    let candidate = dir.join(format!(
                        "clipline-replay-cache-{stamp}-{}-{attempt}",
                        std::process::id()
                    ));
                    match std::fs::create_dir(&candidate) {
                        Ok(()) => Some(Ok(candidate)),
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                        Err(e) => Some(Err(format!(
                            "create replay cache run folder {candidate:?}: {e}"
                        ))),
                    }
                })
                .unwrap_or_else(|| {
                    Err("create replay cache run folder: too many collisions".into())
                })?;
            Ok(Some(run_dir))
        }
    }
}

fn ensure_replay_cache_free_space(opts: &ServiceOptions) -> Result<(), String> {
    let ReplayStorageOptions::Disk { dir, .. } = &opts.replay_storage else {
        return Ok(());
    };
    let free = available_space_bytes(dir)?;
    if free < LOW_REPLAY_CACHE_DISK_RESERVE_BYTES {
        return Err(format!(
            "replay cache disk is low: {} MiB free, need at least 2048 MiB",
            free / (1024 * 1024)
        ));
    }
    Ok(())
}

fn available_space_bytes(path: &Path) -> Result<u64, String> {
    let mut wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if wide.len() == 1 {
        wide = OsStr::new(".")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
    }
    let mut free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(format!("could not read free space for {path:?}"));
    }
    Ok(free)
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

fn send_recording_status(
    events: &Sender<Event>,
    rec: &Recorder<WgcCapture, Box<dyn Encoder>>,
    full_session: &Option<FullSessionRecording>,
    encoder_status: &str,
) {
    let _ = events.send(Event::Status {
        recording: true,
        segments: rec.ring_len(),
        buffered_s: rec.buffered_span_s(),
        buffered_mb: rec.ring_bytes() as f64 / (1024.0 * 1024.0),
        full_session: full_session.is_some(),
        encoder: encoder_status.to_string(),
    });
}

fn recover_abandoned_recordings(clips_dir: &Path, events: &Sender<Event>) {
    static RECOVERED_THIS_PROCESS: AtomicBool = AtomicBool::new(false);
    if RECOVERED_THIS_PROCESS.swap(true, Ordering::AcqRel) {
        return;
    }
    match recover_recording_files(clips_dir) {
        Ok(report) => {
            if !report.recovered.is_empty() {
                warn_user(
                    events,
                    format!(
                        "recovered {} unfinished full-session recording(s)",
                        report.recovered.len()
                    ),
                );
            }
            if report.deleted_empty > 0 {
                warn_user(
                    events,
                    format!(
                        "cleaned up {} empty unfinished full-session recording(s)",
                        report.deleted_empty
                    ),
                );
            }
        }
        Err(e) => warn_user(events, format!("recover unfinished recordings: {e}")),
    }
}

fn shutdown_recorder(
    rec: &mut Recorder<WgcCapture, Box<dyn Encoder>>,
    full_session: &mut Option<FullSessionRecording>,
    marker_log: &MarkerLog,
    clips_dir: &Path,
    opts: &ServiceOptions,
    events: &Sender<Event>,
) -> Option<String> {
    match rec.finish_stream() {
        Ok(()) => {
            finish_full_session_recording(rec, full_session, marker_log, clips_dir, opts, events);
            None
        }
        Err(e) => {
            let message = format!("finish: {e}");
            warn_user(events, message.clone());
            discard_full_session_recording(
                rec,
                full_session,
                events,
                "full session discarded because recording could not finish cleanly",
            );
            Some(message)
        }
    }
}

fn begin_full_session_recording(
    rec: &mut Recorder<WgcCapture, Box<dyn Encoder>>,
    clips_dir: &Path,
    session_label: &str,
    mode: RecordingMode,
    events: &Sender<Event>,
) -> Option<FullSessionRecording> {
    if mode != RecordingMode::FullSession {
        return None;
    }

    let session_dir = clips_dir.join(session_label);
    if let Err(e) = std::fs::create_dir_all(&session_dir) {
        warn_user(
            events,
            format!("full-session recording unavailable; create {session_dir:?}: {e}"),
        );
        return None;
    }
    let final_path = unique_media_path(&session_dir, "session");
    let temp_path = final_path.with_extension("mp4.recording");
    let file = match std::fs::File::create(&temp_path) {
        Ok(file) => file,
        Err(e) => {
            warn_user(
                events,
                format!("full-session recording unavailable; create {temp_path:?}: {e}"),
            );
            return None;
        }
    };
    if let Err(e) = rec.start_full_session(file) {
        let _ = std::fs::remove_file(&temp_path);
        warn_user(
            events,
            format!("full-session recording unavailable; start writer: {e}"),
        );
        return None;
    }
    Some(FullSessionRecording {
        final_path,
        temp_path,
    })
}

fn finish_full_session_recording(
    rec: &mut Recorder<WgcCapture, Box<dyn Encoder>>,
    recording: &mut Option<FullSessionRecording>,
    marker_log: &MarkerLog,
    clips_dir: &Path,
    opts: &ServiceOptions,
    events: &Sender<Event>,
) {
    let Some(recording) = recording.take() else {
        return;
    };
    match rec.finish_full_session() {
        Ok(Some(summary)) if summary.duration_s.is_finite() && summary.duration_s <= 0.0 => {
            warn_user(
                events,
                "full session ended before any footage was written".into(),
            );
            let _ = std::fs::remove_file(&recording.temp_path);
        }
        Ok(Some(summary)) => {
            let seconds = if summary.duration_s.is_finite() {
                summary.duration_s
            } else {
                warn_user(
                    events,
                    "full session duration was invalid; keeping the recording with an unknown duration"
                        .into(),
                );
                0.0
            };
            if !rename_finalized_session(&recording, events) {
                return;
            }
            let markers = write_marker_sidecar(
                events,
                marker_log,
                &recording.final_path,
                summary.start_s,
                summary.end_s,
            );
            emit_saved_clip(
                events,
                clips_dir,
                &recording.final_path,
                seconds,
                markers,
                true,
                opts,
            );
        }
        Ok(None) => {
            warn_user(
                events,
                "full session ended before any footage was written".into(),
            );
            let _ = std::fs::remove_file(&recording.temp_path);
        }
        Err(e) => {
            warn_user(events, format!("finish full session: {e}"));
            let _ = std::fs::remove_file(&recording.temp_path);
        }
    }
}

fn rename_finalized_session(recording: &FullSessionRecording, events: &Sender<Event>) -> bool {
    match std::fs::rename(&recording.temp_path, &recording.final_path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && recording.final_path.is_file() => {
            true
        }
        Err(e) => {
            warn_user(
                events,
                format!(
                    "finalize full session {:?} -> {:?}: {e}",
                    recording.temp_path, recording.final_path
                ),
            );
            let _ = std::fs::remove_file(&recording.temp_path);
            false
        }
    }
}

fn discard_full_session_recording(
    rec: &mut Recorder<WgcCapture, Box<dyn Encoder>>,
    recording: &mut Option<FullSessionRecording>,
    events: &Sender<Event>,
    reason: &str,
) {
    let Some(recording) = recording.take() else {
        return;
    };
    if let Err(e) = rec.finish_full_session() {
        warn_user(events, format!("stop full-session writer: {e}"));
    }
    let _ = std::fs::remove_file(&recording.temp_path);
    warn_user(events, reason.to_string());
}

struct FullSessionRecording {
    final_path: PathBuf,
    temp_path: PathBuf,
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

fn write_marker_sidecar(
    events: &Sender<Event>,
    marker_log: &MarkerLog,
    path: &Path,
    start_s: f64,
    end_s: f64,
) -> usize {
    let clip = marker_log.clip_markers(start_s, end_s);
    let markers = clip.markers.len();
    if markers == 0 {
        return 0;
    }
    match serde_json::to_string_pretty(&clip) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path.with_extension("markers.json"), json) {
                warn_user(events, format!("write marker sidecar for {path:?}: {e}"));
            }
        }
        Err(e) => warn_user(
            events,
            format!("serialize marker sidecar for {path:?}: {e}"),
        ),
    }
    markers
}

fn emit_saved_clip(
    events: &Sender<Event>,
    clips_dir: &Path,
    path: &Path,
    seconds: f64,
    markers: usize,
    full_session: bool,
    opts: &ServiceOptions,
) {
    let report = match enforce_quota(clips_dir, opts.disk_quota_bytes, Some(path)) {
        Ok(report) => report,
        Err(e) => {
            warn_user(events, format!("storage cleanup: {e}"));
            let status = storage_status(clips_dir, opts.disk_quota_bytes).unwrap_or_else(|e| {
                warn_user(events, format!("storage status: {e}"));
                StorageStatus {
                    clip_count: 0,
                    total_bytes: 0,
                    quota_bytes: opts.disk_quota_bytes,
                }
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
        full_session,
        gc_deleted: report.deleted_clips,
        gc_freed_bytes: report.freed_bytes,
        storage_total_bytes: report.status.total_bytes,
        storage_quota_bytes: report.status.quota_bytes,
        storage_over_quota: report.status.is_over_quota(),
    });
}

fn save(
    rec: &Recorder<WgcCapture, Box<dyn Encoder>>,
    path: &Path,
    window_s: f64,
    exclude_before_s: Option<f64>,
) -> Result<(f64, f64), String> {
    let saved_from = rec
        .save_window_bounds(window_s, exclude_before_s)
        .map(|(start, _)| start);
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

    #[test]
    fn video_encoder_id_matches_serde_serialization() {
        // The Settings dropdown sends EncoderOption.id; settings.rs maps it
        // back through VideoEncoder's snake_case serde. id() must stay in
        // lockstep with that derive, including the new codec variants.
        for enc in [
            VideoEncoder::Auto,
            VideoEncoder::NvencH264,
            VideoEncoder::NvencHevc,
            VideoEncoder::NvencAv1,
            VideoEncoder::AmfH264,
            VideoEncoder::AmfHevc,
            VideoEncoder::AmfAv1,
            VideoEncoder::QuickSyncH264,
            VideoEncoder::QuickSyncHevc,
            VideoEncoder::QuickSyncAv1,
            VideoEncoder::SvtAv1,
        ] {
            let serialized = serde_json::to_string(&enc).unwrap();
            assert_eq!(serialized, format!("\"{}\"", enc.id()));
        }
    }

    #[test]
    fn from_parts_round_trips_through_preference() {
        // Every explicit option maps back to the same (backend, codec).
        for (backend, codec) in [
            (EncoderBackend::Amf, Codec::Hevc),
            (EncoderBackend::Nvenc, Codec::Av1),
            (EncoderBackend::SvtAv1, Codec::Av1),
        ] {
            let enc = VideoEncoder::from_parts(backend, codec).unwrap();
            assert_eq!(
                enc.preference(),
                EncoderPreference::Explicit { backend, codec }
            );
        }
        assert!(VideoEncoder::from_parts(EncoderBackend::MfSoftware, Codec::H264).is_none());
        assert!(VideoEncoder::from_parts(EncoderBackend::SvtAv1, Codec::H264).is_none());
    }

    #[test]
    fn output_dimensions_scale_down_to_selected_resolution() {
        assert_eq!(
            output_dimensions(2560, 1440, OutputResolution::Source),
            (2560, 1440)
        );
        assert_eq!(
            output_dimensions(2560, 1440, OutputResolution::P1080),
            (1920, 1080)
        );
        assert_eq!(
            output_dimensions(2560, 1440, OutputResolution::P720),
            (1280, 720)
        );
    }

    #[test]
    fn output_dimensions_preserve_aspect_and_never_upscale() {
        assert_eq!(
            output_dimensions(1600, 1000, OutputResolution::P1080),
            (1600, 1000)
        );
        assert_eq!(
            output_dimensions(5120, 1440, OutputResolution::P1080),
            (1920, 540)
        );
        assert_eq!(
            output_dimensions(5120, 1440, OutputResolution::Source),
            (2560, 720)
        );
    }

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

        let (resolved, fell_back) = clips_dir_resolved(&unusable, || fallback.clone()).unwrap();

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

    #[test]
    fn finalized_session_rename_accepts_preexisting_final_file() {
        let dir = TestDir::new("session-rename-recovered");
        let final_path = dir.path().join("session.mp4");
        std::fs::write(&final_path, b"mp4").unwrap();
        let recording = FullSessionRecording {
            final_path,
            temp_path: dir.path().join("session.mp4.recording"),
        };
        let (tx, rx) = mpsc::channel();

        assert!(rename_finalized_session(&recording, &tx));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn finalized_session_rename_warns_when_temp_and_final_are_missing() {
        let dir = TestDir::new("session-rename-missing");
        let recording = FullSessionRecording {
            final_path: dir.path().join("session.mp4"),
            temp_path: dir.path().join("session.mp4.recording"),
        };
        let (tx, rx) = mpsc::channel();

        assert!(!rename_finalized_session(&recording, &tx));
        let Event::Error { message } = rx.try_recv().unwrap() else {
            panic!("expected warning");
        };
        assert!(message.contains("finalize full session"));
    }
}
