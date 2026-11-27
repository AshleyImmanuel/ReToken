//! Decides what happens to one tool output: kept verbatim, replaced by a duplicate marker
//! (lossless), or elided with a recovery marker (lossy, needs a retrieval tool).

use serde_json::Value;
use std::collections::HashMap;
use tracing::{debug, warn};

use super::calls::ToolCall;
use super::classify::{is_command, is_file_content_read, is_file_read, is_ranged_read};
use super::CompressOptions;
use crate::ccr::{ccr_marker, CcrStore};
use crate::code_outline;
use crate::json::compress_json_heuristic_tracked;
use crate::logs::compress_logs;
use crate::pipeline::CompressionStats;

/// A turn that reads at least this many files at once is surveying: it needs the shape of
/// each file, so large reads are outlined on arrival (focused reads stay verbatim).
pub const SURVEY_MIN_READS: usize = 3;

/// Where a tool result sits in the conversation.
pub struct ResultContext<'c> {
    /// The call that produced it, if known.
    pub call: Option<&'c ToolCall>,
    /// True while the model has not yet had a turn with this output.
    pub fresh: bool,
    /// True if its call was one of [`SURVEY_MIN_READS`]+ file reads in the same turn.
    pub survey: bool,
}

/// File reads smaller than this are never outlined (about 2k tokens): below it, the chance
/// that the agent re-reads the file costs more than keeping it verbatim in a cached prefix.
pub(crate) const OUTLINE_MIN_BYTES: usize = 8 * 1024;

/// Tool output smaller than this is never deduplicated (the marker would not pay off).
const DEDUP_MIN_BYTES: usize = 512;

/// Lossy elision must remove at least this fraction of the original to be worth the
/// information loss and a possible retrieval round trip.
const MIN_ELISION_SAVINGS: f64 = 0.25;

/// True if `compressed` removes at least [`MIN_ELISION_SAVINGS`] of `original`.
fn saves_enough(compressed: &str, original: &str) -> bool {
    (compressed.len() as f64) <= (original.len() as f64) * (1.0 - MIN_ELISION_SAVINGS)
}

/// Lossless: pretty-printed JSON from commands or MCP tools, re-serialized compactly (key
/// order and numbers preserved). Never for file reads, whose exact text an edit may need.
fn minify_json(
    text: &str,
    call: Option<&ToolCall>,
    stats: &mut CompressionStats,
) -> Option<String> {
    if call.is_some_and(is_file_read) || !text.contains("\n ") {
        return None;
    }
    let value = serde_json::from_str::<Value>(text).ok()?;
    if !(value.is_array() || value.is_object()) {
        return None;
    }
    let compact = serde_json::to_string(&value).ok()?;
    if compact.len() as f64 > text.len() as f64 * 0.9 {
        return None;
    }
    stats.json_minified += 1;
    Some(compact)
}

/// Threshold in bytes for content to be eligible for lossy elision.
pub fn ccr_size_threshold() -> usize {
    std::env::var("RETOKEN_CCR_SIZE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048)
}

/// Minimum line count for command output to be treated as a log.
pub fn log_line_threshold() -> usize {
    std::env::var("RETOKEN_LOG_LINE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15)
}

/// Shortens one command's output the way the proxy shortens command output in a conversation,
/// for callers outside the proxy (`retoken exec`). Uses the same size and line thresholds and
/// the same minimum savings. The original is stored first; `None` means print `text` as is
/// (too small, not worth eliding, or the store failed).
pub fn shorten_command_output(text: &str, store: &CcrStore, retrieve_tool: &str) -> Option<String> {
    if text.len() < ccr_size_threshold() || text.lines().count() < log_line_threshold() {
        return None;
    }
    let compressed = compress_logs(text);
    if !saves_enough(&compressed, text) {
        return None;
    }
    match store.store(text.as_bytes(), "tool_output") {
        Ok(handle) => Some(format!(
            "{}\n{}",
            compressed,
            ccr_marker(&handle, retrieve_tool)
        )),
        Err(e) => {
            warn!("CCR store failed, keeping output verbatim: {}", e);
            None
        }
    }
}

/// The first occurrence of a tool output in the conversation.
struct FirstCopy {
    /// Description of the call that produced it, e.g. `Read(src/lib.rs)`.
    call: Option<String>,
    /// False if that copy was elided or outlined, so it no longer shows the full content.
    verbatim: bool,
}

/// Per-conversation policy state: tool outputs already seen earlier in the request.
pub struct OutputPolicy<'a> {
    store: &'a CcrStore,
    opts: &'a CompressOptions<'a>,
    seen: HashMap<String, FirstCopy>,
}

impl<'a> OutputPolicy<'a> {
    pub fn new(store: &'a CcrStore, opts: &'a CompressOptions<'a>) -> Self {
        Self {
            store,
            opts,
            seen: HashMap::new(),
        }
    }

    /// Returns the replacement for one tool output text, or `None` to keep it verbatim.
    pub fn apply(
        &mut self,
        text: &str,
        ctx: &ResultContext,
        stats: &mut CompressionStats,
    ) -> Option<String> {
        let call = ctx.call;
        if text.len() < DEDUP_MIN_BYTES {
            return None;
        }
        let handle = CcrStore::handle(text.as_bytes());
        if let Some(first) = self.seen.get(&handle) {
            // A fresh re-read of a file whose earlier copy was outlined must show the content.
            if ctx.fresh && !first.verbatim && call.is_some_and(is_file_content_read) {
                return None;
            }
            let first_call = first.call.clone();
            return Some(self.duplicate_marker(text, &handle, first_call.as_deref(), stats));
        }
        let replacement = self
            .elide(text, ctx, stats)
            .or_else(|| minify_json(text, call, stats));
        self.seen.insert(
            handle,
            FirstCopy {
                call: call.map(ToolCall::describe),
                verbatim: replacement.is_none(),
            },
        );
        replacement
    }

    /// Lossless: the first copy is still in the conversation. The marker names the call that
    /// produced it, so the model can find it in a long history.
    fn duplicate_marker(
        &self,
        text: &str,
        handle: &str,
        first_call: Option<&str>,
        stats: &mut CompressionStats,
    ) -> String {
        stats.duplicates_removed += 1;
        let note = match first_call {
            Some(call) => format!(
                "[ReToken: identical to the earlier `{call}` result in this conversation; not repeated."
            ),
            None => "[ReToken: identical to an earlier tool result in this conversation; not repeated."
                .to_string(),
        };
        let recoverable = self
            .opts
            .retrieve_tool
            .filter(|_| self.store_original(text, "duplicate", stats).is_some());
        match recoverable {
            Some(tool) => format!("{} {}]", note, ccr_marker(handle, tool)),
            None => format!("{}]", note),
        }
    }

    /// Lossy, only with a retrieval route: large JSON or command output at first sight; large
    /// file reads once stale, or on arrival when surveyed (outlined). Searches and git output
    /// are never elided.
    fn elide(
        &self,
        text: &str,
        ctx: &ResultContext,
        stats: &mut CompressionStats,
    ) -> Option<String> {
        let call = ctx.call;
        let tool = self.opts.retrieve_tool?;
        if text.len() < ccr_size_threshold() {
            return None;
        }
        if call.is_some_and(is_file_read) {
            // A ranged read is the slice the agent chose; a small file costs little to keep.
            // Outlining either makes the agent re-read lines it already had, and one extra
            // turn re-sends the whole cached prefix: more than the outline ever saves.
            let whole_read = call.is_some_and(|c| is_file_content_read(c) && !is_ranged_read(c));
            if !whole_read || text.len() < OUTLINE_MIN_BYTES {
                return None;
            }
            let outline = code_outline::outline(text).filter(|o| saves_enough(o, text))?;
            let outline_now = !ctx.fresh || ctx.survey;
            if !outline_now {
                // Shown in full now, outlined next turn: not worth caching (see `volatile_tail`).
                stats.volatile_tail = true;
                return None;
            }
            stats.code_outlines += 1;
            let handle = self.store_original(text, "file", stats)?;
            return Some(format!(
                "{}\n{} Or re-read the file; line numbers above are exact.",
                outline,
                ccr_marker(&handle, tool)
            ));
        }

        let compressed = if let Ok(json) = serde_json::from_str::<Value>(text) {
            if !(json.is_array() || json.is_object()) {
                return None;
            }
            let (collapsed, did_compress) =
                compress_json_heuristic_tracked(&json, self.opts.elision_note);
            if !did_compress {
                return None;
            }
            stats.json_arrays_collapsed += 1;
            serde_json::to_string(&collapsed).ok()?
        } else if call.is_some_and(is_command) && text.lines().count() >= log_line_threshold() {
            stats.log_blocks_compressed += 1;
            compress_logs(text)
        } else {
            return None;
        };

        if !saves_enough(&compressed, text) {
            return None;
        }
        let handle = self.store_original(text, "tool_output", stats)?;
        Some(format!("{}\n{}", compressed, ccr_marker(&handle, tool)))
    }

    /// Stores the original before anything referencing it is forwarded. On failure the
    /// caller keeps the content verbatim.
    fn store_original(
        &self,
        text: &str,
        content_type: &str,
        stats: &mut CompressionStats,
    ) -> Option<String> {
        match self.store.store(text.as_bytes(), content_type) {
            Ok(handle) => {
                stats.ccr_entries_stored += 1;
                debug!("CCR stored original: {}", handle);
                Some(handle)
            }
            Err(e) => {
                warn!("CCR store failed, keeping content verbatim: {}", e);
                None
            }
        }
    }
}
