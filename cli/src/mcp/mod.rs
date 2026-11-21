//! `retoken mcp`: a stdio MCP server exposing `retoken_retrieve`.
//!
//! When an agent registers this server, its requests carry a `*retoken_retrieve` tool. The
//! gateway detects it and may elide large command/JSON output (even on streaming requests),
//! because the agent can now fetch originals itself from the shared CCR store.
//!
//! This module is the transport: newline-delimited JSON-RPC 2.0 on stdin/stdout. Nothing else
//! may be written to stdout, so this command runs without the tracing subscriber. The tool
//! itself lives in [`retrieve`].

mod retrieve;

use compressors::ccr::CcrStore;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

pub fn run_mcp() -> anyhow::Result<()> {
    let store = CcrStore::open(None).map_err(anyhow::Error::msg)?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle_message(&store, &msg),
            Err(e) => Some(error_response(
                Value::Null,
                -32700,
                &format!("Parse error: {e}"),
            )),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Handles one JSON-RPC message. Returns `None` for notifications.
fn handle_message(store: &CcrStore, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?; // notifications carry no id and get no reply
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "retoken", "version": env!("CARGO_PKG_VERSION") }
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [retrieve::definition()] }),
        "tools/call" => match params.get("name").and_then(|n| n.as_str()) {
            Some(retrieve::TOOL_NAME) => {
                retrieve::call(store, params.get("arguments").unwrap_or(&Value::Null))
            }
            _ => retrieve::error("Unknown tool"),
        },
        _ => {
            return Some(error_response(
                id,
                -32601,
                &format!("Method not found: {method}"),
            ))
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_round_trip() {
        let store = CcrStore::open_in_memory().unwrap();
        let handle = store.store(b"payload", "log").unwrap();

        let init = handle_message(
            &store,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle_message(
            &store,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());

        let list = handle_message(
            &store,
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .unwrap();
        assert_eq!(list["result"]["tools"][0]["name"], retrieve::TOOL_NAME);

        let call = handle_message(
            &store,
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
                    "params":{"name": retrieve::TOOL_NAME, "arguments": {"handle": handle}}}),
        )
        .unwrap();
        assert_eq!(call["result"]["content"][0]["text"], "payload");

        let unknown =
            handle_message(&store, &json!({"jsonrpc":"2.0","id":4,"method":"nope"})).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
