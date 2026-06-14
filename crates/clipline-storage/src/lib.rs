//! Filesystem storage management for saved clips.

pub mod sessions;

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageStatus {
    pub clip_count: usize,
    pub total_bytes: u64,
    pub quota_bytes: Option<u64>,
}

impl StorageStatus {
    pub fn is_over_quota(&self) -> bool {
        self.quota_bytes
            .is_some_and(|quota| self.total_bytes > quota)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcReport {
    pub deleted_clips: usize,
    pub freed_bytes: u64,
    pub status: StorageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingRecoveryReport {
    pub recovered: Vec<PathBuf>,
    pub deleted_empty: usize,
}

pub fn storage_status(dir: &Path, quota_bytes: Option<u64>) -> io::Result<StorageStatus> {
    let clips = inventory(dir)?;
    Ok(status_from_clips(&clips, quota_bytes))
}

pub fn recover_recording_files(dir: &Path) -> io::Result<RecordingRecoveryReport> {
    let mut report = RecordingRecoveryReport {
        recovered: Vec::new(),
        deleted_empty: 0,
    };
    visit_media_dirs(dir, |media_dir| {
        for entry in fs::read_dir(media_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !is_recording_mp4(&path) {
                continue;
            }
            let meta = entry.metadata()?;
            if !meta.is_file() {
                continue;
            }
            if meta.len() == 0 {
                remove_file_if_exists(&path)?;
                report.deleted_empty += 1;
                continue;
            }
            let final_path = recording_final_path(&path)
                .map(|candidate| unique_recovered_path(&candidate))
                .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "invalid recording name"))?;
            fs::rename(&path, &final_path)?;
            report.recovered.push(final_path);
        }
        Ok(())
    })?;
    Ok(report)
}

pub fn enforce_quota(
    dir: &Path,
    quota_bytes: Option<u64>,
    protect: Option<&Path>,
) -> io::Result<GcReport> {
    let Some(quota) = quota_bytes else {
        return Ok(GcReport {
            deleted_clips: 0,
            freed_bytes: 0,
            status: storage_status(dir, quota_bytes)?,
        });
    };

    let mut clips = inventory(dir)?;
    let mut total_bytes = clips.iter().map(ClipFile::total_bytes).sum::<u64>();
    let mut deleted_clips = 0usize;
    let mut freed_bytes = 0u64;

    let undeletable_bytes = clips
        .iter()
        .filter(|clip| !clip.can_delete(protect))
        .map(ClipFile::total_bytes)
        .sum::<u64>();
    if undeletable_bytes > quota {
        return Ok(GcReport {
            deleted_clips,
            freed_bytes,
            status: status_from_clips(&clips, quota_bytes),
        });
    }

    clips.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
    });

    for clip in clips {
        if total_bytes <= quota {
            break;
        }
        if !clip.can_delete(protect) {
            continue;
        }

        let clip_bytes = clip.total_bytes();
        remove_file_if_exists(&clip.path)?;
        if let Some(sidecar) = &clip.sidecar {
            remove_file_if_exists(sidecar)?;
        }
        // Session folders disappear with their last clip; remove_dir refuses
        // non-empty directories, so a leftover sidecar/export keeps it alive.
        if let Some(parent) = clip.path.parent() {
            if parent != dir {
                let _ = fs::remove_dir(parent);
            }
        }
        total_bytes = total_bytes.saturating_sub(clip_bytes);
        freed_bytes += clip_bytes;
        deleted_clips += 1;
    }

    Ok(GcReport {
        deleted_clips,
        freed_bytes,
        status: storage_status(dir, quota_bytes)?,
    })
}

#[derive(Debug, Clone)]
struct ClipFile {
    path: PathBuf,
    sidecar: Option<PathBuf>,
    mp4_bytes: u64,
    sidecar_bytes: u64,
    modified: SystemTime,
    recording: bool,
}

impl ClipFile {
    fn total_bytes(&self) -> u64 {
        self.mp4_bytes + self.sidecar_bytes
    }

    fn can_delete(&self, protect: Option<&Path>) -> bool {
        !self.recording && !protect.is_some_and(|protected| same_path(&self.path, protected))
    }
}

/// Clips live at the root (legacy) or one level down in session folders.
fn inventory(dir: &Path) -> io::Result<Vec<ClipFile>> {
    let mut clips = Vec::new();
    visit_media_dirs(dir, |media_dir| collect_clips(media_dir, &mut clips))?;
    Ok(clips)
}

fn collect_clips(dir: &Path, clips: &mut Vec<ClipFile>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let recording = is_recording_mp4(&path);
        if !is_mp4(&path) && !recording {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let sidecar = sidecar_path(&path);
        let sidecar_bytes = if recording {
            0
        } else {
            optional_file_len(&sidecar)?
        };
        clips.push(ClipFile {
            path,
            sidecar: (sidecar_bytes > 0 || sidecar.exists()).then_some(sidecar),
            mp4_bytes: meta.len(),
            sidecar_bytes,
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
            recording,
        });
    }
    Ok(())
}

fn status_from_clips(clips: &[ClipFile], quota_bytes: Option<u64>) -> StorageStatus {
    StorageStatus {
        clip_count: clips.iter().filter(|clip| !clip.recording).count(),
        total_bytes: clips.iter().map(ClipFile::total_bytes).sum(),
        quota_bytes,
    }
}

fn is_mp4(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
}

fn is_recording_mp4(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".mp4.recording"))
}

fn recording_final_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let final_name = name.strip_suffix(".recording")?;
    Some(path.with_file_name(final_name))
}

fn unique_recovered_path(candidate: &Path) -> PathBuf {
    if !candidate.exists() {
        return candidate.to_path_buf();
    }
    let parent = candidate.parent().unwrap_or_else(|| Path::new(""));
    let stem = candidate
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    for attempt in 0u32..1024 {
        let name = if attempt == 0 {
            format!("{stem}_recovered.mp4")
        } else {
            format!("{stem}_recovered_{attempt}.mp4")
        };
        let recovered = parent.join(name);
        if !recovered.exists() {
            return recovered;
        }
    }
    parent.join(format!(
        "{stem}_recovered_{}.mp4",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn sidecar_path(path: &Path) -> PathBuf {
    path.with_extension("markers.json")
}

fn optional_file_len(path: &Path) -> io::Result<u64> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => Ok(meta.len()),
        Ok(_) => Ok(0),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn visit_media_dirs(dir: &Path, mut f: impl FnMut(&Path) -> io::Result<()>) -> io::Result<()> {
    f(dir)?;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.metadata()?.is_dir() {
            f(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "clipline-storage-{name}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, name: &str, bytes: usize) -> PathBuf {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, vec![0u8; bytes]).unwrap();
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tick_mtime() {
        std::thread::sleep(Duration::from_millis(20));
    }

    #[test]
    fn status_counts_mp4_and_marker_sidecars() {
        let dir = TestDir::new("status-counts");
        dir.write("a.mp4", 10);
        dir.write("a.markers.json", 3);
        dir.write("b.mp4", 7);

        let status = storage_status(dir.path(), Some(100)).unwrap();

        assert_eq!(status.clip_count, 2);
        assert_eq!(status.total_bytes, 20);
        assert_eq!(status.quota_bytes, Some(100));
        assert!(!status.is_over_quota());
    }

    #[test]
    fn status_counts_recording_bytes_without_counting_a_clip() {
        let dir = TestDir::new("status-recording");
        dir.write("saved.mp4", 10);
        dir.write("session.mp4.recording", 90);

        let status = storage_status(dir.path(), Some(100)).unwrap();

        assert_eq!(status.clip_count, 1);
        assert_eq!(status.total_bytes, 100);
    }

    #[test]
    fn inventory_ignores_non_mp4_files() {
        let dir = TestDir::new("ignore-non-mp4");
        dir.write("notes.txt", 99);
        dir.write("clip.mp4", 4);

        let status = storage_status(dir.path(), None).unwrap();

        assert_eq!(status.clip_count, 1);
        assert_eq!(status.total_bytes, 4);
    }

    #[test]
    fn enforce_quota_deletes_oldest_until_under_budget() {
        let dir = TestDir::new("oldest-first");
        let a = dir.write("a.mp4", 10);
        tick_mtime();
        let b = dir.write("b.mp4", 10);
        tick_mtime();
        let c = dir.write("c.mp4", 10);

        let report = enforce_quota(dir.path(), Some(15), None).unwrap();

        assert_eq!(report.deleted_clips, 2);
        assert_eq!(report.freed_bytes, 20);
        assert!(!a.exists());
        assert!(!b.exists());
        assert!(c.exists());
        assert_eq!(report.status.total_bytes, 10);
    }

    #[test]
    fn enforce_quota_deletes_marker_sidecar_with_clip() {
        let dir = TestDir::new("sidecar-delete");
        let old = dir.write("old.mp4", 10);
        let sidecar = dir.write("old.markers.json", 2);
        tick_mtime();
        let keep = dir.write("keep.mp4", 10);

        let report = enforce_quota(dir.path(), Some(10), None).unwrap();

        assert_eq!(report.deleted_clips, 1);
        assert_eq!(report.freed_bytes, 12);
        assert!(!old.exists());
        assert!(!sidecar.exists());
        assert!(keep.exists());
        assert_eq!(report.status.total_bytes, 10);
    }

    #[test]
    fn enforce_quota_leaves_library_when_protected_clip_alone_exceeds_budget() {
        let dir = TestDir::new("protect-fresh");
        let old = dir.write("old.mp4", 10);
        tick_mtime();
        let fresh = dir.write("fresh.mp4", 20);

        let report = enforce_quota(dir.path(), Some(15), Some(&fresh)).unwrap();

        assert_eq!(report.deleted_clips, 0);
        assert_eq!(report.freed_bytes, 0);
        assert!(old.exists());
        assert!(fresh.exists());
        assert_eq!(report.status.total_bytes, 30);
        assert!(report.status.is_over_quota());
    }

    #[test]
    fn enforce_quota_counts_active_recording_but_never_deletes_it() {
        let dir = TestDir::new("recording-quota");
        let old = dir.write("old.mp4", 10);
        tick_mtime();
        let recording = dir.write("session.mp4.recording", 12);
        tick_mtime();
        let keep = dir.write("keep.mp4", 5);

        let report = enforce_quota(dir.path(), Some(20), None).unwrap();

        assert_eq!(report.deleted_clips, 1);
        assert!(!old.exists());
        assert!(recording.exists());
        assert!(keep.exists());
        assert_eq!(report.status.clip_count, 1);
        assert_eq!(report.status.total_bytes, 17);
    }

    #[test]
    fn recover_recording_files_renames_non_empty_and_deletes_empty() {
        let dir = TestDir::new("recording-recovery");
        let recording = dir.write("2026-06-13 15-04/session_1.mp4.recording", 10);
        let empty = dir.write("empty.mp4.recording", 0);

        let report = recover_recording_files(dir.path()).unwrap();

        assert_eq!(report.deleted_empty, 1);
        assert!(!recording.exists());
        assert!(!empty.exists());
        assert_eq!(report.recovered.len(), 1);
        assert_eq!(
            report.recovered[0]
                .file_name()
                .and_then(|name| name.to_str()),
            Some("session_1.mp4")
        );
        assert!(report.recovered[0].exists());
    }

    #[test]
    fn status_counts_clips_inside_session_folders() {
        let dir = TestDir::new("session-status");
        dir.write("legacy.mp4", 10);
        dir.write("2026-06-12 14-30/clip.mp4", 7);
        dir.write("2026-06-12 14-30/clip.markers.json", 3);

        let status = storage_status(dir.path(), Some(100)).unwrap();

        assert_eq!(status.clip_count, 2);
        assert_eq!(status.total_bytes, 20);
    }

    #[test]
    fn enforce_quota_crosses_folders_and_removes_emptied_session_dirs() {
        let dir = TestDir::new("session-gc");
        let old = dir.write("2026-06-11 09-00/old.mp4", 10);
        let old_sidecar = dir.write("2026-06-11 09-00/old.markers.json", 2);
        tick_mtime();
        let legacy = dir.write("legacy.mp4", 10);
        tick_mtime();
        let fresh = dir.write("2026-06-12 14-30/fresh.mp4", 10);

        let report = enforce_quota(dir.path(), Some(20), None).unwrap();

        assert_eq!(report.deleted_clips, 1);
        assert_eq!(report.freed_bytes, 12);
        assert!(!old.exists());
        assert!(!old_sidecar.exists());
        assert!(
            !old.parent().unwrap().exists(),
            "emptied session folder must be removed"
        );
        assert!(legacy.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn enforce_quota_keeps_session_dirs_that_still_hold_clips() {
        let dir = TestDir::new("session-keep");
        let old = dir.write("2026-06-12 14-30/old.mp4", 10);
        tick_mtime();
        let new = dir.write("2026-06-12 14-30/new.mp4", 10);

        let report = enforce_quota(dir.path(), Some(10), None).unwrap();

        assert_eq!(report.deleted_clips, 1);
        assert!(!old.exists());
        assert!(new.exists());
        assert!(new.parent().unwrap().exists());
    }

    #[test]
    fn disabled_quota_does_not_delete() {
        let dir = TestDir::new("disabled");
        let clip = dir.write("clip.mp4", 10);

        let report = enforce_quota(dir.path(), None, None).unwrap();

        assert_eq!(report.deleted_clips, 0);
        assert_eq!(report.freed_bytes, 0);
        assert!(clip.exists());
        assert_eq!(report.status.total_bytes, 10);
    }
}
