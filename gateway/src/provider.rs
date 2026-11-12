use retoken_core::config::AppConfig;
use serde::{Deserialize, Serialize};

/// Supported API providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Anthropic,
    OpenAI,
    Generic { name: String, base_url: String },
}

impl Provider {
    /// The base URL for this provider's API.
    pub fn base_url<'a>(&'a self, config: &'a AppConfig) -> &'a str {
        match self {
            Provider::Anthropic => &config.anthropic_base_url,
            Provider::OpenAI => &config.openai_base_url,
            Provider::Generic { base_url, .. } => base_url.as_str(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
            Provider::Generic { name, .. } => name.as_str(),
        }
    }
}

/// Validates a custom upstream URL from untrusted headers.
/// Only valid HTTP and HTTPS URLs with a non-empty host are accepted.
pub fn validate_upstream_url(url_str: &str) -> Result<String, String> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Err("Custom upstream URL is empty".to_string());
    }

    let parsed =
        reqwest::Url::parse(trimmed).map_err(|e| format!("Invalid upstream URL: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "Unsupported upstream scheme '{}': only http and https are allowed",
                scheme
            ));
        }
    }

    if parsed.host_str().is_none() || parsed.host_str().unwrap().is_empty() {
        return Err("Upstream URL must include a non-empty host".to_string());
    }

    let mut clean = parsed.to_string();
    if clean.ends_with('/') {
        clean.pop();
    }
    Ok(clean)
}

/// Validates a custom upstream URL from untrusted headers.
/// Only valid HTTP and HTTPS URLs with a non-empty host are accepted.
pub fn validate_upstream_url(url_str: &str) -> Result<String, String> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Err("Custom upstream URL is empty".to_string());
    }

    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| format!("Invalid upstream URL: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "Unsupported upstream scheme '{}': only http and https are allowed",
                scheme
            ));
        }
    }

    if parsed.host_str().is_none() || parsed.host_str().unwrap().is_empty() {
        return Err("Upstream URL must include a non-empty host".to_string());
    }

    let mut clean = parsed.to_string();
    if clean.ends_with('/') {
        clean.pop();
    }
    Ok(clean)
}

/// Detects the provider from the incoming request's path and headers.
///
/// Detection strategy:
///   1. If `x-retoken-upstream-url` header is present -> Generic provider (validated)
///   2. If path contains `/v1/messages` or starts with `/messages` -> Anthropic (Claude API)
///   3. If path contains `/v1/chat/completions` or starts with `/chat/completions` -> OpenAI
///   4. If `x-api-key` header present -> Anthropic
///   5. If `Authorization: Bearer` header present -> OpenAI
///   6. Fallback to Anthropic
pub fn detect_provider(path: &str, headers: &[(String, String)]) -> Result<Provider, String> {
    // 1. Generic provider via explicit upstream URL
    let mut upstream_url = None;
    let mut provider_name = "generic".to_string();

    for (name, value) in headers {
        if name.eq_ignore_ascii_case("x-retoken-upstream-url") {
            upstream_url = Some(value.clone());
        }
        if name.eq_ignore_ascii_case("x-retoken-provider-name") {
            provider_name = value.clone();
        }
    }

    if let Some(url) = upstream_url {
        let clean_url = validate_upstream_url(&url)?;
        return Ok(Provider::Generic {
            name: provider_name,
            base_url: clean_url,
        });
    }

    // 2 & 3. Path-based detection
    if path.contains("/v1/messages") || path.starts_with("/messages") {
        return Ok(Provider::Anthropic);
    }
    if path.contains("/v1/chat/completions")
        || path.contains("/v1/completions")
        || path.starts_with("/chat/completions")
        || path.starts_with("/completions")
    {
        return Ok(Provider::OpenAI);
    }

    // 4 & 5. Header-based detection
    for (name, _value) in headers {
        if name.eq_ignore_ascii_case("x-api-key") {
            return Ok(Provider::Anthropic);
        }
    }
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("authorization") && value.starts_with("Bearer ") {
            return Ok(Provider::OpenAI);
        }
    }

    // Default fallback
    Ok(Provider::Anthropic)
}

/// Safely constructs the destination upstream URL, normalizing redundant `/v1` segments
/// and ensuring valid HTTP(S) URL structure.
pub fn build_target_url(
    base_url: &str,
    path_and_query: &str,
    provider: &Provider,
) -> Result<String, String> {
    let clean_base = base_url.trim_end_matches('/');

    // Normalize path
    let mut normalized_path = path_and_query.to_string();

    // Strip redundant leading /v1/v1/ prefix (e.g. from client baseURL=/v1)
    if normalized_path.starts_with("/v1/v1/") {
        normalized_path = normalized_path.replacen("/v1/v1/", "/v1/", 1);
    }

    // Normalize paths missing /v1 for known providers
    match provider {
        Provider::Anthropic => {
            if normalized_path.starts_with("/messages") {
                normalized_path = format!("/v1{}", normalized_path);
            }
        }
        Provider::OpenAI => {
            if normalized_path.starts_with("/chat/completions")
                || normalized_path.starts_with("/completions")
                || normalized_path.starts_with("/models")
            {
                normalized_path = format!("/v1{}", normalized_path);
            }
        }
        Provider::Generic { .. } => {}
    }

    // Avoid duplicating /v1 if base_url already ends with /v1
    if clean_base.ends_with("/v1") && normalized_path.starts_with("/v1/") {
        normalized_path = normalized_path.replacen("/v1/", "/", 1);
    }

    if !normalized_path.starts_with('/') {
        normalized_path = format!("/{}", normalized_path);
    }

    let combined = format!("{}{}", clean_base, normalized_path);
    let parsed = reqwest::Url::parse(&combined).map_err(|e| {
        format!(
            "Failed to build valid target URL from '{}': {}",
            combined, e
        )
    })?;

    Ok(parsed.to_string())
}

/// Safely constructs the destination upstream URL, normalizing redundant `/v1` segments
/// and ensuring valid HTTP(S) URL structure.
pub fn build_target_url(
    base_url: &str,
    path_and_query: &str,
    provider: &Provider,
) -> Result<String, String> {
    let clean_base = base_url.trim_end_matches('/');

    // Normalize path
    let mut normalized_path = path_and_query.to_string();

    // Strip redundant leading /v1/v1/ prefix (e.g. from client baseURL=/v1)
    if normalized_path.starts_with("/v1/v1/") {
        normalized_path = normalized_path.replacen("/v1/v1/", "/v1/", 1);
    }

    // Normalize paths missing /v1 for known providers
    match provider {
        Provider::Anthropic => {
            if normalized_path.starts_with("/messages") {
                normalized_path = format!("/v1{}", normalized_path);
            }
        }
        Provider::OpenAI => {
            if normalized_path.starts_with("/chat/completions")
                || normalized_path.starts_with("/completions")
                || normalized_path.starts_with("/models")
            {
                normalized_path = format!("/v1{}", normalized_path);
            }
        }
        Provider::Generic { .. } => {}
    }

    // Avoid duplicating /v1 if base_url already ends with /v1
    if clean_base.ends_with("/v1") && normalized_path.starts_with("/v1/") {
        normalized_path = normalized_path.replacen("/v1/", "/", 1);
    }

    if !normalized_path.starts_with('/') {
        normalized_path = format!("/{}", normalized_path);
    }

    let combined = format!("{}{}", clean_base, normalized_path);
    let parsed = reqwest::Url::parse(&combined)
        .map_err(|e| format!("Failed to build valid target URL from '{}': {}", combined, e))?;

    Ok(parsed.to_string())
}

/// Extracts token usage from a provider's response body.
pub struct TokenUsage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub cached_tokens: Option<usize>,
}

pub fn extract_token_usage(provider: &Provider, response_body: &[u8]) -> TokenUsage {
    let mut usage = TokenUsage {
        input_tokens: None,
        output_tokens: None,
        cached_tokens: None,
    };

    let json: serde_json::Value = match serde_json::from_slice(response_body) {
        Ok(v) => v,
        Err(_) => return usage,
    };

    let usage_obj = match json.get("usage") {
        Some(u) => u,
        None => return usage,
    };

    match provider {
        Provider::Anthropic => {
            usage.input_tokens = usage_obj
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.output_tokens = usage_obj
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.cached_tokens = usage_obj
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
        }
        Provider::OpenAI => {
            usage.input_tokens = usage_obj
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.output_tokens = usage_obj
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.cached_tokens = usage_obj
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
        }
        Provider::Generic { .. } => {
            usage.input_tokens = usage_obj
                .get("input_tokens")
                .or_else(|| usage_obj.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.output_tokens = usage_obj
                .get("output_tokens")
                .or_else(|| usage_obj.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
        }
    }

    usage
}

/// Returns true if the header is sensitive and should NEVER be logged or leaked.
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "x-api-key"
            | "api-key"
            | "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-auth-token"
            | "x-amz-security-token"
    ) || lower.contains("api-key")
        || lower.contains("secret")
        || lower.contains("token")
}

/// Returns true if the header is a hop-by-hop header that should not be forwarded by a proxy.
pub fn is_hop_by_hop_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Returns true if the header is an internal ReToken control header.
pub fn is_internal_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "x-retoken-bypass"
            | "x-retoken-upstream-url"
            | "x-retoken-provider-name"
            | "x-retoken-repo-hash"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_upstream_url() {
        assert!(validate_upstream_url("https://api.groq.com/openai/v1").is_ok());
        assert!(validate_upstream_url("http://127.0.0.1:11434").is_ok());
        assert_eq!(
            validate_upstream_url("https://example.com/api/").unwrap(),
            "https://example.com/api"
        );

        // Disallow dangerous or malformed schemes
        assert!(validate_upstream_url("file:///etc/passwd").is_err());
        assert!(validate_upstream_url("ftp://server/path").is_err());
        assert!(validate_upstream_url("javascript:alert(1)").is_err());
        assert!(validate_upstream_url("").is_err());
        assert!(validate_upstream_url("not a url").is_err());
    }

    #[test]
    fn test_detect_provider_path_and_headers() {
        let empty_headers = vec![];

        // Anthropic path
        let p = detect_provider("/v1/messages", &empty_headers).unwrap();
        assert_eq!(p, Provider::Anthropic);

        // OpenAI path
        let p = detect_provider("/v1/chat/completions", &empty_headers).unwrap();
        assert_eq!(p, Provider::OpenAI);

        // Anthropic header
        let anthropic_headers = vec![("x-api-key".into(), "secret".into())];
        let p = detect_provider("/custom", &anthropic_headers).unwrap();
        assert_eq!(p, Provider::Anthropic);

        // OpenAI header
        let openai_headers = vec![("authorization".into(), "Bearer sk-12345".into())];
        let p = detect_provider("/custom", &openai_headers).unwrap();
        assert_eq!(p, Provider::OpenAI);

        // Generic upstream URL
        let generic_headers = vec![
            (
                "x-retoken-upstream-url".into(),
                "https://api.together.xyz/v1".into(),
            ),
            ("x-retoken-provider-name".into(), "together".into()),
        ];
        let p = detect_provider("/v1/chat/completions", &generic_headers).unwrap();
        assert_eq!(
            p,
            Provider::Generic {
                name: "together".into(),
                base_url: "https://api.together.xyz/v1".into(),
            }
        );
    }

    #[test]
    fn test_build_target_url_normalization() {
        let anthropic = Provider::Anthropic;
        let openai = Provider::OpenAI;

        // Normal path
        let u1 = build_target_url("https://api.anthropic.com", "/v1/messages", &anthropic).unwrap();
        assert_eq!(u1, "https://api.anthropic.com/v1/messages");

        // Double /v1 path (client had base_url/v1)
        let u2 =
            build_target_url("https://api.anthropic.com", "/v1/v1/messages", &anthropic).unwrap();
        assert_eq!(u2, "https://api.anthropic.com/v1/messages");

        // Path missing /v1
        let u3 = build_target_url("https://api.anthropic.com", "/messages", &anthropic).unwrap();
        assert_eq!(u3, "https://api.anthropic.com/v1/messages");

        // OpenAI with base ending in /v1
        let u4 =
            build_target_url("https://api.openai.com/v1", "/v1/chat/completions", &openai).unwrap();
        assert_eq!(u4, "https://api.openai.com/v1/chat/completions");

        // OpenAI path with query
        let u5 =
            build_target_url("https://api.openai.com", "/v1/models?limit=10", &openai).unwrap();
        assert_eq!(u5, "https://api.openai.com/v1/models?limit=10");
    }

    #[test]
    fn test_sensitive_headers() {
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("x-api-key"));
        assert!(is_sensitive_header("api-key"));
        assert!(is_sensitive_header("Cookie"));
        assert!(is_sensitive_header("Proxy-Authorization"));
        assert!(is_sensitive_header("x-auth-token"));

        assert!(!is_sensitive_header("Content-Type"));
        assert!(!is_sensitive_header("Accept"));
        assert!(!is_sensitive_header("User-Agent"));
    }
}

/// Returns true if the header is an internal ReToken control header.
pub fn is_internal_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "x-retoken-bypass"
            | "x-retoken-upstream-url"
            | "x-retoken-provider-name"
            | "x-retoken-repo-hash"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_upstream_url() {
        assert!(validate_upstream_url("https://api.groq.com/openai/v1").is_ok());
        assert!(validate_upstream_url("http://127.0.0.1:11434").is_ok());
        assert_eq!(
            validate_upstream_url("https://example.com/api/").unwrap(),
            "https://example.com/api"
        );

        // Disallow dangerous or malformed schemes
        assert!(validate_upstream_url("file:///etc/passwd").is_err());
        assert!(validate_upstream_url("ftp://server/path").is_err());
        assert!(validate_upstream_url("javascript:alert(1)").is_err());
        assert!(validate_upstream_url("").is_err());
        assert!(validate_upstream_url("not a url").is_err());
    }

    #[test]
    fn test_detect_provider_path_and_headers() {
        let empty_headers = vec![];

        // Anthropic path
        let p = detect_provider("/v1/messages", &empty_headers).unwrap();
        assert_eq!(p, Provider::Anthropic);

        // OpenAI path
        let p = detect_provider("/v1/chat/completions", &empty_headers).unwrap();
        assert_eq!(p, Provider::OpenAI);

        // Anthropic header
        let anthropic_headers = vec![("x-api-key".into(), "secret".into())];
        let p = detect_provider("/custom", &anthropic_headers).unwrap();
        assert_eq!(p, Provider::Anthropic);

        // OpenAI header
        let openai_headers = vec![("authorization".into(), "Bearer sk-12345".into())];
        let p = detect_provider("/custom", &openai_headers).unwrap();
        assert_eq!(p, Provider::OpenAI);

        // Generic upstream URL
        let generic_headers = vec![
            ("x-retoken-upstream-url".into(), "https://api.together.xyz/v1".into()),
            ("x-retoken-provider-name".into(), "together".into()),
        ];
        let p = detect_provider("/v1/chat/completions", &generic_headers).unwrap();
        assert_eq!(
            p,
            Provider::Generic {
                name: "together".into(),
                base_url: "https://api.together.xyz/v1".into(),
            }
        );
    }

    #[test]
    fn test_build_target_url_normalization() {
        let anthropic = Provider::Anthropic;
        let openai = Provider::OpenAI;

        // Normal path
        let u1 = build_target_url("https://api.anthropic.com", "/v1/messages", &anthropic).unwrap();
        assert_eq!(u1, "https://api.anthropic.com/v1/messages");

        // Double /v1 path (client had base_url/v1)
        let u2 = build_target_url("https://api.anthropic.com", "/v1/v1/messages", &anthropic).unwrap();
        assert_eq!(u2, "https://api.anthropic.com/v1/messages");

        // Path missing /v1
        let u3 = build_target_url("https://api.anthropic.com", "/messages", &anthropic).unwrap();
        assert_eq!(u3, "https://api.anthropic.com/v1/messages");

        // OpenAI with base ending in /v1
        let u4 = build_target_url("https://api.openai.com/v1", "/v1/chat/completions", &openai).unwrap();
        assert_eq!(u4, "https://api.openai.com/v1/chat/completions");

        // OpenAI path with query
        let u5 = build_target_url("https://api.openai.com", "/v1/models?limit=10", &openai).unwrap();
        assert_eq!(u5, "https://api.openai.com/v1/models?limit=10");
    }

    #[test]
    fn test_sensitive_headers() {
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("x-api-key"));
        assert!(is_sensitive_header("api-key"));
        assert!(is_sensitive_header("Cookie"));
        assert!(is_sensitive_header("Proxy-Authorization"));
        assert!(is_sensitive_header("x-auth-token"));

        assert!(!is_sensitive_header("Content-Type"));
        assert!(!is_sensitive_header("Accept"));
        assert!(!is_sensitive_header("User-Agent"));
    }
}
