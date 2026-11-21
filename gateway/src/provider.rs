use reqwest::header::HeaderMap;
use retoken_core::config::AppConfig;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

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

/// The request/response schema a request uses. A generic upstream (e.g. an OpenAI-compatible
/// gateway that also serves Claude) can speak either, so this is decided per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    /// Anthropic Messages API: top-level `system`, `tool_use`/`tool_result` content blocks.
    Anthropic,
    /// OpenAI Chat Completions and compatible APIs: `system` role messages, `tool_calls`.
    OpenAI,
}

impl WireFormat {
    /// Anthropic for the Anthropic provider and for any `/messages` endpoint; OpenAI otherwise.
    pub fn detect(provider: &Provider, path: &str) -> Self {
        let path = path.split('?').next().unwrap_or(path);
        match provider {
            Provider::Anthropic => WireFormat::Anthropic,
            Provider::OpenAI => WireFormat::OpenAI,
            Provider::Generic { .. } if path.contains("/messages") => WireFormat::Anthropic,
            Provider::Generic { .. } => WireFormat::OpenAI,
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

/// Returns true if the host is a loopback, link-local, or private network address by name
/// or IP literal. Performs no DNS lookup: hostnames are verified against their resolved
/// addresses at connect time by [`SsrfResolver`].
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

/// A destination rejected by the SSRF policy. Kept as a distinct error type so it can be
/// recognized (via [`is_ssrf_block`]) after reqwest wraps it in a connect error.
#[derive(Debug)]
pub struct SsrfBlocked(pub String);

impl std::fmt::Display for SsrfBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SsrfBlocked {}

/// Rejects a destination host that is private by name or IP literal. Performs no DNS lookup.
///
/// reqwest never calls the DNS resolver for IP-literal hosts, so callers must run this check
/// before sending; hostnames are additionally verified by [`SsrfResolver`] at connect time.
pub fn check_destination_host(host: &str) -> Result<(), SsrfBlocked> {
    if is_private_or_loopback_host(host) {
        return Err(SsrfBlocked(format!(
            "Destination host '{}' targets a loopback, local, or private address (disallowed by SSRF policy)",
            host
        )));
    }
    Ok(())
}

/// Verifies that a hostname resolved to at least one address and that none of them are
/// loopback, private, link-local, or metadata addresses.
pub fn verify_resolved_addrs(host: &str, addrs: &[SocketAddr]) -> Result<(), SsrfBlocked> {
    if addrs.is_empty() {
        return Err(SsrfBlocked(format!(
            "Destination host '{}' did not resolve to any addresses",
            host
        )));
    }
    if let Some(bad) = addrs.iter().find(|a| is_private_or_loopback_ip(a.ip())) {
        return Err(SsrfBlocked(format!(
            "Destination host '{}' resolved to private/loopback IP '{}' (blocked by SSRF protection)",
            host,
            bad.ip()
        )));
    }
    Ok(())
}

/// DNS resolver for the shared upstream client that enforces the SSRF policy at connect time.
///
/// reqwest connects only to the addresses this resolver returns, so the addresses that were
/// verified are exactly the ones dialed (no DNS-rebinding TOCTOU window). Resolution is async
/// and the shared client keeps its connection pool across requests.
pub struct SsrfResolver;

impl reqwest::dns::Resolve for SsrfResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            check_destination_host(&host)?;
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            verify_resolved_addrs(&host, &addrs)?;
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Returns true if the error (or any error in its source chain) is an SSRF policy rejection.
pub fn is_ssrf_block(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(err);
    while let Some(e) = current {
        if e.is::<SsrfBlocked>() {
            return true;
        }
        current = e.source();
    }
    false
}

/// Copies request headers that may be forwarded upstream, dropping `Host`, `Content-Length`,
/// hop-by-hop headers, and internal `x-retoken-*` control headers. Values are copied as raw
/// bytes, so non-UTF-8 header values survive unchanged.
///
/// `Accept-Encoding` is dropped too: the gateway reads response bodies (token usage, CCR
/// recovery, caching) and does not decompress, so upstream must answer uncompressed. Only the
/// loopback leg to the client is affected.
pub fn forwardable_request_headers(headers: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let name_str = name.as_str();
        if name != reqwest::header::HOST
            && name != reqwest::header::CONTENT_LENGTH
            && name != reqwest::header::ACCEPT_ENCODING
            && !is_hop_by_hop_header(name_str)
            && !is_internal_header(name_str)
        {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

/// Detects the provider from the incoming request's path and headers.
///
/// Detection strategy:
///   1. If `x-retoken-upstream-url` header is present -> Generic provider (validated)
///   2. Credentials of a provider ReToken cannot route (Gemini `x-goog-api-key`, `?key=`)
///      -> rejected, so they are never forwarded to another provider
///   3. If path contains `/v1/messages` or starts with `/messages` -> Anthropic (Claude API)
///   4. If path contains `/v1/chat/completions` or starts with `/chat/completions` -> OpenAI
///   5. Anthropic headers (`x-api-key`, `anthropic-version`, `anthropic-beta`) -> Anthropic.
///      Checked before `Bearer` because Anthropic OAuth clients (Claude Code subscriptions)
///      also send `Authorization: Bearer`.
///   6. If `Authorization: Bearer` header present -> OpenAI
///   7. Fallback to Anthropic (no credentials present, so nothing can leak)
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

    // 2. Never guess a destination for another provider's credentials.
    let has_header = |wanted: &str| headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(wanted));
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    if has_header("x-goog-api-key")
        || query.split('&').any(|kv| kv.starts_with("key="))
        || path.starts_with("/v1beta/")
    {
        return Err(
            "Gemini API requests are not supported; set x-retoken-upstream-url to route them explicitly"
                .to_string(),
        );
    }

    // 3 & 4. Path-based detection
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

    // 5 & 6. Header-based detection
    if has_header("x-api-key") || has_header("anthropic-version") || has_header("anthropic-beta") {
        return Ok(Provider::Anthropic);
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

/// Token counts reported by a provider.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub cached_tokens: Option<usize>,
    pub reasoning_tokens: Option<usize>,
}

pub fn extract_token_usage(provider: &Provider, response_body: &[u8]) -> TokenUsage {
    match serde_json::from_slice::<serde_json::Value>(response_body) {
        Ok(json) => json
            .get("usage")
            .map(|u| usage_from_object(provider, u))
            .unwrap_or_default(),
        Err(_) => TokenUsage::default(),
    }
}

/// Parses a provider `usage` object into token counts.
pub fn usage_from_object(provider: &Provider, usage_obj: &serde_json::Value) -> TokenUsage {
    let mut usage = TokenUsage::default();

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
            usage.reasoning_tokens = usage_obj
                .get("completion_tokens_details")
                .and_then(|d| d.get("reasoning_tokens"))
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
    fn test_detect_provider_never_misroutes_credentials() {
        let h = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        // Gemini credentials must never be forwarded to Anthropic or OpenAI.
        assert!(detect_provider("/v1beta/models/g:generateContent", &h(&[]), false, &[]).is_err());
        assert!(detect_provider("/v1/models", &h(&[("x-goog-api-key", "k")]), false, &[]).is_err());
        assert!(detect_provider("/v1/models?key=abc", &h(&[]), false, &[]).is_err());
        // Anthropic OAuth (Bearer + anthropic-version) on a non-messages path stays Anthropic.
        let oauth = h(&[
            ("authorization", "Bearer sk-ant-oat"),
            ("anthropic-version", "2023-06-01"),
        ]);
        assert_eq!(
            detect_provider("/v1/models", &oauth, false, &[]).unwrap(),
            Provider::Anthropic
        );
        // Plain Bearer is still OpenAI.
        let openai = h(&[("authorization", "Bearer sk-1")]);
        assert_eq!(
            detect_provider("/v1/responses", &openai, false, &[]).unwrap(),
            Provider::OpenAI
        );
    }

    #[test]
    fn test_wire_format_follows_the_endpoint_for_generic_upstreams() {
        let generic = Provider::Generic {
            name: "zen".into(),
            base_url: "https://opencode.ai/zen".into(),
        };
        assert_eq!(
            WireFormat::detect(&generic, "/v1/messages"),
            WireFormat::Anthropic
        );
        assert_eq!(
            WireFormat::detect(&generic, "/v1/chat/completions"),
            WireFormat::OpenAI
        );
        assert_eq!(
            WireFormat::detect(&Provider::Anthropic, "/v1/models"),
            WireFormat::Anthropic
        );
        assert_eq!(
            WireFormat::detect(&Provider::OpenAI, "/v1/messages?x=1"),
            WireFormat::OpenAI
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
