//! Request-side compression of tool output.
//!
//! Invariants:
//! - Only tool results are touched. User prompts, system prompts, and assistant turns are
//!   never modified.
//! - The transform of each message depends only on that message, the messages before it, and
//!   whether it is still fresh (after the last assistant turn), never on proxy state. Agents
//!   resend the whole history every turn, so each tool result changes at most once (when it
//!   stops being fresh) and is byte-identical from then on: provider prompt caches stay valid.
//! - Lossy elision happens only when the model has a retrieval route (`retrieve_tool`).
//!   Command output (logs, JSON) is elided at first sight, keeping errors and summaries.
//!   File reads and searches reach the model verbatim on the turn they are read; once the
//!   model has had that turn, large code reads shrink to an outline (signatures, types, line
//!   numbers) with a handle to the full original. Searches and git output are never elided.
//! - Exact-duplicate removal is lossless (the first copy stays in the conversation) and always
//!   applies.
//!
//! Modules: [`calls`] attributes results to tool calls, [`classify`] decides what kind of
//! output a call produces, [`policy`] decides what happens to each output.

mod calls;
mod classify;
mod policy;

pub use policy::{ccr_size_threshold, log_line_threshold, shorten_command_output};

use serde_json::Value;

use crate::ccr::CcrStore;
use crate::pipeline::CompressionStats;
use calls::ToolCallIndex;
#[cfg(test)]
use policy::OUTLINE_MIN_BYTES;
use policy::{OutputPolicy, ResultContext, SURVEY_MIN_READS};

/// Options for [`compress_messages`].
pub struct CompressOptions<'a> {
    /// Name of the tool the model can call to recover elided originals.
    /// `None` restricts compression to lossless transforms.
    pub retrieve_tool: Option<&'a str>,
    /// Note embedded in collapsed JSON arrays.
    pub elision_note: &'a str,
}

/// Walks the conversation in order and compresses tool results (see module docs).
pub fn compress_messages(
    body: &mut Value,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
    opts: &CompressOptions,
) {
    let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return;
    };

    let mut calls = ToolCallIndex::default();
    let mut policy = OutputPolicy::new(ccr_store, opts);
    // Tool results after the last assistant turn have not been seen by the model yet.
    let last_assistant = messages
        .iter()
        .rposition(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"));

    for (idx, msg) in messages.iter_mut().enumerate() {
        let fresh = last_assistant.is_none_or(|last| idx > last);
        match msg.get("role").and_then(|r| r.as_str()).unwrap_or("") {
            "assistant" => calls.record(msg),
            // OpenAI tool result message
            "tool" => {
                let id = msg.get("tool_call_id").and_then(|id| id.as_str());
                let ctx = ResultContext {
                    call: calls.get(id),
                    fresh,
                    survey: calls.read_batch_size(id) >= SURVEY_MIN_READS,
                };
                if let Some(content) = msg.get_mut("content") {
                    compress_content(content, &ctx, &mut policy, stats);
                }
            }
            // Anthropic tool_result blocks inside user messages
            _ => {
                let Some(blocks) = msg.get_mut("content").and_then(|c| c.as_array_mut()) else {
                    continue;
                };
                for block in blocks.iter_mut() {
                    if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                        continue;
                    }
                    let id = block.get("tool_use_id").and_then(|id| id.as_str());
                    let ctx = ResultContext {
                        call: calls.get(id),
                        fresh,
                        survey: calls.read_batch_size(id) >= SURVEY_MIN_READS,
                    };
                    if let Some(content) = block.get_mut("content") {
                        compress_content(content, &ctx, &mut policy, stats);
                    }
                }
            }
        }
    }
}

/// Applies the policy to a tool result's content: a string, or an array of content blocks
/// (only text blocks are touched).
fn compress_content(
    content: &mut Value,
    ctx: &ResultContext,
    policy: &mut OutputPolicy,
    stats: &mut CompressionStats,
) {
    let mut apply = |text: &mut String| {
        if let Some(new) = policy.apply(text, ctx, stats) {
            *text = new;
        }
    };
    match content {
        Value::String(text) => apply(text),
        Value::Array(blocks) => {
            for block in blocks.iter_mut() {
                if block
                    .get("type")
                    .and_then(|t| t.as_str())
                    .is_some_and(|t| t != "text")
                {
                    continue; // images, documents, etc.
                }
                if let Some(Value::String(text)) = block.get_mut("text") {
                    apply(text);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(body: &mut Value, store: &CcrStore, retrieve_tool: Option<&str>) -> CompressionStats {
        let mut stats = CompressionStats::default();
        let opts = CompressOptions {
            retrieve_tool,
            elision_note: "n",
        };
        compress_messages(body, store, &mut stats, &opts);
        stats
    }

    #[test]
    fn lossless_mode_dedups_but_never_elides() {
        let store = CcrStore::open_in_memory().unwrap();
        let file = "fn main() {}\n".repeat(100);
        let log: String = (0..300)
            .map(|i| format!("Compiling dep_{i} v0.1.0\n"))
            .collect();
        let mut body = json!({"messages": [
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "a.rs"}},
                {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "cargo build"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": file.clone()},
                {"type": "tool_result", "tool_use_id": "t2", "content": log.clone()}
            ]},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t3", "name": "Read", "input": {"file_path": "a.rs"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t3", "content": file.clone()}
            ]}
        ]});
        let stats = run(&mut body, &store, None);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(
            msgs[1]["content"][0]["content"], file,
            "first read verbatim"
        );
        assert_eq!(
            msgs[1]["content"][1]["content"], log,
            "no elision without recovery"
        );
        let dup = msgs[3]["content"][0]["content"].as_str().unwrap();
        assert!(
            dup.contains("identical to the earlier `Read(a.rs)` result"),
            "{dup}"
        );
        assert!(
            !dup.contains("ccr:"),
            "no handle when nothing can retrieve it"
        );
        assert_eq!(stats.duplicates_removed, 1);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn recovery_mode_elides_command_logs_but_not_file_reads() {
        let store = CcrStore::open_in_memory().unwrap();
        let file: String = (0..200).map(|i| format!("let x{i} = {i};\n")).collect();
        let mut log: String = (0..300)
            .map(|i| format!("   Compiling dep_{i} v0.1.0\n"))
            .collect();
        log.push_str("error[E0425]: cannot find value `y` in this scope\n");
        log.push_str("test result: FAILED. 7 passed; 1 failed\n");
        let mut body = json!({"messages": [
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "a.rs"}},
                {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "cargo test"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": file.clone()},
                {"type": "tool_result", "tool_use_id": "t2", "content": log.clone()}
            ]}
        ]});
        run(&mut body, &store, Some("mcp__retoken__retoken_retrieve"));

        let blocks = &body["messages"][1]["content"];
        assert_eq!(blocks[0]["content"], file, "file reads are never elided");
        let out = blocks[1]["content"].as_str().unwrap();
        assert!(out.len() < log.len() / 4);
        assert!(out.contains("error[E0425]"), "errors kept");
        assert!(out.contains("test result: FAILED"), "summary kept");
        assert!(
            out.contains("mcp__retoken__retoken_retrieve"),
            "marker names the real tool"
        );
        let handle = CcrStore::handle(log.as_bytes());
        assert!(out.contains(&handle));
        assert_eq!(store.retrieve(&handle).unwrap().unwrap(), log.as_bytes());
    }

    #[test]
    fn openai_tool_calls_are_matched_to_results() {
        let store = CcrStore::open_in_memory().unwrap();
        let log: String = (0..300).map(|i| format!("step {i} ok\n")).collect();
        let mut body = json!({"messages": [
            {"role": "assistant", "tool_calls": [
                {"id": "c1", "type": "function",
                 "function": {"name": "shell", "arguments": "{\"command\":\"make\"}"}}
            ]},
            {"role": "tool", "tool_call_id": "c1", "content": log.clone()}
        ]});
        let stats = run(&mut body, &store, Some("retoken_retrieve"));
        assert!(body["messages"][1]["content"].as_str().unwrap().len() < log.len() / 4);
        assert_eq!(stats.log_blocks_compressed, 1);
    }

    #[test]
    fn compressed_history_is_stable_across_turns() {
        // Turn N+1 resends turn N's history plus new messages; the shared prefix must
        // compress to identical bytes so provider prompt caches keep hitting.
        let store = CcrStore::open_in_memory().unwrap();
        let log: String = (0..300).map(|i| format!("step {i} ok\n")).collect();
        let turn = |n: usize| -> Value {
            let mut msgs = vec![];
            for i in 0..n {
                msgs.push(json!({"role": "assistant", "content": [
                    {"type": "tool_use", "id": format!("t{i}"), "name": "Bash",
                     "input": {"command": "make"}}]}));
                msgs.push(json!({"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": format!("t{i}"), "content": log.clone()}]}));
            }
            json!({ "messages": msgs })
        };
        let mut t1 = turn(1);
        let mut t2 = turn(2);
        run(&mut t1, &store, Some("retoken_retrieve"));
        run(&mut t2, &store, Some("retoken_retrieve"));
        assert_eq!(
            t1["messages"][1], t2["messages"][1],
            "history prefix must not change"
        );
        assert!(t2["messages"][3]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("identical to the earlier `Bash(make)` result"));
    }

    /// A Rust source file large enough to outline, in Read-tool `cat -n` format.
    fn source_file() -> String {
        let mut src = String::new();
        for f in 0..20 {
            src.push_str(&format!("pub fn func_{f}(x: u32) -> u32 {{\n"));
            for l in 0..8 {
                src.push_str(&format!(
                    "    let v{l} = x + {l}; // step {l} of func_{f}\n"
                ));
            }
            src.push_str("    x\n}\n\n");
        }
        src.lines()
            .enumerate()
            .map(|(i, l)| format!("{:>6}\t{}\n", i + 1, l))
            .collect()
    }

    /// History of `n` turns, each reading the same file path (contents may differ per turn).
    fn read_turns(files: &[String]) -> Value {
        let mut msgs = vec![json!({"role": "user", "content": "refactor"})];
        for (i, file) in files.iter().enumerate() {
            msgs.push(json!({"role": "assistant", "content": [
                {"type": "tool_use", "id": format!("r{i}"), "name": "Read",
                 "input": {"file_path": "src/lib.rs"}}]}));
            msgs.push(json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": format!("r{i}"), "content": file}]}));
        }
        json!({ "messages": msgs })
    }

    #[test]
    fn file_reads_are_verbatim_when_fresh_and_outlined_once_stale() {
        let store = CcrStore::open_in_memory().unwrap();
        let file = source_file();
        let edited = file.replace("step 3 of func_7", "step 3 (edited) of func_7");
        let tool = Some("mcp__retoken__retoken_retrieve");

        // Turn 1: the read is fresh, so the model gets the whole file.
        let mut t1 = read_turns(std::slice::from_ref(&file));
        run(&mut t1, &store, tool);
        assert_eq!(t1["messages"][2]["content"][0]["content"], file);

        // Turn 2: a new read arrived; the first one is stale and becomes an outline.
        let mut t2 = read_turns(&[file.clone(), edited.clone()]);
        let stats = run(&mut t2, &store, tool);
        let outlined = t2["messages"][2]["content"][0]["content"].as_str().unwrap();
        assert!(
            outlined.len() < file.len() / 2,
            "{} vs {}",
            outlined.len(),
            file.len()
        );
        assert!(
            outlined.contains("pub fn func_7(x: u32) -> u32 {"),
            "signatures kept"
        );
        assert!(!outlined.contains("step 3 of func_7"), "bodies elided");
        assert!(outlined.contains("body elided (lines"), "line ranges named");
        assert!(outlined.contains(&CcrStore::handle(file.as_bytes())));
        assert_eq!(
            t2["messages"][4]["content"][0]["content"], edited,
            "fresh read verbatim"
        );
        assert_eq!(stats.code_outlines, 1);

        // Turn 3: history from turn 2 is unchanged except the newly stale read.
        let mut t3 = read_turns(&[file.clone(), edited.clone(), file.clone()]);
        run(&mut t3, &store, tool);
        assert_eq!(
            t3["messages"][2], t2["messages"][2],
            "stale outline is stable"
        );
        // Fresh re-read of content whose first copy was outlined: shown in full, not deduped.
        assert_eq!(t3["messages"][6]["content"][0]["content"], file);
    }

    #[test]
    fn stale_ranged_and_small_reads_are_never_outlined() {
        // Benchmark trace: outlining a ranged 3 KB read of store.rs made the agent re-read
        // lines 60 and 98 it already had, costing two extra turns.
        let store = CcrStore::open_in_memory().unwrap();
        let tool = Some("mcp__retoken__retoken_retrieve");
        let file = source_file();
        assert!(file.len() >= OUTLINE_MIN_BYTES, "fixture is outline-sized");

        let mut ranged = read_turns(&[file.clone(), file.replace("func_", "g_")]);
        ranged["messages"][1]["content"][0]["input"] =
            json!({"file_path": "src/lib.rs", "offset": 40, "limit": 300});
        let stats = run(&mut ranged, &store, tool);
        assert_eq!(ranged["messages"][2]["content"][0]["content"], file);
        assert_eq!(stats.code_outlines, 0);

        let small: String = file.chars().take(OUTLINE_MIN_BYTES - 1).collect();
        let mut t = read_turns(&[small.clone(), file.replace("func_", "h_")]);
        let stats = run(&mut t, &store, tool);
        assert_eq!(t["messages"][2]["content"][0]["content"], small);
        assert_eq!(stats.code_outlines, 0);
    }

    /// One assistant turn reading `n` different files, all results fresh.
    fn parallel_reads(n: usize, ranged: bool) -> (Value, Vec<String>) {
        let files: Vec<String> = (0..n)
            .map(|i| source_file().replace("func_", &format!("f{i}_")))
            .collect();
        let input = |i: usize| {
            if ranged {
                json!({"file_path": format!("src/m{i}.rs"), "offset": 1, "limit": 400})
            } else {
                json!({"file_path": format!("src/m{i}.rs")})
            }
        };
        let uses: Vec<Value> = (0..n)
            .map(|i| json!({"type": "tool_use", "id": format!("p{i}"), "name": "Read", "input": input(i)}))
            .collect();
        let results: Vec<Value> = (0..n)
            .map(|i| json!({"type": "tool_result", "tool_use_id": format!("p{i}"), "content": files[i]}))
            .collect();
        let body = json!({"messages": [
            {"role": "user", "content": "summarize each module"},
            {"role": "assistant", "content": uses},
            {"role": "user", "content": results}
        ]});
        (body, files)
    }

    #[test]
    fn surveyed_reads_are_outlined_on_arrival_focused_reads_are_not() {
        let store = CcrStore::open_in_memory().unwrap();
        let tool = Some("retoken_retrieve");

        let (mut survey, files) = parallel_reads(3, false);
        let stats = run(&mut survey, &store, tool);
        for (i, file) in files.iter().enumerate() {
            let sent = survey["messages"][2]["content"][i]["content"]
                .as_str()
                .unwrap();
            assert!(sent.len() < file.len() / 2, "survey read {i} outlined");
            assert!(sent.contains(&format!("pub fn f{i}_7(x: u32) -> u32 {{")));
        }
        assert_eq!(stats.code_outlines, 3);
        assert!(
            !stats.volatile_tail,
            "outlined now, nothing is rewritten next turn"
        );

        // Two reads in a turn: focused, shown in full, flagged as rewritten next turn.
        let (mut focused, files) = parallel_reads(2, false);
        let stats = run(&mut focused, &store, tool);
        assert_eq!(focused["messages"][2]["content"][0]["content"], files[0]);
        assert!(stats.volatile_tail);

        // Ranged reads are deliberate even in a survey.
        let (mut ranged, files) = parallel_reads(3, true);
        run(&mut ranged, &store, tool);
        assert_eq!(ranged["messages"][2]["content"][0]["content"], files[0]);
    }

    #[test]
    fn pretty_json_from_commands_is_minified_losslessly() {
        let store = CcrStore::open_in_memory().unwrap();
        let items: Vec<Value> = (0..40)
            .map(|i| json!({"zeta": i, "name": format!("item-{i}"), "tags": ["a", "b"]}))
            .collect();
        let value = json!({"zeta": [1, 2, {"big": 123456789012345678901234567890u128.to_string()}],
                           "alpha": items});
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        let mut body = json!({"messages": [
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "j1", "name": "Bash", "input": {"command": "gh api repos/x"}}]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "j1", "content": pretty.clone()}]}
        ]});
        let stats = run(&mut body, &store, None);
        let sent = body["messages"][1]["content"][0]["content"]
            .as_str()
            .unwrap();
        assert!(sent.len() < pretty.len());
        assert_eq!(
            serde_json::from_str::<Value>(sent).unwrap(),
            value,
            "lossless"
        );
        assert!(
            sent.find("zeta").unwrap() < sent.find("alpha").unwrap(),
            "key order kept"
        );
        assert_eq!(stats.json_minified, 1);
    }

    #[test]
    fn stale_reads_stay_verbatim_without_a_retrieval_route() {
        let store = CcrStore::open_in_memory().unwrap();
        let file = source_file();
        let edited = file.replace("func_7", "func_seven");
        let mut body = read_turns(&[file.clone(), edited]);
        run(&mut body, &store, None);
        assert_eq!(body["messages"][2]["content"][0]["content"], file);
    }
}
