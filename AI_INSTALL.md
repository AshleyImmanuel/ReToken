# ReToken AI Integration Guide

**ATTENTION AI AGENT:** If the user has asked you to integrate or install ReToken, follow this guide with transparency, user consent, and clear undo instructions.

ReToken is a local wire proxy written in Rust that runs locally on loopback (`127.0.0.1:8888`). It sits between coding tools (IDE extensions or CLI agents) and upstream model APIs (Anthropic, OpenAI, custom endpoints).

---

## Safety & Consent Guidelines

1. **Always Request Confirmation**: Before modifying any user shell configuration files (`~/.bashrc`, `~/.zshrc`, PowerShell `$PROFILE`), agent configuration (`claude mcp add`, `opencode.jsonc`), or IDE workspace settings (`.vscode/settings.json`, `.cursor/settings.json`), present the exact changes to the user and request confirmation.
2. **Transparent Configuration**: Clearly state what local files will be edited and what environment variables will be injected.
3. **Realistic Expectations**: Do not claim guaranteed savings on every request. Savings come from large or repeated tool output (logs, JSON, duplicate reads), prompt-cache placement, and optional terse replies. Short sessions with small tool output save little; nothing is changed that the model is still reading.

---

## Installation Steps (AI Execution)

Do the steps in order. Step 3 is what lets ReToken compress the most; without it, only lossless compression is applied.

### Step 1: Install the ReToken CLI Binary

Check if the `retoken` CLI is already installed on the PATH (`retoken --version`).
If not installed:
- Build from source in the workspace:
  ```bash
  cargo build --release -p cli
  ```
- Binary location: `target/release/retoken` (or `target/release/retoken.exe` on Windows). Ask the user before copying it onto the PATH (for example `~/.cargo/bin`).

### Step 2: Route the Agent Through the Proxy

Preferred method for CLI agents: `retoken wrap`.
When invoked as `retoken wrap <command>`, ReToken automatically:
- Checks if the local gateway is running on `127.0.0.1:8888` (or the configured port).
- If not running, launches the proxy daemon in the background.
- Injects standard SDK environment variables into the child process:
  - `ANTHROPIC_BASE_URL=http://127.0.0.1:8888/v1`
  - `OPENAI_BASE_URL=http://127.0.0.1:8888/v1` (plus `OPENAI_API_BASE`, `GROQ_BASE_URL`, `MISTRAL_API_BASE`, `LITELLM_BASE_URL`)
  - Gemini clients are not redirected: the proxy does not support the Gemini API format yet.
  - `_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL=1`, so Claude Code keeps its first-party features (prompt caching, tool search, long context) through a local base URL.
  - Any additional variables passed via `--inject <VAR>`.

To make this convenient for the user, ask for permission to add a shell alias:
- Bash/Zsh (`~/.bashrc` or `~/.zshrc`):
  ```bash
  alias claude="retoken wrap claude"
  alias aider="retoken wrap aider"
  ```
- PowerShell (`$PROFILE`):
  ```powershell
  function claude { retoken wrap claude @args }
  function aider { retoken wrap aider @args }
  ```

Alternative for Claude Code without a wrapper: with permission, add the same two variables to the `env` block of `~/.claude/settings.json` (start the proxy separately with `retoken run`):
```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8888",
    "_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL": "1"
  }
}
```
If `ANTHROPIC_BASE_URL` is already set (another proxy), tell the user and ask which one to keep. Do not chain proxies silently.

GUI IDEs (Cursor, Windsurf, VS Code / RooCode): with the user's permission, set the provider base URL in workspace or user settings:
- In `.vscode/settings.json` or `.cursor/settings.json`:
  ```json
  {
    "openai.baseUrl": "http://127.0.0.1:8888/v1",
    "anthropic.baseUrl": "http://127.0.0.1:8888/v1"
  }
  ```
- Keep the user's existing API keys unchanged. ReToken forwards client authorization headers to the upstream.

### Step 3: Register the Retrieval Tool (MCP)

ReToken only shortens tool output in a lossy way (large logs, large JSON, outlines of large files the agent has moved past) when the model can get the original back. The `retoken mcp` server provides that route as the `retoken_retrieve` tool. Agents stream their responses, so this MCP server is the recovery route for Claude Code, opencode and Cursor.

With the user's permission:
- Claude Code:
  ```bash
  claude mcp add retoken -- retoken mcp
  ```
- opencode (`opencode.jsonc`):
  ```jsonc
  "mcp": { "retoken": { "type": "local", "command": ["retoken", "mcp"] } }
  ```
- Any other MCP client: a stdio server with command `retoken` and argument `mcp`.

ReToken finds the tool in each request by its name ending in `retoken_retrieve`. This works when the client defers MCP tools behind tool search (Claude Code with `ENABLE_TOOL_SEARCH`): the tool counts when it is listed in the deferred-tools reminder and a `ToolSearch` tool is present, and elision markers tell the model to load it with ToolSearch first. If the proxy and the MCP server use a custom data directory, set the same `RETOKEN_DATA_DIR` for both.

### Step 4: Choose Settings (Optional)

Defaults suit most users. Settings come from `retoken.json`, `.retoken.json` or `.retoken/config.json` in the directory where `retoken run` starts, and environment variables override the file.

| Setting (file key / env var) | Default | Effect |
|---|---|---|
| `terse_mode` / `RETOKEN_TERSE_MODE` | `false` | Asks the model for terse prose replies (code, commands and tool inputs stay normal). Lowest cost and fewest turns in our benchmark. Ask the user: it changes the assistant's writing style. |
| `compression_enabled` / `RETOKEN_COMPRESSION_ENABLED` | `true` | Tool-output compression (deduplication always; lossy elision only with Step 3). |
| `prompt_caching_enabled` / `RETOKEN_PROMPT_CACHING_ENABLED` | `true` | Anthropic cache breakpoint placement. Respects the client's breakpoints and TTL. |
| `cache_enabled` / `RETOKEN_CACHE_ENABLED` | `true` | Exact-match response cache for identical non-streaming, deterministic requests. |
| `cache_ttl_secs` / `RETOKEN_CACHE_TTL_SECS` | `300` | Response cache lifetime. |
| `ccr_enabled` / `RETOKEN_CCR_ENABLED` | `false` | Proxy-side recovery for non-streaming Anthropic requests without an MCP retrieval tool. |
| `port` / `RETOKEN_PORT` | `8888` | Listen port (always loopback). Update base URLs if changed. |
| env only: `RETOKEN_DATA_DIR` | `%LOCALAPPDATA%\ReToken` (Windows), `~/.retoken` (macOS/Linux) | Where elided originals are stored (`ccr/`). |
| env only: `RETOKEN_CCR_SIZE_THRESHOLD` | `2048` | Minimum bytes before command output can be elided. |

Example `retoken.json`:
```json
{ "terse_mode": true, "port": 8888 }
```

Elided originals are raw tool output (file contents, command logs). They are stored under `<data dir>/ccr` (an owner-only directory on macOS/Linux; per-user `%LOCALAPPDATA%` on Windows). Entries no conversation has used for 7 days are deleted every hour.

---

## How to Verify Integration

1. Start the proxy if not running:
   ```bash
   retoken run
   ```
2. Run diagnostic doctor check:
   ```bash
   retoken doctor
   ```
3. Check the retrieval tool is registered (Claude Code): `claude mcp list` shows `retoken`.
4. Check telemetry and cache status:
   ```bash
   retoken stats
   retoken cache status
   ```
5. After a session with tool use, the proxy log shows lines such as `Pipeline: ... | logs=1 json=0 dups=0 outlines=0 ccr=1`. `ccr` greater than 0 means the retrieval route was detected and originals were stored.

---

## How to Undo / Rollback

To completely uninstall or revert ReToken routing:

1. **Remove Shell Aliases**:
   - Open your shell profile (`~/.bashrc`, `~/.zshrc`, or `$PROFILE`) and delete the `alias claude=...` or `function claude...` lines.
2. **Revert Agent Settings**:
   - Remove the `ANTHROPIC_BASE_URL` and `_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL` entries from `~/.claude/settings.json` if you added them.
   - Remove the MCP server: `claude mcp remove retoken`, or delete the `retoken` entry from `opencode.jsonc`.
3. **Revert IDE Settings**:
   - Remove the custom base URL entries (`http://127.0.0.1:8888/v1`) from `.vscode/settings.json` or `.cursor/settings.json`.
4. **Stop the Proxy Daemon**:
   - Terminate the background `retoken run` process or close its terminal.
5. **Remove Cache and Storage (Optional)**:
   - Run `retoken cache clear` to purge the live gateway's in-memory response cache.
   - Delete `<data dir>/ccr` (see Step 4) to remove stored originals of elided tool output.
