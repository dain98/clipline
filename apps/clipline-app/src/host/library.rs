#[allow(
    dead_code,
    reason = "fallback media dispatch will use this classifier once library media routes are wired"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMediaKind {
    Clip,
    Poster,
}

#[allow(
    dead_code,
    reason = "fallback media dispatch will use this classifier once library media routes are wired"
)]
pub fn media_kind_for_path(path: &str) -> Option<HostMediaKind> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())?;
    match ext.to_ascii_lowercase().as_str() {
        "mp4" => Some(HostMediaKind::Clip),
        "png" | "jpg" | "jpeg" | "webp" => Some(HostMediaKind::Poster),
        _ => None,
    }
}

pub fn list_clips_from_dir(
    dir: std::path::PathBuf,
) -> Result<Vec<crate::library::ClipInfo>, String> {
    crate::library::list_clips_from_dir(dir)
}

pub fn storage_status_for_dir(
    dir: std::path::PathBuf,
    quota_bytes: Option<u64>,
) -> Result<crate::library::StorageInfo, String> {
    crate::library::storage_status_for_dir(dir, quota_bytes)
}

pub fn reveal_clip(path: &str, settings: &crate::library::StorageSettings) -> Result<(), String> {
    let target = crate::library::validate_clip_path(settings, path)?;
    let dir = target
        .parent()
        .ok_or_else(|| "clip has no containing folder".to_string())?;
    crate::library::open_folder_path(dir)
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
