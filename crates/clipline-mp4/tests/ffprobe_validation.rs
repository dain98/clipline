use std::io::Cursor;
use std::process::Command;

use clipline_mp4::{FragSample, HybridMp4Writer, VideoTrackConfig};

fn ffprobe_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let local = std::path::Path::new(&home).join("bin/ffprobe");
    if local.exists() {
        return Some(local);
    }
    which("ffprobe")
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| std::path::Path::new(d).join(bin))
        .find(|p| p.exists())
}

#[test]
fn ffprobe_parses_finalized_file_as_standard_mp4() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found; container validated by walker tests only");
        return;
    };

    let cfg = VideoTrackConfig::h264(
        128,
        128,
        90_000,
        vec![0x67, 0x64, 0x00, 0x0A, 0xAC, 0x72, 0x84, 0x44, 0x26, 0x84],
        vec![0x68, 0xEE, 0x38, 0x80],
    );
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg).unwrap();
    // 60 frames at 30 fps in GOPs of 10 → 2.0 s duration.
    for _ in 0..6 {
        let samples: Vec<FragSample> = (0..10)
            .map(|i| FragSample {
                data: vec![0xAB; 100 + i],
                duration: 3000,
                is_sync: i == 0,
            })
            .collect();
        w.write_fragment(&samples).unwrap();
    }
    let buf = w.finalize().unwrap().into_inner();

    let path = std::env::temp_dir().join("clipline_hybrid_test.mp4");
    std::fs::write(&path, &buf).unwrap();

    let out = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,nb_frames",
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
    assert!(stdout.contains("nb_frames=60"), "got: {stdout}");
    // duration=2.000000 (±ffprobe formatting)
    assert!(stdout.contains("duration=2.0"), "got: {stdout}");

    std::fs::remove_file(&path).ok();
}
