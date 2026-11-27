//! Decides whether a request may be served from, or stored in, the exact response cache.

use super::request::RequestParams;

/// True if the request must bypass the response cache:
/// - the client opted out (`Cache-Control: no-cache/no-store`, `Pragma: no-cache`,
///   `x-retoken-cache: false`, `x-retoken-no-cache: true`);
/// - it is a generation endpoint and the client did not opt in (`x-retoken-cache`,
///   `x-retoken-cache-exact`, or `x-retoken-cache-stochastic`);
/// - sampling is non-deterministic (temperature omitted or > 0 without a seed, or `n > 1`)
///   and the client did not opt in with `x-retoken-cache-stochastic`.
pub fn bypasses_cache(path: &str, headers: &[(String, String)], params: &RequestParams) -> bool {
    let header_is = |name: &str, values: &[&str]| {
        headers
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case(name) && values.contains(&v.as_str()))
    };
    let header_contains = |name: &str, needle: &str| {
        headers
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case(name) && v.contains(needle))
    };
    const TRUE: &[&str] = &["true", "1"];

    let client_opt_out = header_contains("cache-control", "no-cache")
        || header_contains("cache-control", "no-store")
        || header_contains("pragma", "no-cache")
        || header_is("x-retoken-cache", &["false", "0", "off"])
        || header_is("x-retoken-no-cache", TRUE);

    // Generation endpoints require explicit opt-in so stale generations are never replayed.
    let is_generation = is_generation_endpoint(path);
    let allow_stochastic = header_is("x-retoken-cache-stochastic", TRUE);
    let allow_generation = allow_stochastic
        || header_is("x-retoken-cache", TRUE)
        || header_is("x-retoken-cache-exact", TRUE);

    let is_stochastic = if is_generation {
        params.multiple_completions || !(params.has_seed || params.temperature == Some(0.0))
    } else {
        params.temperature.is_some_and(|t| t > 0.0)
    };

    client_opt_out
        || (is_generation && !allow_generation)
        || (is_stochastic && !allow_stochastic)
        || params.multiple_completions
}

/// The request path without its query string or trailing slash, lowercased.
fn endpoint(path: &str) -> String {
    path.split('?')
        .next()
        .unwrap_or(path)
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Returns true if the request path targets an LLM completion/generation endpoint
/// (Anthropic Messages, OpenAI Chat Completions, legacy Completions, OpenAI Responses,
/// Gemini `generateContent` and `streamGenerateContent`).
/// Matched by suffix so prefixed base URLs (`/w/claude/v1/messages`) still count, while
/// sub-resources such as `/v1/responses/input_tokens` do not.
pub fn is_generation_endpoint(path: &str) -> bool {
    let path = endpoint(path);
    [
        "/messages",
        "/chat/completions",
        "/completions",
        "/responses",
        ":generatecontent",
        ":streamgeneratecontent",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

/// Returns true for the OpenAI Responses API, whose structured items are forwarded
/// byte-for-byte: its upstream prompt caching works without rewriting the body.
pub fn is_openai_responses_endpoint(path: &str) -> bool {
    endpoint(path).ends_with("/responses")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn test_is_generation_endpoint() {
        assert!(is_generation_endpoint("/v1/messages"));
        assert!(is_generation_endpoint("/messages"));
        assert!(is_generation_endpoint("/v1/chat/completions"));
        assert!(is_generation_endpoint("/chat/completions"));
        assert!(is_generation_endpoint("/v1/completions"));
        assert!(is_generation_endpoint("/v1/responses"));
        assert!(is_generation_endpoint("/v1/responses?stream=true"));
        assert!(is_generation_endpoint("/w/claude/v1/messages/"));
        assert!(!is_generation_endpoint("/v1/models"));
        assert!(!is_generation_endpoint("/v1/responses/input_tokens"));
        assert!(!is_generation_endpoint("/v1/embeddings"));
        assert!(!is_generation_endpoint("/_retoken/stats"));
    }

    #[test]
    fn test_is_openai_responses_endpoint() {
        assert!(is_openai_responses_endpoint("/v1/responses"));
        assert!(is_openai_responses_endpoint("/responses?stream=true"));
        assert!(!is_openai_responses_endpoint("/v1/responses/input_tokens"));
        assert!(!is_openai_responses_endpoint("/v1/chat/completions"));
    }

    #[test]
    fn generation_requires_opt_in_and_determinism() {
        let deterministic = RequestParams {
            temperature: Some(0.0),
            ..Default::default()
        };
        assert!(bypasses_cache(
            "/v1/messages",
            &headers(&[]),
            &deterministic
        ));
        let opted_in = headers(&[("x-retoken-cache", "true")]);
        assert!(!bypasses_cache("/v1/messages", &opted_in, &deterministic));
        // Omitted temperature samples stochastically: needs the stochastic opt-in.
        assert!(bypasses_cache(
            "/v1/messages",
            &opted_in,
            &RequestParams::default()
        ));
        let stochastic_ok = headers(&[("x-retoken-cache-stochastic", "1")]);
        assert!(!bypasses_cache(
            "/v1/messages",
            &stochastic_ok,
            &RequestParams::default()
        ));
        // n > 1 never caches.
        let multi = RequestParams {
            multiple_completions: true,
            ..Default::default()
        };
        assert!(bypasses_cache("/v1/messages", &stochastic_ok, &multi));
    }

    #[test]
    fn non_generation_endpoints_cache_unless_opted_out() {
        let params = RequestParams::default();
        assert!(!bypasses_cache("/v1/models", &headers(&[]), &params));
        assert!(bypasses_cache(
            "/v1/models",
            &headers(&[("Cache-Control", "no-store")]),
            &params
        ));
        assert!(bypasses_cache(
            "/v1/models",
            &headers(&[("x-retoken-cache", "off")]),
            &params
        ));
    }
}
