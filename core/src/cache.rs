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

    /// Compute a cache key including full request context, upstream base URL, and credential isolation (SHA256 hex digest).
    pub fn cache_key_with_context(
        method: &str,
        path: &str,
        provider: &str,
        upstream_base: Option<&str>,
        request_body: &[u8],
        context_hash: Option<&str>,
        auth_fingerprint: Option<&str>,
    ) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(method.as_bytes());
        hasher.update(b":");
        hasher.update(path.as_bytes());
        hasher.update(b":");
        hasher.update(provider.as_bytes());
        if let Some(base) = upstream_base {
            hasher.update(b"@base:");
            hasher.update(base.as_bytes());
        }
        hasher.update(b":");
        hasher.update(request_body);
        if let Some(hash) = context_hash {
            hasher.update(b"||repo:");
            hasher.update(hash.as_bytes());
        }
        if let Some(auth) = auth_fingerprint {
            hasher.update(b"||auth:");
            hasher.update(auth.as_bytes());
        }
        let result = hasher.finalize();
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Returns true if a header is a hop-by-hop or local proxy control header that should
    /// not affect caching keys.
    pub fn is_hop_by_hop_or_proxy_header(header_name: &str) -> bool {
        let lower = header_name.to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailer"
                | "trailers"
                | "transfer-encoding"
                | "upgrade"
                | "host"
                | "content-length"
                | "x-forwarded-for"
                | "x-forwarded-proto"
                | "x-forwarded-host"
        ) || lower.starts_with("x-retoken-")
    }

    /// Compute a SHA256 fingerprint from credentials and all response-affecting headers.
    /// Excludes only hop-by-hop transport headers and internal proxy control headers.
    /// Any difference in client headers (auth tokens, custom model options, organization IDs,
    /// beta flags, versions) yields a distinct fingerprint to guarantee isolation.
    pub fn compute_context_fingerprint(headers: &[(String, String)]) -> Option<String> {
        use sha2::{Digest, Sha256};
        let mut relevant: Vec<(&str, &str)> = headers
            .iter()
            .filter_map(|(k, v)| {
                if Self::is_hop_by_hop_or_proxy_header(k) {
                    None
                } else {
                    Some((k.as_str(), v.as_str()))
                }
            })
            .collect();

        if relevant.is_empty() {
            return None;
        }

        relevant.sort_by_key(|a| a.0.to_ascii_lowercase());
        let mut hasher = Sha256::new();
        for (k, v) in relevant {
            hasher.update(k.to_ascii_lowercase().as_bytes());
            hasher.update(b":");
            hasher.update(v.as_bytes());
            hasher.update(b"\n");
        }
        let result = hasher.finalize();
        Some(result.iter().map(|b| format!("{:02x}", b)).collect())
    }

    /// Compute a SHA256 hex digest fingerprint for an authentication credential.
    pub fn hash_credential(credential: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(credential.as_bytes());
        let result = hasher.finalize();
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Legacy / simple cache key computation from request body and optional context hash.
    pub fn cache_key(request_body: &[u8], context_hash: Option<&str>) -> String {
        Self::cache_key_with_context("", "", "", None, request_body, context_hash, None)
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

    /// Maximum capacity of the cache.
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    /// Returns a list of summaries for all entries currently in the cache.
    pub fn list_entries(&self) -> Vec<CacheEntrySummary> {
        let mut list: Vec<CacheEntrySummary> = self
            .entries
            .iter()
            .map(|(k, v)| CacheEntrySummary {
                key: k.clone(),
                age_secs: v.created_at.elapsed().as_secs(),
                hit_count: v.hit_count,
                status: v.response_status,
                size_bytes: v.response_bytes.len(),
            })
            .collect();
        list.sort_by_key(|e| std::cmp::Reverse(e.hit_count));
        list
    }
}

/// Summary metadata for a cached response entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheEntrySummary {
    pub key: String,
    pub age_secs: u64,
    pub hit_count: u64,
    pub status: u16,
    pub size_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_context_differentiation() {
        let body = b"{\"model\": \"test\"}";
        let k1 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/messages",
            "anthropic",
            Some("https://api.anthropic.com"),
            body,
            None,
            None,
        );
        let k2 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/chat/completions",
            "openai",
            Some("https://api.openai.com"),
            body,
            None,
            None,
        );
        let k3 = ResponseCache::cache_key_with_context(
            "GET",
            "/v1/messages",
            "anthropic",
            Some("https://api.anthropic.com"),
            body,
            None,
            None,
        );
        let k4 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/messages",
            "anthropic",
            Some("https://api.anthropic.com"),
            body,
            Some("repo1"),
            None,
        );
        let k5 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/messages",
            "anthropic",
            Some("https://api.anthropic.com"),
            body,
            None,
            Some("user1_auth_hash"),
        );
        let k6 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/messages",
            "anthropic",
            Some("https://api.anthropic.com"),
            body,
            None,
            Some("user2_auth_hash"),
        );
        let k7 = ResponseCache::cache_key_with_context(
            "POST",
            "/v1/messages",
            "anthropic",
            Some("https://custom.upstream.org"),
            body,
            None,
            None,
        );

        assert_ne!(k1, k2, "different paths/providers must not collide");
        assert_ne!(k1, k3, "different methods must not collide");
        assert_ne!(k1, k4, "different context hashes must not collide");
        assert_ne!(
            k1, k5,
            "credentialed requests must not collide with uncredentialed"
        );
        assert_ne!(k5, k6, "different user credentials must isolate cache keys");
        assert_ne!(
            k1, k7,
            "different upstream base URLs must isolate cache keys"
        );
    }

    #[test]
    fn test_compute_context_fingerprint() {
        let headers1 = vec![
            ("Authorization".to_string(), "Bearer sk-123".to_string()),
            ("openai-organization".to_string(), "org-1".to_string()),
        ];
        let headers2 = vec![
            ("openai-organization".to_string(), "org-1".to_string()),
            ("authorization".to_string(), "Bearer sk-123".to_string()),
        ];
        let headers3 = vec![
            ("Authorization".to_string(), "Bearer sk-123".to_string()),
            ("openai-organization".to_string(), "org-2".to_string()),
        ];

        let fp1 = ResponseCache::compute_context_fingerprint(&headers1);
        let fp2 = ResponseCache::compute_context_fingerprint(&headers2);
        let fp3 = ResponseCache::compute_context_fingerprint(&headers3);

        assert!(fp1.is_some());
        assert_eq!(fp1, fp2, "header order or case must not alter fingerprint");
        assert_ne!(
            fp1, fp3,
            "differing organization must yield different fingerprint"
        );

        // Custom headers isolation test
        let custom_a = vec![
            ("Authorization".to_string(), "Bearer sk-123".to_string()),
            (
                "X-Custom-Model-Variant".to_string(),
                "fast-tier".to_string(),
            ),
        ];
        let custom_b = vec![
            ("Authorization".to_string(), "Bearer sk-123".to_string()),
            (
                "X-Custom-Model-Variant".to_string(),
                "precision-tier".to_string(),
            ),
        ];
        let fp_custom_a = ResponseCache::compute_context_fingerprint(&custom_a);
        let fp_custom_b = ResponseCache::compute_context_fingerprint(&custom_b);
        assert_ne!(
            fp_custom_a, fp_custom_b,
            "differing custom headers must isolate cache fingerprints"
        );

        // Hop-by-hop and proxy headers exclusion test
        let with_transport = vec![
            ("Authorization".to_string(), "Bearer sk-123".to_string()),
            ("Host".to_string(), "127.0.0.1:8888".to_string()),
            ("Connection".to_string(), "keep-alive".to_string()),
            (
                "x-retoken-upstream-url".to_string(),
                "https://api.anthropic.com".to_string(),
            ),
            ("x-retoken-cache".to_string(), "true".to_string()),
        ];
        let without_transport = vec![("Authorization".to_string(), "Bearer sk-123".to_string())];
        assert_eq!(
            ResponseCache::compute_context_fingerprint(&with_transport),
            ResponseCache::compute_context_fingerprint(&without_transport),
            "hop-by-hop and retoken proxy control headers must be excluded from context fingerprint"
        );
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
