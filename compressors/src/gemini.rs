//! Gemini API (`generateContent`) support for the compression pipeline.
//!
//! Gemini requests carry the conversation in `contents`: `model` turns hold `functionCall`
//! parts and the following `user` turn holds the matching `functionResponse` parts. Rather than
//! duplicate the tool-output policy, this module presents the conversation to
//! [`compress_messages`] in the Anthropic message shape, then writes changed tool-result text
//! back into the Gemini body. The policy (dedup, log elision, outlines, freshness) is therefore
//! identical across wire formats.
//!
//! Only a `functionResponse.response` whose `output`, `result` or `content` field is a string
//! is ever changed; other response shapes are forwarded untouched.

use serde_json::{json, Value};

use crate::ccr::CcrStore;
use crate::pipeline::{CompressionStats, PROXY_RETRIEVE_TOOL};
use crate::tool_output::{compress_messages, CompressOptions};

/// `functionResponse.response` fields that hold a tool's text output, in preference order.
const TEXT_FIELDS: &[&str] = &["output", "result", "content"];

/// Where a tool result's text lives in the Gemini body.
struct ResultSlot {
    content_idx: usize,
    part_idx: usize,
    part_key: &'static str,
    field: &'static str,
}

/// Field lookup accepting the camelCase the SDKs send and the snake_case the REST API also
/// accepts.
fn field<'a>(
    value: &'a Value,
    camel: &'static str,
    snake: &'static str,
) -> Option<(&'static str, &'a Value)> {
    value
        .get(camel)
        .map(|v| (camel, v))
        .or_else(|| value.get(snake).map(|v| (snake, v)))
}

/// The client's retrieval tool (from `retoken mcp`), if declared: a function declaration whose
/// name ends with `retoken_retrieve`.
pub fn find_retrieve_tool(body: &Value) -> Option<String> {
    body.get("tools")?
        .as_array()?
        .iter()
        .filter_map(|tool| field(tool, "functionDeclarations", "function_declarations"))
        .filter_map(|(_, decls)| decls.as_array())
        .flatten()
        .filter_map(|decl| decl.get("name").and_then(|n| n.as_str()))
        .find(|name| name.ends_with(PROXY_RETRIEVE_TOOL))
        .map(str::to_string)
}

/// Compresses tool results in a Gemini request body with the shared tool-output policy.
pub fn compress_contents(
    body: &mut Value,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
    opts: &CompressOptions,
) {
    let Some((mut view, slots)) = build_view(body) else {
        return;
    };
    compress_messages(&mut view, ccr_store, stats, opts);
    write_back(body, &view, &slots);
}

/// Builds the Anthropic-shaped view of the conversation and the location of every tool result.
fn build_view(body: &Value) -> Option<(Value, Vec<ResultSlot>)> {
    let contents = body.get("contents")?.as_array()?;
    let mut messages = Vec::with_capacity(contents.len());
    let mut slots = Vec::new();
    // Calls of the latest model turn not yet answered, for responses that carry no `id`.
    let mut pending: Vec<(String, String)> = Vec::new();

    for (ci, content) in contents.iter().enumerate() {
        let parts = content.get("parts").and_then(|p| p.as_array());
        let parts = parts.map(Vec::as_slice).unwrap_or_default();
        if content.get("role").and_then(|r| r.as_str()) == Some("model") {
            pending.clear();
            let blocks: Vec<Value> = parts
                .iter()
                .enumerate()
                .filter_map(|(pi, part)| {
                    let (_, call) = field(part, "functionCall", "function_call")?;
                    let name = call.get("name")?.as_str()?.to_string();
                    let id = call
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("gemini-call-{ci}-{pi}"));
                    pending.push((name.clone(), id.clone()));
                    let input = call.get("args").cloned().unwrap_or(Value::Null);
                    Some(json!({"type": "tool_use", "id": id, "name": name, "input": input}))
                })
                .collect();
            messages.push(json!({"role": "assistant", "content": blocks}));
            continue;
        }

        let mut blocks = Vec::new();
        for (pi, part) in parts.iter().enumerate() {
            let Some((part_key, response)) = field(part, "functionResponse", "function_response")
            else {
                continue;
            };
            let Some((field_name, text)) = TEXT_FIELDS.iter().find_map(|f| {
                response
                    .get("response")
                    .and_then(|r| r.get(*f))
                    .and_then(|t| t.as_str())
                    .map(|t| (*f, t))
            }) else {
                continue;
            };
            let id = match response.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => {
                    let name = response.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    match pending.iter().position(|(n, _)| n == name) {
                        Some(i) => pending.remove(i).1,
                        None => format!("gemini-result-{ci}-{pi}"),
                    }
                }
            };
            blocks.push(json!({"type": "tool_result", "tool_use_id": id, "content": text}));
            slots.push(ResultSlot {
                content_idx: ci,
                part_idx: pi,
                part_key,
                field: field_name,
            });
        }
        messages.push(json!({"role": "user", "content": blocks}));
    }
    Some((json!({ "messages": messages }), slots))
}

/// Copies every tool-result text the policy changed back into the Gemini body.
fn write_back(body: &mut Value, view: &Value, slots: &[ResultSlot]) {
    let results = view["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("content").and_then(|c| c.as_array()))
        .flatten()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
    for (slot, block) in slots.iter().zip(results) {
        let Some(new_text) = block.get("content").and_then(|c| c.as_str()) else {
            continue;
        };
        let target = body.pointer_mut(&format!(
            "/contents/{}/parts/{}/{}/response/{}",
            slot.content_idx, slot.part_idx, slot.part_key, slot.field
        ));
        if let Some(target) = target {
            if target.as_str() != Some(new_text) {
                *target = Value::String(new_text.to_string());
            }
        }
    }
}

/// Appends `persona` to the system instruction (creating one if absent), unless present.
pub fn inject_persona(body: &mut Value, persona: &str) {
    if persona.trim().is_empty() {
        return;
    }
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    let key = if obj.contains_key("system_instruction") {
        "system_instruction"
    } else {
        "systemInstruction"
    };
    let instruction = obj.entry(key).or_insert_with(|| json!({"parts": []}));
    if !instruction.is_object() {
        *instruction = json!({"parts": []});
    }
    let parts = instruction
        .as_object_mut()
        .map(|o| o.entry("parts").or_insert_with(|| json!([])));
    let Some(Value::Array(parts)) = parts else {
        return;
    };
    let already = parts.iter().any(|p| {
        p.get("text")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.contains(persona))
    });
    if !already {
        parts.push(json!({ "text": persona }));
    }
}

/// Gemini counterpart of [`crate::pipeline::run_pipeline_for_provider`]: compresses tool
/// results (lossy only when `retrieve_tool` is given) and applies the terse persona.
pub fn run_pipeline(
    body: &mut Value,
    ccr_store: &CcrStore,
    persona_prompt: &str,
    elision_note: &str,
    retrieve_tool: Option<&str>,
) -> CompressionStats {
    let original_size = serde_json::to_vec(body).map(|b| b.len()).unwrap_or(0);
    let mut stats = CompressionStats {
        original_size,
        ..Default::default()
    };
    let opts = CompressOptions {
        retrieve_tool,
        elision_note,
    };
    compress_contents(body, ccr_store, &mut stats, &opts);
    inject_persona(body, persona_prompt);
    stats.compressed_size = serde_json::to_vec(body).map(|b| b.len()).unwrap_or(0);
    stats.bytes_saved = original_size.saturating_sub(stats.compressed_size);
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_log() -> String {
        (0..300)
            .map(|i| {
                if i == 150 {
                    "error[E0425]: cannot find value `x` in this scope\n".to_string()
                } else {
                    format!("   Compiling dep_{i} v0.1.0\n")
                }
            })
            .collect()
    }

    fn shell_turns(log: &str, with_ids: bool) -> Value {
        let call = if with_ids {
            json!({"functionCall": {"id": "c1", "name": "run_shell_command", "args": {"command": "cargo build"}}})
        } else {
            json!({"functionCall": {"name": "run_shell_command", "args": {"command": "cargo build"}}})
        };
        let mut response = json!({"name": "run_shell_command", "response": {"output": log}});
        if with_ids {
            response["id"] = json!("c1");
        }
        json!({
            "systemInstruction": {"parts": [{"text": "You are a coding agent."}]},
            "tools": [{"functionDeclarations": [
                {"name": "run_shell_command", "parameters": {"type": "object"}},
                {"name": "retoken_retrieve", "parameters": {"type": "object"}}
            ]}],
            "contents": [
                {"role": "user", "parts": [{"text": "Build the project"}]},
                {"role": "model", "parts": [call]},
                {"role": "user", "parts": [{"functionResponse": response}]}
            ]
        })
    }

    fn output_of(body: &Value) -> &str {
        body["contents"][2]["parts"][0]["functionResponse"]["response"]["output"]
            .as_str()
            .unwrap()
    }

    #[test]
    fn finds_declared_retrieve_tool() {
        let body = shell_turns("x", true);
        assert_eq!(
            find_retrieve_tool(&body).as_deref(),
            Some("retoken_retrieve")
        );
        assert_eq!(find_retrieve_tool(&json!({"contents": []})), None);
    }

    #[test]
    fn elides_shell_logs_with_or_without_call_ids() {
        for with_ids in [true, false] {
            let store = CcrStore::open_in_memory().unwrap();
            let log = build_log();
            let mut body = shell_turns(&log, with_ids);
            let stats = run_pipeline(&mut body, &store, "", "n", Some("retoken_retrieve"));
            let out = output_of(&body);
            assert!(out.len() < log.len() / 2, "with_ids={with_ids}");
            assert!(out.contains("error[E0425]"), "errors are kept");
            assert!(out.contains("call `retoken_retrieve` with handle"));
            assert_eq!(stats.log_blocks_compressed, 1);
            assert_eq!(store.count(), 1);
            assert_eq!(body["contents"][0]["parts"][0]["text"], "Build the project");
        }
    }

    #[test]
    fn without_a_retrieve_tool_nothing_lossy_happens() {
        let store = CcrStore::open_in_memory().unwrap();
        let log = build_log();
        let mut body = shell_turns(&log, true);
        let before = body.clone();
        run_pipeline(&mut body, &store, "", "n", None);
        assert_eq!(body, before);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn unknown_response_shapes_are_untouched() {
        let store = CcrStore::open_in_memory().unwrap();
        let log = build_log();
        let mut body = shell_turns(&log, true);
        body["contents"][2]["parts"][0]["functionResponse"]["response"] = json!({"lines": [log]});
        let before = body.clone();
        run_pipeline(&mut body, &store, "", "n", Some("retoken_retrieve"));
        assert_eq!(body, before);
    }

    #[test]
    fn snake_case_fields_are_supported() {
        let store = CcrStore::open_in_memory().unwrap();
        let log = build_log();
        let mut body = json!({
            "tools": [{"function_declarations": [{"name": "retoken_retrieve"}]}],
            "contents": [
                {"role": "model", "parts": [{"function_call": {"name": "run_shell_command", "args": {"command": "make"}}}]},
                {"role": "user", "parts": [{"function_response": {"name": "run_shell_command", "response": {"output": log}}}]}
            ]
        });
        let tool = find_retrieve_tool(&body);
        run_pipeline(&mut body, &store, "", "n", tool.as_deref());
        let out = body["contents"][1]["parts"][0]["function_response"]["response"]["output"]
            .as_str()
            .unwrap();
        assert!(out.contains("lines elided (retoken)"));
    }

    #[test]
    fn persona_is_appended_once() {
        let mut body = shell_turns("x", true);
        inject_persona(&mut body, "Be terse.");
        inject_persona(&mut body, "Be terse.");
        let parts = body["systemInstruction"]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["text"], "Be terse.");

        let mut bare = json!({"contents": []});
        inject_persona(&mut bare, "Be terse.");
        assert_eq!(bare["systemInstruction"]["parts"][0]["text"], "Be terse.");
    }
}
