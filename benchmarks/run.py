#!/usr/bin/env python3
"""Reproducible cost benchmark: headless Claude Code, direct vs through ReToken.

    python benchmarks/run.py --self-test            # check fixtures and checks (free)
    python benchmarks/run.py --dry-run              # whole pipeline against a mock API (free)
    python benchmarks/run.py --estimate             # print what a real run would cost
    python benchmarks/run.py --tasks explain-code --arms direct --repeats 1   # one cheap paid run
    python benchmarks/run.py                        # full run: 4 tasks x 3 arms x 3 repeats

See benchmarks/README.md. Real runs spend API money (or subscription usage) and
ask for confirmation first unless --yes is given. Standard library only.
"""

from __future__ import annotations

import argparse
import json
import math
import platform
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from harness import claude as agent  # noqa: E402
from harness.arms import ARMS, DEFAULT_ARMS, STATS_KEYS, Arm, RetokenProxy  # noqa: E402
from harness.mock_upstream import MockUpstream  # noqa: E402
from harness.report import aggregate, render_markdown  # noqa: E402
from harness.tasks import Task, apply_reference, load_tasks, prepare_workspace, run_check  # noqa: E402

FIXTURE = HERE / "fixture"
TASKS_DIR = HERE / "tasks"
RESULTS_DIR = HERE / "results"
DRY_RUN_TOKEN = "retoken-bench-dry-run-placeholder"  # sent only to the local mock


# --------------------------------------------------------------------- helpers
def _now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _which(name: str, override: Optional[str]) -> str:
    found = shutil.which(override or name)
    if not found:
        raise SystemExit(f"cannot find `{override or name}` on PATH (pass --{name} /path/to/{name})")
    return found


def _version(command: List[str]) -> str:
    try:
        out = subprocess.run(command, capture_output=True, text=True, timeout=30)
        return (out.stdout or out.stderr).strip().splitlines()[0]
    except (OSError, subprocess.SubprocessError, IndexError):
        return "unknown"


def _retoken_commit() -> str:
    try:
        sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=HERE, capture_output=True,
                             text=True, check=True).stdout.strip()
        dirty = subprocess.run(["git", "status", "--porcelain", "--", "."], cwd=HERE.parent,
                               capture_output=True, text=True).stdout.strip()
        return sha + ("-dirty" if dirty else "")
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def budget_for(task: Task, override: Optional[float]) -> float:
    """Per-run spending cap passed to Claude Code (5x the task's estimate by default)."""
    if override is not None:
        return override
    return max(0.25, math.ceil(task.est_cost_usd * 5 * 100) / 100)


def plan_runs(tasks: List[Task], arms: List[str], repeats: int) -> List[Dict[str, Any]]:
    """Interleave arms so provider-side caching and time-of-day drift hit every arm alike."""
    runs = []
    for repeat in range(1, repeats + 1):
        for t_index, task in enumerate(tasks):
            shift = (repeat - 1 + t_index) % len(arms)
            for arm in arms[shift:] + arms[:shift]:
                runs.append({"task": task, "arm": ARMS[arm], "repeat": repeat})
    return runs


def estimate(runs: List[Dict[str, Any]], budget_override: Optional[float]) -> Dict[str, float]:
    expected = sum(r["task"].est_cost_usd for r in runs)
    cap = sum(budget_for(r["task"], budget_override) for r in runs)
    return {"runs": len(runs), "expected_usd": expected, "cap_usd": cap}


def print_estimate(runs: List[Dict[str, Any]], budget_override: Optional[float]) -> Dict[str, float]:
    est = estimate(runs, budget_override)
    per_task: Dict[str, List[Any]] = {}
    for r in runs:
        entry = per_task.setdefault(r["task"].id, [0, r["task"].est_cost_usd, budget_for(r["task"], budget_override)])
        entry[0] += 1
    print("Planned paid runs:")
    for task_id, (count, est_cost, cap) in per_task.items():
        print(f"  {task_id:<24} {count:>3} runs x ~${est_cost:.3f}  (cap ${cap:.2f} per run)")
    print(f"  total: {est['runs']} runs, expected ~${est['expected_usd']:.2f}, "
          f"hard ceiling ${est['cap_usd']:.2f} (sum of per-run --max-budget-usd caps)")
    print("  Estimates come from the November 2026 runs (direct arm, Sonnet); ReToken arms "
          "usually cost less. On a Claude subscription this is usage, not a bill.")
    return est


# ------------------------------------------------------------------ one run
def run_one(
    index: int,
    plan: Dict[str, Any],
    args: argparse.Namespace,
    claude_bin: str,
    retoken_bin: Optional[str],
    work_root: Path,
    out_dir: Path,
    mock: Optional[MockUpstream],
) -> Dict[str, Any]:
    task: Task = plan["task"]
    arm: Arm = plan["arm"]
    run_id = f"{index:03d}-{task.id}-{arm.name}-r{plan['repeat']}"
    scratch = work_root / run_id
    scratch.mkdir(parents=True, exist_ok=True)
    record: Dict[str, Any] = {
        "run_id": run_id, "task": task.id, "suite": task.suite, "arm": arm.name,
        "repeat": plan["repeat"], "started_at": _now(), "model_requested": args.model,
    }
    try:
        workspace = prepare_workspace(task, FIXTURE, scratch / "workspace")
    except (OSError, subprocess.CalledProcessError) as exc:
        record.update(harness_error=f"workspace setup failed: {exc}", solved=False)
        return record

    env = agent.clean_env()
    if mock is not None:
        env["ANTHROPIC_AUTH_TOKEN"] = DRY_RUN_TOKEN
        env["_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL"] = "1"
    cap = budget_for(task, args.max_budget_usd)

    def launch(proxy: Optional[RetokenProxy]) -> agent.AgentResult:
        mcp = proxy.mcp_config(scratch / "mcp.json") if proxy and arm.retrieval_tool else None
        claude_args = agent.build_args(task.prompt, args.model, task.max_turns, cap, mcp, args.claude_arg)
        if proxy is None:
            if mock is not None:
                env["ANTHROPIC_BASE_URL"] = mock.url
            command = [claude_bin] + claude_args
        else:
            env.update(proxy.env())
            command = [retoken_bin, "wrap", claude_bin] + claude_args
        return agent.run_agent(command, env, workspace, args.timeout)

    stats: Dict[str, Any] = {}
    try:
        if arm.via_retoken:
            with RetokenProxy(retoken_bin, arm, scratch, mock.url if mock else None) as proxy:
                result = launch(proxy)
                full = proxy.stats()
                stats = {k: full.get(k) for k in STATS_KEYS if k in full} or full
            shutil.copy2(scratch / "retoken.log", out_dir / "runs" / f"{run_id}.retoken.log")
        else:
            result = launch(None)
    except Exception as exc:  # a harness failure must not stop the remaining runs
        record.update(harness_error=f"{type(exc).__name__}: {exc}", solved=False)
        return record

    (out_dir / "runs" / f"{run_id}.claude.json").write_text(result.stdout, encoding="utf-8")
    if result.stderr.strip():
        (out_dir / "runs" / f"{run_id}.stderr.txt").write_text(result.stderr, encoding="utf-8")

    usage = agent.read_usage(result.raw)
    check = run_check(task, workspace, result.answer, FIXTURE)
    record.update(
        exit_code=result.exit_code,
        timed_out=result.timed_out,
        result_subtype=result.raw.get("subtype", "no-json-output" if not result.raw else ""),
        is_error=bool(result.raw.get("is_error")) or not result.raw,
        solved=check.passed and not result.timed_out,
        check_detail=check.detail,
        changed_files=check.changed_files,
        cost_usd=usage.cost_usd,
        turns=usage.turns,
        wall_seconds=round(result.wall_seconds, 2),
        api_seconds=round(usage.api_seconds, 2),
        input_tokens=usage.input_tokens,
        output_tokens=usage.output_tokens,
        cache_read_tokens=usage.cache_read_tokens,
        cache_write_tokens=usage.cache_write_tokens,
        models_used=usage.models,
        max_budget_usd=cap,
        retoken_stats=stats or None,
        answer_preview=result.answer[:400],
    )
    if mock is not None:
        record["plumbing"] = _plumbing(arm, mock.drain(), stats)
    if not args.keep_workspaces:
        shutil.rmtree(scratch, ignore_errors=True)
    return record


def _plumbing(arm: Arm, requests: List[Dict[str, Any]], stats: Dict[str, Any]) -> Dict[str, Any]:
    """Dry-run assertions: did each arm's traffic take the path it claims to?"""
    messages = [r for r in requests if r.get("path", "").endswith("/messages")]
    terse = any(r.get("terse_instruction") for r in messages)
    retrieval = any(r.get("retoken_retrieve_tool") for r in messages)
    proxy_requests = stats.get("total_requests") if arm.via_retoken else None
    problems = []
    if not messages:
        problems.append("no /v1/messages request reached the mock")
    if arm.via_retoken and not proxy_requests:
        problems.append("ReToken proxy saw no requests")
    if terse != arm.terse:
        problems.append(f"terse instruction {'missing' if arm.terse else 'present'}")
    if retrieval != arm.retrieval_tool:
        problems.append(f"retoken_retrieve tool {'missing' if arm.retrieval_tool else 'present'}")
    return {
        "mock_messages": len(messages),
        "proxy_requests": proxy_requests if arm.via_retoken else "-",
        "terse_instruction": terse,
        "retrieval_tool": retrieval,
        "cache_breakpoints": max((r.get("cache_breakpoints", 0) for r in messages), default=0),
        "ok": not problems,
        "problem": "; ".join(problems),
    }


# ------------------------------------------------------------------- modes
def self_test(tasks: List[Task]) -> int:
    """Each check must fail on the untouched task and pass on its reference solution."""
    failures = 0
    with tempfile.TemporaryDirectory(prefix="retoken-bench-selftest-") as tmp:
        for task in tasks:
            before = run_check(task, prepare_workspace(task, FIXTURE, Path(tmp) / f"{task.id}-before"), "", FIXTURE)
            ws = prepare_workspace(task, FIXTURE, Path(tmp) / f"{task.id}-after")
            answer = apply_reference(task, ws)
            after = run_check(task, ws, answer, FIXTURE)
            ok = (not before.passed) and after.passed
            failures += not ok
            print(f"{'ok  ' if ok else 'FAIL'} {task.id:<24} untouched: {'pass' if before.passed else 'fail'}"
                  f" (want fail)   reference: {'pass' if after.passed else 'fail'} (want pass)")
            if not after.passed:
                print("     " + after.detail.replace("\n", "\n     "))
    print("self-test " + ("passed" if not failures else f"FAILED ({failures})"))
    return 1 if failures else 0


def write_results(out_dir: Path, meta: Dict[str, Any], records: List[Dict[str, Any]], arms: List[str]) -> Dict[str, Any]:
    meta["models_used"] = sorted({m for r in records for m in r.get("models_used") or []})
    results = {"meta": meta, "summary": aggregate(records, arms), "runs": records}
    (out_dir / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    (out_dir / "results.md").write_text(render_markdown(results), encoding="utf-8")
    return results


def confirm_spend(est: Dict[str, float], assume_yes: bool) -> bool:
    if assume_yes:
        return True
    if not sys.stdin.isatty():
        print("Refusing to start paid runs without a terminal to confirm; pass --yes to proceed.")
        return False
    reply = input(f"Start {int(est['runs'])} real runs (~${est['expected_usd']:.2f}, ceiling "
                  f"${est['cap_usd']:.2f})? Type 'yes' to continue: ")
    return reply.strip().lower() == "yes"


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--tasks", help="comma-separated task ids (default: all)")
    p.add_argument("--arms", default=",".join(DEFAULT_ARMS), help=f"comma-separated, from: {', '.join(ARMS)}")
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--model", default="sonnet", help="Claude Code --model value (default: sonnet)")
    p.add_argument("--max-budget-usd", type=float, help="per-run cap (default: 5x the task estimate, min $0.25)")
    p.add_argument("--timeout", type=int, default=1200, help="seconds before a run is killed")
    p.add_argument("--out", type=Path, help="results directory (default: benchmarks/results/<timestamp>)")
    p.add_argument("--work-dir", type=Path, help="where workspaces are created (default: system temp)")
    p.add_argument("--claude", help="claude executable (default: from PATH)")
    p.add_argument("--retoken", help="retoken executable (default: from PATH; use target/release/retoken to test this checkout)")
    p.add_argument("--claude-arg", action="append", default=[], help="extra argument passed to every claude run")
    p.add_argument("--keep-workspaces", action="store_true", help="keep each run's workspace for inspection")
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true", help="use a local mock API: free, proves the plumbing")
    mode.add_argument("--estimate", action="store_true", help="print the planned runs and cost, then exit")
    mode.add_argument("--self-test", action="store_true", help="check that every task's check works (no agent)")
    mode.add_argument("--report", type=Path, help="re-render results.md from an existing results.json")
    p.add_argument("--yes", action="store_true", help="do not ask before spending money")
    return p.parse_args(argv)


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_args(argv)
    if args.report:
        results = json.loads(args.report.read_text(encoding="utf-8"))
        args.report.with_suffix(".md").write_text(render_markdown(results), encoding="utf-8")
        print(f"wrote {args.report.with_suffix('.md')}")
        return 0

    tasks = load_tasks(TASKS_DIR, args.tasks.split(",") if args.tasks else None)
    if args.self_test:
        return self_test(tasks)

    arms = [a.strip() for a in args.arms.split(",") if a.strip()]
    unknown = [a for a in arms if a not in ARMS]
    if unknown:
        raise SystemExit(f"unknown arm(s): {', '.join(unknown)}; have {', '.join(ARMS)}")
    runs = plan_runs(tasks, arms, args.repeats)

    if args.estimate:
        print_estimate(runs, args.max_budget_usd)
        return 0

    claude_bin = _which("claude", args.claude)
    retoken_bin = _which("retoken", args.retoken) if any(ARMS[a].via_retoken for a in arms) else None
    if not args.dry_run:
        est = print_estimate(runs, args.max_budget_usd)
        if not confirm_spend(est, args.yes):
            return 1

    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out_dir = args.out or RESULTS_DIR / (stamp + ("-dry-run" if args.dry_run else ""))
    (out_dir / "runs").mkdir(parents=True, exist_ok=True)
    work_root = Path(tempfile.mkdtemp(prefix="retoken-bench-", dir=args.work_dir))
    meta = {
        "started_at": _now(), "dry_run": args.dry_run, "model": args.model, "repeats": args.repeats,
        "tasks": [t.id for t in tasks], "arms": arms,
        "claude_version": _version([claude_bin, "--version"]),
        "retoken_binary": retoken_bin, "retoken_commit": _retoken_commit(),
        "platform": f"{platform.system()} {platform.release()} / Python {platform.python_version()}",
        "claude_flags": agent.build_args("<prompt>", args.model, 0, None, None, args.claude_arg),
        "isolation_env": agent.ISOLATION_ENV,
    }
    print(f"{len(runs)} runs -> {out_dir}")

    records: List[Dict[str, Any]] = []
    mock_ctx = MockUpstream() if args.dry_run else None
    mock = mock_ctx.__enter__() if mock_ctx else None
    try:
        for index, plan in enumerate(runs, start=1):
            label = f"[{index}/{len(runs)}] {plan['task'].id} / {plan['arm'].name} / repeat {plan['repeat']}"
            print(label + " ...", flush=True)
            record = run_one(index, plan, args, claude_bin, retoken_bin, work_root, out_dir, mock)
            records.append(record)
            with open(out_dir / "runs.jsonl", "a", encoding="utf-8") as fh:
                fh.write(json.dumps(record) + "\n")
            write_results(out_dir, meta, records, arms)
            outcome = record.get("harness_error") or (
                f"{'solved' if record['solved'] else 'not solved'}, ${record['cost_usd']:.4f}, "
                f"{record['turns']} turns, {record['wall_seconds']:.0f}s")
            if mock is not None:
                p = record.get("plumbing") or {}
                outcome += "; plumbing " + ("ok" if p.get("ok") else "FAIL: " + p.get("problem", "no data"))
            print("    " + outcome, flush=True)
    finally:
        if mock_ctx:
            mock_ctx.__exit__(None, None, None)
        if not args.keep_workspaces:
            shutil.rmtree(work_root, ignore_errors=True)

    results = write_results(out_dir, meta, records, arms)
    print(f"\nwrote {out_dir / 'results.md'} and results.json")
    for suite, groups in results["summary"]["by_suite"].items():
        print(f"suite {suite}:")
        for arm, s in groups.items():
            cps = s["cost_per_solved_task_usd"]
            rel = s["vs_direct_cost_per_solved"]
            print(f"  {arm:<14} solved {s['solved']}/{s['runs']}  cost/solved "
                  f"{'n/a' if cps is None else f'${cps:.4f}'}"
                  f"{'' if rel is None else f'  ({rel * 100:+.0f}% vs direct)'}")
    if args.dry_run:
        bad = [r for r in records if not (r.get("plumbing") or {}).get("ok")]
        print("dry-run plumbing: " + ("all arms ok" if not bad else f"{len(bad)} run(s) FAILED"))
        return 1 if bad else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
