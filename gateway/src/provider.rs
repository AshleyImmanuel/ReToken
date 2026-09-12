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
    pub fn base_url(&self) -> &str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::OpenAI => "https://api.openai.com",
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

/// Detects the provider from the incoming request's path and headers.
///
/// Detection strategy:
///   1. If `x-retoken-upstream-url` header is present -> Generic provider (supports ANY API like Kimi, Groq, etc)
///   2. If path contains `/v1/messages` -> Anthropic (Claude API)
///   3. If path contains `/v1/chat/completions` -> OpenAI
///   4. If `x-api-key` header present -> Anthropic
///   5. If `Authorization: Bearer` header present -> OpenAI
///   6. Fallback to Anthropic
pub fn detect_provider(
    path: &str,
    headers: &[(String, String)],
) -> Provider {
    // 1. Generic provider via explicit upstream URL (Supports ANY provider)
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
        // Remove trailing slash if present to cleanly append paths
        let clean_url = url.trim_end_matches('/').to_string();
        return Provider::Generic {
            name: provider_name,
            base_url: clean_url,
        };
    }

    // 2 & 3. Path-based detection
    if path.contains("/v1/messages") {
        return Provider::Anthropic;
    }
    if path.contains("/v1/chat/completions") || path.contains("/v1/completions") {
        return Provider::OpenAI;
    }

    // 4 & 5. Header-based detection
    for (name, _value) in headers {
        if name.eq_ignore_ascii_case("x-api-key") {
            return Provider::Anthropic;
        }
    }
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("authorization") && value.starts_with("Bearer ") {
            return Provider::OpenAI;
        }
    }

    // Default fallback
    Provider::Anthropic
}

/// Extracts token usage from a provider's response body.
///
/// Anthropic response format:
/// ```json
/// { "usage": { "input_tokens": 100, "output_tokens": 50, "cache_read_input_tokens": 80 } }
/// ```
///
/// OpenAI response format:
/// ```json
/// { "usage": { "prompt_tokens": 100, "completion_tokens": 50, "prompt_tokens_details": { "cached_tokens": 80 } } }
/// ```
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
            usage.input_tokens = usage_obj.get("input_tokens").and_then(|v| v.as_u64()).map(|v| v as usize);
            usage.output_tokens = usage_obj.get("output_tokens").and_then(|v| v.as_u64()).map(|v| v as usize);
            usage.cached_tokens = usage_obj.get("cache_read_input_tokens").and_then(|v| v.as_u64()).map(|v| v as usize);
        }
        Provider::OpenAI => {
            usage.input_tokens = usage_obj.get("prompt_tokens").and_then(|v| v.as_u64()).map(|v| v as usize);
            usage.output_tokens = usage_obj.get("completion_tokens").and_then(|v| v.as_u64()).map(|v| v as usize);
            usage.cached_tokens = usage_obj.get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
        }
        Provider::Generic { .. } => {
            // Try Anthropic format first, then OpenAI
            usage.input_tokens = usage_obj.get("input_tokens")
                .or_else(|| usage_obj.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.output_tokens = usage_obj.get("output_tokens")
                .or_else(|| usage_obj.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
        }
    }

    usage
}

/// Returns a list of header names that should NEVER be logged.
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "x-api-key" | "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
    )
}
