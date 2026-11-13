use std::time::Duration;

use rmcp::{
    handler::server::wrapper::Parameters, schemars, tool, tool_router, transport::stdio, ServiceExt,
};
use serde::Deserialize;
use serde_json::{json, Value};

const GATEWAY_URL: &str = "http://127.0.0.1:8888";

#[derive(Clone)]
struct ReTokenMcp {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CacheClearParams {
    /// Must be `true` before ReToken clears its local in-memory response cache.
    confirm: bool,
}

#[tool_router(server_handler)]
impl ReTokenMcp {
    #[tool(
        description = "Return ReToken gateway health, token telemetry, and response-cache status. Read only."
    )]
    async fn retoken_status(&self) -> String {
        match self.gateway_json("/_retoken/stats").await {
            Ok(stats) => pretty_json(stats),
            Err(message) => tool_error(message),
        }
    }

    #[tool(
        description = "Diagnose whether ReToken is reachable on loopback and explain how to start it. Read only."
    )]
    async fn retoken_doctor(&self) -> String {
        match self.gateway_json("/_retoken/stats").await {
            Ok(stats) => pretty_json(json!({
                "status": "ok",
                "gateway": GATEWAY_URL,
                "message": "ReToken is reachable and its local administrative API is responding.",
                "telemetry": stats
            })),
            Err(message) => pretty_json(json!({
                "status": "error",
                "gateway": GATEWAY_URL,
                "message": format!("ReToken is not reachable: {message}"),
                "next_step": "Start a provider CLI through tools\\retoken-codex.ps1 or run ReToken\\target\\release\\retoken.exe run."
            })),
        }
    }

    #[tool(
        description = "Clear ReToken's local response cache. Set confirm to true only when cache removal is intended."
    )]
    async fn retoken_cache_clear(
        &self,
        Parameters(CacheClearParams { confirm }): Parameters<CacheClearParams>,
    ) -> String {
        if !confirm {
            return pretty_json(json!({
                "status": "refused",
                "message": "Cache was not cleared. Call again with confirm: true to perform this local destructive action."
            }));
        }

        match self.gateway_post_json("/_retoken/cache/clear").await {
            Ok(result) => pretty_json(result),
            Err(message) => tool_error(message),
        }
    }

    #[tool(
        description = "Summarize ReToken's current request, token, cache, and error metrics. Read only."
    )]
    async fn retoken_report(&self) -> String {
        match self.gateway_json("/_retoken/stats").await {
            Ok(stats) => {
                let report = json!({
                    "status": "ok",
                    "gateway": GATEWAY_URL,
                    "requests": stats.get("total_requests").or_else(|| stats.get("requests")),
                    "provider_errors": stats.get("provider_errors"),
                    "input_tokens": stats.get("input_tokens").or_else(|| stats.get("total_input_tokens")),
                    "output_tokens": stats.get("output_tokens").or_else(|| stats.get("total_output_tokens")),
                    "cached_tokens": stats.get("cached_tokens").or_else(|| stats.get("total_cached_tokens")),
                    "cache": stats.get("cache"),
                    "note": "Raw token totals and provider cache discounts are reported separately. Cost-per-solved-task benchmarking is not enabled yet."
                });
                pretty_json(report)
            }
            Err(message) => tool_error(message),
        }
    }
}

impl ReTokenMcp {
    async fn gateway_json(&self, path: &str) -> Result<Value, String> {
        let response = self
            .client
            .get(format!("{GATEWAY_URL}{path}"))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        response_json(response).await
    }

    async fn gateway_post_json(&self, path: &str) -> Result<Value, String> {
        let response = self
            .client
            .post(format!("{GATEWAY_URL}{path}"))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        response_json(response).await
    }
}

async fn response_json(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let body = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("gateway returned HTTP {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|error| format!("gateway returned invalid JSON: {error}"))
}

fn pretty_json(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn tool_error(message: String) -> String {
    pretty_json(json!({ "status": "error", "message": message }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(800))
        .build()?;
    let server = ReTokenMcp { client }.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}
