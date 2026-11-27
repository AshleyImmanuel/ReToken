"""Benchmark arms (direct, ReToken, ReToken + terse) and the proxy each one needs."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import time
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional


@dataclass(frozen=True)
class Arm:
    name: str
    label: str
    via_retoken: bool
    terse: bool = False
    #: Give the agent ReToken's ``retoken_retrieve`` MCP tool, as the README quick start does.
    retrieval_tool: bool = False


ARMS: Dict[str, Arm] = {
    "direct": Arm("direct", "Direct to the API", via_retoken=False),
    "retoken": Arm("retoken", "ReToken", via_retoken=True, retrieval_tool=True),
    "retoken-terse": Arm(
        "retoken-terse", "ReToken + terse mode", via_retoken=True, terse=True, retrieval_tool=True
    ),
}
DEFAULT_ARMS = ["direct", "retoken", "retoken-terse"]


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _port_open(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
            return True
    except OSError:
        return False


class RetokenProxy:
    """A private ``retoken run`` for one benchmark run.

    Each run gets its own port, data directory and empty working directory (so
    no ``retoken.json`` from elsewhere is picked up). The user's own daemon on
    8888 is never touched, and stats are per run.
    """

    def __init__(
        self,
        retoken_bin: str,
        arm: Arm,
        run_dir: Path,
        upstream_override: Optional[str] = None,
    ) -> None:
        self.retoken_bin = retoken_bin
        self.arm = arm
        self.port = free_port()
        self.data_dir = run_dir / "retoken-data"
        self.home = run_dir / "retoken-cwd"
        self.log_path = run_dir / "retoken.log"
        self.upstream_override = upstream_override
        self._process: Optional[subprocess.Popen] = None
        self._log = None

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def env(self) -> Dict[str, str]:
        """Settings shared by the proxy, ``retoken wrap`` and ``retoken mcp``."""
        env = {
            "RETOKEN_PORT": str(self.port),
            "RETOKEN_DATA_DIR": str(self.data_dir),
            "RETOKEN_TERSE_MODE": "true" if self.arm.terse else "false",
        }
        if self.upstream_override:
            # Dry runs only: send provider traffic to the local mock instead of Anthropic.
            env["RETOKEN_ANTHROPIC_BASE_URL"] = self.upstream_override
            env["RETOKEN_ALLOW_PRIVATE_UPSTREAMS"] = "true"
        return env

    def __enter__(self) -> "RetokenProxy":
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.home.mkdir(parents=True, exist_ok=True)
        env = {k: v for k, v in os.environ.items() if not k.startswith("RETOKEN_")}
        env.update(self.env())
        env.setdefault("RUST_LOG", "warn")
        self._log = open(self.log_path, "wb")
        self._process = subprocess.Popen(
            [self.retoken_bin, "run"], cwd=self.home, env=env,
            stdout=self._log, stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self._process.poll() is not None:
                raise RuntimeError(f"retoken run exited early; see {self.log_path}")
            if _port_open(self.port):
                return self
            time.sleep(0.1)
        self.__exit__()
        raise RuntimeError(f"retoken run did not listen on {self.port} within 15s")

    def stats(self) -> Dict[str, Any]:
        try:
            with urllib.request.urlopen(f"{self.url}/_retoken/stats", timeout=5) as response:
                return json.loads(response.read())
        except (OSError, ValueError) as exc:
            return {"error": str(exc)}

    def __exit__(self, *exc: Any) -> None:
        if self._process is not None and self._process.poll() is None:
            self._process.terminate()
            try:
                self._process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self._process.kill()
        self._process = None
        if self._log is not None:
            self._log.close()
            self._log = None

    def mcp_config(self, path: Path) -> Path:
        """Write a Claude Code ``--mcp-config`` file for ``retoken mcp``."""
        config = {
            "mcpServers": {
                "retoken": {
                    "command": self.retoken_bin,
                    "args": ["mcp"],
                    "env": {"RETOKEN_DATA_DIR": str(self.data_dir)},
                }
            }
        }
        path.write_text(json.dumps(config, indent=2), encoding="utf-8")
        return path


STATS_KEYS: List[str] = [
    "total_requests",
    "total_errors",
    "total_bytes_transferred",
    "total_bytes_saved",
    "total_input_tokens",
    "total_output_tokens",
    "total_cached_tokens",
    "terse_mode",
]
