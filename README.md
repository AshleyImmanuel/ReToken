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
| `retoken analyze [path]` | Implemented | Scans repository source files and builds symbol and dependency graphs. |
| `retoken init` | Implemented | Injects ReToken agent guidance skills into `.claudecode` and `.gemini`. |
| `retoken stats` | Roadmap | Aggregated token savings, compression ratios, and flight recorder telemetry dashboard. |
| `retoken trace` | Roadmap | Per-request payload inspection and CCR handle resolution tracking. |
| `retoken benchmark` | Roadmap | End-to-end compression ratio and latency benchmarking against standardized workloads. |
| `retoken doctor` | Roadmap | Diagnostic suite checking loopback binding, provider connectivity, and environment variables. |

---

## Security & Operational Controls

- **SSRF Protection**: Custom upstreams passed via `x-retoken-upstream-url` strictly disallow loopback (`127.0.0.0/8`), private RFC1918 subnets, and cloud metadata addresses (`169.254.169.254`). Set `RETOKEN_ALLOW_PRIVATE_UPSTREAMS=true` only for local testing.
- **Credential-Isolated Caching**: Response cache keys include a SHA-256 fingerprint of the request's credentials (`Authorization` or `x-api-key`), preventing cross-tenant cache pollution.
- **Cache Opt-Out**: Requests with `Cache-Control: no-cache`, `no-store`, or `x-retoken-cache: false` bypass cache lookup and storage entirely.
- **Request Size Limits**: Max request body size is capped by `RETOKEN_MAX_REQUEST_BODY_BYTES` (default 10 MB). Requests exceeding the limit return `413 Payload Too Large`.

---

## Architecture Overview

- **`gateway`**: The core `axum` HTTP proxy handling incoming requests, provider routing, URL normalization, cache lookups, and response processing.
- **`compressors`**: Heuristic parsers for log slicing, JSON array collapsing with MAD anomaly detection, base64 roundtripping, and content-addressed CCR storage.
- **`core`**: Configuration (`AppConfig`), exact response caching (`ResponseCache`), in-memory session state tracking (`StateEngine`), and telemetry metrics (`FlightRecord`).
- **`retrieval`**: Context budgeting (`ContextPackage`), dependency-aware ranking (`ContextPlanner`), and cache breakpoint formatting (`packer`).
- **`scheduler`**: Tool call DAG scheduling (`ExecutionDag`), next-file prediction (`Prefetcher`), log virtualization (`slice_log`), and background task tracking (`TaskManager`). Containerized sandbox execution is a roadmap item.
- **`analyzers`**: AST symbol extraction and dependency graph scanner.

## License

MIT License. See `LICENSE` for more information.
