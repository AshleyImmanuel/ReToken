# ReToken benchmarks

A reproducible version of the benchmark behind the savings numbers in the main
[`README.md`](../README.md): headless Claude Code solves the same pinned coding
tasks from the same clean checkout, once **direct to the API**, once **through
ReToken**, and once **through ReToken with terse mode**, and the harness reports
**cost per solved task** for each (the metric in
[`docs/superpowers/specs/2026-11-13-retoken-cost-harness-design.md`](../docs/superpowers/specs/2026-11-13-retoken-cost-harness-design.md)).

The numbers in the main README come from a November 2026 run whose script was
never committed. This directory is that benchmark rebuilt so anyone can re-run
it. It has the same task mix (explain code, fix a failing test, summarize a
module, plus one larger multi-file task) but a new fixture repository, so its
numbers will not match the old ones exactly. Until a run of this harness is
committed, treat the README figures as indicative.

## Quick start

Everything runs from the repository root and needs Python 3.9+, `git`,
Claude Code (`claude`) signed in, and `retoken` on `PATH`.

```bash
python benchmarks/run.py --self-test      # 1. free: every task's check fails before, passes on the reference fix
python benchmarks/run.py --dry-run        # 2. free: the whole pipeline against a local mock API
python benchmarks/run.py --estimate       # 3. free: prints the planned runs and expected cost
python benchmarks/run.py --tasks explain-code --arms direct,retoken --repeats 1   # 4. ~$0.08: one cheap real task
python benchmarks/run.py                  # 5. the full run (about $5; asks before spending)
```

To benchmark this checkout rather than the installed binary, build it and pass
it in: `cargo build --release -p cli`, then
add `--retoken target/release/retoken` (`.exe` on Windows).

Real runs print the estimate and ask you to type `yes` before starting. Pass
`--yes` for unattended runs (CI, scripts).

## What it costs

| Run | Runs | Expected | Hard ceiling |
|---|---|---|---|
| `--self-test`, `--dry-run`, `--estimate` | 0 paid | $0 | $0 |
| `--tasks explain-code --arms direct --repeats 1` | 1 | ~$0.04 | $0.25 |
| Core suite only: `--tasks explain-code,fix-failing-test,summarize-module` | 27 | ~$1.20 | $6.75 |
| Full default: 4 tasks x 3 arms x 3 repeats | 36 | ~$5.20 | $27.00 |

Expected costs are the per-task `est_cost_usd` values in `tasks/*/task.json`,
taken from the November 2026 direct-arm runs on Claude Sonnet; ReToken arms
usually cost less. The ceiling is the sum of Claude Code's per-run
`--max-budget-usd` caps (5x the estimate, at least $0.25; override with
`--max-budget-usd`). A run that hits its cap stops and counts as not solved.
On a Claude Pro/Max subscription the runs use your plan's usage instead of
being billed. A different `--model` (for example `opus`) changes the cost.

## What a run does

For every (task, arm, repeat), in an order that rotates which arm goes first so
provider-side prompt caching and time-of-day drift hit every arm alike:

1. Copies `fixture/` (plus the task's `overlay/`, if any) into a fresh temporary
   directory and commits it as a new git repository, so every run starts from
   the identical clean checkout.
2. For ReToken arms, starts a private `retoken run` on a free port with its own
   data directory and an empty working directory. Your own daemon on 8888 is
   not touched, and stats are per run. The agent is launched with
   `retoken wrap claude ...`, which is the path users take, and is given the
   `retoken mcp` retrieval tool, as the main README's quick start sets up.
3. Runs `claude -p "<task prompt>" --output-format json --model sonnet ...`
   with an isolated configuration (below).
4. Runs the task's check, then records cost, tokens, turns, time, ReToken's
   own stats and whether the check passed.

| Arm | Route | Retrieval tool | Terse mode |
|---|---|---|---|
| `direct` | Claude Code -> Anthropic | no | no |
| `retoken` | Claude Code -> ReToken (defaults) -> Anthropic | yes | no |
| `retoken-terse` | Claude Code -> ReToken (`RETOKEN_TERSE_MODE=true`) -> Anthropic | yes | yes |

### Isolation

Every arm sees the same context, whatever is configured on the machine:

- `--setting-sources project` ignores `~/.claude/settings.json`, including any
  `env` block (a global `ANTHROPIC_BASE_URL` there would otherwise send the
  "direct" arm through a proxy), plugins and hooks.
- `--strict-mcp-config` loads no MCP servers except the one an arm passes.
- `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` and `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`
  keep `CLAUDE.md`/`AGENTS.md` files from your home or parent folders and
  auto-memory out of the prompt.
- Inherited `CLAUDE*`, `ANTHROPIC*` and `RETOKEN*` variables are dropped, except
  `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` and
  `CLAUDE_CONFIG_DIR`, so running the benchmark from inside another agent
  session behaves like a clean terminal.
- `--permission-mode bypassPermissions` lets the agent run tests and edit files
  without prompts. It only ever works inside the throwaway temp workspace.

The exact flags and environment are written into each `results.json`.

## Tasks

The fixture is `ledgerkit`, a small double-entry bookkeeping library
(fixed-point money, a ledger, exchange rates, a journal parser, reports,
recurring entries, a CLI; ~870 lines of code, 57 tests, standard library only).

| Task | Suite | What the agent is asked | How it is checked |
|---|---|---|---|
| `explain-code` | core | Explain how `Money.allocate` distributes leftover cents | Answer must mention each key fact (regexes in `task.json`); no files changed |
| `fix-failing-test` | core | A month-end drift bug in `recurring.py` fails one test; fix it without touching tests | Original `tests/` restored, then the whole suite must pass |
| `summarize-module` | core | Summarize `ledger.py` and every rule `Ledger.post` enforces | Answer must mention each rule; no files changed |
| `multicurrency-report` | large | Add `rate` directives and a trial balance converted to a base currency, across parser, fx, report and CLI | Original `tests/` restored, 11 hidden acceptance tests added, everything must pass |

Each task also has a `reference/` solution. `--self-test` checks that every
check fails on the untouched task and passes on the reference, so a broken
check cannot silently mark everything solved (or nothing).

## Reading the output

Each run writes `benchmarks/results/<UTC timestamp>/` (git-ignored):

- `results.md`: the tables below, ready to paste.
- `results.json`: metadata (Claude Code version, models actually used, ReToken
  commit and binary, flags), the aggregates and every run record.
- `runs.jsonl`: one line per run, appended as the benchmark goes, so an
  interrupted run keeps what it finished.
- `runs/<run>.claude.json`, `.stderr.txt`, `.retoken.log`: raw per-run output.

`results.md` has one table per suite (`core` = the three small tasks, `large` =
the multi-file task), one per task, and a list of every run:

| Column | Meaning |
|---|---|
| Solved | Runs whose check passed / runs |
| Cost per solved task | Total cost of the arm's runs / solved runs. **The headline number**: an arm that is cheap but fails more often does not look better |
| vs direct | Change in cost per solved task against the `direct` arm (negative = cheaper) |
| Cost per run | Mean Claude Code `total_cost_usd` per run, solved or not |
| Turns, Seconds | Mean model turns and wall-clock seconds per run |
| Input, Cache read, Cache write, Output | Mean tokens per run, summed over every model the run used. Cache reads are billed at about a tenth of the input rate, so compare cost, not raw input |

Re-render the markdown from a JSON file with
`python benchmarks/run.py --report benchmarks/results/<run>/results.json`.

### Caveats

- Agent runs are not deterministic. Three repeats per arm is a small sample;
  use `--repeats 5` or more before quoting a percentage.
- Anthropic's prompt cache is shared by every run on the same account, so the
  first run of a session pays to warm it and later runs, of any arm, benefit.
  The rotating order spreads this evenly; it does not remove it.
- `--model sonnet` resolves to the newest Sonnet; the models actually used are
  recorded in `results.json`. Pin a full model name to compare runs over time.
- The answer checks for the two read-only tasks are keyword checks. They catch
  wrong or missing explanations, not subtle inaccuracies.

## Updating the main README

Change the numbers in the main `README.md` only from a run of this harness, and
commit that run so the numbers can be traced:

```bash
git add -f benchmarks/results/<run>/results.md benchmarks/results/<run>/results.json
```

## Adding a task

Create `tasks/<id>/task.json`:

```json
{
  "id": "my-task",
  "suite": "core",
  "title": "One line",
  "prompt": "What the agent is told.",
  "overlay": "overlay",
  "max_turns": 25,
  "est_cost_usd": 0.05,
  "check": { "type": "tests", "restore": ["tests"], "hidden": "hidden",
             "command": ["{python}", "-m", "unittest", "discover", "-s", "tests", "-t", "."] }
}
```

`overlay/` files are copied over the fixture before the run (to plant a bug),
`hidden/` files are copied in only for the check, and `{python}` is the
interpreter running the harness. For explanation tasks use
`{"type": "answer", "patterns": [...], "forbid_changes": true}`. Add a
`reference/` solution (files, or `answer.md` for answer checks), then run
`--self-test`.

## Harness tests

```bash
python -m unittest discover -s benchmarks/tests
```
