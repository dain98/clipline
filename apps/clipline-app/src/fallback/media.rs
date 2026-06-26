use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[allow(
    dead_code,
    reason = "non-clip media kinds are reserved for later fallback registration tasks"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MediaKind {
    Clip,
    Poster,
    AudioPreview,
    CloudCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    pub id: String,
    pub path: PathBuf,
    pub kind: MediaKind,
}

#[derive(Default)]
pub struct MediaRegistry {
    inner: Mutex<MediaRegistryInner>,
}

#[derive(Default)]
struct MediaRegistryInner {
    entries: BTreeMap<String, MediaEntry>,
    reverse: BTreeMap<(PathBuf, MediaKind), String>,
    next_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaRegistryError;

type MediaRegistryResult<T> = Result<T, MediaRegistryError>;

impl std::fmt::Display for MediaRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("media registry unavailable")
    }
}

impl std::error::Error for MediaRegistryError {}

impl MediaRegistryInner {
    fn allocate_id(&mut self) -> MediaRegistryResult<String> {
        self.next_id = self.next_id.checked_add(1).ok_or(MediaRegistryError)?;
        Ok(format!("m{}", self.next_id))
    }
}

impl MediaRegistry {
    pub fn register(&self, path: PathBuf, kind: MediaKind) -> MediaRegistryResult<String> {
        let key = (path.clone(), kind);
        let mut inner = self.inner.lock().map_err(|_| MediaRegistryError)?;
        if let Some(id) = inner.reverse.get(&key) {
            return Ok(id.clone());
        }

        let id = inner.allocate_id()?;
        let entry = MediaEntry {
            id: id.clone(),
            path,
            kind,
        };
        inner.entries.insert(id.clone(), entry.clone());
        inner.reverse.insert((entry.path, entry.kind), id.clone());
        Ok(id)
    }

    pub fn lookup(&self, id: &str) -> MediaRegistryResult<Option<MediaEntry>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| MediaRegistryError)?
            .entries
            .get(id)
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_registry_returns_stable_opaque_ids() {
        let registry = MediaRegistry::default();
        let path = std::path::PathBuf::from(r"C:\Videos\Clipline\clip.mp4");

        let first = registry
            .register(path.clone(), MediaKind::Clip)
            .expect("register media");
        let second = registry
            .register(path, MediaKind::Clip)
            .expect("register media");

        assert_eq!(first, second);
        assert_eq!(
            registry.lookup(&first).unwrap().unwrap().kind,
            MediaKind::Clip
        );
    }

    #[test]
    fn media_registry_rejects_unknown_ids() {
        let registry = MediaRegistry::default();

        assert!(registry.lookup("missing").unwrap().is_none());
    }

    #[test]
    fn media_registry_allocates_monotonic_process_local_ids() {
        let registry = MediaRegistry::default();

        let first = registry
            .register(std::path::PathBuf::from("first.mp4"), MediaKind::Clip)
            .expect("register first media");
        let second = registry
            .register(std::path::PathBuf::from("second.mp4"), MediaKind::Clip)
            .expect("register second media");
        let first_again = registry
            .register(std::path::PathBuf::from("first.mp4"), MediaKind::Clip)
            .expect("register first media again");

        assert_eq!(first, "m1");
        assert_eq!(second, "m2");
        assert_eq!(first_again, first);
    }
}
