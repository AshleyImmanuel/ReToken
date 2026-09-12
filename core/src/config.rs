use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub token_budget: usize,
    pub sandbox_mode: SandboxMode,
    /// Whether the exact-match request cache is enabled.
    pub cache_enabled: bool,
    /// Time-to-live for cache entries in seconds (0 = no expiry).
    pub cache_ttl_secs: u64,
    /// Maximum number of entries the cache will hold before evicting oldest.
    pub max_cache_entries: usize,
    pub terse_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxMode {
    Native,
    Virtual,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            token_budget: 32_000,
            sandbox_mode: SandboxMode::Native,
            cache_enabled: true,
            cache_ttl_secs: 300, // 5 minutes default
            max_cache_entries: 1024,
            terse_mode: true,
        }
    }
}
