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
    pub port: u16,

    // Dynamic configurations replacing hardcoded logic
    pub persona_prompt: String,
    pub elision_note: String,
    pub anthropic_base_url: String,
    pub openai_base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxMode {
    Native,
    Virtual,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            token_budget: std::env::var("RETOKEN_TOKEN_BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(32_000),
            sandbox_mode: SandboxMode::Native,
            cache_enabled: std::env::var("RETOKEN_CACHE_ENABLED").ok().and_then(|v| v.parse().ok()).unwrap_or(true),
            cache_ttl_secs: std::env::var("RETOKEN_CACHE_TTL_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(300),
            max_cache_entries: std::env::var("RETOKEN_MAX_CACHE_ENTRIES").ok().and_then(|v| v.parse().ok()).unwrap_or(1024),
            terse_mode: std::env::var("RETOKEN_TERSE_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(true),
            port: std::env::var("RETOKEN_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8888),
            persona_prompt: std::env::var("RETOKEN_PERSONA_PROMPT").unwrap_or_else(|_| "[ReToken Terse Mode Active (Inspired by Caveman). Extreme brevity. Zero pleasantries. Provide only raw technical data, code, or direct answers. Omit explanations. If data is elided with <<handle>>, use retoken_retrieve to fetch it instead of guessing.]".to_string()),
            elision_note: std::env::var("RETOKEN_ELISION_NOTE").unwrap_or_else(|_| "Elements elided by ReToken. Original recoverable via retoken_retrieve tool.".to_string()),
            anthropic_base_url: std::env::var("RETOKEN_ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string()),
            openai_base_url: std::env::var("RETOKEN_OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".to_string()),
        }
    }
}
