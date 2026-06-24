use std::path::{Path, PathBuf};

pub fn poster_path(clip: &Path) -> PathBuf {
    clip.with_extension("poster.jpg")
}

pub fn ensure_poster(_clip: &Path, _seek_s: f64) -> Result<PathBuf, String> {
    Err("poster extraction is unavailable on macOS shell stubs".into())
}
