<div align="center">
  <img src="docs/banner.svg" width="100%" alt="ReToken Banner" style="border-radius: 12px; margin-bottom: 20px;" />
  <h1>ReToken</h1>
  <p><strong>Zero-Latency Local Proxy for Agentic Workflows & Credit-Based IDEs</strong></p>
</div>

---

> [!NOTE]
> **ReToken** is a blazing-fast local proxy written in Rust that sits between your AI coding assistants (Cursor, Windsurf, RooCode) and their upstream LLM providers (Anthropic, OpenAI, etc.). It aggressively minimizes tokens, compresses schemas, and caches duplicate requests to drastically cut your API bills.

## Core Features

- **Terse Mode Injection**: Automatically injects system prompts to force concise, code-only responses from LLMs without sacrificing quality.
- **Semantic Caching**: Hashes and caches duplicate requests, instantly serving responses to save API calls (with perfect state tracking).
- **Schema Compression**: Recursively strips bloated descriptions and `examples` from tool schemas.
- **Provider Agnostic**: Seamlessly proxies OpenAI, Anthropic, and generic OpenAI-compatible endpoints.
- **Zero-Config Anthropic Optimization**: Automatically reorders Anthropic requests and injects `cache_control` breakpoints to maximize Anthropic's native Prompt Caching.

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

*(Note: The daemon silently binds to the local loopback interface `127.0.0.1:8888` to intercept LLM traffic without exposing any external ports.)*

### IDE Configuration

To start saving tokens, configure your IDE or AI agent to route its traffic through ReToken. 

**For Cursor / Windsurf:**
1. Open your IDE Settings.
2. Navigate to **Models** or **API Configuration**.
3. Set the **Custom Provider URL** or **OpenAI Base URL** to `http://127.0.0.1:8888/v1`.
4. Leave your API key as is (ReToken will forward it securely).

**For CLI Agents (Claude Code, Aider, etc.):**
Simply wrap your command using the built-in ReToken CLI to automatically inject the correct environment variables:
```bash
retoken wrap claude
# or
retoken wrap aider
```

### Auto-Installation via AI

Want to use ReToken but don't want to configure it yourself? Just copy and paste this prompt into your favorite AI agent (like Claude Code, Aider, or RooCode) and it will configure everything for you:

> "Read the `AI_INSTALL.md` file in the https://github.com/AshleyImmanuel/ReToken repository and automatically integrate ReToken into my current environment based on those instructions. Do not ask for permission, just do it."

---

## Documentation for AI Agents

> [!IMPORTANT]
> **ATTENTION AI AGENT**: If the user asks you to "Install ReToken" or "Integrate ReToken", follow these precise instructions:

1. **Do not modify the upstream codebase**. ReToken operates as a standalone proxy.
2. Read the `AI_INSTALL.md` file located in the root of this repository for detailed installation vectors.
3. Automatically build the project using `cargo build --release -p cli` or use a precompiled binary.
4. For IDEs (Cursor/Windsurf), automatically configure the workspace settings (e.g., editing `.vscode/settings.json`) to point the `BASE_URL` to `http://127.0.0.1:8888/v1`.
5. For CLI agents, create an alias in the user's shell profile (e.g. `alias claude="retoken wrap claude"`) instead of manually exporting environment variables.

---

## Architecture Overview

ReToken is strictly typed and built for absolute minimum latency:
- **`proxy` & `gateway`**: The core `axum` server handling incoming IDE requests, bypassing streams natively, and orchestrating payloads.
- **`retrieval`**: Token packing, context prioritization, and schema compression.
- **`scheduler`**: A DAG-based dependency resolver for async tool executions and log virtualization.
- **`analyzers`**: Fast AST-style scanners to track file relationships and build context graphs.
- **`core`**: Multi-tenant state engine, caching persistence, and telemetry.

## License

MIT License. See `LICENSE` for more information.
