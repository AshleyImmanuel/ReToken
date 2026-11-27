"""Pinned benchmark tasks: loading, clean workspaces and pass/fail checks."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

# Fixed identity and timestamps make the fixture commit hash identical on every run.
_GIT_ENV = {
    "GIT_AUTHOR_NAME": "retoken-bench",
    "GIT_AUTHOR_EMAIL": "bench@localhost",
    "GIT_COMMITTER_NAME": "retoken-bench",
    "GIT_COMMITTER_EMAIL": "bench@localhost",
    "GIT_AUTHOR_DATE": "2026-01-01T00:00:00+00:00",
    "GIT_COMMITTER_DATE": "2026-01-01T00:00:00+00:00",
}
# This throwaway repository lives in a temp directory; a user's global signing or
# line-ending settings must not change its contents or block the commit.
_GIT_CONFIG = ["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false", "-c", "core.safecrlf=false"]

CHECK_TIMEOUT_SECS = 300


@dataclass(frozen=True)
class Task:
    id: str
    suite: str
    title: str
    prompt: str
    max_turns: int
    est_cost_usd: float
    check: Dict[str, Any]
    directory: Path
    overlay: Optional[Path] = None

    @property
    def reference(self) -> Path:
        return self.directory / "reference"


@dataclass
class CheckResult:
    passed: bool
    detail: str
    changed_files: List[str] = field(default_factory=list)


def load_tasks(tasks_dir: Path, wanted: Optional[Sequence[str]] = None) -> List[Task]:
    """Load ``tasks/*/task.json`` in name order, optionally only ``wanted`` ids."""
    tasks: List[Task] = []
    for spec_path in sorted(tasks_dir.glob("*/task.json")):
        spec = json.loads(spec_path.read_text(encoding="utf-8"))
        directory = spec_path.parent
        overlay = directory / spec["overlay"] if spec.get("overlay") else None
        tasks.append(
            Task(
                id=spec["id"],
                suite=spec.get("suite", "core"),
                title=spec.get("title", spec["id"]),
                prompt=spec["prompt"],
                max_turns=int(spec.get("max_turns", 30)),
                est_cost_usd=float(spec.get("est_cost_usd", 0.05)),
                check=spec["check"],
                directory=directory,
                overlay=overlay,
            )
        )
    if wanted:
        by_id = {t.id: t for t in tasks}
        unknown = [w for w in wanted if w not in by_id]
        if unknown:
            raise SystemExit(f"unknown task(s): {', '.join(unknown)}; have {', '.join(by_id)}")
        tasks = [by_id[w] for w in wanted]
    return tasks


def _git(workspace: Path, *args: str) -> str:
    env = dict(os.environ, **_GIT_ENV)
    result = subprocess.run(
        ["git", *_GIT_CONFIG, *args],
        cwd=workspace, env=env, capture_output=True, text=True, encoding="utf-8", check=True,
    )
    return result.stdout


def _copy_tree(source: Path, dest: Path) -> None:
    shutil.copytree(source, dest, dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))


def prepare_workspace(task: Task, fixture: Path, dest: Path) -> Path:
    """Copy the fixture (plus the task's overlay) into ``dest`` as a fresh git repo."""
    if dest.exists():
        shutil.rmtree(dest)
    _copy_tree(fixture, dest)
    if task.overlay is not None:
        _copy_tree(task.overlay, dest)
    _git(dest, "init", "-q", "-b", "main")
    _git(dest, "add", "-A")
    _git(dest, "commit", "-q", "-m", "fixture")
    return dest


def changed_files(workspace: Path) -> List[str]:
    """Paths that differ from the fixture commit, including new untracked files."""
    out = _git(workspace, "status", "--porcelain", "--untracked-files=all")
    return sorted(line[3:].strip() for line in out.splitlines() if line.strip())


def _check_answer(spec: Dict[str, Any], workspace: Path, answer: str) -> CheckResult:
    missing = [p for p in spec.get("patterns", []) if not re.search(p, answer, re.IGNORECASE)]
    changed = changed_files(workspace)
    problems = []
    if missing:
        problems.append("answer is missing: " + "; ".join(missing))
    if spec.get("forbid_changes") and changed:
        problems.append("files were modified: " + ", ".join(changed))
    if not answer.strip():
        problems.append("empty answer")
    passed = not problems
    return CheckResult(passed, "all answer checks matched" if passed else " | ".join(problems), changed)


def _check_tests(task: Task, spec: Dict[str, Any], workspace: Path, fixture: Path) -> CheckResult:
    changed = changed_files(workspace)
    # Put the original tests back so an agent cannot pass by editing them...
    for rel in spec.get("restore", []):
        target = workspace / rel
        if target.exists():
            shutil.rmtree(target) if target.is_dir() else target.unlink()
        source = fixture / rel
        if source.is_dir():
            _copy_tree(source, target)
        elif source.exists():
            shutil.copy2(source, target)
    # ...then add the acceptance tests the agent never saw.
    if spec.get("hidden"):
        _copy_tree(task.directory / spec["hidden"], workspace)
    command = [sys.executable if part == "{python}" else part for part in spec["command"]]
    try:
        result = subprocess.run(
            command, cwd=workspace, capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=CHECK_TIMEOUT_SECS,
        )
    except subprocess.TimeoutExpired:
        return CheckResult(False, f"check timed out after {CHECK_TIMEOUT_SECS}s", changed)
    tail = "\n".join((result.stdout + result.stderr).strip().splitlines()[-12:])
    return CheckResult(result.returncode == 0, tail, changed)


def run_check(task: Task, workspace: Path, answer: str, fixture: Path) -> CheckResult:
    kind = task.check.get("type")
    if kind == "answer":
        return _check_answer(task.check, workspace, answer)
    if kind == "tests":
        return _check_tests(task, task.check, workspace, fixture)
    raise ValueError(f"task {task.id}: unknown check type {kind!r}")


def apply_reference(task: Task, workspace: Path) -> str:
    """Apply the task's known-good solution; returns the model answer if it has one."""
    answer_file = task.reference / "answer.md"
    for item in task.reference.iterdir() if task.reference.is_dir() else []:
        if item.name == "answer.md":
            continue
        if item.is_dir():
            _copy_tree(item, workspace / item.name)
        else:
            shutil.copy2(item, workspace / item.name)
    return answer_file.read_text(encoding="utf-8") if answer_file.exists() else ""
