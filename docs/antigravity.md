# Using ReToken with Google Antigravity

The Antigravity app sends model requests through Google's own service, and it has no setting for a custom endpoint, so ReToken cannot sit in front of its models as a proxy. What ReToken can do is keep noisy command output out of the conversation. Every turn re-sends the conversation, so output that never enters it is never paid for again.

This works on any Antigravity plan and needs no change to how you sign in.

## Setup

**1. Install ReToken** and check that `retoken --version` runs in the terminal Antigravity uses. See the [main README](../README.md#quick-start).

**2. Add the rule and skill to your project:**

```bash
retoken init --antigravity
```

This writes `.agents/rules/retoken.md`, an always-on rule that tells the agent to run builds, tests, installs and linters as `retoken exec -- <command>`. It never overwrites an existing file. It also writes the Claude Code skill (`.claude/skills/retoken/SKILL.md`).

**3. Register the retrieval tool** so the agent can fetch anything `retoken exec` removed. Add this to `~/.gemini/config/mcp_config.json` (all projects) or `.agents/mcp_config.json` (this project only):

```json
{
  "mcpServers": {
    "retoken": { "command": "retoken", "args": ["mcp"] }
  }
}
```

Restart Antigravity, or reload the servers from Agent panel → … → MCP Servers.

## What `retoken exec` does

```bash
retoken exec -- cargo test
retoken exec -- npm run build
retoken exec -- sh -c "make 2>&1 | tee build.log"   # pipes and redirects need a shell
```

- Output under 2 KB or 15 lines is printed unchanged.
- Longer output keeps its first 8 and last 15 lines plus every error, failure, warning, stack frame and test-summary line, with a few lines of context after each failure. Removed runs become `... [N] lines elided (retoken) ...`.
- It only shortens output when that removes at least 25%. Otherwise it prints the original.
- The full original is saved to `<data dir>/ccr` before anything is printed, and a `[ReToken: ...]` marker gives its handle. Originals that have not been stored again for 7 days are deleted the next time `retoken exec` stores one, or hourly by the proxy if it is running.
- The exit code is the command's own, so the agent still sees failures.
- stdout and stderr are merged in arrival order, and output is printed when the command finishes, not while it runs.

The same thresholds apply to the proxy. Override them with `RETOKEN_CCR_SIZE_THRESHOLD` (bytes) and `RETOKEN_LOG_LINE_THRESHOLD` (lines).

**Example:** this repository's `cargo test --workspace` prints 263 lines (13,954 bytes). Through `retoken exec` it prints 72 lines (3,202 bytes), and all 15 `test result` lines are kept. That is one command's output, not a measurement of a whole Antigravity session.

## Other ways to use less quota

- Use Gemini 4 Hydrogen for everyday edits and keep Neon or Carbon for harder tasks.
- Lower the thinking level for routine work.
- Start a new conversation when you change task; long threads re-send everything each turn.
- Keep always-on rules short. Antigravity sends them with every request and caps them at 20,000 tokens in total.

## Antigravity CLI with a Gemini API key

The Antigravity CLI (`agy`) can use a Gemini API key and a custom Gemini endpoint, and ReToken's proxy speaks the Gemini API, so the CLI gets the full proxy treatment there:

1. Set `"modelProvider": "gemini"` in `~/.gemini/antigravity-cli/settings.json` and export `GEMINI_API_KEY`.
2. Register `retoken mcp` in `~/.gemini/config/mcp_config.json` as shown above.
3. Run `retoken wrap agy`. With `GEMINI_API_KEY` set, `wrap` sets `GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8888`.

This bills your API key per token, not your plan's quota. Gemini support is tested against a mock Gemini server, not yet against the live API.

Do not route your Antigravity sign-in through third-party proxies. Google's terms reportedly allow account suspension for that.
