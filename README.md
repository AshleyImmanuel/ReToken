<div align="center">
  <h1>ReToken: Cut Claude Code and Coding-Agent Token Costs</h1>
  <p><strong>A local Rust proxy that lowers LLM API bills for Claude Code, opencode, Cursor, Aider and other coding agents, without hiding what the model is reading.</strong></p>

  <p>
    <a href="https://github.com/AshleyImmanuel/ReToken/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/AshleyImmanuel/ReToken/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/AshleyImmanuel/ReToken/releases"><img alt="Release" src="https://img.shields.io/github/v/release/AshleyImmanuel/ReToken"></a>
    <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
    <img alt="Rust" src="https://img.shields.io/badge/built%20with-Rust-orange.svg">
  </p>
</div>

---

**ReToken is a free, open-source local proxy that reduces the token cost of AI coding agents.** It runs on `127.0.0.1:8888` between your agent (Claude Code, opencode, Cursor, Windsurf, RooCode, Aider, Codex) and the model provider (Anthropic, OpenAI or any compatible endpoint). It compresses tool output, places prompt-cache breakpoints, and optionally asks for terse replies. Your prompts, code and API keys stay on your machine and are forwarded only to the provider you already use.

## Quick Start

```bash
git clone https://github.com/AshleyImmanuel/ReToken.git && cd ReToken
cargo install --path cli              # installs `retoken` into ~/.cargo/bin
claude mcp add retoken -- retoken mcp # lets ReToken shorten large output safely
retoken wrap claude                   # starts the proxy and runs Claude Code through it
```

Prefer to let your AI agent set it up? Ask it: *"Read AI_INSTALL.md in https://github.com/AshleyImmanuel/ReToken and integrate ReToken after showing me what will change."* See [`AI_INSTALL.md`](AI_INSTALL.md) for every setting.

## How Much Does ReToken Save?

ReToken saved 29% against calling the API directly in a headless Claude Code benchmark (Claude Sonnet, Claude Code 2.1, November 2026). Each arm ran 3 tasks (explain code, fix a failing test, summarize a module) 3 times from a clean checkout; all 36 runs answered correctly.

| Setup | Cost per task | Turns | Seconds | Output tokens |
|---|---|---|---|---|
| Direct to the API | $0.0425 | 4.78 | 12.7 | 859 |
| caveman proxy | $0.0330 | 4.33 | 13.4 | 870 |
| ReToken | $0.0329 | 4.33 | 12.8 | 826 |
| **ReToken + terse mode** | **$0.0302** | **3.78** | **11.4** | **719** |

On a single larger multi-file task, ReToken + terse cost $0.264 against $0.452 direct (42% lower). Savings grow with session length and tool-output size; these are small samples, so treat them as indicative.

## How ReToken Works

1. **Never hides what the model is reading.** A file read reaches the model in full on the turn it is read. Only once the model has moved on does a large file shrink to an outline (signatures, types, exact line numbers) with a handle to the original.
2. **Removes noise, keeps failures.** Large build, test and log output is shortened, but errors, assertion details and test summaries are always kept. Exact duplicate tool output is replaced by a reference to the earlier copy.
3. **Everything lossy is recoverable.** The `retoken mcp` server gives the model a `retoken_retrieve` tool to fetch any original, filtered by pattern or line range.
4. **Keeps prompt caches warm.** Each tool result changes at most once, and Anthropic cache breakpoints follow the client's TTL, so repeated history is billed at the cached rate.

## FAQ

**Does ReToken work with Claude Code subscriptions (Pro/Max)?** Yes. It forwards the client's own authorization headers, including Claude Code OAuth, and sets `_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL` so prompt caching and tool search keep working.

**Does ReToken send my code anywhere else?** No. It listens on loopback only and forwards requests to the provider your agent already uses. Elided originals are stored locally under `<data dir>/ccr` and deleted after 7 days unused.

**Will compression make the agent worse?** Lossy changes happen only when the agent has the `retoken_retrieve` tool to undo them, and fresh file reads are never shortened. Without the tool, ReToken applies only lossless changes.

**How is ReToken different from a terse-prompt plugin?** Prompt plugins only shorten replies. ReToken also cuts input: tool output, duplicate reads and cache placement, which is most of an agent's bill. Its terse mode covers replies too.

**Which providers and agents are supported?** Anthropic Messages and OpenAI Chat Completions/Responses wire formats, so Claude Code, opencode, Cursor, Windsurf, RooCode, Aider, Codex and OpenAI-compatible endpoints.

## Features

- **Lossless by Default, Savings On by Default**: User prompts, system instructions, assistant reasoning history, tool definitions, and code comments are never altered. Out of the box ReToken removes exact-duplicate tool output (lossless: the first copy stays in the conversation) and adds Anthropic prompt cache breakpoints, including a rolling breakpoint on the conversation tail, so repeated history is billed at the cached rate. Rewritten requests keep the client's JSON key order. Lossy elision only happens when your agent exposes the `retoken mcp` retrieval tool. Terse mode (`terse_mode`) and proxy-side CCR replay (`ccr_enabled`) stay opt-in; set `compression_enabled: false` and `prompt_caching_enabled: false` for literal pass-through.
- **Zero-Tradeoff Prefix Optimization for OpenAI/Codex & Anthropic**: When `prompt_caching_enabled` is active, ReToken canonicalizes requests strictly losslessly: sorting OpenAI/Codex tools deterministically by function name and stabilizing system messages at index 0 (triggering automatic 50% discount prefix caching on `gpt-4o`/Codex), and placing ephemeral breakpoints on Anthropic (triggering 90% discount prompt caching). Zero code or text tokens are discarded.
- **Exact-Hash Response Caching**: Hashes full request context (HTTP method, endpoint path, provider name, upstream URL, request payload, repo hash, and non-transport headers) to serve duplicate requests instantly with configurable TTL and LRU eviction.
- **Dual Limit Cache Memory Protection**: Cache is constrained by both entry count (`max_cache_entries`, default 1024) and total memory budget (`max_cache_bytes`, default 256MB), preventing memory exhaustion.
- **Connect-Time SSRF Validation & IP Pinning**: Enforces strict destination safety for custom upstreams, blocking loopback, private subnets, carrier-grade NAT, benchmark subnets, and cloud metadata. Pins verified socket destinations at connection time to eliminate TOCTOU DNS rebinding risks.
- **Bounded Request & Response Buffering**: Caches and buffers are bounded by configurable byte limits (`max_request_body_bytes` and `max_response_body_bytes`, default 10MB each), protecting against memory exhaustion.
- **Tool-Output Compression That Works With Streaming Agents** (`compression_enabled`): Exact repeats of earlier tool output are replaced losslessly. With the `retoken mcp` retrieval server registered in your agent, large build/test/log output and uniform JSON arrays are also elided (errors, failure details, and test summaries are always kept), and **file reads are progressive**: the model sees a file in full on the turn it reads it, and from the next turn on, large code reads shrink to an outline (signatures, types, exact line numbers) with a handle to the original. Searches and git output are never elided. Each tool result changes at most once, so provider prompt caching keeps working.
- **Dual Request/Response Streaming Detection**: Automatically bypasses caching and buffering when `stream: true` is requested or when upstream returns `Content-Type: text/event-stream`.
- **Partial Configuration with Serde Defaults**: Supports minimal config files (e.g. `{"port": 9000}`) with explicit warning reports on invalid environment variables.

---

## Maturity Status & Operational Caveats

> [!WARNING]
> **Project Maturity**: ReToken is an **early production candidate / serious prototype**, not mature enterprise LLM proxy infrastructure.
>
> 1. **Default Lossless Operation**: In default configuration (`compression_enabled: true`, `prompt_caching_enabled: true`, `ccr_enabled: false`, `terse_mode: false`), ReToken never changes what the model reads beyond replacing exact repeats of earlier tool output with a reference, and adds cache breakpoints. Set both flags to `false` for a bit-for-bit transparent pass-through.
> 2. **Stateful Replay in CCR**: The live CCR engine dispatches protocol-compliant `tool_result` turns back to the provider when the model invokes `retoken_retrieve`. However, stateful request replay across a transparent HTTP proxy is an advanced pattern that has not been battle-tested against high-volume production Anthropic traffic under real-world edge cases (e.g. concurrent tool execution, aggressive rate limits, stream conversion).
> 3. **Non-Trivial Behavior Modifications**: Registering the `retoken mcp` retrieval tool (which enables lossy elision under `compression_enabled`), `ccr_enabled`, or `terse_mode` actively modifies prompt structures, elides repetitive tool outputs, or instructs the model toward extreme brevity. Keep these flags disabled if model answer richness and exact prompt preservation are required.

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

### Recovering Elided Output (MCP)

Register the retrieval server so ReToken can elide large command output on streaming requests (Claude Code, opencode, Cursor all stream):

```bash
# Claude Code
claude mcp add retoken -- retoken mcp
```

```jsonc
// opencode (opencode.jsonc)
"mcp": { "retoken": { "type": "local", "command": ["retoken", "mcp"] } }
```

ReToken detects the `*retoken_retrieve` tool in each request, including when Claude Code defers MCP tools behind `ToolSearch` (`ENABLE_TOOL_SEARCH`). Without it, only lossless compression is applied.

Optional: `RETOKEN_TERSE_MODE=true` (or `"terse_mode": true` in `retoken.json`) asks the model for terse prose replies; code and tool inputs stay normal. All settings and their defaults are listed in [`AI_INSTALL.md`](AI_INSTALL.md#step-4-choose-settings-optional).

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
| `retoken cache clear` | Implemented | Purges the live gateway's in-memory response cache (stored CCR originals live in `<data dir>/ccr`). |
| `retoken cache invalidate <key>` | Implemented | Invalidates a specific cached response entry by key. |
| `retoken analyze [path]` | Implemented | Comment-aware symbol scanner and dependency graph resolution (Rust, Python, TS/JS). |
| `retoken mcp` | Implemented | Serves `retoken_retrieve` over MCP (stdio) so agents can fetch elided originals, optionally filtered by `pattern` or a line range. |
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
- **Prompt & Reasoning Preservation**: User prompts, system messages, assistant history, and code comments are preserved bit-for-bit without elision or comment stripping. Lossy elision runs only when the agent exposes the `retoken mcp` retrieval tool, and terse mode (`terse_mode`) is disabled by default. Contextual Compression and Retrieval (CCR) is opt-in (`ccr_enabled`) to ensure model reasoning loops remain unhindered.
- **Upstream Timeouts**: Network requests to upstream providers enforce a strict safety timeout (`upstream_timeout_secs`, default 60s).

---

## Architecture Overview

- **`gateway` (Implemented)**: The core `axum` HTTP wire proxy handling incoming requests, provider routing, URL normalization, cache lookups, admin endpoints, and response processing.
- **`compressors` (Implemented)**: Tool-output compression (provenance-aware dedup and elision), log slicing, JSON array collapsing with MAD anomaly detection, and the content-addressed CCR store.
- **`core` (Implemented)**: Configuration (`AppConfig`), exact response caching (`ResponseCache`), in-memory session state tracking (`StateEngine`), flight recorder (`FlightRecord`), and process-level telemetry accumulator (`FlightTelemetry`).
- **`analyzers` (Implemented)**: Comment-aware symbol scanner and module/symbol dependency graph resolution (relative paths, stem matching, exported symbols).
- **`retrieval` (Experimental Scaffold)**: Context budgeting (`ContextPackage`), dependency-aware ranking (`ContextPlanner`), and cache breakpoint formatting (`packer`).
- **`scheduler` (Experimental Scaffold)**: Tool call DAG scheduling (`ExecutionDag`), next-file prediction (`Prefetcher`), log virtualization (`slice_log`), and background task tracking (`TaskManager`). Containerized sandbox execution is a roadmap item.

## Contributing

Contributions are welcome, from bug reports to new compression strategies. Read [`CONTRIBUTING.md`](CONTRIBUTING.md) to set up, find a first issue, and open a pull request. Good starting points are issues labelled `good first issue` and `help wanted`.

## License

MIT License. See `LICENSE` for more information.
