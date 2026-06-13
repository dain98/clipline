pub mod boxes;
pub mod fragment;
pub mod init;
pub mod trim;
pub mod walker;
pub mod writer;

pub use fragment::{FragSample, TrackRun};
pub use init::{AudioTrackConfig, TrackConfig, VideoTrackConfig};
pub use trim::{
    trim_keyframe_aligned, trim_keyframe_aligned_file, trim_keyframe_aligned_to_writer, TrimError,
    TrimInfo,
};
pub use writer::{HybridMp4Writer, SourceSample};
