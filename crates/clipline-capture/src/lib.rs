pub mod mock;
pub mod pipeline;
pub mod probe;
pub mod traits;

pub use mock::{MockAudioSource, MockCapture, MockEncoder};
pub use pipeline::{PipelineError, Recorder};
pub use probe::{select_encoder, Codec, EncoderBackend, EncoderCapability};
pub use traits::{
    AudioPacket, AudioSource, CaptureEngine, CaptureError, EncodeError, EncodedPacket,
    Encoder, Frame, FrameData,
};
