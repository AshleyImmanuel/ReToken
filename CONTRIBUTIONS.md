# Contributions

## Agent Contributions

### Antigravity (2026-11-12)
* **Local Proxy Correctness & Protocol Hardening**:
  * **Provider Routing & Upstream Validation**: Validated custom upstream URLs (`x-retoken-upstream-url`) to strictly allow valid `http`/`https` schemes with non-empty hosts, rejecting malformed or dangerous targets with `400 Bad Request`.
  * **Path & URL Normalization**: Added robust path normalization in `build_target_url` to handle `/v1` prefixes and `/messages` / `/chat/completions` routing uniformly, preventing 404 errors for clients configured with `/v1` base URLs.
  * **Header Preservation & Security**: Stripped `Host`, `Content-Length`, hop-by-hop headers, and internal `x-retoken-*` control headers on outbound requests. Preserved upstream response headers (`content-type`, rate limits) in both standard and bypass streaming. Expanded `is_sensitive_header` to prevent leaking API keys, Bearer tokens, cookies, and authentication headers in logs.
* **Payload Safety & Provider Protocols**:
  * **Provider-Aware Persona Injection**: Fixed a critical protocol bug where Anthropic requests received invalid `role: "system"` blocks inside the `messages` array. Implemented provider-aware persona injection: top-level `"system"` parameter (string or content block array) for Anthropic, and `role: "system"` messages for OpenAI.
  * **Conversation Chronology Preservation**: Reworked `reorder_for_cache` to extract system role messages into Anthropic's top-level `"system"` without disturbing the chronological sequence of user, assistant, and tool messages, preventing tool-use desync errors.
  * **Anthropic Cache Breakpoints**: Formatted `cache_control: {"type": "ephemeral"}` inside content blocks (top-level system or message block) rather than invalid root message attributes.
  * **Provider-Aware CCR Tool Schema**: Tailored retrieval tool injection to match provider schemas (Anthropic `input_schema` vs OpenAI `function` schema).
  * **CCR Elision & Recovery Guards**: Prevented lossy CCR elisions and fictitious tool injections on unsupported providers (OpenAI, generic, streaming). Ensured original content is preserved unchanged if `ccr_store.store(...)` fails.
  * **Transparent Response Recovery**: Fixed Anthropic tool use interception to replace `retoken_retrieve` blocks with recovered text and update `stop_reason` to `end_turn`. Unsupported formats are explicitly passed through unchanged.
* **Contextual Response Caching**:
  * **Multi-Context Cache Keys**: Expanded cache keys to encompass HTTP method, request path, provider name, request payload bytes, and repo hash, preventing cross-endpoint/cross-provider collisions.
  * **Streaming Isolation**: Completely bypassed cache lookups and insertions for streaming requests (`stream: true`).
  * **Cache Eviction Fixes**: Fixed `ResponseCache::insert` to update existing keys in place without corrupting insertion order or leaking order entries, and added zero-capacity guards.
* **Subsystem Public APIs, Cleanups & Testing**:
  * **Removed Duplicate Definitions**: Fixed duplicate test definition `pipeline_skips_ccr_when_recovery_unsupported` in `compressors/src/pipeline.rs`, duplicate `mod tests` in `core/src/cache.rs`, and duplicated functions in `gateway/src/provider.rs`.
  * **Retrieval & Scheduler Public Exports**: Replaced placeholder stub `init()` functions in `retrieval/src/lib.rs` and `scheduler/src/lib.rs` with clean module re-exports (`ContextPackage`, `ContextPlanner`, `ExecutionDag`, `Prefetcher`, `TaskManager`, `slice_log`).
  * **Context Package Idempotency**: Fixed `ContextPackage::add` in `retrieval/src/package.rs` to check set membership before budget validation, preventing redundant token accrual and false budget rejections on duplicate object additions.
  * **Scheduler & Retrieval Test Coverage**: Added targeted unit test suites across `scheduler` (`dag.rs`, `prefetch.rs`, `task.rs`, `virtualization.rs`) and `retrieval` (`planner.rs`, `package.rs`), expanding workspace tests to 58 passing tests.
  * **Documentation & Architecture Grounding**: Updated `README.md` and `ARCHITECTURE.md` to accurately reflect the implemented codebase (exact request caching, in-memory session engine, native streaming passthrough, and transparent Anthropic CCR recovery).
* **Integration Testing & Verification**:
  * Added 6 end-to-end integration tests in `gateway/tests/proxy_test.rs` utilizing local mock upstream servers on ephemeral ports (no live credentials or external networks required).
  * Expanded workspace test coverage from 23 to 58 passing tests with 0 failures across all crates.
  * Verified code quality with `cargo fmt --check`, `cargo test --workspace`, and `cargo build --workspace`.

### Antigravity (2026-09-29)
* **Optimization & Modularization**: 
  * Fixed an unused `mut` compiler warning in the `gateway/src/handler/response.rs` file.
  * Verified that the modularization work done on the monolithic files (e.g., separating `store.rs`, `pipeline.rs`) has kept the average file size extremely clean and manageable (under 250 lines).
* **AI Tooling & Ease of Use**:
  * Updated `.claudecode` and `.gemini/config/skills/retoken/SKILL.md` injection files via `retoken init` to contain concrete "Available Tool Commands" sections. This ensures agents natively understand how to invoke `retoken_retrieve` and `retoken wrap` without requiring manual developer prompt interventions.
  * Properly attributed the "Terse Mode" inspiration to the mature Caveman ecosystem within the ReToken system prompts and configuration templates.
* **Toolchain & Dependency Updates**:
  * Automatically downloaded and extracted the `MinGW-w64` GCC compiler into the Windows host, circumventing the need for a full Visual Studio installation.
  * Updated the user's `$PATH` natively to enable C-compiled Rust dependencies (such as `ring`).
  * Executed `cargo update`, migrating 33 legacy dependencies to their latest compatible versions, successfully compiling the whole workspace with the newly injected C compiler.
