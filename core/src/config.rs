use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

    /// Maximum request body size in bytes before returning 413 Payload Too Large (default: 10MB).
    pub max_request_body_bytes: usize,
    /// Maximum response body size in bytes before returning 502 Bad Gateway (default: 10MB).
    pub max_response_body_bytes: usize,
    /// Whether custom upstream URLs are allowed to target loopback, private, or local network hosts (default: false).
    pub allow_private_upstreams: bool,
    /// Explicit allowlist of permissible upstream host patterns (e.g. `["api.anthropic.com", "*.openai.com"]`). Empty allows any non-private host.
    pub allowed_upstream_hosts: Vec<String>,
    /// Upstream HTTP client request timeout in seconds (default: 60s).
    pub upstream_timeout_secs: u64,

    // Dynamic configurations replacing hardcoded logic
    pub persona_prompt: String,
    pub elision_note: String,
    pub anthropic_base_url: String,
    pub openai_base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SandboxMode {
    #[default]
    Native,
    Virtual,
}

fn parse_env_var<T: std::str::FromStr>(name: &str) -> Option<T> {
    if let Ok(val) = std::env::var(name) {
        match val.parse::<T>() {
            Ok(parsed) => Some(parsed),
            Err(_) => {
                tracing::warn!(
                    "Environment variable {} has invalid value '{}'. Ignoring and retaining previous value.",
                    name,
                    val
                );
                None
            }
        }
    } else {
        None
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            token_budget: parse_env_var("RETOKEN_TOKEN_BUDGET").unwrap_or(32_000),
            sandbox_mode: SandboxMode::Native,
            cache_enabled: parse_env_var("RETOKEN_CACHE_ENABLED").unwrap_or(true),
            cache_ttl_secs: parse_env_var("RETOKEN_CACHE_TTL_SECS").unwrap_or(300),
            max_cache_entries: parse_env_var("RETOKEN_MAX_CACHE_ENTRIES").unwrap_or(1024),
            terse_mode: parse_env_var("RETOKEN_TERSE_MODE").unwrap_or(true),
            port: parse_env_var("RETOKEN_PORT").unwrap_or(8888),
            max_request_body_bytes: parse_env_var("RETOKEN_MAX_REQUEST_BODY_BYTES")
                .unwrap_or(10 * 1024 * 1024),
            max_response_body_bytes: parse_env_var("RETOKEN_MAX_RESPONSE_BODY_BYTES")
                .unwrap_or(10 * 1024 * 1024),
            allow_private_upstreams: parse_env_var("RETOKEN_ALLOW_PRIVATE_UPSTREAMS")
                .unwrap_or(false),
            allowed_upstream_hosts: std::env::var("RETOKEN_ALLOWED_UPSTREAM_HOSTS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            upstream_timeout_secs: parse_env_var("RETOKEN_UPSTREAM_TIMEOUT_SECS")
                .unwrap_or(60),
            persona_prompt: std::env::var("RETOKEN_PERSONA_PROMPT").unwrap_or_else(|_| "[ReToken Terse Mode Active (Inspired by Caveman). Extreme brevity. Zero pleasantries. Provide only raw technical data, code, or direct answers. Omit explanations. If data is elided with <<handle>>, use retoken_retrieve to fetch it instead of guessing.]".to_string()),
            elision_note: std::env::var("RETOKEN_ELISION_NOTE").unwrap_or_else(|_| "Elements elided by ReToken. Original recoverable via retoken_retrieve tool.".to_string()),
            anthropic_base_url: std::env::var("RETOKEN_ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string()),
            openai_base_url: std::env::var("RETOKEN_OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".to_string()),
        }
    }
}

impl AppConfig {
    /// Loads configuration from `retoken.json`, `.retoken.json`, or `.retoken/config.json` if found,
    /// then overlays environment variables on top. Fails loudly if a found configuration file is malformed.
    pub fn load() -> anyhow::Result<Self> {
        let mut config = Self::load_from_file_or_default()?;
        config.apply_env_overrides();
        Ok(config)
    }

    fn load_from_file_or_default() -> anyhow::Result<Self> {
        let candidates = ["retoken.json", ".retoken.json", ".retoken/config.json"];
        for path in &candidates {
            if std::path::Path::new(path).exists() {
                let content = std::fs::read_to_string(path).map_err(|e| {
                    anyhow::anyhow!("Failed to read configuration file '{}': {}", path, e)
                })?;
                let config = serde_json::from_str::<AppConfig>(&content).map_err(|e| {
                    anyhow::anyhow!("Configuration file '{}' is malformed: {}", path, e)
                })?;
                tracing::info!("Loaded configuration from {}", path);
                return Ok(config);
            }
        }
        Ok(Self::default())
    }

    pub fn apply_env_overrides(&mut self) {
        if let Some(v) = parse_env_var("RETOKEN_TOKEN_BUDGET") {
            self.token_budget = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_CACHE_ENABLED") {
            self.cache_enabled = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_CACHE_TTL_SECS") {
            self.cache_ttl_secs = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_MAX_CACHE_ENTRIES") {
            self.max_cache_entries = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_TERSE_MODE") {
            self.terse_mode = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_PORT") {
            self.port = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_MAX_REQUEST_BODY_BYTES") {
            self.max_request_body_bytes = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_MAX_RESPONSE_BODY_BYTES") {
            self.max_response_body_bytes = v;
        }
        if let Some(v) = parse_env_var("RETOKEN_ALLOW_PRIVATE_UPSTREAMS") {
            self.allow_private_upstreams = v;
        }
        if let Ok(hosts) = std::env::var("RETOKEN_ALLOWED_UPSTREAM_HOSTS") {
            self.allowed_upstream_hosts = hosts
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect();
        }
        if let Some(v) = parse_env_var("RETOKEN_UPSTREAM_TIMEOUT_SECS") {
            self.upstream_timeout_secs = v;
        }
        if let Ok(v) = std::env::var("RETOKEN_PERSONA_PROMPT") {
            self.persona_prompt = v;
        }
        if let Ok(v) = std::env::var("RETOKEN_ELISION_NOTE") {
            self.elision_note = v;
        }
        if let Ok(v) = std::env::var("RETOKEN_ANTHROPIC_BASE_URL") {
            self.anthropic_base_url = v;
        }
        if let Ok(v) = std::env::var("RETOKEN_OPENAI_BASE_URL") {
            self.openai_base_url = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_defaults() {
        let config = AppConfig::default();
        assert_eq!(config.port, 8888);
        assert!(!config.allow_private_upstreams);
        assert_eq!(config.max_request_body_bytes, 10 * 1024 * 1024);
        assert_eq!(config.max_response_body_bytes, 10 * 1024 * 1024);
        assert_eq!(config.upstream_timeout_secs, 60);
        assert!(config.allowed_upstream_hosts.is_empty());
    }

    #[test]
    fn test_app_config_partial_json_deserialization() {
        let partial_json = r#"{"port": 9090, "upstream_timeout_secs": 120}"#;
        let loaded: AppConfig = serde_json::from_str(partial_json)
            .expect("Partial JSON configuration must succeed with serde defaults");
        assert_eq!(loaded.port, 9090);
        assert_eq!(loaded.upstream_timeout_secs, 120);
        assert_eq!(loaded.token_budget, 32_000);
        assert!(loaded.cache_enabled);
        assert_eq!(loaded.max_response_body_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn test_app_config_json_roundtrip() {
        let mut config = AppConfig::default();
        config.port = 9999;
        config.allowed_upstream_hosts = vec!["api.anthropic.com".into(), "*.openai.com".into()];

        let json = serde_json::to_string(&config).unwrap();
        let loaded: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.port, 9999);
        assert_eq!(loaded.allowed_upstream_hosts.len(), 2);
    }

    #[test]
    fn test_app_config_malformed_json_fails_loudly() {
        let malformed = "{ \"port\": \"not_a_number\", ";
        let res = serde_json::from_str::<AppConfig>(malformed);
        assert!(res.is_err(), "Malformed config JSON must return an Err");
    }
}
