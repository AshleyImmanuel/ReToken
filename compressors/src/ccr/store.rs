use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

use crate::ccr::entry::CcrEntry;
use crate::ccr::paths::dirs_or_default;

/// The ReToken Cache & Recovery (CCR) store.
///
/// A file-backed, content-addressed store that preserves original data before
/// compression. When the proxy compresses/elides data from a payload, the
/// original bytes are stored here under a SHA-256 handle.
pub struct CcrStore {
    inner: Arc<Mutex<CcrInner>>,
}

struct CcrInner {
    entries: HashMap<String, CcrEntry>,
    file_states: HashMap<String, String>, // Track per-file state for Contextual Delta Encoding
    db_path: Option<PathBuf>,
    dirty: bool,
}

impl CcrInner {
    fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(ref path) = self.db_path {
            match serde_json::to_string(&self.entries) {
                Ok(json) => {
                    if let Err(e) = fs::write(path, json) {
                        error!("CCR flush failed: {}", e);
                    } else {
                        self.dirty = false;
                    }
                }
                Err(e) => error!("CCR serialize failed: {}", e),
            }
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl CcrStore {
    /// Opens or creates the CCR database at the given path.
    pub fn open(db_path: Option<PathBuf>) -> Result<Self, String> {
        let path = db_path.unwrap_or_else(|| {
            let mut p = dirs_or_default();
            p.push("retoken_ccr.json");
            p
        });

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        // Load existing entries
        let entries: HashMap<String, CcrEntry> = if path.exists() {
            match fs::read_to_string(&path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
                Err(e) => {
                    warn!("CCR load failed (starting fresh): {}", e);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        info!(
            "CCR store opened at {} ({} entries)",
            path.display(),
            entries.len()
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(CcrInner {
                entries,
                file_states: HashMap::new(),
                db_path: Some(path),
                dirty: false,
            })),
        })
    }

    /// Opens an in-memory CCR store (for testing).
    pub fn open_in_memory() -> Result<Self, String> {
        Ok(Self {
            inner: Arc::new(Mutex::new(CcrInner {
                entries: HashMap::new(),
                file_states: HashMap::new(),
                db_path: None,
                dirty: false,
            })),
        })
    }

    /// Gets the most recent state of a file from the session cache for Contextual Delta Encoding.
    pub fn get_file_state(&self, path: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.file_states.get(path).cloned()
    }

    /// Saves the most recent state of a file in the session cache.
    pub fn set_file_state(&self, path: &str, content: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .file_states
            .insert(path.to_string(), content.to_string());
    }

    /// Compute the content-addressed handle for a byte slice: `ccr:<sha256_hex>`.
    pub fn handle(content: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash = hasher.finalize();
        format!("ccr:{}", hex::encode(hash))
    }

    /// Store content under its content-addressed handle. Returns the handle.
    pub fn store(&self, content: &[u8], content_type: &str) -> Result<String, String> {
        let handle = Self::handle(content);
        let size = content.len();

        let mut inner = self.inner.lock().map_err(|e| format!("CCR lock: {}", e))?;

        // Content-addressed: if already stored, skip
        if inner.entries.contains_key(&handle) {
            debug!("CCR dedup hit: {} ({} bytes)", handle, size);
            return Ok(handle);
        }

        // Base64 encode the content for JSON-safe storage
        let content_b64 = crate::base64::encode(content);

        inner.entries.insert(
            handle.clone(),
            CcrEntry {
                content_b64,
                content_type: content_type.to_string(),
                created_at: now_secs(),
                access_count: 0,
                size_bytes: size,
            },
        );
        inner.dirty = true;

        // Flush every 10 new entries to avoid data loss
        if inner.entries.len() % 10 == 0 {
            inner.flush();
        }

        debug!(
            "CCR stored: {} ({} bytes, type={})",
            handle, size, content_type
        );
        Ok(handle)
    }

    /// Retrieve content by handle. Returns None if not found.
    pub fn retrieve(&self, handle: &str) -> Result<Option<Vec<u8>>, String> {
        let mut inner = self.inner.lock().map_err(|e| format!("CCR lock: {}", e))?;

        if let Some(entry) = inner.entries.get(handle) {
            let bytes = crate::base64::decode(&entry.content_b64);
            debug!("CCR hit: {} ({} bytes)", handle, bytes.len());
            // Bump access count after we're done reading
            if let Some(entry_mut) = inner.entries.get_mut(handle) {
                entry_mut.access_count += 1;
            }
            inner.dirty = true;
            Ok(Some(bytes))
        } else {
            warn!("CCR miss: {}", handle);
            Ok(None)
        }
    }

    /// Count of stored entries.
    pub fn count(&self) -> usize {
        self.inner.lock().map(|i| i.entries.len()).unwrap_or(0)
    }

    /// Total original bytes stored.
    pub fn total_bytes(&self) -> u64 {
        self.inner
            .lock()
            .map(|i| i.entries.values().map(|e| e.size_bytes as u64).sum())
            .unwrap_or(0)
    }

    /// Garbage-collect entries older than `max_age_secs` that have never been accessed.
    pub fn gc(&self, max_age_secs: u64) -> usize {
        let now = now_secs();
        let mut inner = match self.inner.lock() {
            Ok(i) => i,
            Err(e) => {
                error!("CCR gc lock: {}", e);
                return 0;
            }
        };

        let before = inner.entries.len();
        inner.entries.retain(|_, entry| {
            !(entry.access_count == 0 && now.saturating_sub(entry.created_at) > max_age_secs)
        });
        let deleted = before - inner.entries.len();

        if deleted > 0 {
            inner.dirty = true;
            inner.flush();
            info!("CCR gc: purged {} stale entries", deleted);
        }
        deleted
    }

    /// Force flush to disk.
    pub fn flush(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.flush();
        }
    }
}

impl Drop for CcrStore {
    fn drop(&mut self) {
        self.flush();
    }
}
