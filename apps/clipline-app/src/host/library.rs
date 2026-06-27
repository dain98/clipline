use std::path::{Path, PathBuf};

#[allow(
    dead_code,
    reason = "fallback media dispatch will use this classifier once library media routes are wired"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMediaKind {
    Clip,
    Poster,
    AudioPreview,
    CloudCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedHostMedia {
    pub path: PathBuf,
    pub kind: HostMediaKind,
}

#[allow(
    dead_code,
    reason = "fallback media dispatch will use this classifier once library media routes are wired"
)]
pub fn media_kind_for_path(path: &str) -> Option<HostMediaKind> {
    let ext = Path::new(path).extension().and_then(|ext| ext.to_str())?;
    match ext.to_ascii_lowercase().as_str() {
        "mp4" => Some(HostMediaKind::Clip),
        "png" | "jpg" | "jpeg" | "webp" => Some(HostMediaKind::Poster),
        _ => None,
    }
}

pub fn list_clips_from_dir(dir: PathBuf) -> Result<Vec<crate::library::ClipInfo>, String> {
    crate::library::list_clips_from_dir(dir)
}

pub fn storage_status_for_dir(
    dir: PathBuf,
    quota_bytes: Option<u64>,
) -> Result<crate::library::StorageInfo, String> {
    crate::library::storage_status_for_dir(dir, quota_bytes)
}

pub fn validate_media_path(
    settings: &crate::library::StorageSettings,
    path: &Path,
) -> Result<ValidatedHostMedia, String> {
    if let Ok(path) = validate_clip_path(settings, path) {
        return Ok(ValidatedHostMedia {
            path,
            kind: HostMediaKind::Clip,
        });
    }
    if let Ok(path) = validate_audio_preview_path(path) {
        return Ok(ValidatedHostMedia {
            path,
            kind: HostMediaKind::AudioPreview,
        });
    }
    if let Ok(path) = validate_poster_path(settings, path) {
        return Ok(ValidatedHostMedia {
            path,
            kind: HostMediaKind::Poster,
        });
    }
    if let Ok(path) = validate_cloud_cache_path(path) {
        return Ok(ValidatedHostMedia {
            path,
            kind: HostMediaKind::CloudCache,
        });
    }
    Err("refusing to register unvalidated media path".into())
}

pub fn validate_clip_path(
    settings: &crate::library::StorageSettings,
    path: &Path,
) -> Result<PathBuf, String> {
    let path = path
        .to_str()
        .ok_or_else(|| "clip path is not valid UTF-8".to_string())?;
    crate::library::validate_clip_path(settings, path)
}

pub fn validate_audio_preview_path(path: &Path) -> Result<PathBuf, String> {
    let path = validate_existing_file_under_dir(
        path,
        &crate::settings::audio_preview_cache_dir(),
        "audio preview",
    )?;
    if !has_extension(&path, "mp4") {
        return Err(format!("audio preview {path:?} is not an MP4"));
    }
    Ok(path)
}

pub fn validate_cloud_cache_path(path: &Path) -> Result<PathBuf, String> {
    validate_existing_file_under_dir(
        path,
        &crate::settings::persistence::config_base().join("cloud-cache"),
        "cloud cache asset",
    )
}

pub fn validate_poster_path(
    settings: &crate::library::StorageSettings,
    path: &Path,
) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "poster path has no file name".to_string())?;
    let clip_stem = file_name
        .strip_suffix(".poster.jpg")
        .ok_or_else(|| "poster path is not a Clipline poster".to_string())?;
    if clip_stem.is_empty() {
        return Err("poster path is not a Clipline poster".into());
    }
    let clip = path.with_file_name(format!("{clip_stem}.mp4"));
    let clip = validate_clip_path(settings, &clip)?;
    let expected = crate::poster::poster_path(&clip);
    let expected_parent = expected
        .parent()
        .ok_or_else(|| "poster path has no parent".to_string())?
        .canonicalize()
        .map_err(|e| format!("canonicalize poster parent: {e}"))?;
    let expected_abs = expected_parent.join(
        expected
            .file_name()
            .ok_or_else(|| "poster path has no file name".to_string())?,
    );
    let candidate = path
        .canonicalize()
        .map_err(|e| format!("canonicalize poster {path:?}: {e}"))?;
    if candidate != expected_abs {
        return Err("poster path does not match a validated clip".into());
    }
    Ok(candidate)
}

pub fn reveal_clip(path: &str, settings: &crate::library::StorageSettings) -> Result<(), String> {
    let target = crate::library::validate_clip_path(settings, path)?;
    let dir = target
        .parent()
        .ok_or_else(|| "clip has no containing folder".to_string())?;
    crate::library::open_folder_path(dir)
}

fn validate_existing_file_under_dir(
    path: &Path,
    dir: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    let canonical_dir = dir
        .canonicalize()
        .map_err(|e| format!("canonicalize {label} directory {dir:?}: {e}"))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|e| format!("canonicalize {label} {path:?}: {e}"))?;
    if !canonical_path.starts_with(&canonical_dir) {
        return Err(format!(
            "{label} {canonical_path:?} escaped {canonical_dir:?}"
        ));
    }
    if !canonical_path.is_file() {
        return Err(format!("{label} {canonical_path:?} is not a file"));
    }
    Ok(canonical_path)
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_validates_library_media_kind_from_extension() {
        assert_eq!(media_kind_for_path("clip.mp4"), Some(HostMediaKind::Clip));
        assert_eq!(
            media_kind_for_path("poster.png"),
            Some(HostMediaKind::Poster)
        );
        assert_eq!(media_kind_for_path("notes.txt"), None);
    }
}
