use clipline_mp4::{AudioTrackConfig, VideoTrackConfig};

/// One captured video frame. Platform implementations keep pixels on the
/// GPU (ddoc §3: frames stay as GPU textures); the pipeline only needs
/// timing plus an opaque payload handle.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Seconds since capture start (monotonic, from the capture clock).
    pub pts_s: f64,
    pub data: FrameData,
}

/// Frame payload. `Cpu` serves mocks/tests/software paths; `Gpu` keeps
/// pixels on the GPU as ddoc §3 requires (no CPU round-trips).
#[derive(Debug, Clone)]
pub enum FrameData {
    Cpu(Vec<u8>),
    #[cfg(windows)]
    Gpu(::windows::Win32::Graphics::Direct3D11::ID3D11Texture2D),
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("capture init failed: {0}")]
    Init(String),
    #[error("capture device lost: {0}")]
    DeviceLost(String),
    #[error("no frame arrived within {0:?}")]
    Timeout(std::time::Duration),
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("encoder failed: {0}")]
    Backend(String),
}

/// Pull-model capture source. `Ok(None)` means the source ended.
pub trait CaptureEngine {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError>;
}

/// One encoded sample out of the encoder.
#[derive(Debug, Clone)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts_s: f64,
    pub duration_s: f64,
    pub is_keyframe: bool,
}

/// Video encoder. May buffer internally (B-frames later), hence Vec out.
pub trait Encoder {
    fn encode(&mut self, frame: &Frame) -> Result<Vec<EncodedPacket>, EncodeError>;
    /// Track parameters for muxing the produced stream.
    fn track_config(&self) -> VideoTrackConfig;
    /// Drain any internally buffered packets at end of stream. Called by
    /// the pipeline after the capture source ends.
    fn finish(&mut self) -> Result<Vec<EncodedPacket>, EncodeError> {
        Ok(Vec::new())
    }
}

/// One encoded audio packet (e.g. a 20 ms Opus frame).
#[derive(Debug, Clone)]
pub struct AudioPacket {
    pub data: Vec<u8>,
    /// Seconds since capture start, same timebase as video frames.
    pub pts_s: f64,
    pub duration_s: f64,
}

/// An encoded-audio producer (ddoc §10: WASAPI loopback / per-process /
/// mic, each composed with an Opus encoder behind this trait). Drain
/// model: return every packet that ends at or before `until_pts_s`.
pub trait AudioSource {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError>;
    /// Track parameters for muxing this source's stream.
    fn track_config(&self) -> AudioTrackConfig;
}
