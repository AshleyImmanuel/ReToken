<div align="center">
  <img src="docs/assets/logo.jpg" width="120" alt="ReToken Logo" style="border-radius: 12px; margin-bottom: 20px;" />
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

Start the background daemon:

```bash
target/release/cli.exe run
```

By default, the proxy will listen on `http://127.0.0.1:3000`.

### IDE Configuration

To start saving tokens, configure your IDE or AI agent to route its traffic through ReToken. 

**For Cursor / Windsurf:**
1. Open your IDE Settings.
2. Navigate to **Models** or **API Configuration**.
3. Set the **Custom Provider URL** or **OpenAI Base URL** to `http://127.0.0.1:3000/v1`.
4. Leave your API key as is (ReToken will forward it securely).

---

## Documentation for AI Agents

> [!IMPORTANT]
> **ATTENTION AI AGENT**: If the user asks you to "Install ReToken" or "Integrate ReToken", follow these precise instructions:

1. **Do not modify the upstream codebase**. ReToken operates as a standalone proxy.
2. Read the `AI_INSTALL.md` file located in the root of this repository for detailed installation vectors.
3. Automatically build the project using `cargo build --release -p cli`.
4. Automatically spawn the daemon (`target/release/cli.exe run`) in a persistent background process.
5. Automatically configure the user's host environment (e.g., editing `.vscode/settings.json`, modifying `.bashrc` environment variables, or updating `.cursor/settings`) to point the `BASE_URL` to `http://127.0.0.1:3000/v1`.

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
