<div align="center">
  <h1>ReToken</h1>
  <p><strong>Zero-Latency Local Proxy for Agentic Workflows & Credit-Based IDEs</strong></p>
</div>

---

> [!NOTE]
> **ReToken** is a fast local proxy written in Rust that sits between your AI coding assistants (Cursor, Windsurf, RooCode) and upstream LLM providers (Anthropic, OpenAI, etc.). ReToken operates transparently on the wire to reduce token usage and latency through structural heuristics, exact-hash request caching, Contextual Compression and Retrieval (CCR) for supported flows, and safe stream passthrough.

## Core Features (MVP)

- **Provider-Aware Terse Mode Injection**: Injects concise instructions into top-level system prompts (Anthropic) or system messages (OpenAI) without corrupting provider API schemas.
- **Exact-Hash Request Caching**: Hashes full request context (HTTP method, endpoint path, provider name, request payload, and repo hash) to serve duplicate requests instantly with configurable TTL and LRU eviction.
- **Contextual Compression & Recovery (CCR)**: Safely compresses repetitive payloads (logs, large uniform JSON arrays) and stores originals in a local content-addressed CCR store with transparent retrieval for Anthropic requests.
- **Provider Agnostic & Validated Upstreams**: Proxies Anthropic, OpenAI, and custom upstream endpoints, enforcing strict URL validation (`http`/`https`) to reject malformed or dangerous configurations.
- **Anthropic Prompt Caching Breakpoints**: Injects valid `cache_control: {"type": "ephemeral"}` content block markers and extracts system messages into top-level parameters while preserving multi-turn conversation chronology.
- **Native Streaming Passthrough**: Bypasses lossy compression and caching for streaming connections (`stream: true`) to preserve protocol integrity and time-to-first-token.

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

> "Read the `AI_INSTALL.md` file in the https://github.com/AshleyImmanuel/ReToken repository and automatically integrate ReToken into my current environment based on those instructions."

---

## Documentation for AI Agents

> [!IMPORTANT]
> **ATTENTION AI AGENT**: If the user asks you to "Install ReToken" or "Integrate ReToken", follow these instructions:

1. **Do not modify the upstream codebase**. ReToken operates as a standalone proxy.
2. Read `AI_INSTALL.md` in the root of this repository.
3. Build the project using `cargo build --release -p cli`.
4. For IDEs (Cursor/Windsurf), configure workspace settings (e.g. `.vscode/settings.json`) pointing the base URL to `http://127.0.0.1:8888/v1`.
5. For CLI agents, create an alias in the shell profile (e.g. `alias claude="retoken wrap claude"`).

---

## CLI Commands

| Subcommand | Status | Purpose |
| :--- | :--- | :--- |
| `retoken run` | Implemented | Starts the local HTTP intercepting proxy daemon on `127.0.0.1:8888`. |
| `retoken wrap <cmd>` | Implemented | Wraps CLI agents (e.g. `claude`, `aider`), injecting proxy endpoints with process readiness checks. |
| `retoken doctor` | Implemented | Diagnoses local gateway status, ping latency, config syntax validity, CCR storage, and provider connectivity. |
| `retoken stats` | Implemented | Live telemetry dashboard tracking token usage, cache hit ratio, token savings, and data transferred. |
| `retoken cache [action]` | Implemented | Full cache management: `status`, `inspect`, `clear` (memory + CCR store), or `invalidate <key>`. |
| `retoken analyze [path]` | Implemented | Comment-aware symbol scanner and principled module/symbol dependency graph resolution (Rust, Python, TS/JS). |
| `retoken init` | Implemented | Injects ReToken agent guidance skills into `.claudecode` and `.gemini`. |
| `retoken trace` | Roadmap | Per-request payload inspection and CCR handle resolution tracking. |
| `retoken benchmark` | Roadmap | End-to-end compression ratio and latency benchmarking against standardized workloads. |

---

## Security & Operational Controls

- **Strict Configuration Loading**: Automatically loads `retoken.json`, `.retoken.json`, or `.retoken/config.json` with strict validation that fails loudly on malformed JSON, preventing misconfigured proxies from running silently with wrong defaults. Overlaid with environment variable overrides (`RETOKEN_*`).
- **Airtight SSRF Protection, Host Allowlists & Redirect Policy**: Custom upstreams passed via `x-retoken-upstream-url` strictly disallow loopback (`127.0.0.0/8`, `localhost`, `::1`), private RFC1918 subnets, carrier-grade NAT (`100.64.0.0/10`), testing/benchmark subnets (`192.0.2.0/24`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`), multicast (`224.0.0.0/4`), reserved class E (`240.0.0.0/4`), IPv6 unique local (`fc00::/7`), link-local (`fe80::/10`), discard prefix (`100::/64`), and cloud metadata (`169.254.169.254`). Hostnames resolving via DNS to private addresses and internal domains (`.local`, `.internal`, `.home.arpa`, `.lan`) are blocked. Automatic HTTP redirects are disabled on proxy upstream requests. Pre-flight checks on outgoing URLs enforce destination safety right before dispatch.
- **LLM Non-Determinism Modeling & Generation Cache Opt-In**: LLM generation endpoints (`/v1/messages`, `/v1/chat/completions`) model non-deterministic sampling: requests with omitted temperature (defaulting to 1.0), non-zero temperature without a fixed seed, or multiple completions (`n > 1`) strictly bypass the response cache unless `x-retoken-cache-stochastic: true` is explicitly passed.
- **Context & Credential Cache Isolation**: Cache keys incorporate the upstream base URL, request method, endpoint path, request payload, repository hash, and a SHA-256 fingerprint combining authorization credentials (`Authorization`, `x-api-key`, `api-key`, `x-goog-api-key`) and response-affecting headers (`anthropic-version`, `anthropic-beta`, `openai-organization`, `openai-project`, `openai-version`, `accept`, `content-type`).
- **Administrative Endpoints & Telemetry**: The running gateway daemon exposes loopback administrative routes `POST /_retoken/cache/clear`, `GET /_retoken/cache` (active entry listing), `DELETE /_retoken/cache?key=<k>` (single entry invalidation), and `GET /_retoken/stats` (live telemetry dashboard).
- **Cache Opt-Out**: Requests with `Cache-Control: no-cache`, `no-store`, `Pragma: no-cache`, or `x-retoken-cache: false` bypass cache lookup and storage entirely.
- **Request Size Limits & Timeouts**: Request body size is capped by `RETOKEN_MAX_REQUEST_BODY_BYTES` (default 10 MB, returning 413 on exceed). Network requests to upstream providers enforce a strict safety timeout (`upstream_timeout_secs`, default 60s).

---

## Architecture Overview

- **`gateway`**: The core `axum` HTTP proxy handling incoming requests, provider routing, URL normalization, cache lookups, admin endpoints, and response processing.
- **`compressors`**: Heuristic parsers for log slicing, JSON array collapsing with MAD anomaly detection, base64 roundtripping, and content-addressed CCR storage.
- **`core`**: Strict configuration (`AppConfig`), exact response caching (`ResponseCache`), in-memory session state tracking (`StateEngine`), flight recorder (`FlightRecord`), and process-level telemetry accumulator (`FlightTelemetry`).
- **`retrieval`**: Context budgeting (`ContextPackage`), dependency-aware ranking (`ContextPlanner`), and cache breakpoint formatting (`packer`).
- **`scheduler`**: Tool call DAG scheduling (`ExecutionDag`), next-file prediction (`Prefetcher`), log virtualization (`slice_log`), and background task tracking (`TaskManager`). Containerized sandbox execution is a roadmap item.
- **`analyzers`**: Comment-aware symbol scanner and principled module/symbol dependency graph resolution (relative paths, stem matching, exported symbols).

## License

MIT License. See `LICENSE` for more information.
