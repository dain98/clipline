use std::io::Cursor;
use std::process::Command;

use clipline_mp4::walker::{children, find, walk};
use clipline_mp4::{
    AudioTrackConfig, FragSample, HybridMp4Writer, TrackConfig, VideoTrackConfig,
};

fn tracks() -> Vec<TrackConfig> {
    vec![
        TrackConfig::Video(VideoTrackConfig::h264(
            128,
            128,
            90_000,
            vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
            vec![0x68, 0xEE, 0x38, 0x80],
        )),
        TrackConfig::Audio(AudioTrackConfig {
            channels: 2,
            sample_rate: 48_000,
            pre_skip: 312,
        }),
    ]
}

fn video_gop(start: u32, frames: u32) -> Vec<FragSample> {
    (0..frames)
        .map(|i| FragSample {
            data: format!("V{:05}", start + i).into_bytes(),
            duration: 3000, // 30 fps @ 90 kHz
            is_sync: i == 0,
        })
        .collect()
}

fn audio_packets(start: u32, count: u32) -> Vec<FragSample> {
    (0..count)
        .map(|i| FragSample {
            data: format!("A{:05}", start + i).into_bytes(),
            duration: 960, // 20 ms @ 48 kHz
            is_sync: true,
        })
        .collect()
}

fn mux_2s() -> Vec<u8> {
    let mut w = HybridMp4Writer::new_multi(Cursor::new(Vec::new()), tracks()).unwrap();
    // 2 fragments of 1 s each: 30 video frames + 50 audio packets per fragment.
    for f in 0..2u32 {
        let v = video_gop(f * 30, 30);
        let a = audio_packets(f * 50, 50);
        w.write_fragment_multi(&[&v, &a]).unwrap();
    }
    w.finalize().unwrap().into_inner()
}

#[test]
fn finalized_multitrack_file_has_two_fully_tabled_traks() {
    let buf = mux_2s();
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(fourccs, vec![b"ftyp", b"mdat", b"moov"]);

    let moov = find(&boxes, b"moov").unwrap();
    let kids = children(&buf, moov);
    let traks: Vec<_> = kids.iter().filter(|b| &b.fourcc == b"trak").collect();
    assert_eq!(traks.len(), 2);
    assert!(find(&kids, b"mvex").is_none());

    // Both tracks' first samples are reachable: video "V00000", audio "A00000".
    assert!(buf.windows(6).any(|w| w == b"V00000"));
    assert!(buf.windows(6).any(|w| w == b"A00000"));
}

#[test]
fn single_track_write_fragment_still_works() {
    let cfg = match &tracks()[0] {
        TrackConfig::Video(v) => v.clone(),
        _ => unreachable!(),
    };
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg).unwrap();
    w.write_fragment(&video_gop(0, 30)).unwrap();
    let buf = w.finalize().unwrap().into_inner();
    let boxes = walk(&buf);
    assert_eq!(boxes.len(), 3); // ftyp mdat moov
}

#[test]
fn ffprobe_sees_h264_and_opus_streams() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found");
        return;
    };
    let buf = mux_2s();
    let path = std::env::temp_dir().join("clipline_multitrack.mp4");
    std::fs::write(&path, &buf).unwrap();
    let out = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,nb_frames",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(&path)
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "ffprobe failed: {stderr}");
    assert!(stdout.contains("codec_name=h264"), "got: {stdout}");
    assert!(stdout.contains("codec_name=opus"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=60"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=100"), "got: {stdout}");
    assert!(stdout.contains("duration=2.0"), "got: {stdout}");
    std::fs::remove_file(&path).ok();
}

fn ffprobe_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let local = std::path::Path::new(&home).join("bin/ffprobe");
    if local.exists() {
        return Some(local);
    }
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| std::path::Path::new(d).join("ffprobe"))
        .find(|p| p.exists())
}
