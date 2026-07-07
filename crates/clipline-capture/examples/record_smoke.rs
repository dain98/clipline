//! End-to-end smoke: WGC → NV12 → encoder → ReplayRing → save_replay →
//! finalized hybrid MP4 on disk. Run manually:
//!   cargo run -p clipline-capture --example record_smoke -- --seconds 5 --window "league"
//!   cargo run -p clipline-capture --example record_smoke -- --seconds 5 --out replay.mp4
//! Default encoder is the hardware H.264 MFT. `--encoder ffmpeg:<backend>:<codec>`
//! drives the FFmpeg subprocess path (real GPU NV12 convert + readback), e.g.
//!   cargo run -p clipline-capture --example record_smoke -- --encoder ffmpeg:amf:hevc
//!   cargo run -p clipline-capture --example record_smoke -- --encoder ffmpeg:svtav1:av1 --out av1.mp4

#[cfg(not(windows))]
fn main() {
    eprintln!("record_smoke is Windows-only");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clipline_capture::ffmpeg_encoder::FfmpegVideoEncoder;
    use clipline_capture::probe::{Codec, EncoderBackend};
    use clipline_capture::traits::{Encoder, FrameData};
    use clipline_capture::windows::{
        d3d11, find_window_by_title, MftConfig, MftH264Encoder, WasapiLoopback, WgcCapture,
    };
    use clipline_capture::{even_dimensions, ffmpeg, LimitedCapture, Recorder};

    let args: Vec<String> = std::env::args().collect();
    let arg = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let seconds: u64 = arg("--seconds")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(5);
    let out_path = arg("--out").unwrap_or_else(|| "replay_smoke.mp4".into());
    const FPS: u32 = 60;

    let (device, _ctx) = d3d11::create_device()?;
    // One clock for every engine of the recording (ddoc §6).
    let clock = WgcCapture::new_clock()?;
    let mut cap = match arg("--window") {
        Some(needle) => {
            let hwnd = find_window_by_title(&needle)
                .ok_or_else(|| format!("no visible window matching {needle:?}"))?;
            println!("capturing window matching {needle:?}");
            WgcCapture::for_window_on(device.clone(), hwnd, clock)?
        }
        None => {
            println!("capturing primary monitor");
            WgcCapture::primary_monitor_on(device.clone(), clock)?
        }
    };

    // First frame tells us the capture size; encode size is even-rounded
    // and capped at 2560 wide (H.264 hardware limits; ultrawides scale).
    let first = cap
        .next_frame_timeout(std::time::Duration::from_secs(5))?
        .ok_or("no frame")?;
    let FrameData::Gpu(tex) = &first.data else {
        return Err("expected GPU frame".into());
    };
    let (in_w, in_h) = d3d11::texture_size(tex);
    let scale = if in_w > 2560 {
        2560.0 / in_w as f64
    } else {
        1.0
    };
    let (enc_w, enc_h) = even_dimensions(
        (in_w as f64 * scale).round() as u32,
        (in_h as f64 * scale).round() as u32,
    );
    println!("capture {in_w}x{in_h} -> encode {enc_w}x{enc_h} @ {FPS} fps");

    let encoder: Box<dyn Encoder> = match arg("--encoder") {
        Some(spec) => {
            // ffmpeg:<backend>:<codec>, e.g. ffmpeg:amf:hevc, ffmpeg:svtav1:av1.
            let parts: Vec<&str> = spec.split(':').collect();
            if parts.first() != Some(&"ffmpeg") || parts.len() != 3 {
                return Err(
                    format!("bad --encoder {spec:?}; want ffmpeg:<backend>:<codec>").into(),
                );
            }
            let backend = match parts[1] {
                "nvenc" => EncoderBackend::Nvenc,
                "amf" => EncoderBackend::Amf,
                "qsv" => EncoderBackend::QuickSync,
                "svtav1" => EncoderBackend::SvtAv1,
                other => return Err(format!("unknown backend {other:?}").into()),
            };
            let codec = match parts[2] {
                "h264" => Codec::H264,
                "hevc" => Codec::Hevc,
                "av1" => Codec::Av1,
                other => return Err(format!("unknown codec {other:?}").into()),
            };
            let ffmpeg = ffmpeg::locate().ok_or("no ffmpeg located")?;
            println!(
                "ffmpeg encoder {backend:?}/{codec:?} via {}",
                ffmpeg.display()
            );
            Box::new(FfmpegVideoEncoder::new_on(
                &device, &ffmpeg, backend, codec, in_w, in_h, None, enc_w, enc_h, FPS, 12_000_000,
            )?)
        }
        None => {
            let cfg = MftConfig {
                width: enc_w,
                height: enc_h,
                fps: FPS,
                bitrate_bps: 12_000_000,
                encoder_backend: None,
                resize_mode: clipline_capture::windows::nv12::ResizeMode::Stretch,
            };
            Box::new(MftH264Encoder::new(&device, in_w, in_h, cfg)?)
        }
    };

    let started = std::time::Instant::now();
    let mut rec = Recorder::new(
        LimitedCapture::new(cap, seconds * FPS as u64),
        encoder,
        usize::MAX,
    );
    if args.iter().any(|a| a == "--audio") {
        rec = rec.with_audio(Box::new(WasapiLoopback::start(clock)?));
        println!("system loopback audio attached");
    }
    rec.run_to_end()?;
    let elapsed = started.elapsed();

    let ring = rec
        .ring()
        .expect("record_smoke uses the default in-memory replay ring");
    let segments = ring.len();
    {
        use clipline_capture::{validate_timeline, SyncTolerances};
        let segs: Vec<_> = ring.segments().collect();
        match validate_timeline(&segs, &SyncTolerances::default()) {
            Ok(r) => println!(
                "sync: video {:.3}s, audio {:?}s, max gap {:.1}ms, drift {:?}ms",
                r.video_duration_s,
                r.audio_duration_s
                    .iter()
                    .map(|d| (d * 1e3).round() / 1e3)
                    .collect::<Vec<_>>(),
                r.max_video_gap_s * 1e3,
                r.total_drift_s
                    .iter()
                    .map(|d| (d * 1e4).round() / 10.0)
                    .collect::<Vec<_>>(),
            ),
            Err(v) => return Err(format!("timeline validation failed: {v}").into()),
        }
    }
    let file = std::fs::File::create(&out_path)?;
    let (file, end_pts) = rec.save_replay(file, seconds as f64 + 2.0, None)?;
    drop(file);
    let bytes = std::fs::metadata(&out_path)?.len();
    println!(
        "recorded {segments} GOP segment(s) in {:.1}s, saved {} ({:.2} MiB, end pts {end_pts:.2}s)",
        elapsed.as_secs_f64(),
        out_path,
        bytes as f64 / (1024.0 * 1024.0)
    );

    // ffprobe verification, skip-if-absent like the e2e tests.
    match std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,width,height,nb_frames,avg_frame_rate",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(&out_path)
        .output()
    {
        Ok(out) => {
            println!("--- ffprobe ---");
            print!("{}", String::from_utf8_lossy(&out.stdout));
            if !out.status.success() {
                eprint!("{}", String::from_utf8_lossy(&out.stderr));
                return Err("ffprobe rejected the file".into());
            }
        }
        Err(_) => println!("SKIP: ffprobe not found — verify {out_path} manually"),
    }
    Ok(())
}
