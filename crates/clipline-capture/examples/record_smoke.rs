//! End-to-end smoke: WGC → NV12 → hardware H.264 MFT → ReplayRing →
//! save_replay → finalized hybrid MP4 on disk. Run manually:
//!   cargo run -p clipline-capture --example record_smoke -- --seconds 5 --window "league"
//!   cargo run -p clipline-capture --example record_smoke -- --seconds 5 --out replay.mp4

#[cfg(not(windows))]
fn main() {
    eprintln!("record_smoke is Windows-only");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clipline_capture::traits::FrameData;
    use clipline_capture::windows::{
        d3d11, find_window_by_title, MftConfig, MftH264Encoder, WasapiLoopback, WgcCapture,
    };
    use clipline_capture::{even_dimensions, LimitedCapture, Recorder};

    let args: Vec<String> = std::env::args().collect();
    let arg = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let seconds: u64 = arg("--seconds").map(|v| v.parse()).transpose()?.unwrap_or(5);
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
    let first = cap.next_frame_timeout(std::time::Duration::from_secs(5))?.ok_or("no frame")?;
    let FrameData::Gpu(tex) = &first.data else { return Err("expected GPU frame".into()) };
    let (in_w, in_h) = d3d11::texture_size(tex);
    let scale = if in_w > 2560 { 2560.0 / in_w as f64 } else { 1.0 };
    let (enc_w, enc_h) = even_dimensions(
        (in_w as f64 * scale).round() as u32,
        (in_h as f64 * scale).round() as u32,
    );
    println!("capture {in_w}x{in_h} -> encode {enc_w}x{enc_h} @ {FPS} fps");

    let cfg = MftConfig { width: enc_w, height: enc_h, fps: FPS, bitrate_bps: 12_000_000 };
    let encoder = MftH264Encoder::new(&device, in_w, in_h, cfg)?;

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

    let segments = rec.ring().len();
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
            "-v", "error",
            "-show_entries", "stream=codec_name,width,height,nb_frames,avg_frame_rate",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1",
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
