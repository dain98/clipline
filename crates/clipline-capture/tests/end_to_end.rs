use std::io::Cursor;
use std::process::Command;

use clipline_capture::{MockAudioSource, MockCapture, MockEncoder, Recorder};
use clipline_mp4::walker::walk;

fn recorder_with_3s_of_footage() -> Recorder<MockCapture, MockEncoder> {
    let mut rec = Recorder::new(
        MockCapture::new(90, 30),
        MockEncoder::new(30, 30),
        usize::MAX,
    );
    rec.run_to_end().unwrap();
    rec
}

#[test]
fn save_replay_produces_a_standard_mp4_of_the_window() {
    let rec = recorder_with_3s_of_footage();
    // Save the trailing 2 s → GOPs at t=1.0 and t=2.0 → 60 samples.
    let (buf, end_pts) = rec
        .save_replay(Cursor::new(Vec::new()), 2.0, None)
        .map(|(w, end)| (w.into_inner(), end))
        .unwrap();

    assert!((end_pts - 3.0).abs() < 1e-6);
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(fourccs, vec![b"ftyp", b"mdat", b"moov"]);
    // First saved sample is frame 30 (the GOP at t=1.0): "F000030".
    assert!(
        buf.windows(7).any(|w| w == b"F000030"),
        "window starts at frame 30"
    );
    assert!(
        !buf.windows(7).any(|w| w == b"F000029"),
        "frame 29 excluded"
    );
}

#[test]
fn smart_mode_skips_already_saved_footage() {
    let rec = recorder_with_3s_of_footage();
    let (_, end) = rec.save_replay(Cursor::new(Vec::new()), 2.0, None).unwrap();
    // Nothing new since the last save → empty result, no file content.
    let res = rec.save_replay(Cursor::new(Vec::new()), 2.0, Some(end));
    assert!(
        res.is_err(),
        "saving zero new footage is an error, not a silent empty file"
    );
}

#[test]
fn ffprobe_accepts_the_saved_replay() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found");
        return;
    };
    let rec = recorder_with_3s_of_footage();
    let (w, _) = rec.save_replay(Cursor::new(Vec::new()), 2.0, None).unwrap();
    let buf = w.into_inner();

    let path = std::env::temp_dir().join("clipline_e2e_replay.mp4");
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
    assert!(out.status.success());
    assert!(stdout.contains("codec_name=h264"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=60"), "got: {stdout}");
    assert!(stdout.contains("duration=2.0"), "got: {stdout}");
    std::fs::remove_file(&path).ok();
}

#[test]
fn ffprobe_accepts_a_video_plus_audio_replay() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found");
        return;
    };
    let mut rec = Recorder::new(
        MockCapture::new(90, 30),
        MockEncoder::new(30, 30),
        usize::MAX,
    )
    .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
    rec.run_to_end().unwrap();
    let (w, _) = rec.save_replay(Cursor::new(Vec::new()), 2.0, None).unwrap();
    let buf = w.into_inner();

    let path = std::env::temp_dir().join("clipline_e2e_av.mp4");
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
    assert!(out.status.success());
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
