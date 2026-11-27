"""Aggregating run records into the results table (JSON and markdown)."""

from __future__ import annotations

from statistics import mean
from typing import Any, Dict, Iterable, List, Optional

from .arms import ARMS

METRICS = ("cost_usd", "turns", "wall_seconds", "input_tokens", "cache_read_tokens",
           "cache_write_tokens", "output_tokens")


def _mean(rows: List[Dict[str, Any]], key: str) -> float:
    values = [float(r.get(key) or 0) for r in rows]
    return mean(values) if values else 0.0


def summarize(rows: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Stats for one group of runs. Cost per solved task is the headline metric."""
    solved = sum(1 for r in rows if r.get("solved"))
    total_cost = sum(float(r.get("cost_usd") or 0) for r in rows)
    summary: Dict[str, Any] = {
        "runs": len(rows),
        "solved": solved,
        "solve_rate": solved / len(rows) if rows else 0.0,
        "total_cost_usd": total_cost,
        "cost_per_solved_task_usd": total_cost / solved if solved else None,
    }
    for key in METRICS:
        summary[f"mean_{key}"] = _mean(rows, key)
    return summary


def aggregate(records: Iterable[Dict[str, Any]], arm_order: List[str]) -> Dict[str, Any]:
    """Group runs by suite x arm and task x arm, with each arm compared to ``direct``."""
    records = [r for r in records if not r.get("harness_error")]
    suites = sorted({r["suite"] for r in records})
    tasks = sorted({(r["suite"], r["task"]) for r in records})

    def by_arm(rows: List[Dict[str, Any]]) -> Dict[str, Dict[str, Any]]:
        groups = {arm: summarize([r for r in rows if r["arm"] == arm]) for arm in arm_order}
        groups = {arm: s for arm, s in groups.items() if s["runs"]}
        base = groups.get("direct")
        for arm, summary in groups.items():
            summary["vs_direct_cost_per_solved"] = _relative(
                summary["cost_per_solved_task_usd"],
                base["cost_per_solved_task_usd"] if base else None,
            )
            summary["vs_direct_cost_per_run"] = _relative(
                summary["mean_cost_usd"], base["mean_cost_usd"] if base else None
            )
        return groups

    return {
        "by_suite": {s: by_arm([r for r in records if r["suite"] == s]) for s in suites},
        "by_task": {t: by_arm([r for r in records if r["task"] == t]) for _, t in tasks},
        "overall": by_arm(records),
    }


def _relative(value: Optional[float], base: Optional[float]) -> Optional[float]:
    if value is None or not base:
        return None
    return value / base - 1.0


def _money(value: Optional[float]) -> str:
    return "n/a" if value is None else f"${value:.4f}"


def _pct(value: Optional[float]) -> str:
    if value is None:
        return ""
    return f"{value * 100:+.0f}%"


def _table(groups: Dict[str, Dict[str, Any]]) -> List[str]:
    lines = [
        "| Setup | Solved | Cost per solved task | vs direct | Cost per run | Turns | Seconds "
        "| Input | Cache read | Cache write | Output |",
        "|---|---|---|---|---|---|---|---|---|---|---|",
    ]
    for arm, s in groups.items():
        label = ARMS[arm].label if arm in ARMS else arm
        lines.append(
            f"| {label} | {s['solved']}/{s['runs']} | {_money(s['cost_per_solved_task_usd'])} "
            f"| {_pct(s['vs_direct_cost_per_solved'])} | {_money(s['mean_cost_usd'])} "
            f"| {s['mean_turns']:.2f} | {s['mean_wall_seconds']:.1f} "
            f"| {s['mean_input_tokens']:,.0f} | {s['mean_cache_read_tokens']:,.0f} "
            f"| {s['mean_cache_write_tokens']:,.0f} | {s['mean_output_tokens']:,.0f} |"
        )
    return lines


def render_markdown(results: Dict[str, Any]) -> str:
    meta = results["meta"]
    summary = results["summary"]
    lines = [f"# ReToken benchmark results{' (DRY RUN: mock upstream, no model)' if meta.get('dry_run') else ''}", ""]
    lines += [
        "| | |",
        "|---|---|",
        f"| Started | {meta['started_at']} |",
        f"| Claude Code | {meta.get('claude_version', '?')} |",
        f"| Model requested | {meta['model']} |",
        f"| Models used | {', '.join(meta.get('models_used') or []) or '?'} |",
        f"| ReToken | {meta.get('retoken_commit', '?')} |",
        f"| Repeats | {meta['repeats']} |",
        f"| Tasks | {', '.join(meta['tasks'])} |",
        f"| Platform | {meta.get('platform', '?')} |",
        "",
        "Token columns are means per run. Input is uncached input; cache read and "
        "cache write are billed at different rates, which the cost column already "
        "accounts for (Claude Code's own `total_cost_usd`). Cost per solved task = "
        "total cost of the arm / runs whose check passed.",
        "",
    ]
    if meta.get("dry_run"):
        lines += [
            "> Dry run: every reply came from a local mock, so nothing was billed, no task "
            "can be solved, and the costs shown are notional (Claude Code pricing the mock's "
            "made-up token counts). Use it only to check the plumbing table at the end.",
            "",
        ]
    for suite, groups in summary["by_suite"].items():
        tasks = sorted({r["task"] for r in results["runs"] if r["suite"] == suite})
        lines += [f"## Suite: {suite} ({', '.join(tasks)})", ""] + _table(groups) + [""]
    lines += ["## Per task", ""]
    for task, groups in summary["by_task"].items():
        lines += [f"### {task}", ""] + _table(groups) + [""]
    lines += ["## Runs", "",
              "| # | Task | Arm | Repeat | Solved | Cost | Turns | Seconds | Result | Check |",
              "|---|---|---|---|---|---|---|---|---|---|"]
    for i, r in enumerate(results["runs"], start=1):
        detail = (r.get("harness_error") or r.get("check_detail") or "").replace("\n", " ").replace("|", "/")
        lines.append(
            f"| {i} | {r['task']} | {r['arm']} | {r['repeat']} | {'yes' if r.get('solved') else 'no'} "
            f"| {_money(r.get('cost_usd'))} | {r.get('turns', 0)} | {float(r.get('wall_seconds') or 0):.1f} "
            f"| {r.get('result_subtype', '')} | {detail[:120]} |"
        )
    if meta.get("dry_run"):
        lines += ["", "## Dry-run plumbing checks", "",
                  "| Task | Arm | Requests seen by mock | Via ReToken proxy | Terse instruction | retoken_retrieve offered | Verdict |",
                  "|---|---|---|---|---|---|---|"]
        for r in results["runs"]:
            p = r.get("plumbing") or {}
            lines.append(
                f"| {r['task']} | {r['arm']} | {p.get('mock_messages', 0)} | {p.get('proxy_requests', '-')} "
                f"| {p.get('terse_instruction')} | {p.get('retrieval_tool')} | {'ok' if p.get('ok') else 'FAIL: ' + p.get('problem', '')} |"
            )
    return "\n".join(lines) + "\n"
