//! Shared test utilities for the Clipline workspace.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A temporary directory that is automatically removed when dropped.
///
/// Every test site passes a `prefix` (e.g. `"clipline-library"`) and a
/// per-test `name`. The resulting directory is unique per process and
/// timestamp, so parallel tests never collide.
pub struct TestDir(PathBuf);

impl TestDir {
    pub fn new(prefix: &str, name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Create a file filled with `bytes` zero-bytes, creating parent dirs as
    /// needed. Returns the full path.
    pub fn write(&self, name: &str, bytes: usize) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, vec![0u8; bytes]).unwrap();
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
