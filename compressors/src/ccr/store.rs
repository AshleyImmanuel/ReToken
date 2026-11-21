use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

use crate::ccr::paths::dirs_or_default;

/// Handle prefix for content-addressed CCR entries.
const HANDLE_PREFIX: &str = "ccr:";

/// The ReToken Cache & Recovery (CCR) store.
///
/// Content-addressed: every original is stored under `ccr:<sha256 hex>` before the elided
/// payload is forwarded upstream. The persistent backend keeps one file per entry in a
/// directory, so a separate process (the `retoken mcp` retrieval server) can read an entry
/// the moment the proxy writes it, and identical content is only ever written once.
pub struct CcrStore {
    backend: Backend,
}

enum Backend {
    Dir(PathBuf),
    Memory(Mutex<HashMap<String, Vec<u8>>>),
}

impl CcrStore {
    /// Default persistent location: `<data dir>/ccr`.
    pub fn default_dir() -> PathBuf {
        dirs_or_default().join("ccr")
    }

    /// Opens (creating if needed) the persistent store at `dir`, or [`Self::default_dir`].
    pub fn open(dir: Option<PathBuf>) -> Result<Self, String> {
        let dir = dir.unwrap_or_else(Self::default_dir);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("CCR store dir {} unavailable: {}", dir.display(), e))?;
        info!("CCR store opened at {}", dir.display());
        Ok(Self {
            backend: Backend::Dir(dir),
        })
    }

    /// Opens an in-memory store (tests, or fallback when the data dir is unavailable).
    pub fn open_in_memory() -> Result<Self, String> {
        Ok(Self {
            backend: Backend::Memory(Mutex::new(HashMap::new())),
        })
    }

    /// Computes the content-addressed handle for a byte slice: `ccr:<sha256_hex>`.
    pub fn handle(content: &[u8]) -> String {
        let hash = Sha256::digest(content);
        format!("{}{}", HANDLE_PREFIX, hex::encode(hash))
    }

    /// Returns the hex digest of a well-formed handle (`ccr:` + 64 lowercase hex chars).
    /// Handles arrive from model output, so anything else is rejected before touching disk.
    fn digest_of(handle: &str) -> Option<&str> {
        let digest = handle.trim().strip_prefix(HANDLE_PREFIX)?;
        let valid = digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then_some(digest)
    }

    /// Stores content under its handle and returns the handle. Idempotent.
    pub fn store(&self, content: &[u8], content_type: &str) -> Result<String, String> {
        let handle = Self::handle(content);
        match &self.backend {
            Backend::Memory(map) => {
                let mut map = map.lock().map_err(|e| format!("CCR lock: {}", e))?;
                map.entry(handle.clone())
                    .or_insert_with(|| content.to_vec());
            }
            Backend::Dir(dir) => {
                let digest = Self::digest_of(&handle).expect("handle() is well-formed");
                let path = dir.join(digest);
                if !path.exists() {
                    write_atomically(dir, &path, content)?;
                    debug!(
                        "CCR stored: {} ({} bytes, type={})",
                        handle,
                        content.len(),
                        content_type
                    );
                }
            }
        }
        Ok(handle)
    }

    /// Retrieves content by handle. Returns `Ok(None)` for unknown or malformed handles.
    pub fn retrieve(&self, handle: &str) -> Result<Option<Vec<u8>>, String> {
        let Some(digest) = Self::digest_of(handle) else {
            warn!("CCR rejected malformed handle");
            return Ok(None);
        };
        match &self.backend {
            Backend::Memory(map) => {
                let map = map.lock().map_err(|e| format!("CCR lock: {}", e))?;
                Ok(map.get(&format!("{}{}", HANDLE_PREFIX, digest)).cloned())
            }
            Backend::Dir(dir) => match fs::read(dir.join(digest)) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    warn!("CCR miss: {}", handle);
                    Ok(None)
                }
                Err(e) => Err(format!("CCR read {}: {}", handle, e)),
            },
        }
    }

    /// Number of stored entries.
    pub fn count(&self) -> usize {
        match &self.backend {
            Backend::Memory(map) => map.lock().map(|m| m.len()).unwrap_or(0),
            Backend::Dir(dir) => entry_files(dir).count(),
        }
    }

    /// Total original bytes stored.
    pub fn total_bytes(&self) -> u64 {
        match &self.backend {
            Backend::Memory(map) => map
                .lock()
                .map(|m| m.values().map(|v| v.len() as u64).sum())
                .unwrap_or(0),
            Backend::Dir(dir) => entry_files(dir)
                .filter_map(|p| fs::metadata(p).ok())
                .map(|m| m.len())
                .sum(),
        }
    }

    /// Deletes persistent entries not modified within `max_age_secs`. Returns the count removed.
    /// Content is addressed by hash, so an old entry that is still referenced in a live
    /// conversation is simply re-stored the next time that conversation is compressed.
    pub fn gc(&self, max_age_secs: u64) -> usize {
        let Backend::Dir(dir) = &self.backend else {
            return 0;
        };
        let Some(cutoff) = SystemTime::now().checked_sub(Duration::from_secs(max_age_secs)) else {
            return 0;
        };
        let removed = entry_files(dir)
            .filter(|p| {
                fs::metadata(p)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t < cutoff)
            })
            .filter(|p| fs::remove_file(p).is_ok())
            .count();
        if removed > 0 {
            info!("CCR gc: purged {} stale entries", removed);
        }
        removed
    }
}

/// Lists entry files (named by 64-char hex digest) in the store directory.
fn entry_files(dir: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit()))
        })
}

/// Writes via a temp file + rename so a concurrent reader never sees a partial entry.
fn write_atomically(dir: &Path, path: &Path, content: &[u8]) -> Result<(), String> {
    let tmp = dir.join(format!(
        ".tmp-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let result = fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(content).and_then(|_| f.sync_all()))
        .and_then(|_| fs::rename(&tmp, path));
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        // A concurrent writer may have stored the same content first; that is success.
        if path.exists() {
            return Ok(());
        }
        return Err(format!("CCR write {}: {}", path.display(), e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("retoken-ccr-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn dir_store_round_trip_and_visible_to_second_instance() {
        let dir = temp_dir("roundtrip");
        let writer = CcrStore::open(Some(dir.clone())).unwrap();
        let handle = writer.store(b"original output", "log").unwrap();

        // A separate store instance (e.g. the MCP server process) sees it immediately.
        let reader = CcrStore::open(Some(dir.clone())).unwrap();
        assert_eq!(
            reader.retrieve(&handle).unwrap().as_deref(),
            Some(&b"original output"[..])
        );
        assert_eq!(reader.count(), 1);
        assert_eq!(reader.total_bytes(), 15);

        // Idempotent
        assert_eq!(writer.store(b"original output", "log").unwrap(), handle);
        assert_eq!(reader.count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_handles_never_touch_the_filesystem() {
        let dir = temp_dir("malformed");
        let store = CcrStore::open(Some(dir.clone())).unwrap();
        for bad in [
            "ccr:../../etc/passwd",
            "ccr:",
            "nothex",
            "ccr:ZZ",
            &format!("ccr:{}", "A".repeat(64)),
            &format!("ccr:{}/x", "a".repeat(63)),
        ] {
            assert_eq!(store.retrieve(bad).unwrap(), None, "{bad}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gc_removes_only_stale_entries() {
        let dir = temp_dir("gc");
        let store = CcrStore::open(Some(dir.clone())).unwrap();
        store.store(b"fresh", "log").unwrap();
        assert_eq!(store.gc(3600), 0);
        assert_eq!(store.count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
