use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

/// A single CCR entry stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CcrEntry {
    /// Base64-encoded original content.
    content_b64: String,
    content_type: String,
    created_at: u64,
    access_count: u64,
    size_bytes: usize,
}

/// The ReToken Cache & Recovery (CCR) store.
///
/// A file-backed, content-addressed store that preserves original data before
/// compression. When the proxy compresses/elides data from a payload, the
/// original bytes are stored here under a SHA-256 handle. If the LLM needs the
/// dropped data, it invokes the injected `retoken_retrieve(handle)` tool. The
/// proxy intercepts this call, fetches from CCR, and returns it -- entirely
/// transparently to the client.
///
/// Uses a `HashMap` with JSON file persistence instead of SQLite, avoiding
/// native C compiler dependencies.
pub struct CcrStore {
    inner: Arc<Mutex<CcrInner>>,
}

struct CcrInner {
    entries: HashMap<String, CcrEntry>,
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

        info!("CCR store opened at {} ({} entries)", path.display(), entries.len());

        Ok(Self {
            inner: Arc::new(Mutex::new(CcrInner {
                entries,
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
                db_path: None,
                dirty: false,
            })),
        })
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
        let content_b64 = base64_encode(content);

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

        debug!("CCR stored: {} ({} bytes, type={})", handle, size, content_type);
        Ok(handle)
    }

    /// Retrieve content by handle. Returns None if not found.
    pub fn retrieve(&self, handle: &str) -> Result<Option<Vec<u8>>, String> {
        let mut inner = self.inner.lock().map_err(|e| format!("CCR lock: {}", e))?;

        if let Some(entry) = inner.entries.get(handle) {
            let bytes = base64_decode(&entry.content_b64);
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

/// The tool schema injected into outbound requests so the LLM can retrieve
/// elided data. This is a standard OpenAI/Anthropic tool definition.
pub fn ccr_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "retoken_retrieve",
        "description": "Retrieve original data that was compressed by the ReToken proxy. Use this when you see a CCR handle (ccr:...) or a __retoken_elided__ marker and need the full original content.",
        "input_schema": {
            "type": "object",
            "properties": {
                "handle": {
                    "type": "string",
                    "description": "The CCR handle (ccr:<hash>) of the data to retrieve."
                }
            },
            "required": ["handle"]
        }
    })
}

/// Returns the recovery marker string for embedding in compressed payloads.
pub fn ccr_marker(handle: &str) -> String {
    format!("<<ccr:{}>>", handle.strip_prefix("ccr:").unwrap_or(handle))
}

/// Resolve the default data directory for the CCR database.
fn dirs_or_default() -> PathBuf {
    if let Some(data_dir) = std::env::var_os("RETOKEN_DATA_DIR") {
        return PathBuf::from(data_dir);
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("LOCALAPPDATA") {
            let mut p = PathBuf::from(appdata);
            p.push("ReToken");
            return p;
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let mut p = PathBuf::from(home);
            p.push(".retoken");
            return p;
        }
    }
    PathBuf::from(".")
}

// Minimal base64 encode/decode to avoid adding another dependency.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn base64_decode(encoded: &str) -> Vec<u8> {
    fn char_val(c: u8) -> u8 {
        match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let bytes: Vec<u8> = encoded.bytes().filter(|&b| b != b'=' && b != b'\n' && b != b'\r').collect();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let b0 = char_val(chunk[0]) as u32;
        let b1 = char_val(chunk[1]) as u32;
        let b2 = if chunk.len() > 2 { char_val(chunk[2]) as u32 } else { 0 };
        let b3 = if chunk.len() > 3 { char_val(chunk[3]) as u32 } else { 0 };
        let n = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
        result.push((n >> 16) as u8);
        if chunk.len() > 2 {
            result.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            result.push(n as u8);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_retrieve() {
        let store = CcrStore::open_in_memory().unwrap();
        let data = b"hello world this is some test data";
        let handle = store.store(data, "text").unwrap();
        assert!(handle.starts_with("ccr:"));

        let retrieved = store.retrieve(&handle).unwrap().unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn missing_handle_returns_none() {
        let store = CcrStore::open_in_memory().unwrap();
        let result = store.retrieve("ccr:nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn duplicate_stores_are_idempotent() {
        let store = CcrStore::open_in_memory().unwrap();
        let data = b"same content";
        let h1 = store.store(data, "text").unwrap();
        let h2 = store.store(data, "text").unwrap();
        assert_eq!(h1, h2);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn handle_is_deterministic() {
        let h1 = CcrStore::handle(b"test");
        let h2 = CcrStore::handle(b"test");
        assert_eq!(h1, h2);
        let h3 = CcrStore::handle(b"different");
        assert_ne!(h1, h3);
    }

    #[test]
    fn tool_schema_is_valid_json() {
        let schema = ccr_tool_schema();
        assert!(schema.get("name").is_some());
        assert!(schema.get("input_schema").is_some());
    }

    #[test]
    fn base64_roundtrip() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded);
        assert_eq!(decoded, data);
    }

    #[test]
    fn base64_binary_roundtrip() {
        let data: Vec<u8> = (0..=255).collect();
        let encoded = base64_encode(&data);
        let decoded = base64_decode(&encoded);
        assert_eq!(decoded, data);
    }
}
