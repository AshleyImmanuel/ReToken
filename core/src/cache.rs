use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A single cache entry holding the provider's response bytes and metadata.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub response_bytes: Vec<u8>,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
    pub created_at: Instant,
    pub hit_count: u64,
}

/// Multi-level response cache.
///
/// L1: Exact request cache — keyed by SHA256 of the full request body.
/// L3: Tool result cache — keyed by tool name + arguments hash.
///
/// Cache entries are evicted based on TTL and max capacity (LRU-style via insertion order).
pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    ttl: Duration,
    max_entries: usize,
    /// Insertion order tracking for simple eviction.
    insertion_order: Vec<String>,
    // Counters for telemetry
    pub total_hits: u64,
    pub total_misses: u64,
}

impl ResponseCache {
    pub fn new(ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            ttl: if ttl_secs == 0 {
                Duration::from_secs(u64::MAX) // effectively no expiry
            } else {
                Duration::from_secs(ttl_secs)
            },
            max_entries,
            insertion_order: Vec::new(),
            total_hits: 0,
            total_misses: 0,
        }
    }

    /// Compute a cache key from arbitrary bytes (SHA256 hex digest).
    pub fn cache_key(request_body: &[u8], context_hash: Option<&str>) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(request_body);
        if let Some(hash) = context_hash {
            hasher.update(b"||");
            hasher.update(hash.as_bytes());
        }
        let result = hasher.finalize();
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Look up a cache entry by key. Returns `None` if not found or expired.
    pub fn get(&mut self, key: &str) -> Option<&CacheEntry> {
        // Check if entry exists and is not expired
        let expired = if let Some(entry) = self.entries.get(key) {
            entry.created_at.elapsed() > self.ttl
        } else {
            self.total_misses += 1;
            return None;
        };

        if expired {
            self.entries.remove(key);
            self.insertion_order.retain(|k| k != key);
            self.total_misses += 1;
            return None;
        }

        // Bump hit count
        if let Some(entry) = self.entries.get_mut(key) {
            entry.hit_count += 1;
        }
        self.total_hits += 1;
        self.entries.get(key)
    }

    /// Insert a new cache entry. Evicts the oldest entry if at capacity.
    pub fn insert(
        &mut self,
        key: String,
        response_bytes: Vec<u8>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
    ) {
        // Evict oldest if at capacity
        if self.entries.len() >= self.max_entries {
            if let Some(oldest_key) = self.insertion_order.first().cloned() {
                self.entries.remove(&oldest_key);
                self.insertion_order.remove(0);
            }
        }

        let entry = CacheEntry {
            response_bytes,
            response_status,
            response_headers,
            created_at: Instant::now(),
            hit_count: 0,
        };

        self.insertion_order.push(key.clone());
        self.entries.insert(key, entry);
    }

    /// Invalidate a specific cache entry.
    pub fn invalidate(&mut self, key: &str) {
        self.entries.remove(key);
        self.insertion_order.retain(|k| k != key);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
    }

    /// Number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the cache has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
