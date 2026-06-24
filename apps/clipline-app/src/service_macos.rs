use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

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

pub fn codec_id(codec: Codec) -> &'static str {
    match codec {
        Codec::Av1 => "av1",
        Codec::Hevc => "hevc",
        Codec::H264 => "h264",
    }
}

#[derive(serde::Serialize)]
pub struct EncoderOption {
    pub id: String,
    pub name: String,
    pub codec: String,
}

pub fn available_encoder_options() -> Vec<EncoderOption> {
    let _ = codec_id(Codec::H264);
    Vec::new()
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
#[allow(dead_code)]
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

pub fn spawn(opts: ServiceOptions) -> (Sender<Cmd>, Receiver<Event>) {
    let ServiceOptions {
        capture_source,
        capture_backend,
        active_game_plugin_id,
        active_game,
        media_dir,
        lol_url,
        replay_window_s,
        buffer_bytes,
        replay_storage,
        disk_quota_bytes,
        recording_mode,
        fps,
        bitrate_bps,
        video_encoder,
        output_resolution,
        decodable_codecs,
        audio,
    } = opts;
    let _ = (
        capture_source,
        capture_backend,
        active_game_plugin_id,
        media_dir,
        lol_url,
        replay_window_s,
        buffer_bytes,
        replay_storage,
        disk_quota_bytes,
        recording_mode,
        fps,
        bitrate_bps,
        video_encoder,
        output_resolution,
        decodable_codecs,
        audio,
    );
    if let Some(active_game) = active_game {
        let _ = (active_game.id, active_game.name);
    }
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("clipline-recorder-stub".into())
        .spawn(move || {
            let _ = event_tx.send(Event::Status {
                recording: false,
                segments: 0,
                buffered_s: 0.0,
                buffered_mb: 0.0,
                full_session: false,
                encoder: "Unavailable on macOS M1".into(),
            });
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    Cmd::Save => {
                        let _ = event_tx.send(Event::Error {
                            message: "macOS recording is not implemented in Milestone 1".into(),
                        });
                    }
                    Cmd::Stop { announce } => {
                        if announce {
                            let _ = event_tx.send(Event::Status {
                                recording: false,
                                segments: 0,
                                buffered_s: 0.0,
                                buffered_mb: 0.0,
                                full_session: false,
                                encoder: String::new(),
                            });
                        }
                        break;
                    }
                }
            }
        })
        .expect("spawn macOS recorder stub thread");
    (cmd_tx, event_rx)
}

pub fn ensure_recording_available() -> Result<(), String> {
    Err("macOS recording is not implemented in Milestone 1".into())
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
    Ok(root.to_path_buf())
}
