# ReToken AI Integration Guide

**ATTENTION AI AGENT:** If the user has asked you to integrate or install ReToken, follow this guide with transparency, user consent, and clear undo instructions.

ReToken is a local wire proxy written in Rust that runs locally on loopback (`127.0.0.1:8888`). It sits between coding tools (IDE extensions or CLI agents) and upstream model APIs (Anthropic, OpenAI, custom endpoints).

---

## Safety & Consent Guidelines

1. **Always Request Confirmation**: Before modifying any user shell configuration files (`~/.bashrc`, `~/.zshrc`, PowerShell `$PROFILE`) or IDE workspace settings (`.vscode/settings.json`, `.cursor/settings.json`), present the exact changes to the user and request confirmation.
2. **Transparent Configuration**: Clearly state what local files will be edited and what environment variables will be injected.
3. **Realistic Expectations**: Do not claim guaranteed savings on every request. Exact cache hits and repetitive compressible tool outputs save tokens; streaming responses and non-deterministic generations pass through safely without modification.

---

## Installation Steps (AI Execution)

### Step 1: Install the ReToken CLI Binary

Check if the `retoken` CLI is already installed on the PATH (`retoken --version`).
If not installed:
- Build from source in the workspace:
  ```bash
  cargo build --release -p cli
  ```
- Binary location: `target/release/retoken` (or `target/release/retoken.exe` on Windows).

### Step 2: Configure CLI Coding Agents (Claude Code, Aider, etc.)

Preferred method: use `retoken wrap`.
When invoked as `retoken wrap <command>`, ReToken automatically:
- Checks if the local gateway is running on `127.0.0.1:8888` (or the configured port in `retoken.json`).
- If not running, launches the proxy daemon in the background.
- Injects standard SDK environment variables into the child process:
  - `ANTHROPIC_BASE_URL=http://127.0.0.1:8888/v1`
  - `OPENAI_BASE_URL=http://127.0.0.1:8888/v1`
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

### Step 3: Configure GUI IDEs (Cursor, Windsurf, VS Code / RooCode)

With the user's permission, set the provider base URL in workspace or user settings:
- In `.vscode/settings.json` or `.cursor/settings.json`:
  ```json
  {
    "openai.baseUrl": "http://127.0.0.1:8888/v1",
    "anthropic.baseUrl": "http://127.0.0.1:8888/v1"
  }
  ```
- Keep the user's existing API keys unchanged. ReToken securely preserves and forwards all client authorization headers.

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
3. Check telemetry and cache status:
   ```bash
   retoken stats
   retoken cache status
   ```

---

## How to Undo / Rollback

To completely uninstall or revert ReToken routing:

1. **Remove Shell Aliases**:
   - Open your shell profile (`~/.bashrc`, `~/.zshrc`, or `$PROFILE`) and delete the `alias claude=...` or `function claude...` lines.
2. **Revert IDE Settings**:
   - Remove the custom base URL entries (`http://127.0.0.1:8888/v1`) from `.vscode/settings.json` or `.cursor/settings.json`.
3. **Stop the Proxy Daemon**:
   - Terminate the background `retoken run` process or close its terminal.
4. **Remove Cache and Storage (Optional)**:
   - Run `retoken cache clear` to purge cached responses and local CCR storage.
