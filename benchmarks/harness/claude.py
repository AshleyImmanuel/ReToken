"""Running headless Claude Code for one benchmark run and reading its usage."""

from __future__ import annotations

import json
import os
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional

# Variables kept from the caller's environment. Everything else that starts with
# CLAUDE / ANTHROPIC / RETOKEN is dropped so a run started inside another agent
# session (or with a globally exported base URL) behaves like a clean terminal.
_KEEP = {"ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CONFIG_DIR"}
_DROP_PREFIXES = ("CLAUDE", "ANTHROPIC", "RETOKEN", "_CLAUDE")

# Same context for every arm and every machine: no CLAUDE.md / AGENTS.md from the
# user's home or parent folders, and no auto-memory.
ISOLATION_ENV = {
    "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1",
    "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
    "DISABLE_AUTOUPDATER": "1",
}


def clean_env() -> Dict[str, str]:
    env = {
        key: value
        for key, value in os.environ.items()
        if key in _KEEP or not key.upper().startswith(_DROP_PREFIXES)
    }
    env.update(ISOLATION_ENV)
    env["PYTHONIOENCODING"] = "utf-8"
    return env


def build_args(
    prompt: str,
    model: str,
    max_turns: int,
    max_budget_usd: Optional[float],
    mcp_config: Optional[Path],
    extra: List[str],
) -> List[str]:
    """Claude Code arguments shared by every arm."""
    args = [
        "-p", prompt,
        "--output-format", "json",
        "--model", model,
        "--max-turns", str(max_turns),
        "--permission-mode", "bypassPermissions",
        "--no-session-persistence",
        # Ignore ~/.claude/settings.json (its env, plugins and hooks) and any MCP
        # servers except the one an arm passes explicitly.
        "--setting-sources", "project",
        "--strict-mcp-config",
    ]
    if mcp_config is not None:
        args += ["--mcp-config", str(mcp_config)]
    if max_budget_usd is not None:
        args += ["--max-budget-usd", f"{max_budget_usd:.2f}"]
    return args + list(extra)


@dataclass
class AgentResult:
    exit_code: int
    wall_seconds: float
    raw: Dict[str, Any]
    stdout: str
    stderr: str
    timed_out: bool = False

    @property
    def answer(self) -> str:
        result = self.raw.get("result")
        return result if isinstance(result, str) else ""


def run_agent(command: List[str], env: Dict[str, str], cwd: Path, timeout: int) -> AgentResult:
    started = time.monotonic()
    try:
        proc = subprocess.run(
            command, cwd=cwd, env=env, capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=timeout, stdin=subprocess.DEVNULL,
        )
        stdout, stderr, code, timed_out = proc.stdout, proc.stderr, proc.returncode, False
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
        code, timed_out = -1, True
    wall = time.monotonic() - started
    return AgentResult(code, wall, _last_json_object(stdout), stdout, stderr, timed_out)


def _last_json_object(text: str) -> Dict[str, Any]:
    text = text.strip()
    if not text:
        return {}
    try:
        value = json.loads(text)
        return value if isinstance(value, dict) else {}
    except ValueError:
        pass
    for line in reversed(text.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                value = json.loads(line)
                if isinstance(value, dict):
                    return value
            except ValueError:
                continue
    return {}


@dataclass
class Usage:
    cost_usd: float = 0.0
    turns: int = 0
    api_seconds: float = 0.0
    input_tokens: int = 0
    output_tokens: int = 0
    cache_read_tokens: int = 0
    cache_write_tokens: int = 0
    models: List[str] = field(default_factory=list)


def read_usage(raw: Dict[str, Any]) -> Usage:
    """Totals across every model the run used (main model plus any helper model)."""
    usage = Usage(
        cost_usd=float(raw.get("total_cost_usd") or 0.0),
        turns=int(raw.get("num_turns") or 0),
        api_seconds=float(raw.get("duration_api_ms") or 0) / 1000.0,
    )
    per_model = raw.get("modelUsage") or {}
    if per_model:
        for model, item in sorted(per_model.items()):
            usage.models.append(model)
            usage.input_tokens += int(item.get("inputTokens") or 0)
            usage.output_tokens += int(item.get("outputTokens") or 0)
            usage.cache_read_tokens += int(item.get("cacheReadInputTokens") or 0)
            usage.cache_write_tokens += int(item.get("cacheCreationInputTokens") or 0)
    else:
        totals = raw.get("usage") or {}
        usage.input_tokens = int(totals.get("input_tokens") or 0)
        usage.output_tokens = int(totals.get("output_tokens") or 0)
        usage.cache_read_tokens = int(totals.get("cache_read_input_tokens") or 0)
        usage.cache_write_tokens = int(totals.get("cache_creation_input_tokens") or 0)
    return usage
