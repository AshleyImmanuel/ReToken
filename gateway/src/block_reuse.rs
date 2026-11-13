//! Quality-preserving reuse of exact duplicate tool output inside one request.
//!
//! The first copy stays in the provider payload verbatim. A later copy is
//! replaced only when its complete bytes are already present earlier in that
//! same request, so the model never needs an external retrieval round-trip to
//! recover omitted context. This is deliberately narrower than lossy CCR.

use serde_json::Value;
use std::collections::HashMap;

/// Tool output below this threshold is not worth replacing with a reference.
pub const MIN_REUSABLE_BLOCK_BYTES: usize = 2_048;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BlockReuseStats {
    pub candidates: usize,
    pub reused_blocks: usize,
    pub bytes_saved: usize,
}

/// Replaces exact duplicate tool output in the OpenAI Chat, Anthropic Messages,
/// and OpenAI Responses request shapes. Unknown shapes are left unchanged.
pub fn reuse_exact_tool_outputs(body: &mut Value) -> BlockReuseStats {
    let mut stats = BlockReuseStats::default();
    let mut seen = HashMap::<String, usize>::new();
    let mut ordinal = 0;

    reuse_openai_chat_messages(body, &mut seen, &mut ordinal, &mut stats);
    reuse_anthropic_tool_results(body, &mut seen, &mut ordinal, &mut stats);
    reuse_openai_response_items(body, &mut seen, &mut ordinal, &mut stats);

    stats
}

fn reuse_openai_chat_messages(
    body: &mut Value,
    seen: &mut HashMap<String, usize>,
    ordinal: &mut usize,
    stats: &mut BlockReuseStats,
) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if let Some(content) = message.get_mut("content").and_then(|value| value.as_str()) {
            let mut content = content.to_string();
            reuse_text(&mut content, seen, ordinal, stats);
            message["content"] = Value::String(content);
        }
    }
}

fn reuse_anthropic_tool_results(
    body: &mut Value,
    seen: &mut HashMap<String, usize>,
    ordinal: &mut usize,
    stats: &mut BlockReuseStats,
) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    for message in messages {
        let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            if let Some(content) = block.get_mut("content").and_then(|value| value.as_str()) {
                let mut content = content.to_string();
                reuse_text(&mut content, seen, ordinal, stats);
                block["content"] = Value::String(content);
            }
        }
    }
}

fn reuse_openai_response_items(
    body: &mut Value,
    seen: &mut HashMap<String, usize>,
    ordinal: &mut usize,
    stats: &mut BlockReuseStats,
) {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };

    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("function_call_output") {
            continue;
        }
        if let Some(output) = item.get_mut("output").and_then(|value| value.as_str()) {
            let mut output = output.to_string();
            reuse_text(&mut output, seen, ordinal, stats);
            item["output"] = Value::String(output);
        }
    }
}

fn reuse_text(
    text: &mut String,
    seen: &mut HashMap<String, usize>,
    ordinal: &mut usize,
    stats: &mut BlockReuseStats,
) {
    if text.len() < MIN_REUSABLE_BLOCK_BYTES || !is_safe_successful_tool_output(text) {
        return;
    }
    stats.candidates += 1;
    *ordinal += 1;

    let Some(first_ordinal) = seen.get(text).copied() else {
        seen.insert(text.clone(), *ordinal);
        return;
    };

    let pointer = format!(
        "[ReToken exact duplicate of tool output #{first_ordinal}; the complete identical bytes appear earlier in this request.]"
    );
    if pointer.len() >= text.len() {
        return;
    }

    stats.reused_blocks += 1;
    stats.bytes_saved += text.len() - pointer.len();
    *text = pointer;
}

/// Reuse is deliberately allow-listed. A duplicated failure, warning, secret,
/// or source excerpt may be important to the model even when an earlier copy
/// exists, so it stays verbatim. The false-negative cost is only a missed
/// optimisation; the false-positive cost could be a worse coding decision.
fn is_safe_successful_tool_output(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const UNSAFE_MARKERS: &[&str] = &[
        "error",
        "failed",
        "failure",
        "fatal",
        "panic",
        "exception",
        "stack trace",
        "assertion",
        "warning",
        "warn:",
        "access denied",
        "permission denied",
        "not found",
        "api_key",
        "api-key",
        "authorization:",
        "bearer ",
        "password=",
        "secret",
        "private key",
        "begin rsa",
        "begin openssh",
    ];
    if UNSAFE_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return false;
    }
    if looks_like_source_code(text) {
        return false;
    }

    // A result needs a positive success signal as well as no danger signal.
    const SUCCESS_MARKERS: &[&str] = &[
        "success",
        "passed",
        "completed",
        "up to date",
        "no changes",
        "all checks",
        "exit code: 0",
        "finished",
    ];
    SUCCESS_MARKERS.iter().any(|marker| lower.contains(marker))
}

fn looks_like_source_code(text: &str) -> bool {
    let code_lines = text
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with("fn ")
                || line.starts_with("pub ")
                || line.starts_with("class ")
                || line.starts_with("function ")
                || line.starts_with("import ")
                || line.starts_with("export ")
                || line.starts_with("const ")
                || line.starts_with("let ")
                || line.starts_with("def ")
        })
        .count();
    code_lines >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn large_output() -> String {
        "successful command output\n".repeat(120)
    }

    #[test]
    fn reuses_only_second_exact_openai_response_output() {
        let output = large_output();
        let mut body = json!({
            "input": [
                {"type": "function_call_output", "call_id": "first", "output": output},
                {"type": "function_call_output", "call_id": "second", "output": large_output()}
            ]
        });

        let stats = reuse_exact_tool_outputs(&mut body);
        assert_eq!(stats.reused_blocks, 1);
        assert!(stats.bytes_saved > 0);
        assert_eq!(body["input"][0]["output"], large_output());
        assert!(body["input"][1]["output"]
            .as_str()
            .unwrap()
            .contains("tool output #1"));
    }

    #[test]
    fn leaves_distinct_and_small_tool_outputs_intact() {
        let mut body = json!({
            "messages": [
                {"role": "tool", "content": "small"},
                {"role": "tool", "content": large_output()},
                {"role": "tool", "content": "different output\n".repeat(120)}
            ]
        });
        let original = body.clone();

        let stats = reuse_exact_tool_outputs(&mut body);
        assert_eq!(stats.reused_blocks, 0);
        assert_eq!(body, original);
    }

    #[test]
    fn supports_anthropic_tool_result_blocks() {
        let mut body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "a", "content": large_output()},
                    {"type": "tool_result", "tool_use_id": "b", "content": large_output()}
                ]
            }]
        });

        let stats = reuse_exact_tool_outputs(&mut body);
        assert_eq!(stats.reused_blocks, 1);
        assert!(body["messages"][0]["content"][1]["content"]
            .as_str()
            .unwrap()
            .contains("tool output #1"));
    }

    #[test]
    fn refuses_duplicate_errors_secrets_warnings_and_source() {
        let unsafe_outputs = [
            "ERROR: compiler failed\n".repeat(120),
            "warning: deprecated dependency\n".repeat(120),
            "Authorization: Bearer secret-value\n".repeat(120),
            "fn main() {}\npub fn helper() {}\n".repeat(120),
        ];

        for output in unsafe_outputs {
            let mut body = json!({
                "input": [
                    {"type": "function_call_output", "call_id": "first", "output": output},
                    {"type": "function_call_output", "call_id": "second", "output": output}
                ]
            });
            let original = body.clone();
            let stats = reuse_exact_tool_outputs(&mut body);
            assert_eq!(stats.reused_blocks, 0, "unsafe output must remain verbatim");
            assert_eq!(body, original);
        }
    }
}
