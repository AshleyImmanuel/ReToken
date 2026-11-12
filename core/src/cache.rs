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
/// L1: Exact request cache — keyed by SHA256 of request context (method, path, provider, body, context_hash).
///
/// Cache entries are evicted based on TTL and max capacity (FIFO insertion order).
pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    ttl: Duration,
    max_entries: usize,
    /// Insertion order tracking for simple eviction without duplicate keys.
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

    /// Compute a cache key including full request context (SHA256 hex digest).
    pub fn cache_key_with_context(
        method: &str,
        path: &str,
        provider: &str,
        request_body: &[u8],
        context_hash: Option<&str>,
    ) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(method.as_bytes());
        hasher.update(b":");
        hasher.update(path.as_bytes());
        hasher.update(b":");
        hasher.update(provider.as_bytes());
        hasher.update(b":");
        hasher.update(request_body);
        if let Some(hash) = context_hash {
            hasher.update(b"||");
            hasher.update(hash.as_bytes());
        }
        let result = hasher.finalize();
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Legacy / simple cache key computation from request body and optional context hash.
    pub fn cache_key(request_body: &[u8], context_hash: Option<&str>) -> String {
        Self::cache_key_with_context("", "", "", request_body, context_hash)
    }

    /// Look up a cache entry by key. Returns `None` if not found or expired.
    pub fn get(&mut self, key: &str) -> Option<&CacheEntry> {
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
        if self.max_entries == 0 {
            return;
        }

        // If updating an existing key, remove it from insertion_order first
        if self.entries.contains_key(&key) {
            self.insertion_order.retain(|k| k != &key);
        } else {
            // Evict oldest until within capacity
            while self.entries.len() >= self.max_entries {
                if !self.insertion_order.is_empty() {
                    let oldest_key = self.insertion_order.remove(0);
                    self.entries.remove(&oldest_key);
                } else if let Some(first_key) = self.entries.keys().next().cloned() {
                    self.entries.remove(&first_key);
                } else {
                    break;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_context_differentiation() {
        let body = b"{\"model\": \"test\"}";
        let k1 =
            ResponseCache::cache_key_with_context("POST", "/v1/messages", "anthropic", body, None);
        let k2 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/chat/completions",
            "openai",
            body,
            None,
        );
        let k3 =
            ResponseCache::cache_key_with_context("GET", "/v1/messages", "anthropic", body, None);
        let k4 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/messages",
            "anthropic",
            body,
            Some("repo1"),
        );

        assert_ne!(k1, k2, "different paths/providers must not collide");
        assert_ne!(k1, k3, "different methods must not collide");
        assert_ne!(k1, k4, "different context hashes must not collide");
    }

    #[test]
    fn test_cache_insert_and_hit() {
        let mut cache = ResponseCache::new(60, 5);
        let key = "test_key".to_string();
        cache.insert(
            key.clone(),
            b"response payload".to_vec(),
            200,
            vec![("content-type".into(), "application/json".into())],
        );

        assert_eq!(cache.len(), 1);
        let entry = cache.get(&key).expect("cache hit expected");
        assert_eq!(entry.response_bytes, b"response payload");
        assert_eq!(entry.response_status, 200);
        assert_eq!(entry.hit_count, 1);
        assert_eq!(cache.total_hits, 1);
        assert_eq!(cache.total_misses, 0);

        // Repeated request
        let entry2 = cache.get(&key).expect("second hit expected");
        assert_eq!(entry2.hit_count, 2);
        assert_eq!(cache.total_hits, 2);
    }

    #[test]
    fn test_cache_capacity_eviction_lru() {
        let mut cache = ResponseCache::new(60, 2);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);
        cache.insert("k2".into(), b"v2".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);

        // Insert 3rd entry, should evict k1
        cache.insert("k3".into(), b"v3".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("k1").is_none());
        assert!(cache.get("k2").is_some());
        assert!(cache.get("k3").is_some());
    }

    #[test]
    fn test_cache_updating_existing_key_does_not_leak_order() {
        let mut cache = ResponseCache::new(60, 2);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);
        cache.insert("k2".into(), b"v2".to_vec(), 200, vec![]);

        // Update k1
        cache.insert("k1".into(), b"v1_updated".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);

        // Now insert k3; since k1 was updated more recently, k2 should be the oldest and evicted!
        cache.insert("k3".into(), b"v3".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("k2").is_none(), "k2 should have been evicted");
        assert_eq!(cache.get("k1").unwrap().response_bytes, b"v1_updated");
        assert!(cache.get("k3").is_some());
    }

    #[test]
    fn test_cache_ttl_expiry() {
        let mut cache = ResponseCache::new(1, 10);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);

        // Manually simulate expiration by setting created_at in the past
        if let Some(entry) = cache.entries.get_mut("k1") {
            entry.created_at = Instant::now() - Duration::from_secs(2);
        }

        assert!(cache.get("k1").is_none(), "expired entry must return None");
        assert_eq!(cache.len(), 0, "expired entry must be removed on access");
        assert_eq!(cache.total_misses, 1);
    }

    #[test]
    fn test_cache_zero_capacity() {
        let mut cache = ResponseCache::new(60, 0);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 0);
        assert!(cache.get("k1").is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_context_differentiation() {
        let body = b"{\"model\": \"test\"}";
        let k1 = ResponseCache::cache_key_with_context("POST", "/v1/messages", "anthropic", body, None);
        let k2 = ResponseCache::cache_key_with_context("POST", "/v1/chat/completions", "openai", body, None);
        let k3 = ResponseCache::cache_key_with_context("GET", "/v1/messages", "anthropic", body, None);
        let k4 = ResponseCache::cache_key_with_context("POST", "/v1/messages", "anthropic", body, Some("repo1"));

        assert_ne!(k1, k2, "different paths/providers must not collide");
        assert_ne!(k1, k3, "different methods must not collide");
        assert_ne!(k1, k4, "different context hashes must not collide");
    }

    #[test]
    fn test_cache_insert_and_hit() {
        let mut cache = ResponseCache::new(60, 5);
        let key = "test_key".to_string();
        cache.insert(key.clone(), b"response payload".to_vec(), 200, vec![("content-type".into(), "application/json".into())]);

        assert_eq!(cache.len(), 1);
        let entry = cache.get(&key).expect("cache hit expected");
        assert_eq!(entry.response_bytes, b"response payload");
        assert_eq!(entry.response_status, 200);
        assert_eq!(entry.hit_count, 1);
        assert_eq!(cache.total_hits, 1);
        assert_eq!(cache.total_misses, 0);

        // Repeated request
        let entry2 = cache.get(&key).expect("second hit expected");
        assert_eq!(entry2.hit_count, 2);
        assert_eq!(cache.total_hits, 2);
    }

    #[test]
    fn test_cache_capacity_eviction_lru() {
        let mut cache = ResponseCache::new(60, 2);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);
        cache.insert("k2".into(), b"v2".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);

        // Insert 3rd entry, should evict k1
        cache.insert("k3".into(), b"v3".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("k1").is_none());
        assert!(cache.get("k2").is_some());
        assert!(cache.get("k3").is_some());
    }

    #[test]
    fn test_cache_updating_existing_key_does_not_leak_order() {
        let mut cache = ResponseCache::new(60, 2);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);
        cache.insert("k2".into(), b"v2".to_vec(), 200, vec![]);

        // Update k1
        cache.insert("k1".into(), b"v1_updated".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);

        // Now insert k3; since k1 was updated more recently, k2 should be the oldest and evicted!
        cache.insert("k3".into(), b"v3".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("k2").is_none(), "k2 should have been evicted");
        assert_eq!(cache.get("k1").unwrap().response_bytes, b"v1_updated");
        assert!(cache.get("k3").is_some());
    }

    #[test]
    fn test_cache_ttl_expiry() {
        let mut cache = ResponseCache::new(1, 10);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);
        
        // Manually simulate expiration by setting created_at in the past
        if let Some(entry) = cache.entries.get_mut("k1") {
            entry.created_at = Instant::now() - Duration::from_secs(2);
        }

        assert!(cache.get("k1").is_none(), "expired entry must return None");
        assert_eq!(cache.len(), 0, "expired entry must be removed on access");
        assert_eq!(cache.total_misses, 1);
    }

    #[test]
    fn test_cache_zero_capacity() {
        let mut cache = ResponseCache::new(60, 0);
        cache.insert("k1".into(), b"v1".to_vec(), 200, vec![]);
        assert_eq!(cache.len(), 0);
        assert!(cache.get("k1").is_none());
    }
}
