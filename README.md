<div align="center">
  <h1>ReToken</h1>
  <p><strong>Low-Overhead Local Proxy for Agentic Workflows & Coding Assistants</strong></p>
</div>

---

> [!NOTE]
> **ReToken** is a low-overhead local wire proxy written in Rust that sits between your AI coding assistants (Cursor, Windsurf, RooCode, Claude Code) and upstream LLM providers (Anthropic, OpenAI, etc.). ReToken operates transparently on the wire to reduce token usage and cost through structural heuristics, exact-hash request caching, Contextual Compression and Retrieval (CCR) for supported flows, and safe stream passthrough.

## Core Features (MVP)

- **Literal Pass-Through Default with Quality Preservation**: User prompts, system instructions, assistant reasoning history, tool definitions, and code comments are strictly preserved bit-for-bit. Request compression (`compression_enabled`), terse mode (`terse_mode`), Anthropic prompt cache injection (`prompt_caching_enabled`), and CCR (`ccr_enabled`) are all disabled by default, ensuring incoming requests and provider responses pass through untouched unless explicitly opted in.
- **Exact-Hash Response Caching**: Hashes full request context (HTTP method, endpoint path, provider name, upstream URL, request payload, repo hash, and non-transport headers) to serve duplicate requests instantly with configurable TTL and LRU eviction.
- **Dual Limit Cache Memory Protection**: Cache is constrained by both entry count (`max_cache_entries`, default 1024) and total memory budget (`max_cache_bytes`, default 256MB), preventing memory exhaustion.
- **Connect-Time SSRF Validation & IP Pinning**: Enforces strict destination safety for custom upstreams, blocking loopback, private subnets, carrier-grade NAT, benchmark subnets, and cloud metadata. Pins verified socket destinations at connection time to eliminate TOCTOU DNS rebinding risks.
- **Bounded Request & Response Buffering**: Caches and buffers are bounded by configurable byte limits (`max_request_body_bytes` and `max_response_body_bytes`, default 10MB each), protecting against memory exhaustion.
- **Dual Request/Response Streaming Detection**: Automatically bypasses caching and buffering when `stream: true` is requested or when upstream returns `Content-Type: text/event-stream`.
- **Partial Configuration with Serde Defaults**: Supports minimal config files (e.g. `{"port": 9000}`) with explicit warning reports on invalid environment variables.

---

## Maturity Status & Operational Caveats

> [!WARNING]
> **Project Maturity**: ReToken is an **early production candidate / serious prototype**, not mature enterprise LLM proxy infrastructure.
>
> 1. **Default Lossless Operation**: In default configuration (`compression_enabled: false`, `ccr_enabled: false`, `terse_mode: false`, `prompt_caching_enabled: false`), ReToken acts as a strictly lossless, bit-for-bit transparent pass-through proxy with connection-time SSRF pinning, request/response memory limits, and exact-hash caching for idempotent endpoints.
> 2. **Stateful Replay in CCR**: The live CCR engine dispatches protocol-compliant `tool_result` turns back to the provider when the model invokes `retoken_retrieve`. However, stateful request replay across a transparent HTTP proxy is an advanced pattern that has not been battle-tested against high-volume production Anthropic traffic under real-world edge cases (e.g. concurrent tool execution, aggressive rate limits, stream conversion).
> 3. **Non-Trivial Behavior Modifications**: Opting into `compression_enabled`, `ccr_enabled`, or `terse_mode` actively modifies prompt structures, elides repetitive tool outputs, or instructs the model toward extreme brevity. Keep these flags disabled if model answer richness and exact prompt preservation are required.

---

## Documentation for Humans

### Installation

Ensure you have Rust installed. Clone the repository and build the CLI:

```bash
git clone https://github.com/AshleyImmanuel/ReToken.git
cd ReToken
cargo build --release -p cli
```

### Usage

To start intercepting traffic, launch the ReToken engine in the background:

```bash
retoken run
```

*(Note: The daemon binds to the local loopback interface `127.0.0.1:8888` to intercept LLM traffic without exposing external ports.)*

### IDE Configuration

To route traffic through ReToken:

**For Cursor / Windsurf:**
1. Open your IDE Settings.
2. Navigate to **Models** or **API Configuration**.
3. Set the **Custom Provider URL** or **OpenAI Base URL** to `http://127.0.0.1:8888/v1`.
4. Leave your API key as is (ReToken forwards authorization headers securely).

**For CLI Agents (Claude Code, Aider, etc.):**
Wrap your command using the built-in ReToken CLI to inject the correct environment variables:
```bash
retoken wrap claude
# or
retoken wrap aider
```

### Auto-Installation via AI

To configure via an AI agent (Claude Code, Aider, RooCode):

> "Read the `AI_INSTALL.md` file in the https://github.com/AshleyImmanuel/ReToken repository and integrate ReToken into my current environment after showing what will change and asking for my confirmation."

---

## Documentation for AI Agents

> [!IMPORTANT]
> **ATTENTION AI AGENT**: If the user asks you to "Install ReToken" or "Integrate ReToken", follow these instructions:

1. **Always request user consent** before modifying shell profiles (`~/.bashrc`, `~/.zshrc`, PowerShell `$PROFILE`) or IDE settings (`.vscode/settings.json`, `.cursor/settings`).
2. Read `AI_INSTALL.md` in the root of this repository.
3. Build the project using `cargo build --release -p cli`.
4. Clearly show the user the exact diff or changes proposed and provide undo instructions.

---

## CLI Commands

| Subcommand | Status | Purpose |
| :--- | :--- | :--- |
| `retoken run` | Implemented | Starts the local HTTP intercepting proxy daemon on `127.0.0.1:8888`. |
| `retoken wrap <cmd>` | Implemented | Wraps CLI agents (e.g. `claude`, `aider`), honoring config files and environment overrides with daemon auto-start. |
| `retoken doctor` | Implemented | Diagnoses local gateway status, loopback latency, config syntax validity, CCR storage, and provider connectivity. |
| `retoken stats` | Implemented | Displays runtime telemetry tracking token usage, cache hit ratio, token savings, and data transferred. |
| `retoken cache [status]` | Implemented | Displays cache state, active entries, capacity, and hit ratios. |
| `retoken cache clear` | Implemented | Purges live in-memory response cache and persistent CCR storage. |
| `retoken cache invalidate <key>` | Implemented | Invalidates a specific cached response entry by key. |
| `retoken analyze [path]` | Implemented | Comment-aware symbol scanner and dependency graph resolution (Rust, Python, TS/JS). |
| `retoken init` | Implemented | Injects ReToken agent guidance skills into `.claudecode` and `.gemini`. |
| `retoken trace` | Roadmap | Per-request payload inspection and CCR handle resolution tracking. |
| `retoken benchmark` | Roadmap | End-to-end compression ratio and latency benchmarking against standardized workloads. |

---

## Security & Operational Controls

- **Strict Configuration Loading & Defaults**: Automatically loads `retoken.json`, `.retoken.json`, or `.retoken/config.json` supporting partial JSON overrides via serde defaults. Fails loudly on malformed JSON and logs warnings on unparseable environment variables (`RETOKEN_*`).
- **Airtight SSRF Protection & Connect-Time Pinning**: Custom upstreams passed via `x-retoken-upstream-url` strictly disallow loopback (`127.0.0.0/8`, `localhost`, `::1`), private RFC1918 subnets, carrier-grade NAT (`100.64.0.0/10`), testing/benchmark subnets (`192.0.2.0/24`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`), multicast (`224.0.0.0/4`), reserved class E (`240.0.0.0/4`), IPv6 unique local (`fc00::/7`), link-local (`fe80::/10`), discard prefix (`100::/64`), and cloud metadata (`169.254.169.254`). Validated socket destinations are pinned at connect time to prevent DNS rebinding TOCTOU.
- **LLM Non-Determinism Modeling & Generation Cache Opt-In**: LLM generation endpoints (`/v1/messages`, `/v1/chat/completions`) model non-deterministic sampling: requests with omitted temperature (defaulting to 1.0), non-zero temperature without a fixed seed, or multiple completions (`n > 1`) strictly bypass the response cache unless `x-retoken-cache-stochastic: true` is explicitly passed.
- **Conservative Context & Credential Cache Isolation**: Cache keys incorporate the upstream base URL, request method, endpoint path, request payload, repository hash, and a SHA-256 fingerprint encompassing credentials (`Authorization`, `x-api-key`, `api-key`, `x-goog-api-key`), version headers, and any custom client headers (excluding only hop-by-hop transport headers and proxy control headers).
- **Administrative Endpoints & Telemetry**: The running gateway daemon exposes loopback administrative routes `POST /_retoken/cache/clear`, `GET /_retoken/cache` (active entry listing), `DELETE /_retoken/cache?key=<k>` (single entry invalidation), and `GET /_retoken/stats` (live telemetry).
- **Cache Opt-Out**: Requests with `Cache-Control: no-cache`, `no-store`, `Pragma: no-cache`, or `x-retoken-cache: false` bypass cache lookup and storage entirely.
- **Request, Response, and Cache Memory Budgets**: Request body size is capped by `RETOKEN_MAX_REQUEST_BODY_BYTES` (default 10 MB, returning 413 on exceed). Non-streaming response body size is capped by `RETOKEN_MAX_RESPONSE_BODY_BYTES` (default 10 MB, returning 502 on exceed). Total response cache memory is strictly capped by `RETOKEN_MAX_CACHE_BYTES` (default 256 MB) alongside entry capacity (`RETOKEN_MAX_CACHE_ENTRIES`, default 1024), preventing memory leaks.
- **Prompt & Reasoning Preservation**: User prompts, system messages, assistant history, and code comments are preserved bit-for-bit without elision or comment stripping. Lossy compression (`compression_enabled`) and terse mode (`terse_mode`) are disabled by default. Contextual Compression and Retrieval (CCR) is opt-in (`ccr_enabled`) to ensure model reasoning loops remain unhindered.
- **Upstream Timeouts**: Network requests to upstream providers enforce a strict safety timeout (`upstream_timeout_secs`, default 60s).

---

## Architecture Overview

- **`gateway` (Implemented)**: The core `axum` HTTP wire proxy handling incoming requests, provider routing, URL normalization, cache lookups, admin endpoints, and response processing.
- **`compressors` (Implemented)**: Heuristic parsers for log slicing, JSON array collapsing with MAD anomaly detection, base64 roundtripping, and content-addressed CCR storage.
- **`core` (Implemented)**: Configuration (`AppConfig`), exact response caching (`ResponseCache`), in-memory session state tracking (`StateEngine`), flight recorder (`FlightRecord`), and process-level telemetry accumulator (`FlightTelemetry`).
- **`analyzers` (Implemented)**: Comment-aware symbol scanner and module/symbol dependency graph resolution (relative paths, stem matching, exported symbols).
- **`retrieval` (Experimental Scaffold)**: Context budgeting (`ContextPackage`), dependency-aware ranking (`ContextPlanner`), and cache breakpoint formatting (`packer`).
- **`scheduler` (Experimental Scaffold)**: Tool call DAG scheduling (`ExecutionDag`), next-file prediction (`Prefetcher`), log virtualization (`slice_log`), and background task tracking (`TaskManager`). Containerized sandbox execution is a roadmap item.

## License

MIT License. See `LICENSE` for more information.
