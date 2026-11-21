//! DNS-rebinding protection.
//!
//! The gateway listens on 127.0.0.1 only, so every legitimate client addresses it as a
//! loopback host. A web page can still reach it by rebinding its own domain to 127.0.0.1; the
//! browser then sends that domain in the `Host` header. Rejecting non-loopback hosts stops a
//! page from driving the proxy or its `/_retoken/*` admin routes.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use tracing::warn;

/// Middleware: rejects requests that name a non-loopback host with 403. Requests without a
/// host (HTTP/1.0 clients, in-process calls) are allowed; browsers always send one.
pub async fn require_loopback_host(req: Request, next: Next) -> Result<Response, StatusCode> {
    let host = req
        .headers()
        .get(header::HOST)
        .map(|h| h.to_str().unwrap_or(""))
        .or_else(|| req.uri().authority().map(|a| a.as_str()));
    if let Some(host) = host {
        if !is_loopback_host(host) {
            warn!(
                "Rejected request for non-loopback host {:?} (possible DNS rebinding)",
                host
            );
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(next.run(req).await)
}

/// True for `localhost`, `127.0.0.1` and `[::1]`, with or without a port.
fn is_loopback_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts() {
        for host in [
            "127.0.0.1:8888",
            "localhost",
            "LOCALHOST:9000",
            "[::1]:8888",
            "127.0.0.1",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "evil.example",
            "evil.example:8888",
            "127.0.0.1.evil.example",
            "",
            "[::2]:1",
        ] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }
}
