//! Library poster frames: a cached JPEG thumbnail beside each clip
//! (`<clip>.poster.jpg`), extracted with ffmpeg at a representative moment
//! (chosen by the caller — typically the first event marker). The gallery
//! loads these through the asset protocol, the same path clips play back
//! through, so no new scope is needed.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::library::suppress_console;

/// Poster width in pixels. Cards render ~250px wide; 480 covers 2x displays
/// while keeping each JPEG to a few tens of KB. Height follows the aspect
/// ratio (`-2` keeps it even for the encoder).
const POSTER_WIDTH: u32 = 480;
static NEXT_POSTER_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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
    let tmp = PosterTemp::reserve(poster)?;

    let mut cmd = Command::new(ffmpeg);
    suppress_console(&mut cmd);
    // Input-side `-ss` is a fast keyframe seek — fine for a thumbnail.
    let output = cmd
        .args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-ss",
            &seek_arg(seek_s),
            "-i",
        ])
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
        .arg(tmp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn ffmpeg poster: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg poster failed: {stderr}"));
    }
    tmp.publish(poster)
}

struct PosterTemp {
    path: PathBuf,
    armed: bool,
}

impl PosterTemp {
    fn reserve(poster: &Path) -> Result<Self, String> {
        let file_name = poster
            .file_name()
            .ok_or_else(|| "poster path has no file name".to_string())?;
        for _ in 0..64 {
            let id = NEXT_POSTER_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let mut temp_name = file_name.to_os_string();
            temp_name.push(format!(".tmp.{}.{id}", std::process::id()));
            let path = poster.with_file_name(temp_name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path, armed: true }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("reserve poster temp: {error}")),
            }
        }
        Err("reserve poster temp: unique-name attempts exhausted".to_string())
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn publish(mut self, poster: &Path) -> Result<(), String> {
        atomic_replace_file(&self.path, poster)
            .map_err(|error| format!("finalize poster: {error}"))?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for PosterTemp {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
fn atomic_replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from_w: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_w: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are live, NUL-terminated UTF-16 buffers for the duration of the call.
    let moved = unsafe {
        MoveFileExW(
            from_w.as_ptr(),
            to_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
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

    #[test]
    fn concurrent_poster_temps_have_independent_owned_paths() {
        let dir = test_dir("owned-temp");
        let poster = dir.join("clip.poster.jpg");
        let first = PosterTemp::reserve(&poster).unwrap();
        let second = PosterTemp::reserve(&poster).unwrap();
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();

        assert_ne!(first_path, second_path);
        assert_eq!(first_path.parent(), poster.parent());
        assert_eq!(second_path.parent(), poster.parent());
        assert!(first_path.exists());
        assert!(second_path.exists());

        drop(first);
        assert!(!first_path.exists());
        assert!(second_path.exists());
        drop(second);
        assert!(!second_path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn poster_temp_atomically_replaces_stale_destination() {
        let dir = test_dir("atomic-publish");
        let poster = dir.join("clip.poster.jpg");
        std::fs::write(&poster, b"stale").unwrap();
        let temp = PosterTemp::reserve(&poster).unwrap();
        let temp_path = temp.path().to_path_buf();
        std::fs::write(&temp_path, b"complete jpeg").unwrap();

        temp.publish(&poster).unwrap();

        assert_eq!(std::fs::read(&poster).unwrap(), b"complete jpeg");
        assert!(!temp_path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn test_dir(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "clipline-poster-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
