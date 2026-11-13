use retoken_core::config::AppConfig;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

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

/// Returns true if an IPv4 address is loopback, private, link-local, broadcast, unspecified, or reserved/testing.
pub fn is_private_or_loopback_ipv4(ipv4: Ipv4Addr) -> bool {
    let octets = ipv4.octets();
    ipv4.is_loopback()            // 127.0.0.0/8
        || ipv4.is_private()      // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
        || ipv4.is_link_local()   // 169.254.0.0/16 (e.g. AWS/GCP metadata)
        || ipv4.is_broadcast()    // 255.255.255.255
        || ipv4.is_unspecified()  // 0.0.0.0
        || (octets[0] == 100 && (octets[1] & 0xc0) == 64) // 100.64.0.0/10 Shared Address Space (Carrier-grade NAT)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0) // 192.0.0.0/24 IETF Protocol
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2) // 192.0.2.0/24 TEST-NET-1
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19)) // 198.18.0.0/15 Benchmark testing
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100) // 198.51.100.0/24 TEST-NET-2
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113) // 203.0.113.0/24 TEST-NET-3
        || (octets[0] >= 224 && octets[0] <= 239) // 224.0.0.0/4 Multicast
        || (octets[0] >= 240) // 240.0.0.0/4 Reserved
}

/// Returns true if an IPv6 address is loopback, unspecified, unique local (ULA), link-local, multicast, or IPv4-mapped private.
pub fn is_private_or_loopback_ipv6(ipv6: Ipv6Addr) -> bool {
    if ipv6.is_loopback() || ipv6.is_unspecified() || ipv6.is_multicast() {
        return true;
    }
    // Check for IPv4-mapped IPv6 address (::ffff:x.x.x.x) or IPv4-compatible
    let segments = ipv6.segments();
    if segments[0] == 0
        && segments[1] == 0
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0xffff
    {
        let v4 = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        );
        return is_private_or_loopback_ipv4(v4);
    }
    // Also check standard to_ipv4()
    if let Some(v4) = ipv6.to_ipv4() {
        if is_private_or_loopback_ipv4(v4) {
            return true;
        }
    }
    // fc00::/7 Unique Local Address (ULA)
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 Link-Local Unicast
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // 2001:db8::/32 Documentation
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return true;
    }
    // 100::/64 Discard prefix
    if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        return true;
    }
    // 2001:20::/28 ORCHIDv2
    if segments[0] == 0x2001 && (segments[1] & 0xfff0) == 0x0020 {
        return true;
    }
    false
}

/// Returns true if an IP address is private, loopback, or local network.
pub fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_or_loopback_ipv4(v4),
        IpAddr::V6(v6) => is_private_or_loopback_ipv6(v6),
    }
}

/// Returns true if the host is a loopback, link-local, or private network address,
/// or resolves to one via DNS.
pub fn is_private_or_loopback_host(host: &str) -> bool {
    let clean = host.trim().trim_start_matches('[').trim_end_matches(']');
    let lower = clean.to_ascii_lowercase();

    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower.ends_with(".intranet")
        || lower.ends_with(".onion")
        || lower.ends_with(".corp")
        || lower.ends_with(".home.arpa")
        || lower.ends_with(".lan")
        || lower.ends_with(".priv")
    {
        return true;
    }

    if let Ok(ip) = clean.parse::<IpAddr>() {
        return is_private_or_loopback_ip(ip);
    }

    // Attempt DNS lookup. If any resolved IP targets a private or loopback address, reject.
    if let Ok(addrs) = (clean, 80).to_socket_addrs() {
        for addr in addrs {
            if is_private_or_loopback_ip(addr.ip()) {
                return true;
            }
        }
    }

    false
}

/// Checks whether a host matches an allowlist of patterns (supporting exact match and `*.domain` wildcards).
pub fn matches_allowed_hosts(host: &str, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    let lower_host = host.to_lowercase();
    for pattern in allowlist {
        let lower_pattern = pattern.to_lowercase();
        if lower_pattern.starts_with("*.") {
            let suffix = &lower_pattern[1..]; // e.g. ".openai.com"
            if lower_host.ends_with(suffix) {
                return true;
            }
        } else if lower_host == lower_pattern {
            return true;
        }
    }
    false
}

/// Validates a custom upstream URL from untrusted headers with an explicit SSRF trust policy and allowlist.
/// Only valid HTTP and HTTPS URLs with a non-empty host are accepted.
/// Unless `allow_private` is true, loopback, link-local, and private addresses are rejected.
/// If `allowed_hosts` is non-empty, the host must match one of the allowed host patterns.
pub fn validate_upstream_url(
    url_str: &str,
    allow_private: bool,
    allowed_hosts: &[String],
) -> Result<String, String> {
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

    let host = match parsed.host_str() {
        Some(h) if !h.is_empty() => h,
        _ => return Err("Upstream URL must include a non-empty host".to_string()),
    };

    if !allow_private && is_private_or_loopback_host(host) {
        return Err(format!(
            "Upstream host '{}' targets a loopback, local, or private address (disallowed by SSRF protection policy). Set RETOKEN_ALLOW_PRIVATE_UPSTREAMS=true if intended.",
            host
        ));
    }

    if !matches_allowed_hosts(host, allowed_hosts) {
        return Err(format!(
            "Upstream host '{}' is not in the allowed upstream hosts list: {:?}",
            host, allowed_hosts
        ));
    }

    let mut clean = parsed.to_string();
    if clean.ends_with('/') {
        clean.pop();
    }
    Ok(clean)
}

/// Resolves a host to its socket addresses, verifies that all resolved IP addresses are safe
/// under the SSRF policy (non-loopback, non-private, non-metadata), and returns the first
/// verified SocketAddr to allow connect-time IP pinning.
pub fn resolve_and_verify_destination(
    host: &str,
    port: u16,
    allow_private: bool,
) -> Result<Option<std::net::SocketAddr>, String> {
    if allow_private {
        return Ok(None);
    }

    let clean = host.trim().trim_start_matches('[').trim_end_matches(']');

    // Check if host is an explicit IP literal
    if let Ok(ip) = clean.parse::<IpAddr>() {
        if is_private_or_loopback_ip(ip) {
            return Err(format!(
                "Destination IP '{}' targets a loopback, local, or private address (disallowed by SSRF policy)",
                ip
            ));
        }
        return Ok(Some(std::net::SocketAddr::new(ip, port)));
    }

    // Static check for internal domain suffixes
    if is_private_or_loopback_host(clean) {
        return Err(format!(
            "Destination host '{}' targets a loopback, local, or internal address (disallowed by SSRF policy)",
            clean
        ));
    }

    // Resolve DNS and verify ALL resolved addresses
    let addrs = (clean, port)
        .to_socket_addrs()
        .map_err(|e| format!("Failed to resolve destination host '{}': {}", clean, e))?;

    let mut first_addr = None;
    let mut resolved_any = false;
    for addr in addrs {
        resolved_any = true;
        if is_private_or_loopback_ip(addr.ip()) {
            return Err(format!(
                "Destination host '{}' resolved to private/loopback IP '{}' (blocked by SSRF protection)",
                clean,
                addr.ip()
            ));
        }
        if first_addr.is_none() {
            first_addr = Some(addr);
        }
    }

    if !resolved_any {
        return Err(format!(
            "Destination host '{}' did not resolve to any addresses",
            clean
        ));
    }

    Ok(first_addr)
}

/// Detects the provider from the incoming request's path and headers.
///
/// Detection strategy:
///   1. If `x-retoken-upstream-url` header is present -> Generic provider (validated)
///   2. If path contains `/v1/messages` or starts with `/messages` -> Anthropic (Claude API)
///   3. If path targets OpenAI Chat Completions, Completions, or Responses -> OpenAI
///   4. If `x-api-key` header present -> Anthropic
///   5. If `Authorization: Bearer` header present -> OpenAI
///   6. Fallback to Anthropic
pub fn detect_provider(
    path: &str,
    headers: &[(String, String)],
    allow_private_upstreams: bool,
    allowed_upstream_hosts: &[String],
) -> Result<Provider, String> {
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
        let clean_url =
            validate_upstream_url(&url, allow_private_upstreams, allowed_upstream_hosts)?;
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
        || path.contains("/v1/responses")
        || path.starts_with("/chat/completions")
        || path.starts_with("/completions")
        || path.starts_with("/responses")
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
                || normalized_path.starts_with("/responses")
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

/// Extracts token usage from a provider response or terminal stream event.
///
/// OpenAI Responses places final usage at `response.usage` in its streamed
/// `response.completed` event, while non-streaming responses and Anthropic
/// events place it at the root. Keeping both shapes here gives the streaming
/// relay one provider-aware parser without changing any response bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub cached_tokens: Option<usize>,
    pub reasoning_tokens: Option<usize>,
}

pub fn extract_token_usage(provider: &Provider, response_body: &[u8]) -> TokenUsage {
    let json: serde_json::Value = match serde_json::from_slice(response_body) {
        Ok(v) => v,
        Err(_) => return TokenUsage::default(),
    };

    extract_token_usage_value(provider, &json)
}

pub fn extract_token_usage_value(provider: &Provider, json: &serde_json::Value) -> TokenUsage {
    let mut usage = TokenUsage::default();

    let usage_obj = match json.get("usage").or_else(|| {
        json.get("response")
            .and_then(|response| response.get("usage"))
    }) {
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
            usage.reasoning_tokens = usage_obj
                .get("output_tokens_details")
                .and_then(|d| d.get("thinking_tokens"))
                .or_else(|| usage_obj.get("thinking_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
        }
        Provider::OpenAI => {
            usage.input_tokens = usage_obj
                .get("prompt_tokens")
                .or_else(|| usage_obj.get("input_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.output_tokens = usage_obj
                .get("completion_tokens")
                .or_else(|| usage_obj.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.cached_tokens = usage_obj
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .or_else(|| {
                    usage_obj
                        .get("input_tokens_details")
                        .and_then(|d| d.get("cached_tokens"))
                })
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.reasoning_tokens = usage_obj
                .get("completion_tokens_details")
                .and_then(|d| d.get("reasoning_tokens"))
                .or_else(|| {
                    usage_obj
                        .get("output_tokens_details")
                        .and_then(|d| d.get("reasoning_tokens"))
                })
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
            usage.cached_tokens = usage_obj
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .or_else(|| usage_obj.get("cache_read_input_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            usage.reasoning_tokens = usage_obj
                .get("completion_tokens_details")
                .and_then(|d| d.get("reasoning_tokens"))
                .or_else(|| {
                    usage_obj
                        .get("output_tokens_details")
                        .and_then(|d| d.get("thinking_tokens"))
                })
                .or_else(|| usage_obj.get("reasoning_tokens"))
                .or_else(|| usage_obj.get("thinking_tokens"))
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
    lower.starts_with("x-retoken-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_upstream_url() {
        assert!(validate_upstream_url("https://api.groq.com/openai/v1", false, &[]).is_ok());
        assert_eq!(
            validate_upstream_url("https://example.com/api/", false, &[]).unwrap(),
            "https://example.com/api"
        );

        // Disallow dangerous or malformed schemes
        assert!(validate_upstream_url("file:///etc/passwd", false, &[]).is_err());
        assert!(validate_upstream_url("ftp://server/path", false, &[]).is_err());
        assert!(validate_upstream_url("javascript:alert(1)", false, &[]).is_err());
        assert!(validate_upstream_url("", false, &[]).is_err());
        assert!(validate_upstream_url("not a url", false, &[]).is_err());

        // SSRF protection: loopback and private IPs rejected when allow_private=false
        assert!(validate_upstream_url("http://127.0.0.1:11434", false, &[]).is_err());
        assert!(validate_upstream_url("http://localhost:3000", false, &[]).is_err());
        assert!(validate_upstream_url("http://service.local:8080", false, &[]).is_err());
        assert!(validate_upstream_url("http://metadata.internal", false, &[]).is_err());
        assert!(validate_upstream_url("http://169.254.169.254/latest", false, &[]).is_err());
        assert!(validate_upstream_url("http://10.0.0.1/api", false, &[]).is_err());
        assert!(validate_upstream_url("http://192.168.1.1/api", false, &[]).is_err());

        // IPv6 SSRF protection: loopback, ULA, link-local, IPv4-mapped
        assert!(validate_upstream_url("http://[::1]:8080", false, &[]).is_err());
        assert!(validate_upstream_url("http://[fd00::1]/v1", false, &[]).is_err());
        assert!(validate_upstream_url("http://[fe80::1]/v1", false, &[]).is_err());
        assert!(validate_upstream_url("http://[::ffff:127.0.0.1]/v1", false, &[]).is_err());
        assert!(validate_upstream_url("http://[::ffff:169.254.169.254]/v1", false, &[]).is_err());

        // Allowed when allow_private=true
        assert!(validate_upstream_url("http://127.0.0.1:11434", true, &[]).is_ok());
        assert!(validate_upstream_url("http://localhost:3000", true, &[]).is_ok());
        assert!(validate_upstream_url("http://[::1]:8080", true, &[]).is_ok());

        // Host allowlist enforcement
        let allowlist = vec!["api.groq.com".into(), "*.together.xyz".into()];
        assert!(validate_upstream_url("https://api.groq.com/v1", false, &allowlist).is_ok());
        assert!(validate_upstream_url("https://router.together.xyz/v1", false, &allowlist).is_ok());
        assert!(validate_upstream_url("https://malicious.com/v1", false, &allowlist).is_err());
    }

    #[test]
    fn test_detect_provider_path_and_headers() {
        let empty_headers = vec![];

        // Anthropic path
        let p = detect_provider("/v1/messages", &empty_headers, false, &[]).unwrap();
        assert_eq!(p, Provider::Anthropic);

        // OpenAI path
        let p = detect_provider("/v1/chat/completions", &empty_headers, false, &[]).unwrap();
        assert_eq!(p, Provider::OpenAI);

        // OpenAI Responses path (used by current Codex integrations)
        let p = detect_provider("/v1/responses", &empty_headers, false, &[]).unwrap();
        assert_eq!(p, Provider::OpenAI);

        // Anthropic header
        let anthropic_headers = vec![("x-api-key".into(), "secret".into())];
        let p = detect_provider("/custom", &anthropic_headers, false, &[]).unwrap();
        assert_eq!(p, Provider::Anthropic);

        // OpenAI header
        let openai_headers = vec![("authorization".into(), "Bearer sk-12345".into())];
        let p = detect_provider("/custom", &openai_headers, false, &[]).unwrap();
        assert_eq!(p, Provider::OpenAI);

        // Generic upstream URL
        let generic_headers = vec![
            (
                "x-retoken-upstream-url".into(),
                "https://api.together.xyz/v1".into(),
            ),
            ("x-retoken-provider-name".into(), "together".into()),
        ];
        let p = detect_provider("/v1/chat/completions", &generic_headers, false, &[]).unwrap();
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

    #[test]
    fn test_is_internal_header() {
        assert!(is_internal_header("x-retoken-bypass"));
        assert!(is_internal_header("X-ReToken-Upstream-URL"));
        assert!(is_internal_header("x-retoken-cache"));
        assert!(is_internal_header("x-retoken-no-cache"));
        assert!(is_internal_header("x-retoken-repo-hash"));
        assert!(is_internal_header("x-retoken-custom-control"));
        assert!(!is_internal_header("Authorization"));
        assert!(!is_internal_header("x-api-key"));
        assert!(!is_internal_header("anthropic-version"));
        assert!(!is_internal_header("content-type"));
    }

    #[test]
    fn test_extract_token_usage_openai_reasoning_tokens() {
        let response = serde_json::json!({
            "id": "chatcmpl-123",
            "usage": {
                "prompt_tokens": 1500,
                "completion_tokens": 800,
                "total_tokens": 2300,
                "prompt_tokens_details": {
                    "cached_tokens": 1024
                },
                "completion_tokens_details": {
                    "reasoning_tokens": 600
                }
            }
        });
        let body = serde_json::to_vec(&response).unwrap();
        let usage = extract_token_usage(&Provider::OpenAI, &body);
        assert_eq!(usage.input_tokens, Some(1500));
        assert_eq!(usage.output_tokens, Some(800));
        assert_eq!(usage.cached_tokens, Some(1024));
        assert_eq!(usage.reasoning_tokens, Some(600));
    }

    #[test]
    fn test_extract_token_usage_openai_responses_tokens() {
        let response = serde_json::json!({
            "id": "resp_123",
            "usage": {
                "input_tokens": 1500,
                "output_tokens": 800,
                "input_tokens_details": { "cached_tokens": 1024 },
                "output_tokens_details": { "reasoning_tokens": 600 }
            }
        });
        let body = serde_json::to_vec(&response).unwrap();
        let usage = extract_token_usage(&Provider::OpenAI, &body);
        assert_eq!(usage.input_tokens, Some(1500));
        assert_eq!(usage.output_tokens, Some(800));
        assert_eq!(usage.cached_tokens, Some(1024));
        assert_eq!(usage.reasoning_tokens, Some(600));
    }

    #[test]
    fn test_extract_token_usage_openai_stream_terminal_event() {
        let event = serde_json::json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 1500,
                    "output_tokens": 800,
                    "input_tokens_details": { "cached_tokens": 1024 },
                    "output_tokens_details": { "reasoning_tokens": 600 }
                }
            }
        });

        let usage = extract_token_usage_value(&Provider::OpenAI, &event);
        assert_eq!(usage.input_tokens, Some(1500));
        assert_eq!(usage.output_tokens, Some(800));
        assert_eq!(usage.cached_tokens, Some(1024));
        assert_eq!(usage.reasoning_tokens, Some(600));
    }

    #[test]
    fn test_extract_token_usage_anthropic_thinking_tokens() {
        let response = serde_json::json!({
            "id": "msg_01X999",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "Analyzing AST...",
                    "signature": "sig123"
                },
                {
                    "type": "text",
                    "text": "Here is the refactored code."
                }
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 420,
                "output_tokens": 1200,
                "cache_read_input_tokens": 300,
                "output_tokens_details": {
                    "thinking_tokens": 850
                }
            }
        });
        let body = serde_json::to_vec(&response).unwrap();
        let usage = extract_token_usage(&Provider::Anthropic, &body);
        assert_eq!(usage.input_tokens, Some(420));
        assert_eq!(usage.output_tokens, Some(1200));
        assert_eq!(usage.cached_tokens, Some(300));
        assert_eq!(usage.reasoning_tokens, Some(850));
    }
}
