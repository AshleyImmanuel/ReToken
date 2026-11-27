//! Token usage extraction from Server-Sent Event (SSE) streams.
//!
//! Streaming responses are forwarded chunk by chunk without buffering. This scanner observes the
//! same bytes and keeps only the current partial line, so memory stays bounded regardless of
//! stream length.

use crate::provider::{usage_from_object, Provider, TokenUsage};

/// Longest SSE line kept while waiting for its terminating newline. Usage events are small;
/// longer lines (large content deltas) are skipped rather than buffered.
const MAX_LINE_BYTES: usize = 1024 * 1024;

pub struct SseUsageScanner {
    provider: Provider,
    line: Vec<u8>,
    skipping_long_line: bool,
    usage: TokenUsage,
}

impl SseUsageScanner {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            line: Vec::new(),
            skipping_long_line: false,
            usage: TokenUsage::default(),
        }
    }

    /// Feeds the next chunk of the stream. Chunks may split lines at any byte.
    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while let Some(pos) = rest.iter().position(|&b| b == b'\n') {
            let (head, tail) = rest.split_at(pos);
            if !self.skipping_long_line {
                self.line.extend_from_slice(head);
                let line = std::mem::take(&mut self.line);
                self.process_line(&line);
            }
            self.skipping_long_line = false;
            rest = &tail[1..];
        }
        if !self.skipping_long_line {
            if self.line.len() + rest.len() > MAX_LINE_BYTES {
                self.line.clear();
                self.skipping_long_line = true;
            } else {
                self.line.extend_from_slice(rest);
            }
        }
    }

    /// Usage accumulated so far. Later events override earlier values per field, which matches
    /// providers that report cumulative counts (e.g. Anthropic `message_delta.usage.output_tokens`).
    pub fn usage(&self) -> &TokenUsage {
        &self.usage
    }

    fn process_line(&mut self, line: &[u8]) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(data) = line.strip_prefix(b"data:") else {
            return;
        };
        let Ok(event) = serde_json::from_slice::<serde_json::Value>(data.trim_ascii()) else {
            return; // e.g. "[DONE]" or non-JSON payloads
        };
        // Anthropic `message_delta` and OpenAI chat chunks carry `usage` at the top level;
        // Anthropic `message_start` nests it under `message`, OpenAI Responses under `response`,
        // Gemini (`alt=sse`) chunks carry `usageMetadata`.
        let usage_obj = event
            .get("usage")
            .or_else(|| event.get("usageMetadata"))
            .or_else(|| event.get("message").and_then(|m| m.get("usage")))
            .or_else(|| event.get("response").and_then(|r| r.get("usage")));
        if let Some(obj) = usage_obj.filter(|u| u.is_object()) {
            self.merge(usage_from_object(&self.provider, obj));
        }
    }

    fn merge(&mut self, update: TokenUsage) {
        let u = &mut self.usage;
        u.input_tokens = update.input_tokens.or(u.input_tokens);
        u.output_tokens = update.output_tokens.or(u.output_tokens);
        u.cached_tokens = update.cached_tokens.or(u.cached_tokens);
        u.reasoning_tokens = update.reasoning_tokens.or(u.reasoning_tokens);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_stream_usage_split_across_chunks() {
        let stream = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":420,\"cache_read_input_tokens\":300,\"output_tokens\":1}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
            "event: message_delta\r\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":57}}\r\n\r\n",
        );
        let mut scanner = SseUsageScanner::new(Provider::Anthropic);
        // Feed in 7-byte pieces so lines and JSON objects are split mid-way.
        for piece in stream.as_bytes().chunks(7) {
            scanner.feed(piece);
        }
        let usage = scanner.usage();
        assert_eq!(usage.input_tokens, Some(420));
        assert_eq!(usage.cached_tokens, Some(300));
        assert_eq!(usage.output_tokens, Some(57));
    }

    #[test]
    fn openai_stream_usage_from_final_chunk() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}],\"usage\":null}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":6,\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\n",
            "data: [DONE]\n\n",
        );
        let mut scanner = SseUsageScanner::new(Provider::OpenAI);
        scanner.feed(stream.as_bytes());
        let usage = scanner.usage();
        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(6));
        assert_eq!(usage.reasoning_tokens, Some(2));
    }

    #[test]
    fn oversized_line_is_skipped_without_buffering() {
        let mut scanner = SseUsageScanner::new(Provider::Anthropic);
        scanner.feed(b"data: ");
        scanner.feed(&vec![b'a'; MAX_LINE_BYTES + 10]);
        assert!(scanner.line.is_empty());
        scanner.feed(b"tail of long line\ndata: {\"usage\":{\"output_tokens\":3}}\n");
        assert_eq!(scanner.usage().output_tokens, Some(3));
    }

    // Ported from main's SseUsageExtractor tests (v1.1.0): same relay guarantees.
    fn scan(provider: Provider, chunks: &[&[u8]]) -> TokenUsage {
        let mut scanner = SseUsageScanner::new(provider);
        for chunk in chunks {
            scanner.feed(chunk);
        }
        scanner.usage().clone()
    }

    #[test]
    fn collects_openai_responses_usage_across_split_chunks() {
        let usage = scan(
            Provider::OpenAI,
            &[
                b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":120,\"output_tokens\":",
                b"45,\"input_tokens_details\":{\"cached_tokens\":30},\"output_tokens_details\":{\"reasoning_tokens\":11}}}}\n\n",
            ],
        );
        assert_eq!(
            usage,
            TokenUsage {
                input_tokens: Some(120),
                output_tokens: Some(45),
                cached_tokens: Some(30),
                reasoning_tokens: Some(11),
            }
        );
    }

    #[test]
    fn merges_anthropic_start_and_delta_usage_with_crlf() {
        let usage = scan(
            Provider::Anthropic,
            &[
                b"event: message_start\r\ndata: {\"usage\":{\"input_tokens\":300,\"cache_read_input_tokens\":40}}\r\n\r\n",
                b"event: message_delta\ndata: {\"usage\":{\"output_tokens\":25,\"thinking_tokens\":8}}\n\n",
            ],
        );
        assert_eq!(
            usage,
            TokenUsage {
                input_tokens: Some(300),
                output_tokens: Some(25),
                cached_tokens: Some(40),
                reasoning_tokens: Some(8),
            }
        );
    }

    #[test]
    fn ignores_invalid_events_and_done_markers() {
        let usage = scan(Provider::OpenAI, &[b"data: not-json\n\ndata: [DONE]\n\n"]);
        assert_eq!(usage, TokenUsage::default());
    }

    #[test]
    fn gemini_sse_usage_metadata() {
        let stream: &[u8] = b"data: {\"candidates\":[]}\r\n\r\ndata: {\"usageMetadata\":{\"promptTokenCount\":50,\"candidatesTokenCount\":7}}\r\n\r\n";
        let usage = scan(Provider::Gemini, &[stream]);
        assert_eq!(usage.input_tokens, Some(50));
        assert_eq!(usage.output_tokens, Some(7));
    }
}
