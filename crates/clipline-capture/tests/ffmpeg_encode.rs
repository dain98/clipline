//! End-to-end FFmpeg subprocess encoder test: synthetic NV12 CPU frames →
//! `FfmpegVideoEncoder` → access-unit framing → `clipline-mp4` mux → ffprobe.
//! Self-skips when no usable `ffmpeg`/`ffprobe` is present (CI), runs for
//! real on a machine with the bundle. libsvtav1 ships in every LGPL build,
//! so the AV1 software path always exercises here; AMF H.264/HEVC run when
//! the probe finds the hardware (the dev box).

use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

use clipline_capture::ffmpeg;
use clipline_capture::ffmpeg_encoder::FfmpegVideoEncoder;
use clipline_capture::probe::{Codec, EncoderBackend};
use clipline_capture::traits::{Encoder, Frame, FrameData};
use clipline_mp4::{FragSample, HybridMp4Writer, VideoCodecParams};

const W: u32 = 320;
const H: u32 = 240;
const FPS: u32 = 30;
const FRAMES: u32 = 30; // one full 1 s GOP plus the seal boundary

/// A gray NV12 frame whose luma drifts with the frame index so the encoder
/// always has something to compress (avoids degenerate all-identical input).
fn nv12_frame(index: u32) -> Vec<u8> {
    let y = (W * H) as usize;
    let mut data = vec![0u8; y * 3 / 2];
    let luma = 64u8.wrapping_add((index * 3) as u8);
    data[..y].fill(luma);
    data[y..].fill(128); // neutral chroma
    data
}

fn which(bin: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_string() };
    let sep = if cfg!(windows) { ';' } else { ':' };
    std::env::var_os("PATH")?
        .to_str()?
        .split(sep)
        .map(|d| std::path::Path::new(d).join(&exe))
        .find(|p| p.exists())
}

/// Encode FRAMES synthetic frames, mux the packets into a finalized MP4, and
/// return its bytes. Asserts the encoder produced one packet per frame with a
/// leading keyframe and real parameter sets.
fn encode_and_mux(ffmpeg: &std::path::Path, backend: EncoderBackend, codec: Codec) -> Vec<u8> {
    let mut enc =
        FfmpegVideoEncoder::new(ffmpeg, backend, codec, W, H, FPS, 2_000_000).expect("spawn encoder");

    let mut packets = Vec::new();
    for i in 0..FRAMES {
        let frame = Frame { pts_s: i as f64 / FPS as f64, data: FrameData::Cpu(nv12_frame(i)) };
        packets.extend(enc.encode(&frame).expect("encode frame"));
    }
    packets.extend(enc.finish().expect("finish"));

    assert!(!packets.is_empty(), "{backend:?}/{codec:?}: no packets produced");
    assert!(packets[0].is_keyframe, "{backend:?}/{codec:?}: stream must open on a keyframe");

    let cfg = enc.track_config();
    match (&cfg.codec, codec) {
        (VideoCodecParams::H264 { sps, pps }, Codec::H264) => {
            assert!(!sps.is_empty() && !pps.is_empty(), "H.264 params extracted")
        }
        (VideoCodecParams::Hevc { vps, sps, pps }, Codec::Hevc) => {
            assert!(!vps.is_empty() && !sps.is_empty() && !pps.is_empty(), "HEVC params extracted")
        }
        (VideoCodecParams::Av1 { sequence_header_obu }, Codec::Av1) => {
            assert!(!sequence_header_obu.is_empty(), "AV1 sequence header extracted")
        }
        (other, _) => panic!("{backend:?}/{codec:?}: wrong codec params {other:?}"),
    }

    let mut writer = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg).expect("writer");
    let samples: Vec<FragSample> = packets
        .iter()
        .map(|p| FragSample {
            data: p.data.clone(),
            duration: (p.duration_s * 90_000.0).round() as u32,
            is_sync: p.is_keyframe,
        })
        .collect();
    writer.write_fragment(&samples).expect("fragment");
    writer.finalize().expect("finalize").into_inner()
}

fn ffprobe_codec(ffprobe: &std::path::Path, mp4: &[u8]) -> String {
    let path = std::env::temp_dir().join("clipline_ffmpeg_encode_test.mp4");
    std::fs::write(&path, mp4).unwrap();
    let out = Command::new(ffprobe)
        .args([
            "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name", "-of",
            "default=nw=1:nk=1",
        ])
        .arg(&path)
        .output()
        .expect("run ffprobe");
    std::fs::remove_file(&path).ok();
    assert!(out.status.success(), "ffprobe failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn ffmpeg_encoder_round_trips_through_the_muxer() {
    // Hard-skip on CI like the device tests: the runner's apt ffmpeg has an
    // unpredictable encoder set across image updates, and we want CI green
    // tied to the bundled build, not the runner's. Runs for real locally.
    if std::env::var_os("CI").is_some() {
        eprintln!("SKIP: ffmpeg encoder test (CI uses an unmanaged ffmpeg build)");
        return;
    }
    let Some(ffmpeg) = ffmpeg::locate() else {
        eprintln!("SKIP: no ffmpeg located; FFmpeg encoder tier unavailable");
        return;
    };
    let Some(ffprobe) = which("ffprobe") else {
        eprintln!("SKIP: ffprobe not found");
        return;
    };

    let caps = ffmpeg::probe();
    let mut exercised = 0;

    // SVT-AV1 software is in every LGPL build — always exercise it.
    if caps.iter().any(|c| c.backend == EncoderBackend::SvtAv1) {
        let mp4 = encode_and_mux(&ffmpeg, EncoderBackend::SvtAv1, Codec::Av1);
        assert_eq!(ffprobe_codec(&ffprobe, &mp4), "av1", "SVT-AV1 muxes to av01");
        exercised += 1;
    }

    // Any hardware backend the probe confirmed (AMF on the dev box).
    for cap in &caps {
        if !matches!(
            cap.backend,
            EncoderBackend::Nvenc | EncoderBackend::Amf | EncoderBackend::QuickSync
        ) {
            continue;
        }
        for &(codec, expected) in &[(Codec::H264, "h264"), (Codec::Hevc, "hevc")] {
            if cap.codecs.contains(&codec) {
                let mp4 = encode_and_mux(&ffmpeg, cap.backend, codec);
                assert_eq!(
                    ffprobe_codec(&ffprobe, &mp4),
                    expected,
                    "{:?}/{codec:?} muxes to the right codec",
                    cap.backend
                );
                exercised += 1;
            }
        }
    }

    if exercised == 0 {
        eprintln!("SKIP: located ffmpeg offers none of Clipline's target encoders");
        return;
    }
    eprintln!("ffmpeg encoder test exercised {exercised} encoder(s)");
}
