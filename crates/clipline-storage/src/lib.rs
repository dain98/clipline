//! Filesystem storage management for saved clips.

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

pub fn storage_status(dir: &Path, quota_bytes: Option<u64>) -> io::Result<StorageStatus> {
    let clips = inventory(dir)?;
    Ok(status_from_clips(&clips, quota_bytes))
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

    clips.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
    });

    for clip in clips {
        if total_bytes <= quota {
            break;
        }
        if protect.is_some_and(|protected| same_path(&clip.path, protected)) {
            continue;
        }

        let clip_bytes = clip.total_bytes();
        remove_file_if_exists(&clip.path)?;
        if let Some(sidecar) = &clip.sidecar {
            remove_file_if_exists(sidecar)?;
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
}

impl ClipFile {
    fn total_bytes(&self) -> u64 {
        self.mp4_bytes + self.sidecar_bytes
    }
}

fn inventory(dir: &Path) -> io::Result<Vec<ClipFile>> {
    let mut clips = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !is_mp4(&path) {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let sidecar = sidecar_path(&path);
        let sidecar_bytes = optional_file_len(&sidecar)?;
        clips.push(ClipFile {
            path,
            sidecar: (sidecar_bytes > 0 || sidecar.exists()).then_some(sidecar),
            mp4_bytes: meta.len(),
            sidecar_bytes,
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
        });
    }
    Ok(clips)
}

fn status_from_clips(clips: &[ClipFile], quota_bytes: Option<u64>) -> StorageStatus {
    StorageStatus {
        clip_count: clips.len(),
        total_bytes: clips.iter().map(ClipFile::total_bytes).sum(),
        quota_bytes,
    }
}

fn is_mp4(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
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
    fn enforce_quota_protects_the_fresh_clip_even_if_still_over_budget() {
        let dir = TestDir::new("protect-fresh");
        let old = dir.write("old.mp4", 10);
        tick_mtime();
        let fresh = dir.write("fresh.mp4", 20);

        let report = enforce_quota(dir.path(), Some(15), Some(&fresh)).unwrap();

        assert_eq!(report.deleted_clips, 1);
        assert_eq!(report.freed_bytes, 10);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert_eq!(report.status.total_bytes, 20);
        assert!(report.status.is_over_quota());
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
