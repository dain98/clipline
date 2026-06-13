//! WGC capture smoke test (run manually — needs a desktop session):
//!   cargo run -p clipline-capture --example wgc_smoke -- --frames 120
//!   cargo run -p clipline-capture --example wgc_smoke -- --window "notepad"
//! Reports resolution and measured fps from frame pts deltas.

#[cfg(not(windows))]
fn main() {
    eprintln!("wgc_smoke is Windows-only");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clipline_capture::traits::FrameData;
    use clipline_capture::windows::{find_window_by_title, WgcCapture};
    use clipline_capture::CaptureEngine;

    let args: Vec<String> = std::env::args().collect();
    let arg = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let frames: u32 = arg("--frames")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(120);

    let mut cap = match arg("--window") {
        Some(needle) => {
            let hwnd = find_window_by_title(&needle)
                .ok_or_else(|| format!("no visible window matching {needle:?}"))?;
            println!("capturing window matching {needle:?}");
            WgcCapture::for_window(hwnd)?
        }
        None => {
            println!("capturing primary monitor");
            WgcCapture::primary_monitor()?
        }
    };

    let mut pts = Vec::with_capacity(frames as usize);
    let mut resolution = (0, 0);
    while pts.len() < frames as usize {
        let Some(frame) = cap.next_frame()? else {
            println!("capture ended early after {} frames", pts.len());
            break;
        };
        if let FrameData::Gpu(tex) = &frame.data {
            resolution = clipline_capture::windows::d3d11::texture_size(tex);
        }
        pts.push(frame.pts_s);
        if pts.len() % 30 == 0 {
            println!("  {} frames…", pts.len());
        }
    }

    let span = pts.last().unwrap_or(&0.0) - pts.first().unwrap_or(&0.0);
    let fps = if span > 0.0 {
        (pts.len() as f64 - 1.0) / span
    } else {
        0.0
    };
    let deltas: Vec<f64> = pts.windows(2).map(|w| w[1] - w[0]).collect();
    let (min_d, max_d) = deltas
        .iter()
        .fold((f64::MAX, 0.0f64), |(lo, hi), d| (lo.min(*d), hi.max(*d)));

    println!(
        "captured {} frames @ {}x{}",
        pts.len(),
        resolution.0,
        resolution.1
    );
    println!(
        "pts span {span:.3}s -> {fps:.1} fps (frame gap {:.1}-{:.1} ms)",
        min_d * 1e3,
        max_d * 1e3
    );
    println!(
        "first pts {:.4}s, monotonic: {}",
        pts.first().unwrap_or(&0.0),
        pts.windows(2).all(|w| w[1] >= w[0])
    );
    Ok(())
}
