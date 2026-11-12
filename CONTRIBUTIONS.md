# Contributions

## Agent Contributions

### Antigravity (2026-11-12 - Quality & Operational Safeguards)
* **Model Reasoning & Prompt Context Protection**:
  * **Lossless Pass-Through by Default**: Added `compression_enabled` (default `false`) and `ccr_enabled` (default `false`) to `AppConfig`, ensuring that requests are passed through completely losslessly by default.
  * **Strict Role Boundaries**: Updated `compressors/src/process.rs` so that lossy compression is strictly restricted to tool outputs (`role == "tool"` or `type == "tool_result"`). User prompts, system messages, and assistant reasoning history are never elided or truncated.
  * **Code Comment Preservation**: Removed comment stripping in `compressors/src/minify.rs`. Developer comments explaining code behavior, assumptions, and security constraints (`//`) are now 100% preserved.
  * **Opt-In Terse Mode**: Changed `terse_mode` default from `true` to `false` in `AppConfig`, eliminating unwanted prompt persona injection that pushed the model toward extreme brevity.
  * **Safe CCR Guardrails**: Disabled CCR elision and fictitious retrieval tool injection by default (`ccr_enabled: false`) to avoid breaking the model's multi-turn reasoning cycle.
* **Hard Total Cache Memory Limit**:
  * Added `max_cache_bytes` to `AppConfig` (default 256 MB) and implemented byte-budget tracking (`current_bytes`) in `ResponseCache`.
  * The cache now enforces both max entry count AND hard total memory budget, automatically evicting oldest entries when either threshold is exceeded.
* **Telemetry & CLI Updates**:
  * Updated `admin_stats`, `retoken stats`, and `retoken cache status` to display live cache memory consumption and capacity limits.
* **Expanded Verification**:
  * Added integration tests verifying lossless pass-through of comments, multi-line prose, and byte-budget eviction.

### Antigravity (2026-11-12 - Continued)
* **Connect-Time SSRF Validation & Destination IP Pinning**:
  * Implemented `resolve_and_verify_destination` in `gateway/src/provider.rs` and integrated destination resolution into `gateway/src/handler/mod.rs`.
  * Configured `reqwest::Client::builder().resolve(host, verified_addr)` to pin connections directly to the verified socket destination, eliminating time-of-check to time-of-use (TOCTOU) DNS rebinding vulnerabilities between DNS pre-flight and connection time.
* **Bounded Response Body Limits**:
  * Added `max_response_body_bytes` to `AppConfig` (default 10 MB, configurable via `RETOKEN_MAX_RESPONSE_BODY_BYTES`).
  * Implemented chunked response streaming in `gateway/src/handler/response.rs` with running byte accumulation up to `max_response_body_bytes`. Responses exceeding the limit immediately terminate with `502 Bad Gateway`, preventing memory exhaustion attacks from unbounded upstream payloads.
* **Response-Side Event Stream Detection**:
  * Added response-side streaming detection in `gateway/src/handler/response.rs` checking `resp_headers` for `Content-Type: text/event-stream`.
  * Any event stream response passes through natively without buffering or caching, even if the client omitted `stream: true` in the request body.
* **Conservative Context & Header Cache Isolation**:
  * Refactored `compute_context_fingerprint` in `core/src/cache.rs` to encompass all client request headers, excluding only hop-by-hop transport headers (`connection`, `host`, `transfer-encoding`, etc.) and internal proxy control headers (`x-retoken-*`).
  * Ensured custom client headers, model variations, and disparate credentials automatically isolate cache keys. Added unit tests for custom header isolation and hop-by-hop header filtering.
* **Partial Configuration Deserialization & Robust Defaults**:
  * Added `#[serde(default)]` to `AppConfig` and derived `Default` on `SandboxMode`. Minimal configuration files (e.g. `{"port": 9000}`) now deserialize cleanly over defaults without requiring all struct fields.
  * Added `parse_env_var` with explicit warning logs when environment variable values cannot be parsed, preventing silent fallbacks.
* **CLI Wrapper Config Alignment & Typed Cache Subcommands**:
  * Updated `cli/src/wrap.rs` to load configuration via `AppConfig::load()?` instead of `AppConfig::default()`, ensuring `retoken wrap` correctly targets configured custom ports.
  * Refactored `Commands::Cache` in `cli/src/main.rs` into a strongly-typed `CacheCommands` enum (`Status`, `Clear`, `Invalidate { key: String }`), matching documented commands.
  * Updated `cli/src/cache_cmd.rs` and added CLI argument parsing unit tests.
* **Documentation Grounding & Consent-Respecting AI Install**:
  * Refined `README.md` to remove "Zero-Latency" terminology, accurately describe ReToken as a low-overhead wire proxy, document the new cache CLI interface, and explicitly categorize crates into Implemented, Experimental Scaffolding, and Roadmap features.
  * Completely rewrote `AI_INSTALL.md` to eliminate "don't ask permission" directives, requiring AI agents to obtain user consent before modifying shell profiles or IDE settings, detailing all modified files and injected environment variables, and providing complete undo/rollback instructions.
* **Multi-Platform Release Automation**:
  * Rebuilt `.github/workflows/release.yml` with a multi-platform release matrix compiling release binaries, packaging archives, and generating SHA-256 checksums across Linux (`x86_64-unknown-linux-gnu`), macOS Intel (`x86_64-apple-darwin`), macOS Apple Silicon (`aarch64-apple-darwin`), and Windows (`x86_64-pc-windows-msvc`).
* **Expanded Test Suite**:
  * Added new unit and integration tests across `retoken_core`, `gateway`, and `cli`, expanding total test coverage to 84 passing tests with 0 failures.

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
  * **Removed Placeholder Crates & Stub Functions**: Deleted empty `proxy` crate from workspace members, and replaced stub `init()` functions across `retrieval/src/lib.rs`, `scheduler/src/lib.rs`, `core/src/lib.rs`, and `analyzers/src/lib.rs` with clean public module and struct re-exports.
  * **Telemetry Metric Alignment**: Audited `FlightRecord` in `core/src/recorder.rs` to eliminate unconnected placeholder fields (`tool_calls`, `tool_duration_ms`, `ttft_ms`, `generation_duration_ms`, `agent_turns`), grounding telemetry strictly in metrics measured by the proxy.
  * **Sandbox Mode Safety Guards**: Updated `scheduler/src/task.rs` to explicitly reject execution with an informative error if `SandboxMode::Virtual` is configured, preventing unintended native execution on the host when container isolation is expected.
  * **Flexible Token Estimation**: Refactored `retrieval/src/planner.rs` to provide `plan_with_estimator` for custom or exact token resolvers (such as tracked state objects) while retaining the default 4-bytes/token heuristic in `plan`.
  * **Removed Duplicate Definitions**: Fixed duplicate test definition `pipeline_skips_ccr_when_recovery_unsupported` in `compressors/src/pipeline.rs`, duplicate `mod tests` in `core/src/cache.rs`, and duplicated functions in `gateway/src/provider.rs`.
  * **Context Package Idempotency**: Fixed `ContextPackage::add` in `retrieval/src/package.rs` to check set membership before budget validation, preventing redundant token accrual and false budget rejections on duplicate object additions.
  * **Expanded Unit Test Coverage**: Added targeted unit tests across `analyzers` (`scanner.rs`), `core` (`recorder.rs`), `scheduler` (`dag.rs`, `prefetch.rs`, `task.rs`, `virtualization.rs`), and `retrieval` (`planner.rs`, `package.rs`), expanding workspace tests from 23 to 62 passing tests with 0 failures.
  * **Documentation & Architecture Grounding**: Updated `README.md` and `ARCHITECTURE.md` to accurately reflect the implemented codebase (exact request caching, in-memory session engine, native streaming passthrough, and transparent Anthropic CCR recovery).
* **Enterprise Controls, CLI Diagnostics & Analyzer Maturity**:
  * **Configuration File System**: Added `AppConfig::load()` supporting `retoken.json`, `.retoken.json`, and `.retoken/config.json` with seamless environment variable overrides.
  * **Upstream Host Allowlists**: Added `allowed_upstream_hosts` in `AppConfig` and `gateway/src/provider.rs` with exact match and wildcard (`*.domain`) pattern support to strictly confine custom upstreams to approved domains.
  * **Upstream Request Timeouts**: Configured `reqwest::Client` with `upstream_timeout_secs` (default 60s) to prevent hung provider requests from blocking connections indefinitely.
  * **Stochastic Generation Cache Safety**: Guarded against returning stale non-deterministic responses by bypassing cache when `temperature > 0.0`, unless explicitly opted into via `x-retoken-cache-stochastic: true`.
  * **CLI Diagnostics & Administration**: Implemented `retoken doctor` (system, network, gateway reachability, and provider connectivity checks), `retoken stats` (telemetry and runtime configuration overview), and `retoken cache` (status inspection and cache reset).
  * **Multi-Language Symbol & Dependency Analysis**: Enhanced `analyzers/src/symbols.rs` to extract functions, classes, structs, enums, traits, and imports across Rust, Python, and TypeScript/JavaScript. Added bidirectional graph resolution (`dependencies_of` and `dependents_of`) in `analyzers/src/graph.rs`.
  * **Continuous Integration (CI) Workflow**: Created `.github/workflows/ci.yml` running formatting, clippy lints, test suites, and workspace compilation on push and pull requests.
* **Security, Isolation & Operational Hardening**:
  * **SSRF Protection on Custom Upstreams**: Implemented `is_private_or_loopback_host` and trust policy in `gateway/src/provider.rs`. Untrusted `x-retoken-upstream-url` headers strictly disallow loopback (`127.0.0.0/8`, `localhost`), link-local metadata addresses (`169.254.169.254`), and private RFC1918 subnets unless explicitly enabled with `RETOKEN_ALLOW_PRIVATE_UPSTREAMS=true`.
  * **Credential-Isolated Response Caching**: Expanded `ResponseCache::cache_key_with_context` to incorporate a SHA-256 fingerprint of the request's credentials (`Authorization` or `x-api-key`), guaranteeing strict cross-tenant cache isolation and preventing cache leakage across different API keys.
  * **Cache Opt-Out Directives**: Honors `Cache-Control: no-cache`, `no-store`, `Pragma: no-cache`, and `x-retoken-cache: false` / `x-retoken-no-cache: true`, completely bypassing cache lookup and response storage.
  * **Configurable Request Body Limit**: Replaced unbounded `usize::MAX` body reads with `AppConfig::max_request_body_bytes` (default 10 MB, configurable via `RETOKEN_MAX_REQUEST_BODY_BYTES`), rejecting oversized payloads with `413 Payload Too Large`.
  * **Resilient Process Handling (`retoken wrap`)**: Replaced panics (`unwrap`/`expect`) in `cli/src/wrap.rs` with `anyhow::Context` error propagation, and added a readiness socket polling loop (3-second timeout) ensuring the background gateway daemon is healthy and reachable before launching wrapped commands.
  * **CLI Documentation & Roadmap Grounding**: Updated `README.md` with an explicit CLI status matrix clearly separating implemented commands (`run`, `wrap`, `analyze`, `init`, `doctor`, `stats`, `cache`) from planned roadmap diagnostics (`trace`, `benchmark`).
* **Integration Testing & Verification**:
  * Added integration tests covering host allowlist enforcement and stochastic temperature cache bypass.
  * Expanded workspace test coverage from 23 to 71 passing tests with 0 failures across all crates.
  * Verified code quality with `cargo fmt --check`, `cargo test --workspace`, and `cargo build --workspace`.
* **Hardened Cache Policy, Comprehensive SSRF & Administrative Runtime**:
  * **Generation Endpoint Default Cache Opt-In**: Disabled response caching by default for LLM generation endpoints (`/v1/messages`, `/v1/chat/completions`, `/completions`) to eliminate stale completions from omitted temperature or implicit model sampling. Required explicit opt-in headers (`x-retoken-cache: true` or `x-retoken-cache-exact: true`) to enable generation response caching.
  * **Comprehensive IPv6, Link-Local & DNS SSRF Hardening**: Expanded `gateway/src/provider.rs` SSRF validation to reject IPv6 loopback (`::1`), unspecified (`::`), unique local addresses (`fc00::/7`), link-local unicast (`fe80::/10`), IPv4-mapped addresses (`::ffff:0:0/96`), and domains resolving via DNS to private or loopback IPs.
  * **Upstream Redirect Policy**: Disabled automatic HTTP redirects on the proxy's `reqwest::Client` (`redirect::Policy::none()`), preventing open-redirect SSRF escalation to internal services.
  * **Context & Upstream Cache Key Isolation**: Bound the upstream base URL directly into `ResponseCache::cache_key_with_context` to guarantee separation between disparate providers and endpoints. Added `ResponseCache::compute_context_fingerprint` incorporating credentials and response-affecting headers (`authorization`, `x-api-key`, `api-key`, `anthropic-version`, `anthropic-beta`, `openai-organization`, `openai-project`).
  * **Gateway Administrative Endpoints**: Implemented loopback administrative routes `POST /_retoken/cache/clear` (purging in-memory cache and reporting cleared entry count) and `GET /_retoken/stats` (reporting live uptime, hit/miss metrics, and cache utilization).
  * **CLI Telemetry & Cache Integration**: Connected `retoken cache clear` to send an administrative purge request to the running gateway daemon, and enhanced `retoken stats` and `retoken cache status` to display live runtime statistics when the gateway is active.
  * **Doctor Diagnostics & Config Parsing Warnings**: Removed `.unwrap()` on socket parsing in `retoken doctor`, properly categorized HTTP response statuses (2xx/3xx healthy, 401/403/404 reachable with provider response, 5xx server error), and added custom upstream probe testing. Updated `AppConfig` to emit warnings on invalid JSON in configuration files instead of silently ignoring parse errors.
  * **Cross-Platform Matrix CI**: Updated `.github/workflows/ci.yml` to run tests and builds across a multi-OS matrix (`ubuntu-latest`, `windows-latest`, `macos-latest`).
  * **Expanded Test Suite**: Added tests for generation cache bypass by default, admin clear/stats endpoints, IPv6/DNS SSRF validation, and context fingerprinting, bringing the workspace test suite to 75 passing tests with 0 failures.
* **Production-Grade Trust Model, Strict Config & Telemetry Engine**:
  * **Strict Configuration Loading**: Reworked `AppConfig::load()` to return `anyhow::Result<AppConfig>`, failing loudly on malformed configuration files (`retoken.json`, `.retoken.json`) and eliminating silent fallbacks to unintended defaults.
  * **Exhaustive SSRF Subnet Guards**: Expanded IP validation to cover carrier-grade NAT (`100.64.0.0/10`), testing/benchmark subnets (`192.0.2.0/24`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`), multicast (`224.0.0.0/4`), reserved class E (`240.0.0.0/4`), and IPv6 documentation/discard ranges. Added pre-flight destination verification on outgoing requests to enforce defense-in-depth right before wire dispatch.
  * **Comprehensive LLM Non-Determinism Modeling**: Accurately modeled non-deterministic generation modes: requests with omitted temperature (defaulting to 1.0), non-zero temperature without fixed seed, or multiple completions (`n > 1`) strictly bypass the response cache unless `x-retoken-cache-stochastic: true` is explicitly provided.
  * **Extended Context Fingerprinting**: Expanded context fingerprinting to cover all response-affecting headers (`x-goog-api-key`, `openai-version`, `accept`, `content-type`, `x-retoken-provider-name`).
  * **Full Operational Cache Management**: Added `GET /_retoken/cache` (active entry inspection) and `DELETE /_retoken/cache?key=<k>` (single entry invalidation). Updated `retoken cache clear` to purge both in-memory daemon entries and local persistent CCR disk storage (`~/.retoken/ccr`). Added `retoken cache invalidate <key>`.
  * **Runtime Telemetry Dashboard**: Added process-level `FlightTelemetry` accumulator in `GatewayState` and `retoken stats`, providing a live dashboard tracking total requests, provider error rates, data transferred, input/output tokens processed, and cached tokens saved.
  * **Enhanced Diagnostic Doctor**: Added round-trip gateway ping latency measurement, config syntax validation, provider latency timing, and CCR store accessibility checks to `retoken doctor`.
  * **Principled AST-Lite Module & Symbol Resolution**: Replaced naive substring matching in `analyzers/src/graph.rs` with exact symbol exports/definitions, relative path resolution (`./`, `../`), and file stem matching. Added comment stripping (single-line `//`, `#`, and multi-line `/* ... */`) and destructured import extraction in `analyzers/src/symbols.rs`.
  * **Workspace Verification**: Expanded test suite to 79 passing tests with 0 failures, 0 warnings across `cargo fmt`, `cargo clippy`, and `cargo test`.

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
