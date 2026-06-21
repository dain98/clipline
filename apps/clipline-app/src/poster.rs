//! Library poster frames: a cached JPEG thumbnail beside each clip
//! (`<clip>.poster.jpg`), extracted with ffmpeg at a representative moment
//! (chosen by the caller — typically the first event marker). The gallery
//! loads these through the asset protocol, the same path clips play back
//! through, so no new scope is needed.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::library::suppress_console;

/// Poster width in pixels. Cards render ~250px wide; 480 covers 2x displays
/// while keeping each JPEG to a few tens of KB. Height follows the aspect
/// ratio (`-2` keeps it even for the encoder).
const POSTER_WIDTH: u32 = 480;

/// The cached poster path for a clip: `clip.mp4` -> `clip.poster.jpg`. Mirrors
/// the `<clip>.markers.json` sidecar convention so the two travel together.
pub fn poster_path(clip: &Path) -> PathBuf {
    clip.with_extension("poster.jpg")
}

/// Return the cached poster for `clip`, generating it with ffmpeg if missing or
/// stale. `seek_s` is the timestamp to grab the frame from (clamped to >= 0).
/// A fresh cache hit never touches ffmpeg; an error means generation was
/// attempted and failed (e.g. no ffmpeg, unreadable clip).
pub fn ensure_poster(clip: &Path, seek_s: f64) -> Result<PathBuf, String> {
    let poster = poster_path(clip);
    if poster_is_fresh(clip, &poster) {
        return Ok(poster);
    }
    let ffmpeg = clipline_capture::ffmpeg::locate()
        .ok_or_else(|| "ffmpeg is not available for poster extraction".to_string())?;
    generate_poster(&ffmpeg, clip, &poster, seek_s)?;
    Ok(poster)
}

/// A poster is fresh when it exists and is at least as new as its clip, so a
/// clip replaced at the same path regenerates its thumbnail.
fn poster_is_fresh(clip: &Path, poster: &Path) -> bool {
    let Ok(poster_modified) = std::fs::metadata(poster).and_then(|m| m.modified()) else {
        return false;
    };
    match std::fs::metadata(clip).and_then(|m| m.modified()) {
        Ok(clip_modified) => poster_modified >= clip_modified,
        // Can't read the clip's mtime — trust the existing poster rather than
        // churn ffmpeg on every listing.
        Err(_) => true,
    }
}

fn generate_poster(ffmpeg: &Path, clip: &Path, poster: &Path, seek_s: f64) -> Result<(), String> {
    // Write to a sibling temp then rename, so a crash mid-encode never leaves a
    // half-written poster the gallery would cache.
    let mut tmp = poster.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);

    let mut cmd = Command::new(ffmpeg);
    suppress_console(&mut cmd);
    // Input-side `-ss` is a fast keyframe seek — fine for a thumbnail.
    let output = cmd
        .args(["-hide_banner", "-nostdin", "-y", "-ss", &seek_arg(seek_s), "-i"])
        .arg(clip)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &scale_filter(),
            "-q:v",
            "4",
            "-f",
            "image2",
        ])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn ffmpeg poster: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg poster failed: {stderr}"));
    }
    match std::fs::rename(&tmp, poster) {
        Ok(()) => Ok(()),
        Err(_) if poster.exists() => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("finalize poster: {e}"))
        }
    }
}

/// `-ss` value: seconds with millisecond precision, never negative.
fn seek_arg(seek_s: f64) -> String {
    format!("{:.3}", seek_s.max(0.0))
}

fn scale_filter() -> String {
    format!("scale={POSTER_WIDTH}:-2:flags=bicubic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poster_path_swaps_mp4_for_poster_jpg() {
        assert_eq!(
            poster_path(Path::new(r"C:\clips\2026\session_1.mp4")),
            PathBuf::from(r"C:\clips\2026\session_1.poster.jpg")
        );
    }

    #[test]
    fn seek_arg_clamps_negative_and_keeps_millisecond_precision() {
        assert_eq!(seek_arg(12.5), "12.500");
        assert_eq!(seek_arg(0.0), "0.000");
        assert_eq!(seek_arg(-3.0), "0.000");
    }

    #[test]
    fn poster_is_stale_when_missing() {
        let dir = std::env::temp_dir().join(format!("clipline-poster-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        std::fs::write(&clip, b"\0\0\0\0").unwrap();
        assert!(!poster_is_fresh(&clip, &poster_path(&clip)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
