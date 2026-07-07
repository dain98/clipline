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
use clipline_capture::traits::{
    AudioSource, CaptureEngine, CaptureError, Encoder, Frame, FrameData,
};
use clipline_capture::windows::nv12::{CropRect, ResizeMode};
use clipline_capture::windows::wasapi::{
    enumerate_output_processes, process_loopback_available, AudioProcessInfo, WasapiChannelMode,
};
use clipline_capture::windows::{
    d3d11, find_window_by_title, mft_probe, window_from_raw_handle, DxgiDuplicationCapture,
    ID3D11Device, MftConfig, MftH264Encoder, WasapiLoopback, WgcCapture,
};
use clipline_capture::{
    even_dimensions, PipelineError, Recorder, RelativeClock, ReplayStorageConfig,
};
use clipline_events::{is_review_event, ClipAudioTrack, EventKind, MarkerLog, PlayerSummary};
use clipline_storage::sessions::{session_label, SessionTracker};
use clipline_storage::{enforce_quota, recover_recording_files, storage_status, StorageStatus};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::markers::PollerMsg;

/// Re-exported so the app layer can name codecs without its own
/// clipline-capture import.
pub use clipline_capture::probe::Codec;

const LOW_REPLAY_CACHE_DISK_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Save,
    SwitchCapture(SwitchCaptureTarget),
    Stop { announce: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchCaptureTarget {
    Window {
        hwnd: isize,
        title: String,
        active_game: Option<ActiveGame>,
        active_game_plugin_id: Option<String>,
        recording_mode: RecordingMode,
    },
    Slate {
        reason: SlateReason,
    },
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlateReason {
    NoEnabledForegroundGame,
    WindowUnavailable,
    SwitchFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaptureKind {
    Game,
    Slate,
}

trait TimedFrameSource {
    fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError>;
}

impl TimedFrameSource for WgcCapture {
    fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        WgcCapture::next_frame_timeout(self, timeout)
    }
}

impl TimedFrameSource for DxgiDuplicationCapture {
    fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        DxgiDuplicationCapture::next_frame_timeout(self, timeout)
    }
}

/// The live screen-capture engine, chosen at recording start. WGC is the
/// default and the only per-window option; DXGI Desktop Duplication is the
/// opt-in borderless display/region backend (issue #42).
#[derive(Clone, Debug, PartialEq, Eq)]
enum SwitchTargetIdentity {
    Window(isize),
    Slate,
}

fn switch_target_identity(target: &SwitchCaptureTarget) -> SwitchTargetIdentity {
    match target {
        SwitchCaptureTarget::Window { hwnd, .. } => SwitchTargetIdentity::Window(*hwnd),
        SwitchCaptureTarget::Slate { .. } => SwitchTargetIdentity::Slate,
    }
}

enum LiveBackend {
    Wgc(WgcCapture),
    Dxgi(DxgiDuplicationCapture),
    Slate(SlateCapture),
}

struct SlateCapture {
    frame: FrameData,
    next_pts_s: f64,
    frame_interval_s: f64,
    frame_interval: Duration,
    next_frame_at: Option<Instant>,
}

impl SlateCapture {
    fn new(device: &ID3D11Device, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        let pixels = privacy_slate_bgra(width, height);
        let texture = d3d11::create_bgra_texture_from_pixels(device, width, height, &pixels)
            .map_err(|e| format!("slate texture: {e}"))?;
        Ok(Self {
            frame: FrameData::Gpu(texture),
            next_pts_s: 0.0,
            frame_interval_s: 1.0 / fps.max(1) as f64,
            frame_interval: Duration::from_secs_f64(1.0 / fps.max(1) as f64),
            next_frame_at: None,
        })
    }
}

impl TimedFrameSource for LiveBackend {
    fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        match self {
            LiveBackend::Wgc(cap) => cap.next_frame_timeout(timeout),
            LiveBackend::Dxgi(cap) => cap.next_frame_timeout(timeout),
            LiveBackend::Slate(cap) => cap.next_frame_timeout(timeout),
        }
    }
}

impl TimedFrameSource for SlateCapture {
    fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        if let Some(next_frame_at) = self.next_frame_at {
            let now = Instant::now();
            if now < next_frame_at {
                let wait = next_frame_at.duration_since(now);
                if wait > timeout {
                    if !timeout.is_zero() {
                        std::thread::sleep(timeout);
                    }
                    return Err(CaptureError::Timeout(timeout));
                }
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
            }
        }
        let pts_s = self.next_pts_s;
        self.next_pts_s += self.frame_interval_s;
        self.next_frame_at = Some(Instant::now() + self.frame_interval);
        Ok(Some(Frame {
            pts_s,
            data: self.frame.clone(),
        }))
    }
}

/// WGC can go quiet when the captured image is unchanged. Keep the encoder
/// moving at the configured cadence by reusing the latest texture during
/// idle gaps, so GOP/keyframe spacing follows wall-clock time.
struct CadencedCapture<C> {
    inner: C,
    frame_interval: Duration,
    frame_interval_s: f64,
    last_data: Option<FrameData>,
    last_emit_pts_s: Option<f64>,
    next_pts_s: Option<f64>,
}

impl<C> CadencedCapture<C> {
    fn new(inner: C, fps: u32, seed: &Frame) -> Self {
        let frame_interval_s = 1.0 / fps.max(1) as f64;
        Self {
            inner,
            frame_interval: Duration::from_secs_f64(frame_interval_s),
            frame_interval_s,
            last_data: Some(seed.data.clone()),
            last_emit_pts_s: Some(seed.pts_s),
            next_pts_s: Some(seed.pts_s + frame_interval_s),
        }
    }

    fn remember(&mut self, frame: &Frame) {
        self.last_data = Some(frame.data.clone());
        self.last_emit_pts_s = Some(frame.pts_s);
        self.next_pts_s = Some(frame.pts_s + self.frame_interval_s);
    }
}

impl<C: TimedFrameSource> CaptureEngine for CadencedCapture<C> {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        match self.inner.next_frame_timeout(self.frame_interval) {
            Ok(Some(mut frame)) => {
                let min_pts = self
                    .last_emit_pts_s
                    .map(|last| last + 1e-4)
                    .unwrap_or(frame.pts_s);
                let expected_pts = self.next_pts_s.unwrap_or(min_pts);
                frame.pts_s = frame.pts_s.max(min_pts).max(expected_pts);
                self.remember(&frame);
                Ok(Some(frame))
            }
            Ok(None) => Ok(None),
            Err(CaptureError::Timeout(_)) => {
                let Some(data) = self.last_data.clone() else {
                    return Err(CaptureError::Timeout(self.frame_interval));
                };
                let min_pts = self.last_emit_pts_s.map(|last| last + 1e-4).unwrap_or(0.0);
                let pts_s = self.next_pts_s.unwrap_or(min_pts).max(min_pts);
                self.last_emit_pts_s = Some(pts_s);
                self.next_pts_s = Some(pts_s + self.frame_interval_s);
                Ok(Some(Frame { pts_s, data }))
            }
            Err(e) => Err(e),
        }
    }
}

fn privacy_slate_bgra(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = vec![0x12; width as usize * height as usize * 4];
    for px in pixels.chunks_exact_mut(4) {
        px[0] = 0x1b;
        px[1] = 0x18;
        px[2] = 0x14;
        px[3] = 0xff;
    }
    let banner_w = (width * 3 / 5).max(12).min(width);
    let banner_h = (height / 7).max(8).min(height);
    let left = (width - banner_w) / 2;
    let top = (height - banner_h) / 2;
    fill_bgra_rect(
        &mut pixels,
        width,
        left,
        top,
        banner_w,
        banner_h,
        [0x28, 0x2f, 0x39, 0xff],
    );
    fill_bgra_rect(
        &mut pixels,
        width,
        left + banner_w / 12,
        top + banner_h / 3,
        banner_w * 5 / 6,
        (banner_h / 6).max(2),
        [0x80, 0x88, 0x94, 0xff],
    );
    pixels
}

fn fill_bgra_rect(
    pixels: &mut [u8],
    stride_width: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    bgra: [u8; 4],
) {
    for row in y..y.saturating_add(h) {
        for col in x..x.saturating_add(w) {
            let idx = ((row * stride_width + col) * 4) as usize;
            if idx + 4 <= pixels.len() {
                pixels[idx..idx + 4].copy_from_slice(&bgra);
            }
        }
    }
}

struct SwitchableLiveCapture {
    state: std::sync::Arc<std::sync::Mutex<SwitchableLiveCaptureState>>,
}

struct SwitchableCaptureController {
    state: std::sync::Arc<std::sync::Mutex<SwitchableLiveCaptureState>>,
}

struct SwitchableLiveCaptureState {
    device: ID3D11Device,
    clock: RelativeClock,
    fps: u32,
    canvas: (u32, u32),
    active: LiveBackend,
    identity: SwitchTargetIdentity,
}

impl SwitchableLiveCapture {
    fn new(
        device: ID3D11Device,
        clock: RelativeClock,
        fps: u32,
        canvas: (u32, u32),
        active: LiveBackend,
        identity: SwitchTargetIdentity,
    ) -> (Self, SwitchableCaptureController) {
        let state = std::sync::Arc::new(std::sync::Mutex::new(SwitchableLiveCaptureState {
            device,
            clock,
            fps,
            canvas,
            active,
            identity,
        }));
        (
            Self {
                state: state.clone(),
            },
            SwitchableCaptureController { state },
        )
    }
}

impl TimedFrameSource for SwitchableLiveCapture {
    fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CaptureError::DeviceLost("switchable capture lock poisoned".into()))?;
        state.active.next_frame_timeout(timeout)
    }
}

impl SwitchableCaptureController {
    fn switch_to(&self, target: &SwitchCaptureTarget) -> Result<(), String> {
        let next_identity = switch_target_identity(target);
        let mut state = self
            .state
            .lock()
            .map_err(|_| "switchable capture lock poisoned".to_string())?;
        if state.identity == next_identity {
            return Ok(());
        }
        let slate = LiveBackend::Slate(SlateCapture::new(
            &state.device,
            state.canvas.0,
            state.canvas.1,
            state.fps,
        )?);
        let old = std::mem::replace(&mut state.active, slate);
        state.identity = SwitchTargetIdentity::Slate;
        drop(old);
        let next = match target {
            SwitchCaptureTarget::Window { hwnd, title, .. } => {
                let hwnd = window_from_raw_handle(*hwnd)
                    .ok_or_else(|| format!("game window {title:?} is no longer available"))?;
                let cap = WgcCapture::for_window_client_on(state.device.clone(), hwnd, state.clock)
                    .map_err(|e| e.to_string())?;
                LiveBackend::Wgc(cap)
            }
            SwitchCaptureTarget::Slate { .. } => return Ok(()),
        };
        state.active = next;
        state.identity = next_identity;
        Ok(())
    }
}

type LiveCapture = CadencedCapture<SwitchableLiveCapture>;
type LiveRecorder = Recorder<LiveCapture, Box<dyn Encoder>>;

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

/// Which screen-capture backend to use for display/region capture (issue #42).
/// `Auto` and `Wgc` both use Windows Graphics Capture today; `Wgc` is the
/// persisted force-WGC escape hatch that survives any future change to `Auto`.
/// `DesktopDuplication` uses DXGI Desktop Duplication, which has no Windows 10
/// privacy border but is display/region only (never per-window) and silently
/// falls back to WGC when it can't initialize (multi-GPU, rotated display, etc).
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

/// The user's encoder choice. `Auto` prefers H.264 for playback compatibility while
/// respecting backend merit order within a codec; the explicit variants force a
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
    pub split_output_by_process: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputResolutionBounds {
    pub width: u32,
    pub height: u32,
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
        capture_kind: CaptureKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slate_reason: Option<SlateReason>,
    },
    Saved {
        path: String,
        seconds: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recording_start_unix: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recording_end_unix: Option<i64>,
        markers: usize,
        #[serde(default)]
        full_session: bool,
        gc_deleted: usize,
        gc_freed_bytes: u64,
        storage_total_bytes: u64,
        storage_quota_bytes: Option<u64>,
        storage_over_quota: bool,
    },
    FocusFollowRetry {
        hwnd: isize,
    },
    Error {
        message: String,
    },
}

/// The game a recording run is attributed to (plugin or custom), recorded
/// alongside saved clips so the library can show its icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveGame {
    pub id: String,
    pub name: String,
}

pub struct ServiceOptions {
    pub capture_source: CaptureSource,
    pub focus_follow_enabled: bool,
    /// Screen-capture backend preference for display/region capture.
    pub capture_backend: CaptureBackend,
    /// Built-in game plugin id for the active capture target, if any.
    pub active_game_plugin_id: Option<String>,
    /// Active game (plugin or custom) for clip attribution. Unlike
    /// `active_game_plugin_id`, this is set for custom games too.
    pub active_game: Option<ActiveGame>,
    /// Root folder for saved media.
    pub media_dir: PathBuf,
    /// Whether this run should recover leftover `.mp4.recording` files.
    /// Internal recorder restarts disable this to avoid stealing the previous
    /// recorder thread's active full-session temp file while it is shutting down.
    pub recover_abandoned_recordings: bool,
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
    pub output_resolution_bounds: Option<OutputResolutionBounds>,
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
            focus_follow_enabled: false,
            capture_backend: CaptureBackend::Auto,
            active_game_plugin_id: None,
            active_game: None,
            media_dir: default_clips_dir(),
            recover_abandoned_recordings: true,
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
            output_resolution_bounds: None,
            decodable_codecs: vec![Codec::H264],
            audio: AudioOptions::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FocusRunState {
    capture_kind: CaptureKind,
    active_game: Option<ActiveGame>,
    active_game_plugin_id: Option<String>,
    recording_mode: RecordingMode,
    slate_reason: Option<SlateReason>,
}

impl FocusRunState {
    fn from_options(opts: &ServiceOptions) -> Self {
        let is_focus_follow_slate = opts.focus_follow_enabled && opts.active_game.is_none();
        Self {
            capture_kind: if is_focus_follow_slate {
                CaptureKind::Slate
            } else {
                CaptureKind::Game
            },
            active_game: opts.active_game.clone(),
            active_game_plugin_id: opts.active_game_plugin_id.clone(),
            recording_mode: opts.recording_mode,
            slate_reason: is_focus_follow_slate.then_some(SlateReason::NoEnabledForegroundGame),
        }
    }

    fn apply_target(&mut self, target: &SwitchCaptureTarget) {
        match target {
            SwitchCaptureTarget::Window {
                active_game,
                active_game_plugin_id,
                recording_mode,
                ..
            } => {
                self.capture_kind = CaptureKind::Game;
                self.active_game = active_game.clone();
                self.active_game_plugin_id = active_game_plugin_id.clone();
                self.recording_mode = *recording_mode;
                self.slate_reason = None;
            }
            SwitchCaptureTarget::Slate { reason } => {
                self.capture_kind = CaptureKind::Slate;
                self.active_game = None;
                self.active_game_plugin_id = None;
                self.recording_mode = RecordingMode::ReplaysOnly;
                self.slate_reason = Some(*reason);
            }
        }
    }

    fn accepts_plugin_markers(&self, plugin_id: &str) -> bool {
        self.capture_kind == CaptureKind::Game
            && self.active_game_plugin_id.as_deref() == Some(plugin_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FullSessionTransition {
    None,
    Start,
    Finish,
    FinishThenStart,
}

#[derive(Default)]
struct CaptureSwitchLog {
    entries: Vec<CaptureSwitchEntry>,
}

struct CaptureSwitchEntry {
    pts_s: f64,
    state: FocusRunState,
}

impl CaptureSwitchLog {
    fn push(&mut self, pts_s: f64, state: &FocusRunState) {
        self.entries.push(CaptureSwitchEntry {
            pts_s,
            state: state.clone(),
        });
    }

    fn clip_switches(&self, start_s: f64, end_s: f64) -> Vec<clipline_events::ClipSourceSwitch> {
        self.entries
            .iter()
            .filter(|entry| entry.pts_s >= start_s && entry.pts_s < end_s)
            .map(|entry| clipline_events::ClipSourceSwitch {
                t_s: entry.pts_s - start_s,
                kind: match entry.state.capture_kind {
                    CaptureKind::Game => "game".into(),
                    CaptureKind::Slate => "slate".into(),
                },
                game_id: entry.state.active_game.as_ref().map(|game| game.id.clone()),
                game_name: entry
                    .state
                    .active_game
                    .as_ref()
                    .map(|game| game.name.clone()),
                slate_reason: entry.state.slate_reason.map(|reason| match reason {
                    SlateReason::NoEnabledForegroundGame => "no_enabled_foreground_game".into(),
                    SlateReason::WindowUnavailable => "window_unavailable".into(),
                    SlateReason::SwitchFailed => "switch_failed".into(),
                }),
            })
            .collect()
    }
}

fn full_session_transition(
    active_recording_game_id: Option<&str>,
    _old_state: &FocusRunState,
    next_state: &FocusRunState,
) -> FullSessionTransition {
    let next_full = next_state.capture_kind == CaptureKind::Game
        && next_state.recording_mode == RecordingMode::FullSession;
    let next_id = next_state.active_game.as_ref().map(|game| game.id.as_str());
    match (active_recording_game_id, next_full, next_id) {
        (None, true, Some(_)) => FullSessionTransition::Start,
        (Some(_), false, _) if next_state.capture_kind == CaptureKind::Game => {
            FullSessionTransition::Finish
        }
        (Some(current), true, Some(next)) if current != next => {
            FullSessionTransition::FinishThenStart
        }
        _ => FullSessionTransition::None,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkerSourceKey {
    Plugin(String),
    LegacyLeaguePoller,
    NoMarkerSource,
}

fn marker_source_key_for<F>(plugin_id: Option<&str>, has_event_source: F) -> MarkerSourceKey
where
    F: Fn(Option<&str>) -> bool,
{
    match plugin_id {
        Some(id) if has_event_source(Some(id)) => MarkerSourceKey::Plugin(id.into()),
        Some(_) => MarkerSourceKey::NoMarkerSource,
        None => MarkerSourceKey::LegacyLeaguePoller,
    }
}

fn marker_source_key(plugin_id: Option<&str>) -> MarkerSourceKey {
    marker_source_key_for(plugin_id, crate::game_plugins::has_event_source)
}

#[derive(Default)]
struct PlayerSummaryState {
    in_match: bool,
    active_replay: Option<PlayerSummary>,
    full_session: Option<PlayerSummary>,
}

impl PlayerSummaryState {
    fn match_started(&mut self) {
        self.in_match = true;
        self.active_replay = None;
        self.full_session = None;
    }

    fn update(&mut self, summary: PlayerSummary) {
        if self.in_match {
            self.active_replay = Some(summary.clone());
        }
        if self.in_match || self.full_session.is_some() {
            self.full_session = Some(summary);
        }
    }

    fn match_ended(&mut self) {
        self.in_match = false;
        self.active_replay = None;
    }

    fn active_replay_summary(&self) -> Option<&PlayerSummary> {
        self.active_replay.as_ref()
    }

    fn full_session_summary(&self) -> Option<&PlayerSummary> {
        self.active_replay.as_ref().or(self.full_session.as_ref())
    }
}

fn spawn_marker_source(
    source_key: &MarkerSourceKey,
    plugin_id: Option<&str>,
    lol_url: Option<String>,
    recording_t0: Instant,
) -> Receiver<PollerMsg> {
    let context = crate::game_plugins::GameEventSourceContext {
        lol_url,
        recording_t0,
    };
    match source_key {
        MarkerSourceKey::Plugin(_) => crate::game_plugins::spawn_event_source(plugin_id, context)
            .expect("marker source key checked plugin event source"),
        MarkerSourceKey::LegacyLeaguePoller => {
            crate::markers::spawn(context.lol_url, context.recording_t0)
        }
        MarkerSourceKey::NoMarkerSource => {
            let (_tx, rx) = mpsc::channel();
            rx
        }
    }
}

struct MarkerRuntime {
    source_key: MarkerSourceKey,
    marker_rx: Receiver<PollerMsg>,
    lol_url: Option<String>,
    recording_t0: Instant,
}

impl MarkerRuntime {
    fn new(opts: &ServiceOptions, recording_t0: Instant) -> Self {
        let source_key = marker_source_key(opts.active_game_plugin_id.as_deref());
        let marker_rx = spawn_marker_source(
            &source_key,
            opts.active_game_plugin_id.as_deref(),
            opts.lol_url.clone(),
            recording_t0,
        );
        Self {
            source_key,
            marker_rx,
            lol_url: opts.lol_url.clone(),
            recording_t0,
        }
    }

    fn sync_to_plugin(&mut self, plugin_id: Option<&str>) {
        let next_key = marker_source_key(plugin_id);
        if next_key == self.source_key {
            return;
        }
        self.marker_rx = spawn_marker_source(
            &next_key,
            plugin_id,
            self.lol_url.clone(),
            self.recording_t0,
        );
        self.source_key = next_key;
    }

    fn try_recv(&self) -> Result<PollerMsg, TryRecvError> {
        self.marker_rx.try_recv()
    }
}

/// First-frame wait: the frame that fixes the capture size. Matches WGC's own
/// 5 s budget; an idle desktop can legitimately take this long to update.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// Build the capture engine and pull its first frame (which fixes the capture
/// size). DXGI Desktop Duplication is attempted only when the user explicitly
/// selected it for a display/region source; any DXGI failure — at construction
/// or on the first frame — is logged to stderr and silently falls back to WGC,
/// so recording always starts (the user chose silent fallback over a warning).
fn open_screen_capture(
    device: &ID3D11Device,
    clock: RelativeClock,
    source: &CaptureSource,
    backend: CaptureBackend,
    events: &Sender<Event>,
) -> Result<(LiveBackend, Frame), String> {
    if backend == CaptureBackend::DesktopDuplication
        && matches!(
            source,
            CaptureSource::PrimaryMonitor | CaptureSource::DisplayRegion(_)
        )
    {
        match open_dxgi(device, clock, source, events) {
            Ok(pair) => return Ok(pair),
            Err(e) => eprintln!(
                "clipline: Desktop Duplication unavailable ({e}); using Windows Graphics Capture"
            ),
        }
    }

    let init = |e: &dyn std::fmt::Display| format!("init: {e}");
    let mut cap = open_wgc(device, clock, source, events)?;
    let first = cap
        .next_frame_timeout(FIRST_FRAME_TIMEOUT)
        .map_err(|e| init(&e))?
        .ok_or("capture ended before the first frame")?;
    Ok((LiveBackend::Wgc(cap), first))
}

/// DXGI Desktop Duplication for a display/region source (never per-window). The
/// monitor handle is resolved inside the capture crate / via `display`, both of
/// which use the `windows`-crate `HMONITOR` the constructors expect.
fn open_dxgi(
    device: &ID3D11Device,
    clock: RelativeClock,
    source: &CaptureSource,
    events: &Sender<Event>,
) -> Result<(LiveBackend, Frame), String> {
    let mut cap = match source {
        CaptureSource::PrimaryMonitor => {
            DxgiDuplicationCapture::primary_monitor_on(device.clone(), clock)
                .map_err(|e| e.to_string())?
        }
        CaptureSource::DisplayRegion(region) => {
            let (display, recovered) =
                clipline_capture::windows::display::display_handle_by_id_or_primary(
                    region.display_id.as_deref(),
                )
                .map_err(|e| e.to_string())?;
            let (crop, crop_recovered) =
                crop_for_region_or_full_display(region, &display.info, recovered)?;
            warn_capture_display_recovery(events, region, &display.info, recovered, crop_recovered);
            DxgiDuplicationCapture::for_monitor_region_on(
                device.clone(),
                display.handle,
                clock,
                crop,
            )
            .map_err(|e| e.to_string())?
        }
        // Window sources never reach here (guarded by open_screen_capture).
        _ => return Err("Desktop Duplication cannot capture a single window".into()),
    };
    let first = cap
        .next_frame_timeout(FIRST_FRAME_TIMEOUT)
        .map_err(|e| e.to_string())?
        .ok_or("Desktop Duplication ended before the first frame")?;
    Ok((LiveBackend::Dxgi(cap), first))
}

/// Windows Graphics Capture for any source (the default, and the only
/// per-window option).
fn open_wgc(
    device: &ID3D11Device,
    clock: RelativeClock,
    source: &CaptureSource,
    events: &Sender<Event>,
) -> Result<WgcCapture, String> {
    let init = |e: &dyn std::fmt::Display| format!("init: {e}");
    match source {
        CaptureSource::WindowTitle(needle) => {
            let hwnd = find_window_by_title(needle)
                .ok_or_else(|| format!("no visible window matching {needle:?}"))?;
            WgcCapture::for_window_client_on(device.clone(), hwnd, clock).map_err(|e| init(&e))
        }
        CaptureSource::WindowHandle { hwnd, title } => {
            let hwnd = window_from_raw_handle(*hwnd)
                .ok_or_else(|| format!("game window {title:?} is no longer available"))?;
            WgcCapture::for_window_client_on(device.clone(), hwnd, clock).map_err(|e| init(&e))
        }
        CaptureSource::PrimaryMonitor => {
            WgcCapture::primary_monitor_on(device.clone(), clock).map_err(|e| init(&e))
        }
        CaptureSource::DisplayRegion(region) => {
            let (display, recovered) =
                clipline_capture::windows::display::display_handle_by_id_or_primary(
                    region.display_id.as_deref(),
                )
                .map_err(|e| init(&e))?;
            let (crop, crop_recovered) =
                crop_for_region_or_full_display(region, &display.info, recovered)?;
            warn_capture_display_recovery(events, region, &display.info, recovered, crop_recovered);
            WgcCapture::for_monitor_region_on(device.clone(), display.handle, clock, crop)
                .map_err(|e| init(&e))
        }
    }
}

fn canvas_dimensions_from_capture_source(source: &CaptureSource) -> Result<(u32, u32), String> {
    match source {
        CaptureSource::DisplayRegion(region) => Ok((region.width, region.height)),
        CaptureSource::PrimaryMonitor => {
            let (display, _) =
                clipline_capture::windows::display::display_handle_by_id_or_primary(None)
                    .map_err(|e| e.to_string())?;
            Ok((display.info.width, display.info.height))
        }
        CaptureSource::WindowTitle(_) | CaptureSource::WindowHandle { .. } => Ok((1920, 1080)),
    }
}

fn initial_canvas_dimensions(
    opts: &ServiceOptions,
    _device: &ID3D11Device,
    _events: &Sender<Event>,
) -> Result<(u32, u32), String> {
    canvas_dimensions_from_capture_source(&opts.capture_source)
}

fn run(opts: ServiceOptions, cmd_rx: Receiver<Cmd>, events: &Sender<Event>) -> Result<(), String> {
    let init = |e: &dyn std::fmt::Display| format!("init: {e}");
    let (device, _ctx) = d3d11::create_device().map_err(|e| init(&e))?;
    let clock = WgcCapture::new_clock().map_err(|e| init(&e))?;
    // The wall-clock twin of the capture clock origin (both are QPC under
    // the hood; sampled together they describe one timeline — ddoc §5).
    let recording_t0 = Instant::now();
    let mut marker_runtime = MarkerRuntime::new(&opts, recording_t0);
    let mut marker_log = MarkerLog::new();
    let mut player_summary = PlayerSummaryState::default();
    // Build the capture engine — DXGI Desktop Duplication when the user opted
    // in for a display/region source, else WGC — and pull the first frame,
    // which fixes the capture size. A DXGI failure (multi-GPU, rotated display,
    // secure desktop on the first frame, …) silently falls back to WGC.
    let ((in_w, in_h), cap, first, identity) =
        if opts.focus_follow_enabled && opts.active_game.is_none() {
            let (in_w, in_h) = initial_canvas_dimensions(&opts, &device, events)?;
            let mut cap = SlateCapture::new(&device, in_w, in_h, opts.fps)?;
            let first = cap
                .next_frame_timeout(Duration::ZERO)
                .map_err(|e| format!("init: {e}"))?
                .ok_or("capture ended before the first frame")?;
            (
                (in_w, in_h),
                LiveBackend::Slate(cap),
                first,
                SwitchTargetIdentity::Slate,
            )
        } else {
            let (cap, first) = open_screen_capture(
                &device,
                clock,
                &opts.capture_source,
                opts.capture_backend,
                events,
            )?;
            let FrameData::Gpu(tex) = &first.data else {
                return Err("expected a GPU frame".into());
            };
            let (in_w, in_h) = d3d11::texture_size(tex);
            let identity = match &opts.capture_source {
                CaptureSource::WindowHandle { hwnd, .. } => SwitchTargetIdentity::Window(*hwnd),
                _ => SwitchTargetIdentity::Slate,
            };
            ((in_w, in_h), cap, first, identity)
        };
    // Output resolution caps scale down while preserving the captured aspect ratio.
    let (enc_w, enc_h) = output_dimensions_with_bounds(
        in_w,
        in_h,
        opts.output_resolution,
        opts.output_resolution_bounds,
    );

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
    let (cap, switch_controller) =
        SwitchableLiveCapture::new(device.clone(), clock, opts.fps, (in_w, in_h), cap, identity);
    let cap = CadencedCapture::new(cap, opts.fps, &first);
    let mut rec = Recorder::new_with_replay_storage(cap, encoder, replay_storage)
        .map_err(|e| format!("replay cache: {e}"))?;
    let audio_privacy = clipline_capture::AudioPrivacyState::new_game();
    let mut focus_state = FocusRunState::from_options(&opts);
    audio_privacy.set_slate(focus_state.capture_kind == CaptureKind::Slate);
    let audio_tracks = audio_sources_from_options(clock, &opts.audio, events);
    let audio_track_metadata: Vec<ClipAudioTrack> = audio_tracks
        .iter()
        .map(|(_, track)| track.clone())
        .collect();
    for (audio, _) in audio_tracks {
        let gated = clipline_capture::PrivacyAudioGate::new(audio, audio_privacy.clone())
            .map_err(|e| format!("audio privacy: {e}"))?;
        rec = rec.with_audio(Box::new(gated));
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
    if opts.recover_abandoned_recordings {
        recover_abandoned_recordings(&clips_dir, events);
    }
    // Saves land in a session folder: one per recorder run, with a dedicated
    // folder per detected match. Folders are created lazily at save time.
    let mut session = SessionTracker::new(local_session_label(false));
    let mut last_status = Instant::now();
    let mut switch_log = CaptureSwitchLog::default();
    let mut last_frame_pts_s = 0.0;
    switch_log.push(0.0, &focus_state);
    let mut full_session = begin_full_session_recording(
        &mut rec,
        &clips_dir,
        session.current(),
        focus_state.recording_mode,
        focus_state.active_game.as_ref(),
        events,
    );
    send_recording_status(events, &rec, &full_session, &encoder_status, &focus_state);

    loop {
        match rec.step_with_frame(|frame| {
            last_frame_pts_s = frame.pts_s;
        }) {
            Ok(true) => {}
            Ok(false) => break,
            // Idle screen: WGC delivers nothing — keep serving commands.
            Err(PipelineError::Capture(CaptureError::Timeout(_))) => {}
            Err(e) => {
                let _ = shutdown_recorder(
                    &mut rec,
                    &mut full_session,
                    RecorderFinishContext {
                        marker_log: &marker_log,
                        switch_log: &switch_log,
                        player_summary: player_summary.full_session_summary(),
                        audio_tracks: &audio_track_metadata,
                        clips_dir: &clips_dir,
                        opts: &opts,
                        events,
                    },
                );
                return Err(format!("recording: {e}"));
            }
        }

        while let Ok(msg) = marker_runtime.try_recv() {
            let marker_allowed = focus_state
                .active_game_plugin_id
                .as_deref()
                .is_some_and(|plugin_id| focus_state.accepts_plugin_markers(plugin_id));
            match msg {
                PollerMsg::Event(event) => {
                    if marker_allowed {
                        // GameEnd means the match is over even while the Live
                        // Client API lingers; stop attributing saves to it.
                        if event.kind == EventKind::GameEnd {
                            player_summary.match_ended();
                            session.match_ended();
                        }
                        if is_review_event(&event) {
                            marker_log.push(event);
                        }
                    }
                }
                PollerMsg::PlayerSummary(summary) if marker_allowed => {
                    player_summary.update(summary)
                }
                PollerMsg::MatchStarted if marker_allowed => {
                    player_summary.match_started();
                    session.match_started(local_session_label(true));
                }
                PollerMsg::MatchEnded if marker_allowed => {
                    player_summary.match_ended();
                    session.match_ended();
                }
                _ => {}
            }
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            last_status = Instant::now();
            send_recording_status(events, &rec, &full_session, &encoder_status, &focus_state);
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
                    write_session_game_meta(&session_dir, focus_state.active_game.as_ref());
                    let path = unique_media_path(&session_dir, "clip");
                    match save(&rec, &path, opts.replay_window_s) {
                        Ok((end, seconds)) => {
                            // Markers and match summary ride along as a
                            // sidecar (ddoc §5) when either is available.
                            let markers = write_marker_sidecar(
                                events,
                                &marker_log,
                                &switch_log,
                                &path,
                                end - seconds,
                                end,
                                player_summary.active_replay_summary(),
                                &audio_track_metadata,
                            );
                            emit_saved_clip(
                                events,
                                &clips_dir,
                                &path,
                                seconds,
                                SavedClipMeta {
                                    markers,
                                    full_session: false,
                                    recording_start_unix: None,
                                    recording_end_unix: None,
                                },
                                &opts,
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
                        RecorderFinishContext {
                            marker_log: &marker_log,
                            switch_log: &switch_log,
                            player_summary: player_summary.full_session_summary(),
                            audio_tracks: &audio_track_metadata,
                            clips_dir: &clips_dir,
                            opts: &opts,
                            events,
                        },
                    );
                    if announce {
                        send_stopped(events);
                    }
                    return Ok(());
                }
                Ok(Cmd::SwitchCapture(target)) => {
                    let old_state = focus_state.clone();
                    match switch_controller.switch_to(&target) {
                        Ok(()) => {
                            focus_state.apply_target(&target);
                            marker_runtime
                                .sync_to_plugin(focus_state.active_game_plugin_id.as_deref());
                            audio_privacy.set_slate(focus_state.capture_kind == CaptureKind::Slate);
                            if focus_state != old_state {
                                switch_log.push(last_frame_pts_s, &focus_state);
                            }
                            reconcile_full_session_transition(
                                &mut rec,
                                &clips_dir,
                                session.current(),
                                &mut full_session,
                                &old_state,
                                &focus_state,
                                &marker_log,
                                &switch_log,
                                player_summary.full_session_summary(),
                                &audio_track_metadata,
                                events,
                                &opts,
                            );
                            send_recording_status(
                                events,
                                &rec,
                                &full_session,
                                &encoder_status,
                                &focus_state,
                            );
                        }
                        Err(e) => {
                            warn_retryable_switch_failure(events, &target, e);
                            let fallback = SwitchCaptureTarget::Slate {
                                reason: SlateReason::SwitchFailed,
                            };
                            let _ = switch_controller.switch_to(&fallback);
                            focus_state.apply_target(&fallback);
                            marker_runtime
                                .sync_to_plugin(focus_state.active_game_plugin_id.as_deref());
                            audio_privacy.set_slate(true);
                            if focus_state != old_state {
                                switch_log.push(last_frame_pts_s, &focus_state);
                            }
                            send_recording_status(
                                events,
                                &rec,
                                &full_session,
                                &encoder_status,
                                &focus_state,
                            );
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    let _ = shutdown_recorder(
                        &mut rec,
                        &mut full_session,
                        RecorderFinishContext {
                            marker_log: &marker_log,
                            switch_log: &switch_log,
                            player_summary: player_summary.full_session_summary(),
                            audio_tracks: &audio_track_metadata,
                            clips_dir: &clips_dir,
                            opts: &opts,
                            events,
                        },
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
        RecorderFinishContext {
            marker_log: &marker_log,
            switch_log: &switch_log,
            player_summary: player_summary.full_session_summary(),
            audio_tracks: &audio_track_metadata,
            clips_dir: &clips_dir,
            opts: &opts,
            events,
        },
    ) {
        return Err(err);
    }
    send_stopped(events);
    Ok(())
}

fn audio_sources_from_options(
    clock: RelativeClock,
    options: &AudioOptions,
    events: &Sender<Event>,
) -> Vec<(Box<dyn AudioSource>, ClipAudioTrack)> {
    let mic_channels = match options.mic_channels {
        AudioChannelMode::Mono => WasapiChannelMode::Mono,
        AudioChannelMode::Stereo => WasapiChannelMode::Stereo,
    };

    let mut sources = Vec::<(Box<dyn AudioSource>, ClipAudioTrack)>::new();
    if options.output_enabled {
        add_output_audio_sources(clock, options, events, &mut sources);
    }
    if options.mic_enabled {
        match WasapiLoopback::start_microphone(
            clock,
            options.mic_device_id.as_deref(),
            options.mic_volume,
            mic_channels,
        ) {
            Ok(audio) => {
                let index = sources.len() as u32;
                sources.push((
                    Box::new(audio),
                    audio_track("microphone", index, "Microphone", "microphone"),
                ));
            }
            Err(e) => {
                warn_user(events, format!("microphone unavailable; continuing: {e}"));
            }
        }
    }
    sources
}

fn add_output_audio_sources(
    clock: RelativeClock,
    options: &AudioOptions,
    events: &Sender<Event>,
    sources: &mut Vec<(Box<dyn AudioSource>, ClipAudioTrack)>,
) {
    let mut process_tracks = Vec::new();
    let mut process_loopback_failed = false;
    let mut process_loopback_error = None::<String>;
    if options.split_output_by_process && process_loopback_available() {
        match enumerate_output_processes(options.output_device_id.as_deref()) {
            Ok(processes) => {
                for process in split_output_process_candidates(processes, std::process::id()) {
                    match WasapiLoopback::start_process_output(
                        clock,
                        process.pid,
                        options.output_volume,
                    ) {
                        Ok(audio) => process_tracks.push((process, audio)),
                        Err(e) if e.to_string().contains("timed out") => {
                            process_loopback_failed = true;
                            process_loopback_error.get_or_insert_with(|| e.to_string());
                            break;
                        }
                        Err(e) => {
                            process_loopback_failed = true;
                            process_loopback_error
                                .get_or_insert_with(|| format!("{}: {e}", process.label));
                        }
                    }
                }
            }
            Err(e) => {
                process_loopback_failed = true;
                process_loopback_error = Some(e.to_string());
            }
        }
    }

    if process_loopback_failed {
        let detail = process_loopback_error
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        warn_user(
            events,
            format!("some app audio tracks unavailable; adding mixed output fallback{detail}"),
        );
    }

    let process_track_count = process_tracks.len();
    add_mixed_output_audio_source(clock, options, events, sources);

    if process_track_count > 0 {
        for (process, audio) in process_tracks {
            let index = sources.len() as u32;
            let id = format!("process:{}", process.pid);
            sources.push((
                Box::new(audio),
                audio_track(&id, index, &process.label, "process_output"),
            ));
        }
    }
}

fn split_output_process_candidates(
    processes: Vec<AudioProcessInfo>,
    own_pid: u32,
) -> Vec<AudioProcessInfo> {
    // Split process tracks should not include Clipline's own notification
    // sounds. The mixed Output Audio safety track remains raw speaker loopback.
    processes
        .into_iter()
        .filter(|process| process.pid != own_pid)
        .collect()
}

fn add_mixed_output_audio_source(
    clock: RelativeClock,
    options: &AudioOptions,
    events: &Sender<Event>,
    sources: &mut Vec<(Box<dyn AudioSource>, ClipAudioTrack)>,
) {
    match WasapiLoopback::start_output(
        clock,
        options.output_device_id.as_deref(),
        options.output_volume,
    ) {
        Ok(audio) => {
            let index = sources.len() as u32;
            sources.push((
                Box::new(audio),
                audio_track("output", index, "Output Audio", "output"),
            ));
        }
        Err(e) => {
            warn_user(events, format!("output audio unavailable; continuing: {e}"));
        }
    }
}

fn audio_track(id: &str, track_index: u32, label: &str, kind: &str) -> ClipAudioTrack {
    ClipAudioTrack {
        id: id.to_string(),
        track_index,
        label: label.to_string(),
        kind: Some(kind.to_string()),
    }
}

fn warn_user(events: &Sender<Event>, message: String) {
    let _ = events.send(Event::Error { message });
}

fn warn_retryable_switch_failure(
    events: &Sender<Event>,
    target: &SwitchCaptureTarget,
    error: impl std::fmt::Display,
) {
    if let SwitchCaptureTarget::Window { hwnd, .. } = target {
        let _ = events.send(Event::FocusFollowRetry { hwnd: *hwnd });
    }
    warn_user(
        events,
        format!("switch capture target: {error}; using privacy slate"),
    );
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

#[cfg(test)]
fn output_dimensions(in_w: u32, in_h: u32, resolution: OutputResolution) -> (u32, u32) {
    output_dimensions_with_bounds(in_w, in_h, resolution, None)
}

fn output_dimensions_with_bounds(
    in_w: u32,
    in_h: u32,
    resolution: OutputResolution,
    bounds: Option<OutputResolutionBounds>,
) -> (u32, u32) {
    let max_box = bounds
        .map(|bounds| (bounds.width, bounds.height))
        .or_else(|| resolution.bounds())
        .unwrap_or((2560, u32::MAX));
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
    let resize_mode = if opts.focus_follow_enabled {
        ResizeMode::Fit
    } else {
        ResizeMode::Stretch
    };
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
                resize_mode,
            };
            MftH264Encoder::new(device, in_w, in_h, cfg)
                .map(|e| Box::new(e) as Box<dyn Encoder>)
                .map_err(|e| e.to_string())
        }
        EncoderApi::Ffmpeg => {
            let ffmpeg = ffmpeg_path
                .as_deref()
                .ok_or_else(|| "ffmpeg not located".to_string())?;
            FfmpegVideoEncoder::new_on_with_resize(
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
                resize_mode,
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
        capture_kind: CaptureKind::Game,
        capture_label: None,
        slate_reason: None,
    });
}

fn send_recording_status(
    events: &Sender<Event>,
    rec: &LiveRecorder,
    full_session: &Option<FullSessionRecording>,
    encoder_status: &str,
    focus_state: &FocusRunState,
) {
    let _ = events.send(Event::Status {
        recording: true,
        segments: rec.ring_len(),
        buffered_s: rec.buffered_span_s(),
        buffered_mb: rec.ring_bytes() as f64 / (1024.0 * 1024.0),
        full_session: full_session.is_some(),
        encoder: encoder_status.to_string(),
        capture_kind: focus_state.capture_kind,
        capture_label: focus_state
            .active_game
            .as_ref()
            .map(|game| game.name.clone()),
        slate_reason: focus_state.slate_reason,
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

struct RecorderFinishContext<'a> {
    marker_log: &'a MarkerLog,
    switch_log: &'a CaptureSwitchLog,
    player_summary: Option<&'a PlayerSummary>,
    audio_tracks: &'a [ClipAudioTrack],
    clips_dir: &'a Path,
    opts: &'a ServiceOptions,
    events: &'a Sender<Event>,
}

fn shutdown_recorder(
    rec: &mut LiveRecorder,
    full_session: &mut Option<FullSessionRecording>,
    ctx: RecorderFinishContext<'_>,
) -> Option<String> {
    match rec.finish_stream() {
        Ok(()) => {
            finish_full_session_recording(rec, full_session, &ctx);
            None
        }
        Err(e) => {
            let message = format!("finish: {e}");
            warn_user(ctx.events, message.clone());
            discard_full_session_recording(
                rec,
                full_session,
                ctx.events,
                "full session discarded because recording could not finish cleanly",
            );
            Some(message)
        }
    }
}

/// Sidecar that records which game a session folder belongs to, so the
/// library can show its icon. Written once per folder; custom-game clips have
/// no markers, so this is their only game link.
const SESSION_META_FILE: &str = "clipline-session.json";

fn write_session_game_meta(session_dir: &Path, active_game: Option<&ActiveGame>) {
    let Some(game) = active_game else { return };
    let meta_path = session_dir.join(SESSION_META_FILE);
    if meta_path.exists() {
        return;
    }
    let doc = serde_json::json!({ "id": game.id, "name": game.name });
    match serde_json::to_string(&doc) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&meta_path, json) {
                eprintln!("write session game meta {meta_path:?}: {e}");
            }
        }
        Err(e) => eprintln!("serialize session game meta: {e}"),
    }
}

fn active_full_session_game_id(recording: &Option<FullSessionRecording>) -> Option<&str> {
    recording
        .as_ref()
        .and_then(|recording| recording.game_id.as_deref())
}

#[allow(clippy::too_many_arguments)]
fn reconcile_full_session_transition(
    rec: &mut LiveRecorder,
    clips_dir: &Path,
    session_label: &str,
    full_session: &mut Option<FullSessionRecording>,
    old_state: &FocusRunState,
    next_state: &FocusRunState,
    marker_log: &MarkerLog,
    switch_log: &CaptureSwitchLog,
    player_summary: Option<&PlayerSummary>,
    audio_tracks: &[ClipAudioTrack],
    events: &Sender<Event>,
    opts: &ServiceOptions,
) {
    let transition = full_session_transition(
        active_full_session_game_id(full_session),
        old_state,
        next_state,
    );
    let ctx = RecorderFinishContext {
        marker_log,
        switch_log,
        player_summary,
        audio_tracks,
        clips_dir,
        opts,
        events,
    };
    match transition {
        FullSessionTransition::None => {}
        FullSessionTransition::Finish => {
            finish_full_session_recording(rec, full_session, &ctx);
        }
        FullSessionTransition::Start => {
            *full_session = begin_full_session_recording(
                rec,
                clips_dir,
                session_label,
                next_state.recording_mode,
                next_state.active_game.as_ref(),
                events,
            );
        }
        FullSessionTransition::FinishThenStart => {
            finish_full_session_recording(rec, full_session, &ctx);
            *full_session = begin_full_session_recording(
                rec,
                clips_dir,
                session_label,
                next_state.recording_mode,
                next_state.active_game.as_ref(),
                events,
            );
        }
    }
}

fn begin_full_session_recording(
    rec: &mut LiveRecorder,
    clips_dir: &Path,
    session_label: &str,
    mode: RecordingMode,
    active_game: Option<&ActiveGame>,
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
    write_session_game_meta(&session_dir, active_game);
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
        wall_start_unix: unix_now(),
        min_duration_s: minimum_full_session_duration_s(active_game),
        game_id: active_game.map(|game| game.id.clone()),
    })
}

fn finish_full_session_recording(
    rec: &mut LiveRecorder,
    recording: &mut Option<FullSessionRecording>,
    ctx: &RecorderFinishContext<'_>,
) {
    let Some(recording) = recording.take() else {
        return;
    };
    match rec.finish_full_session() {
        Ok(Some(summary)) if summary.duration_s.is_finite() && summary.duration_s <= 0.0 => {
            warn_user(
                ctx.events,
                "full session ended before any footage was written".into(),
            );
            let _ = std::fs::remove_file(&recording.temp_path);
        }
        Ok(Some(summary))
            if should_discard_full_session_for_min_duration(
                recording.min_duration_s,
                summary.duration_s,
            ) =>
        {
            warn_user(
                ctx.events,
                format!(
                    "full session discarded because it was only {:.1}s; ignoring a brief startup/update window",
                    summary.duration_s
                ),
            );
            let _ = std::fs::remove_file(&recording.temp_path);
        }
        Ok(Some(summary)) => {
            let seconds = if summary.duration_s.is_finite() {
                summary.duration_s
            } else {
                warn_user(
                    ctx.events,
                    "full session duration was invalid; keeping the recording with an unknown duration"
                        .into(),
                );
                0.0
            };
            if !rename_finalized_session(&recording, ctx.events) {
                return;
            }
            let markers = write_marker_sidecar(
                ctx.events,
                ctx.marker_log,
                ctx.switch_log,
                &recording.final_path,
                summary.start_s,
                summary.end_s,
                ctx.player_summary,
                ctx.audio_tracks,
            );
            emit_saved_clip(
                ctx.events,
                ctx.clips_dir,
                &recording.final_path,
                seconds,
                SavedClipMeta {
                    markers,
                    full_session: true,
                    recording_start_unix: Some(recording.wall_start_unix),
                    recording_end_unix: Some(unix_now()),
                },
                ctx.opts,
            );
        }
        Ok(None) => {
            warn_user(
                ctx.events,
                "full session ended before any footage was written".into(),
            );
            let _ = std::fs::remove_file(&recording.temp_path);
        }
        Err(e) => {
            warn_user(ctx.events, format!("finish full session: {e}"));
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
    rec: &mut LiveRecorder,
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
    wall_start_unix: i64,
    min_duration_s: f64,
    game_id: Option<String>,
}

fn minimum_full_session_duration_s(active_game: Option<&ActiveGame>) -> f64 {
    match active_game {
        Some(game) if game.id == crate::game_plugins::OSU_ID => 10.0,
        _ => 0.0,
    }
}

#[cfg(test)]
fn should_discard_full_session_duration(active_game: Option<&ActiveGame>, duration_s: f64) -> bool {
    should_discard_full_session_for_min_duration(
        minimum_full_session_duration_s(active_game),
        duration_s,
    )
}

fn should_discard_full_session_for_min_duration(min_duration_s: f64, duration_s: f64) -> bool {
    min_duration_s > 0.0 && duration_s.is_finite() && duration_s < min_duration_s
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

#[allow(clippy::too_many_arguments)]
fn write_marker_sidecar(
    events: &Sender<Event>,
    marker_log: &MarkerLog,
    switch_log: &CaptureSwitchLog,
    path: &Path,
    start_s: f64,
    end_s: f64,
    player_summary: Option<&PlayerSummary>,
    audio_tracks: &[ClipAudioTrack],
) -> usize {
    let mut clip = marker_log.clip_markers(start_s, end_s);
    clip.markers.retain(|m| is_review_event(&m.event));
    clip.player_summary = player_summary.cloned();
    clip.audio_tracks = audio_tracks.to_vec();
    clip.source_switches = switch_log.clip_switches(start_s, end_s);
    let markers = clip.markers.len();
    if markers == 0
        && clip.player_summary.is_none()
        && clip.audio_tracks.is_empty()
        && clip.plays.is_empty()
        && clip.source_switches.is_empty()
    {
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

struct SavedClipMeta {
    markers: usize,
    full_session: bool,
    recording_start_unix: Option<i64>,
    recording_end_unix: Option<i64>,
}

fn emit_saved_clip(
    events: &Sender<Event>,
    clips_dir: &Path,
    path: &Path,
    seconds: f64,
    meta: SavedClipMeta,
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
        recording_start_unix: meta.recording_start_unix,
        recording_end_unix: meta.recording_end_unix,
        markers: meta.markers,
        full_session: meta.full_session,
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

fn crop_for_region_or_full_display(
    region: &CaptureRegion,
    display: &clipline_capture::windows::display::DisplayInfo,
    recovered_display: bool,
) -> Result<(CropRect, bool), String> {
    if region.width < 2 || region.height < 2 {
        return Err("capture region must be at least 2x2 pixels".into());
    }
    if !recovered_display {
        if let Some(crop) = rebased_full_display_crop(region, display) {
            return Ok((crop, false));
        }
        if let Ok(crop) = crop_for_region(region, display) {
            return Ok((crop, false));
        }
        if let Some(crop) = clamped_region_crop(region, display)? {
            return Ok((crop, true));
        }
    }
    Ok((
        CropRect {
            x: 0,
            y: 0,
            width: display.width,
            height: display.height,
        },
        true,
    ))
}

fn rebased_full_display_crop(
    region: &CaptureRegion,
    display: &clipline_capture::windows::display::DisplayInfo,
) -> Option<CropRect> {
    if region.width == display.width && region.height == display.height {
        Some(CropRect {
            x: 0,
            y: 0,
            width: display.width,
            height: display.height,
        })
    } else {
        None
    }
}

fn clamped_region_crop(
    region: &CaptureRegion,
    display: &clipline_capture::windows::display::DisplayInfo,
) -> Result<Option<CropRect>, String> {
    if region.width < 2 || region.height < 2 {
        return Err("capture region must be at least 2x2 pixels".into());
    }
    let region_left = region.x as i64;
    let region_top = region.y as i64;
    let region_right = region_left + region.width as i64;
    let region_bottom = region_top + region.height as i64;
    let display_left = display.x as i64;
    let display_top = display.y as i64;
    let display_right = display_left + display.width as i64;
    let display_bottom = display_top + display.height as i64;

    let left = region_left.max(display_left);
    let top = region_top.max(display_top);
    let right = region_right.min(display_right);
    let bottom = region_bottom.min(display_bottom);
    let width = right - left;
    let height = bottom - top;
    if width < 2 || height < 2 {
        return Ok(None);
    }
    Ok(Some(CropRect {
        x: (left - display_left) as u32,
        y: (top - display_top) as u32,
        width: width as u32,
        height: height as u32,
    }))
}

fn capture_display_recovery_warning(
    region: &CaptureRegion,
    display: &clipline_capture::windows::display::DisplayInfo,
    recovered_display: bool,
    recovered_crop: bool,
) -> Option<String> {
    if !recovered_display && !recovered_crop {
        return None;
    }
    let configured = region
        .display_id
        .as_deref()
        .unwrap_or("the configured display");
    let fallback = if recovered_display {
        format!("using full display {}", display.name)
    } else {
        format!("using the visible part of the region on {}", display.name)
    };
    Some(format!(
        "capture target {configured} is no longer available or no longer fits; {fallback}. Open Settings and save your capture source to update it."
    ))
}

fn warn_capture_display_recovery(
    events: &Sender<Event>,
    region: &CaptureRegion,
    display: &clipline_capture::windows::display::DisplayInfo,
    recovered_display: bool,
    recovered_crop: bool,
) {
    if let Some(message) =
        capture_display_recovery_warning(region, display, recovered_display, recovered_crop)
    {
        eprintln!("clipline: {message}");
        warn_user(events, message);
    }
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

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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
    use std::collections::VecDeque;

    use clipline_capture::{MockCapture, MockEncoder};
    use clipline_test_utils::TestDir;

    struct TimeoutSource;

    impl TimedFrameSource for TimeoutSource {
        fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
            Err(CaptureError::Timeout(timeout))
        }
    }

    struct ScriptedTimedSource {
        frames: VecDeque<Result<Option<Frame>, CaptureError>>,
    }

    impl ScriptedTimedSource {
        fn new(frames: impl IntoIterator<Item = Result<Option<Frame>, CaptureError>>) -> Self {
            Self {
                frames: frames.into_iter().collect(),
            }
        }
    }

    impl TimedFrameSource for ScriptedTimedSource {
        fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
            self.frames
                .pop_front()
                .unwrap_or_else(|| Err(CaptureError::Timeout(timeout)))
        }
    }

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

    #[test]
    fn initial_canvas_uses_display_region_without_opening_capture() {
        let region = CaptureRegion {
            display_id: None,
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };

        assert_eq!(
            canvas_dimensions_from_capture_source(&CaptureSource::DisplayRegion(region)).unwrap(),
            (1280, 720)
        );
    }

    #[test]
    fn switch_target_identity_dedupes_repeated_slate_and_window() {
        let slate = SwitchCaptureTarget::Slate {
            reason: SlateReason::NoEnabledForegroundGame,
        };
        let window = SwitchCaptureTarget::Window {
            hwnd: 42,
            title: "Game".into(),
            active_game: Some(ActiveGame {
                id: "g".into(),
                name: "Game".into(),
            }),
            active_game_plugin_id: None,
            recording_mode: RecordingMode::ReplaysOnly,
        };

        assert_eq!(
            switch_target_identity(&slate),
            switch_target_identity(&slate)
        );
        assert_eq!(
            switch_target_identity(&window),
            switch_target_identity(&window)
        );
        assert_ne!(
            switch_target_identity(&slate),
            switch_target_identity(&window)
        );
    }

    #[test]
    fn missing_display_region_falls_back_to_full_current_display_crop() {
        let region = CaptureRegion {
            display_id: Some(r"\\.\DISPLAY-GHOST".into()),
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let display = clipline_capture::windows::display::DisplayInfo {
            id: r"\\.\DISPLAY1".into(),
            name: "DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: true,
        };

        let (crop, recovered) = crop_for_region_or_full_display(&region, &display, true).unwrap();

        assert!(recovered);
        assert_eq!(
            crop,
            CropRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn recovered_capture_display_builds_user_visible_warning() {
        let region = CaptureRegion {
            display_id: Some(r"\\.\DISPLAY-GHOST".into()),
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let display = clipline_capture::windows::display::DisplayInfo {
            id: r"\\.\DISPLAY1".into(),
            name: "DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: true,
        };

        let message = capture_display_recovery_warning(&region, &display, true, false)
            .expect("recovery warning");

        assert!(message.contains(r"\\.\DISPLAY-GHOST"), "{message}");
        assert!(message.contains("DISPLAY1"), "{message}");
        assert!(message.contains("Settings"), "{message}");
    }

    #[test]
    fn out_of_bounds_region_clamps_to_visible_display_crop() {
        let region = CaptureRegion {
            display_id: Some(r"\\.\DISPLAY1".into()),
            x: 1000,
            y: 500,
            width: 1000,
            height: 800,
        };
        let display = clipline_capture::windows::display::DisplayInfo {
            id: r"\\.\DISPLAY1".into(),
            name: "DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: true,
        };

        let (crop, recovered) = crop_for_region_or_full_display(&region, &display, false).unwrap();

        assert!(recovered);
        assert_eq!(
            crop,
            CropRect {
                x: 1000,
                y: 500,
                width: 920,
                height: 580
            }
        );
    }

    #[test]
    fn full_display_region_survives_virtual_origin_change() {
        let region = CaptureRegion {
            display_id: Some(r"\\.\DISPLAY1".into()),
            x: 1280,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let display = clipline_capture::windows::display::DisplayInfo {
            id: r"\\.\DISPLAY1".into(),
            name: "DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
            is_primary: true,
        };

        let (crop, recovered) = crop_for_region_or_full_display(&region, &display, false).unwrap();

        assert!(
            !recovered,
            "a full-display selection should rebase without a settings warning"
        );
        assert_eq!(
            crop,
            CropRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440
            }
        );
    }

    #[test]
    fn marker_source_falls_back_to_league_poller_without_active_plugin() {
        let opts = ServiceOptions::default();

        assert_eq!(
            marker_source_key(opts.active_game_plugin_id.as_deref()),
            MarkerSourceKey::LegacyLeaguePoller
        );
    }

    #[test]
    fn marker_source_uses_active_plugin_event_source_when_available() {
        let opts = ServiceOptions {
            active_game_plugin_id: Some(crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into()),
            ..ServiceOptions::default()
        };

        assert_eq!(
            marker_source_key(opts.active_game_plugin_id.as_deref()),
            MarkerSourceKey::Plugin(crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into())
        );
    }

    #[test]
    fn marker_source_disables_unknown_plugin_id() {
        let opts = ServiceOptions {
            active_game_plugin_id: Some("community_game_without_source".into()),
            ..ServiceOptions::default()
        };

        assert_eq!(
            marker_source_key(opts.active_game_plugin_id.as_deref()),
            MarkerSourceKey::NoMarkerSource
        );
    }

    #[test]
    fn marker_source_key_switches_from_slate_to_plugin_when_focus_enters_event_source_game() {
        let startup = marker_source_key_for(None, |_| false);
        let focused = marker_source_key_for(Some("plugin_a"), |plugin_id| {
            matches!(plugin_id, Some("plugin_a"))
        });

        assert_eq!(startup, MarkerSourceKey::LegacyLeaguePoller);
        assert_eq!(focused, MarkerSourceKey::Plugin("plugin_a".into()));
        assert_ne!(startup, focused);
    }

    #[test]
    fn marker_source_key_switches_from_legacy_to_none_for_unsupported_focus_plugin() {
        let startup = marker_source_key_for(None, |_| false);
        let focused = marker_source_key_for(Some("plugin_a"), |_| false);

        assert_eq!(startup, MarkerSourceKey::LegacyLeaguePoller);
        assert_eq!(focused, MarkerSourceKey::NoMarkerSource);
        assert_ne!(startup, focused);
    }

    #[test]
    fn marker_source_key_distinguishes_between_plugin_event_sources() {
        let plugin_a = marker_source_key_for(Some("plugin_a"), |plugin_id| {
            matches!(plugin_id, Some("plugin_a" | "plugin_b"))
        });
        let plugin_b = marker_source_key_for(Some("plugin_b"), |plugin_id| {
            matches!(plugin_id, Some("plugin_a" | "plugin_b"))
        });

        assert_eq!(plugin_a, MarkerSourceKey::Plugin("plugin_a".into()));
        assert_eq!(plugin_b, MarkerSourceKey::Plugin("plugin_b".into()));
        assert_ne!(plugin_a, plugin_b);
    }

    #[test]
    fn split_output_candidates_exclude_clipline_process() {
        let own_pid = 42;
        let processes = vec![
            clipline_capture::windows::wasapi::AudioProcessInfo {
                pid: own_pid,
                label: "clipline-app".into(),
                process_name: Some("clipline-app".into()),
                process_path: Some(r"C:\Clipline\clipline-app.exe".into()),
            },
            clipline_capture::windows::wasapi::AudioProcessInfo {
                pid: 99,
                label: "Game".into(),
                process_name: Some("Game".into()),
                process_path: Some(r"C:\Games\Game.exe".into()),
            },
        ];

        let candidates = split_output_process_candidates(processes, own_pid);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].label, "Game");
    }

    fn player_summary(champion_name: &str, kills: u32, deaths: u32, assists: u32) -> PlayerSummary {
        PlayerSummary {
            champion_name: champion_name.into(),
            kills,
            deaths,
            assists,
            creep_score: None,
            game_time_s: None,
            player_name: String::new(),
            team: String::new(),
            participants: Vec::new(),
            summoner_spells: Vec::new(),
            items: Vec::new(),
        }
    }

    fn review_event(
        kind: EventKind,
        actor: &str,
        victim: Option<&str>,
        offset_s: f64,
        involves_local_player: bool,
    ) -> clipline_events::GameEvent {
        clipline_events::GameEvent {
            game_id: clipline_events::GameId::LeagueOfLegends,
            kind,
            actor: actor.into(),
            victim: victim.map(String::from),
            assisters: Vec::new(),
            subtype: None,
            game_time_s: offset_s,
            recording_offset_s: Some(offset_s),
            importance: 7,
            involves_local_player,
        }
    }

    #[test]
    fn player_summary_state_stops_replay_attribution_after_match_end() {
        let mut state = PlayerSummaryState::default();
        let mid_match = player_summary("Nautilus", 3, 4, 22);
        let final_match = player_summary("Nautilus", 3, 4, 23);

        state.match_started();
        state.update(mid_match.clone());
        assert_eq!(state.active_replay_summary(), Some(&mid_match));
        assert_eq!(state.full_session_summary(), Some(&mid_match));

        state.match_ended();
        assert_eq!(state.active_replay_summary(), None);
        assert_eq!(state.full_session_summary(), Some(&mid_match));

        state.update(final_match.clone());
        assert_eq!(state.active_replay_summary(), None);
        assert_eq!(state.full_session_summary(), Some(&final_match));

        state.match_started();
        assert_eq!(state.active_replay_summary(), None);
        assert_eq!(state.full_session_summary(), None);
    }

    #[test]
    fn focus_run_state_uses_latest_active_game_for_save_meta() {
        let mut state = FocusRunState::from_options(&ServiceOptions::default());
        state.apply_target(&SwitchCaptureTarget::Window {
            hwnd: 9,
            title: "Game B".into(),
            active_game: Some(ActiveGame {
                id: "game-b".into(),
                name: "Game B".into(),
            }),
            active_game_plugin_id: None,
            recording_mode: RecordingMode::ReplaysOnly,
        });

        assert_eq!(
            state.active_game.as_ref().map(|game| game.id.as_str()),
            Some("game-b")
        );
        assert_eq!(state.recording_mode, RecordingMode::ReplaysOnly);
    }

    #[test]
    fn focus_run_state_gates_plugin_markers_to_current_plugin() {
        let mut state = FocusRunState::from_options(&ServiceOptions {
            active_game_plugin_id: Some(crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into()),
            ..ServiceOptions::default()
        });
        assert!(state.accepts_plugin_markers(crate::game_plugins::LEAGUE_OF_LEGENDS_ID));

        state.apply_target(&SwitchCaptureTarget::Slate {
            reason: SlateReason::NoEnabledForegroundGame,
        });
        assert!(!state.accepts_plugin_markers(crate::game_plugins::LEAGUE_OF_LEGENDS_ID));
    }

    #[test]
    fn focus_run_state_rejects_stale_plugin_markers_after_plugin_switch() {
        let mut state = FocusRunState::from_options(&ServiceOptions {
            active_game_plugin_id: Some("plugin_a".into()),
            active_game: Some(ActiveGame {
                id: "game-a".into(),
                name: "Game A".into(),
            }),
            ..ServiceOptions::default()
        });
        assert!(state.accepts_plugin_markers("plugin_a"));

        state.apply_target(&SwitchCaptureTarget::Window {
            hwnd: 10,
            title: "Game B".into(),
            active_game: Some(ActiveGame {
                id: "game-b".into(),
                name: "Game B".into(),
            }),
            active_game_plugin_id: Some("plugin_b".into()),
            recording_mode: RecordingMode::ReplaysOnly,
        });

        assert!(!state.accepts_plugin_markers("plugin_a"));
        assert!(state.accepts_plugin_markers("plugin_b"));
    }

    #[test]
    fn full_session_transition_splits_between_different_full_session_games() {
        let old = FocusRunState {
            capture_kind: CaptureKind::Game,
            active_game: Some(ActiveGame {
                id: "a".into(),
                name: "A".into(),
            }),
            active_game_plugin_id: None,
            recording_mode: RecordingMode::FullSession,
            slate_reason: None,
        };
        let next = FocusRunState {
            capture_kind: CaptureKind::Game,
            active_game: Some(ActiveGame {
                id: "b".into(),
                name: "B".into(),
            }),
            active_game_plugin_id: None,
            recording_mode: RecordingMode::FullSession,
            slate_reason: None,
        };

        assert_eq!(
            full_session_transition(Some("a"), &old, &next),
            FullSessionTransition::FinishThenStart
        );
    }

    #[test]
    fn capture_switch_log_filters_to_saved_window() {
        let mut log = CaptureSwitchLog::default();
        log.push(
            0.5,
            &FocusRunState {
                capture_kind: CaptureKind::Game,
                active_game: Some(ActiveGame {
                    id: "a".into(),
                    name: "A".into(),
                }),
                active_game_plugin_id: None,
                recording_mode: RecordingMode::ReplaysOnly,
                slate_reason: None,
            },
        );
        log.push(
            1.5,
            &FocusRunState {
                capture_kind: CaptureKind::Slate,
                active_game: None,
                active_game_plugin_id: None,
                recording_mode: RecordingMode::ReplaysOnly,
                slate_reason: Some(SlateReason::NoEnabledForegroundGame),
            },
        );

        let switches = log.clip_switches(1.0, 2.0);

        assert_eq!(switches.len(), 1);
        assert_eq!(switches[0].t_s, 0.5);
        assert_eq!(switches[0].kind, "slate");
        assert_eq!(
            switches[0].slate_reason.as_deref(),
            Some("no_enabled_foreground_game")
        );
    }

    #[test]
    fn write_marker_sidecar_keeps_player_summary_without_markers() {
        let dir = TestDir::new("clipline-service", "sidecar-summary");
        let path = dir.path().join("clip.mp4");
        let (tx, _rx) = std::sync::mpsc::channel();
        let summary = PlayerSummary {
            champion_name: "Nautilus".into(),
            kills: 3,
            deaths: 4,
            assists: 23,
            creep_score: Some(187),
            game_time_s: Some(1800),
            player_name: String::new(),
            team: String::new(),
            participants: Vec::new(),
            summoner_spells: Vec::new(),
            items: Vec::new(),
        };

        let count = write_marker_sidecar(
            &tx,
            &MarkerLog::new(),
            &CaptureSwitchLog::default(),
            &path,
            0.0,
            10.0,
            Some(&summary),
            &[],
        );

        assert_eq!(count, 0);
        let json = std::fs::read_to_string(path.with_extension("markers.json")).unwrap();
        let sidecar: clipline_events::ClipMarkers = serde_json::from_str(&json).unwrap();
        assert!(sidecar.markers.is_empty());
        assert_eq!(sidecar.player_summary, Some(summary));
    }

    #[test]
    fn write_marker_sidecar_keeps_audio_tracks_without_markers() {
        let dir = TestDir::new("clipline-service", "sidecar-audio-tracks");
        let path = dir.path().join("clip.mp4");
        let (tx, _rx) = std::sync::mpsc::channel();
        let tracks = vec![audio_track("output", 0, "Output Audio", "output")];

        let count = write_marker_sidecar(
            &tx,
            &MarkerLog::new(),
            &CaptureSwitchLog::default(),
            &path,
            0.0,
            10.0,
            None,
            &tracks,
        );

        assert_eq!(count, 0);
        let json = std::fs::read_to_string(path.with_extension("markers.json")).unwrap();
        let sidecar: clipline_events::ClipMarkers = serde_json::from_str(&json).unwrap();
        assert!(sidecar.markers.is_empty());
        assert_eq!(sidecar.audio_tracks, tracks);
    }

    #[test]
    fn write_marker_sidecar_keeps_review_events_for_match_event_filters() {
        let dir = TestDir::new("clipline-service", "sidecar-review-events");
        let path = dir.path().join("clip.mp4");
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut log = MarkerLog::new();
        log.push(review_event(
            EventKind::ChampionKill,
            "Enemy Mid",
            Some("Ally Top"),
            12.0,
            false,
        ));
        log.push(review_event(
            EventKind::ChampionAssist,
            "Dain",
            Some("Enemy Mid"),
            14.0,
            true,
        ));
        log.push(review_event(
            EventKind::HeraldKill,
            "Ally Jungle",
            None,
            16.0,
            false,
        ));
        log.push(review_event(
            EventKind::MinionsSpawning,
            "",
            None,
            18.0,
            false,
        ));

        let count = write_marker_sidecar(
            &tx,
            &log,
            &CaptureSwitchLog::default(),
            &path,
            10.0,
            20.0,
            None,
            &[],
        );

        assert_eq!(count, 3);
        let json = std::fs::read_to_string(path.with_extension("markers.json")).unwrap();
        let sidecar: clipline_events::ClipMarkers = serde_json::from_str(&json).unwrap();
        let kinds: Vec<_> = sidecar
            .markers
            .iter()
            .map(|marker| marker.event.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::ChampionKill,
                EventKind::ChampionAssist,
                EventKind::HeraldKill,
            ]
        );
        assert_eq!(sidecar.markers[0].event.actor, "Enemy Mid");
        assert!(!sidecar.markers[0].event.involves_local_player);
        assert!((sidecar.markers[0].t_s - 2.0).abs() < 1e-9);
    }

    #[test]
    fn cadenced_capture_duplicates_seed_on_idle_timeout() {
        let seed = Frame {
            pts_s: 1.0,
            data: FrameData::Cpu(vec![7, 8, 9]),
        };
        let mut cap = CadencedCapture::new(TimeoutSource, 60, &seed);

        let first = cap
            .next_frame()
            .expect("duplicate frame")
            .expect("capture still open");
        let second = cap
            .next_frame()
            .expect("duplicate frame")
            .expect("capture still open");

        assert!((first.pts_s - (1.0 + 1.0 / 60.0)).abs() < 1e-9);
        assert!((second.pts_s - (1.0 + 2.0 / 60.0)).abs() < 1e-9);
        assert!(matches!(first.data, FrameData::Cpu(ref data) if data == &[7, 8, 9]));
        assert!(matches!(second.data, FrameData::Cpu(ref data) if data == &[7, 8, 9]));
    }

    #[test]
    fn cadenced_capture_keeps_cadence_when_source_timeline_restarts_mid_run() {
        let frame_interval_s = 1.0 / 30.0;
        let seed = Frame {
            pts_s: 1.0,
            data: FrameData::Cpu(vec![1]),
        };
        let mut cap = CadencedCapture::new(
            ScriptedTimedSource::new([
                Ok(Some(Frame {
                    pts_s: 1.0 + frame_interval_s,
                    data: FrameData::Cpu(vec![2]),
                })),
                Ok(Some(Frame {
                    pts_s: 0.0,
                    data: FrameData::Cpu(vec![3]),
                })),
                Ok(Some(Frame {
                    pts_s: frame_interval_s,
                    data: FrameData::Cpu(vec![4]),
                })),
            ]),
            30,
            &seed,
        );

        let first = cap.next_frame().unwrap().unwrap();
        let second = cap.next_frame().unwrap().unwrap();
        let third = cap.next_frame().unwrap().unwrap();

        assert!((first.pts_s - (1.0 + frame_interval_s)).abs() < 1e-9);
        assert!((second.pts_s - (1.0 + 2.0 * frame_interval_s)).abs() < 1e-9);
        assert!((third.pts_s - (1.0 + 3.0 * frame_interval_s)).abs() < 1e-9);
    }

    #[test]
    fn slate_capture_zero_timeout_does_not_emit_early_frame() {
        let mut slate = SlateCapture {
            frame: FrameData::Cpu(vec![0]),
            next_pts_s: 0.0,
            frame_interval_s: 1.0 / 60.0,
            frame_interval: Duration::from_secs_f64(1.0 / 60.0),
            next_frame_at: None,
        };

        let first = slate
            .next_frame_timeout(Duration::ZERO)
            .expect("first slate frame should be immediate")
            .expect("slate source should keep running");
        assert_eq!(first.pts_s, 0.0);

        let second = slate.next_frame_timeout(Duration::ZERO);
        assert!(matches!(
            second,
            Err(CaptureError::Timeout(timeout)) if timeout.is_zero()
        ));
    }

    #[test]
    fn retryable_window_switch_failure_emits_retry_event_before_error() {
        let (tx, rx) = mpsc::channel();
        let target = SwitchCaptureTarget::Window {
            hwnd: 42,
            title: "Game Window".into(),
            active_game: Some(ActiveGame {
                id: "game".into(),
                name: "Game".into(),
            }),
            active_game_plugin_id: None,
            recording_mode: RecordingMode::ReplaysOnly,
        };

        warn_retryable_switch_failure(&tx, &target, "window open failed");

        assert!(matches!(
            rx.recv(),
            Ok(Event::FocusFollowRetry { hwnd: 42 })
        ));
        assert!(matches!(
            rx.recv(),
            Ok(Event::Error { message })
                if message.contains("switch capture target: window open failed; using privacy slate")
        ));
    }

    #[test]
    fn manual_replay_save_does_not_shrink_after_previous_save() {
        let dir = TestDir::new("clipline-service", "manual-save-window");
        let mut rec = Recorder::new(
            MockCapture::new(120, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();

        let first_path = dir.path().join("clip_first.mp4");
        let second_path = dir.path().join("clip_second.mp4");
        let (first_end, first_seconds) = save(&rec, &first_path, 2.0).unwrap();
        let (second_end, second_seconds) = save(&rec, &second_path, 2.0).unwrap();

        assert!((first_end - 4.0).abs() < 1e-6);
        assert!((second_end - 4.0).abs() < 1e-6);
        assert!((first_seconds - 2.0).abs() < 1e-6);
        assert!((second_seconds - 2.0).abs() < 1e-6);
    }

    #[test]
    fn clips_dir_uses_configured_root_when_creatable() {
        let dir = TestDir::new("clipline-service", "configured-root");
        let configured = dir.path().join("media");

        let (resolved, fell_back) =
            clips_dir_resolved(&configured, || panic!("must not fall back")).unwrap();

        assert!(!fell_back);
        assert_eq!(resolved, configured);
        assert!(configured.is_dir());
    }

    #[test]
    fn clips_dir_falls_back_when_configured_root_is_unusable() {
        let dir = TestDir::new("clipline-service", "unusable-root");
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
        let dir = TestDir::new("clipline-service", "temp-guard");
        let temp_root = dir.path().join("temp");
        let inside = temp_root.join("Videos").join("Clipline");
        std::fs::create_dir_all(&inside).unwrap();

        assert!(is_within_temp(&inside, &temp_root));
    }

    #[test]
    fn temp_guard_allows_clips_outside_temp_root() {
        let dir = TestDir::new("clipline-service", "temp-guard-outside");
        let temp_root = dir.path().join("temp");
        let outside = dir.path().join("media").join("Clipline");
        std::fs::create_dir_all(&temp_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        assert!(!is_within_temp(&outside, &temp_root));
    }

    #[test]
    fn finalized_session_rename_accepts_preexisting_final_file() {
        let dir = TestDir::new("clipline-service", "session-rename-recovered");
        let final_path = dir.path().join("session.mp4");
        std::fs::write(&final_path, b"mp4").unwrap();
        let recording = FullSessionRecording {
            final_path,
            temp_path: dir.path().join("session.mp4.recording"),
            wall_start_unix: 0,
            min_duration_s: 0.0,
            game_id: None,
        };
        let (tx, rx) = mpsc::channel();

        assert!(rename_finalized_session(&recording, &tx));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn finalized_session_rename_warns_when_temp_and_final_are_missing() {
        let dir = TestDir::new("clipline-service", "session-rename-missing");
        let recording = FullSessionRecording {
            final_path: dir.path().join("session.mp4"),
            temp_path: dir.path().join("session.mp4.recording"),
            wall_start_unix: 0,
            min_duration_s: 0.0,
            game_id: None,
        };
        let (tx, rx) = mpsc::channel();

        assert!(!rename_finalized_session(&recording, &tx));
        let Event::Error { message } = rx.try_recv().unwrap() else {
            panic!("expected warning");
        };
        assert!(message.contains("finalize full session"));
    }

    #[test]
    fn osu_full_session_duration_policy_discards_boot_transients_only() {
        let osu = ActiveGame {
            id: crate::game_plugins::OSU_ID.into(),
            name: "osu!".into(),
        };
        let league = ActiveGame {
            id: crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into(),
            name: "League of Legends".into(),
        };

        assert_eq!(minimum_full_session_duration_s(Some(&osu)), 10.0);
        assert!(should_discard_full_session_duration(Some(&osu), 9.9));
        assert!(!should_discard_full_session_duration(Some(&osu), 10.0));
        assert_eq!(minimum_full_session_duration_s(Some(&league)), 0.0);
        assert!(!should_discard_full_session_duration(Some(&league), 3.0));
        assert!(!should_discard_full_session_duration(None, 3.0));
    }
}
