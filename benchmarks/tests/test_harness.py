"""Unit tests for the benchmark harness (no agent, no network).

Run: python -m unittest discover -s benchmarks/tests
"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import run  # noqa: E402
from harness.claude import build_args, clean_env, read_usage  # noqa: E402
from harness.mock_upstream import summarize_request  # noqa: E402
from harness.report import aggregate, render_markdown  # noqa: E402
from harness.tasks import load_tasks  # noqa: E402


def record(arm, cost, solved, task="t", suite="core", **extra):
    base = {"arm": arm, "task": task, "suite": suite, "cost_usd": cost, "solved": solved,
            "turns": 2, "wall_seconds": 1.0, "input_tokens": 10, "output_tokens": 5,
            "cache_read_tokens": 0, "cache_write_tokens": 0, "repeat": 1}
    base.update(extra)
    return base


class AggregateTests(unittest.TestCase):
    def test_cost_per_solved_task_and_relative_change(self):
        rows = [record("direct", 0.10, True), record("direct", 0.10, False),
                record("retoken", 0.06, True), record("retoken", 0.06, True)]
        summary = aggregate(rows, ["direct", "retoken"])["overall"]
        self.assertAlmostEqual(summary["direct"]["cost_per_solved_task_usd"], 0.20)
        self.assertAlmostEqual(summary["retoken"]["cost_per_solved_task_usd"], 0.06)
        self.assertAlmostEqual(summary["retoken"]["vs_direct_cost_per_solved"], -0.70)
        self.assertAlmostEqual(summary["retoken"]["vs_direct_cost_per_run"], -0.40)

    def test_nothing_solved_has_no_cost_per_solved(self):
        summary = aggregate([record("direct", 0.1, False)], ["direct"])["overall"]
        self.assertIsNone(summary["direct"]["cost_per_solved_task_usd"])

    def test_harness_errors_are_left_out_of_aggregates(self):
        rows = [record("direct", 0.1, True), {"arm": "direct", "task": "t", "suite": "core",
                                              "harness_error": "boom", "repeat": 1}]
        self.assertEqual(aggregate(rows, ["direct"])["overall"]["direct"]["runs"], 1)

    def test_markdown_renders(self):
        rows = [record("direct", 0.1, True, models_used=["m"])]
        results = {"meta": {"started_at": "x", "model": "sonnet", "repeats": 1, "tasks": ["t"]},
                   "summary": aggregate(rows, ["direct"]), "runs": rows}
        self.assertIn("| Direct to the API | 1/1 | $0.1000 |", render_markdown(results))


class PlanTests(unittest.TestCase):
    def test_every_arm_runs_every_task_each_repeat_in_rotating_order(self):
        tasks = load_tasks(run.TASKS_DIR)
        runs = run.plan_runs(tasks, ["direct", "retoken", "retoken-terse"], 3)
        self.assertEqual(len(runs), len(tasks) * 3 * 3)
        firsts = {r["arm"].name for i, r in enumerate(runs) if i % 3 == 0}
        self.assertEqual(firsts, {"direct", "retoken", "retoken-terse"})

    def test_budget_defaults_to_five_times_estimate(self):
        task = load_tasks(run.TASKS_DIR, ["multicurrency-report"])[0]
        self.assertAlmostEqual(run.budget_for(task, None), 2.25)
        self.assertEqual(run.budget_for(task, 1.0), 1.0)


class ClaudeTests(unittest.TestCase):
    def test_usage_sums_every_model(self):
        raw = {"total_cost_usd": 0.5, "num_turns": 4, "modelUsage": {
            "sonnet": {"inputTokens": 10, "outputTokens": 3, "cacheReadInputTokens": 100,
                       "cacheCreationInputTokens": 7},
            "haiku": {"inputTokens": 1, "outputTokens": 1}}}
        usage = read_usage(raw)
        self.assertEqual((usage.input_tokens, usage.output_tokens, usage.cache_read_tokens,
                          usage.cache_write_tokens, usage.turns), (11, 4, 100, 7, 4))

    def test_env_is_scrubbed(self):
        import os
        os.environ["ANTHROPIC_BASE_URL"] = "http://example.invalid"
        os.environ["CLAUDE_CODE_SOMETHING"] = "1"
        try:
            env = clean_env()
        finally:
            del os.environ["ANTHROPIC_BASE_URL"], os.environ["CLAUDE_CODE_SOMETHING"]
        self.assertNotIn("ANTHROPIC_BASE_URL", env)
        self.assertNotIn("CLAUDE_CODE_SOMETHING", env)
        self.assertEqual(env["CLAUDE_CODE_DISABLE_CLAUDE_MDS"], "1")

    def test_args_ignore_user_settings(self):
        args = build_args("p", "sonnet", 5, 0.5, None, [])
        self.assertIn("--strict-mcp-config", args)
        self.assertEqual(args[args.index("--setting-sources") + 1], "project")


class MockTests(unittest.TestCase):
    def test_summary_detects_terse_and_retrieval(self):
        body = {"model": "m", "stream": True,
                "system": [{"type": "text", "text": "x [ReToken terse mode] y", "cache_control": {}}],
                "tools": [{"name": "mcp__retoken__retoken_retrieve"}],
                "messages": [{"role": "user", "content": "hi"}]}
        s = summarize_request("/v1/messages", body)
        self.assertTrue(s["terse_instruction"])
        self.assertTrue(s["retoken_retrieve_tool"])
        self.assertEqual(s["cache_breakpoints"], 1)


if __name__ == "__main__":
    unittest.main()
