pub mod annexb;
pub mod avsync;
pub mod clock;
pub mod mock;
pub mod opus;
pub mod pcm;
pub mod pipeline;
pub mod probe;
pub mod traits;
#[cfg(windows)]
pub mod windows;

pub use annexb::{annexb_to_avcc, even_dimensions, extract_sps_pps, nal_type, split_annexb};
pub use avsync::{validate_timeline, SyncReport, SyncTolerances, SyncViolation};
pub use clock::{qpc_to_ticks_100ns, RelativeClock};
pub use mock::{LimitedCapture, MockAudioSource, MockCapture, MockEncoder};
pub use opus::OpusFrameEncoder;
pub use pcm::{extract_stereo, LoopbackAssembler};
pub use pipeline::{PipelineError, Recorder, ReplayStorageConfig};
pub use probe::{select_encoder, Codec, EncoderBackend, EncoderCapability};
pub use traits::{
    AudioPacket, AudioSource, CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder,
    Frame, FrameData,
};
