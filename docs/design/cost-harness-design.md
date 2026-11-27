# ReToken Cost Harness and MCP Design

## Objective

Make ReToken a cost-aware coding harness for Codex CLI and Claude Code. It
must reduce total provider cost per successfully completed coding task without
requiring API keys, silently changing user instructions, or increasing the
number of provider calls.

The user-facing success metric is cost per solved task, measured against the
same agent, model, repository, and task without ReToken. A run is comparable
only when its required tests pass.

## Scope

This design adds two cooperating subsystems:

1. A local stdio MCP server for observability and explicit control.
2. A deterministic cost harness for command-result reuse, structured tool
   output, and repository-context deltas.

The existing loopback proxy remains responsible for provider transport,
provider prefix caching, and response-cache isolation. It does not become an
MCP server and never stores credentials.

## Non-negotiable rules

- No API key is required. Codex keeps its existing ChatGPT sign-in and Claude
  Code keeps its existing account sign-in.
- No optimizer may alter user prompts, source code, commands, paths, error
  messages, tool schemas, or safety-critical text.
- No optimization may trigger an additional provider call. A candidate that
  cannot demonstrate lower request cost is bypassed.
- Command reuse is allowed only for deterministic, read-only commands and only
  when its complete declared input fingerprint is unchanged.
- Cache entries are isolated by workspace and credential fingerprint, have a
  bounded TTL and byte budget, and can be cleared explicitly.
- Codex Desktop networking is not changed.

## MCP control plane

Add a Rust `retoken-mcp` binary using stdio JSON-RPC MCP transport. It only
calls ReToken's local `127.0.0.1` administrative API and local harness state.
It does not proxy inference traffic, read provider credentials, or access the
network.

Tools:

- `retoken_status`: reports gateway health, cache state, token telemetry, and
  cost-harness state.
- `retoken_doctor`: reports launch, reachability, configuration, and capability
  diagnostics.
- `retoken_cache_clear`: clears local caches only after `confirm: true`.
- `retoken_report`: returns a task/session cost report that separates raw input
  reduction, provider cache discounts, execution reuse, and provider errors.
- `retoken_fetch_artifact`: returns a requested bounded line range from a local
  tool-result artifact. It is a retrieval tool, not an automatic fallback.

Codex and Claude Code receive project-scoped MCP configuration that starts the
binary from this workspace. The generic ReToken proxy wrappers continue to
scope provider base URLs to their child process.

## Cost harness

### Deterministic execution reuse

The harness records eligible command runs as immutable blocks. A block key is a
hash of: normalized command and arguments, workspace root, relevant source and
configuration file hashes, tool version, and selected environment values.

Only an explicit allowlist is eligible initially: test, lint, formatting check,
and type-check commands with no write or network effects. Commands with shell
redirection, package installation, migration, deployment, mutation, network
access, or an unknown executable are never reused.

When the same key is requested again, the harness returns the exact prior exit
status and artifact reference without executing the command. This removes a
repeated command and its duplicate output from future model context. A changed
fingerprint always executes the command normally.

### Structured tool-result artifacts

Every eligible command result is stored locally with a content hash, command
metadata, bounded retention, and line index. The first result remains faithful:
exit status, diagnostics, changed paths, and relevant output lines are exact.

For repeated reads, the harness sends a compact structured projection containing
the exact exit status, error/warning lines, failing test names, changed files,
and artifact identifier. Boilerplate and repeated passing-test lines are not
repeated. The complete output stays retrievable with `retoken_fetch_artifact`.

The initial implementation applies this only to known structured outputs such
as test, lint, formatter, and compiler results. Unrecognized output is passed
through unchanged.

### Symbol-level repository deltas

The harness records source hashes and a language-aware symbol/import index.
When a file has not changed, it is represented by its existing symbol version.
When it changes, only changed symbols plus direct import/export dependencies are
selected. The agent retains normal file tools and can request any exact source
range directly from the workspace.

This feature is advisory in the first release: it provides a context manifest
and retrieval tools rather than rewriting arbitrary provider payloads. It
therefore cannot remove required context invisibly.

### Provider request policy

Provider request transformations stay conservative:

- preserve OpenAI Responses payloads byte-for-byte;
- use stable ordering only for provider-supported prompt caching;
- retain exact request cache safety rules for generation endpoints;
- do not claim raw-token reduction when only provider cache discount applies;
- parse streaming usage so reports reflect Codex and Claude streaming sessions.

## Cost non-regression policy

Before a transformed payload is forwarded, the harness estimates token count
using the target provider tokenizer where available. It forwards the candidate
only when it is strictly smaller than the original payload and does not require
an extra provider call. Otherwise it forwards the original payload.

Command reuse does not make a model call. Artifact retrieval is initiated by an
agent only when it needs precise detail; it is local and does not spend provider
tokens by itself. The report must display retrieval occurrences so a benchmark
can detect whether a workflow merely moved tokens to later turns.

## Integration boundaries

The first release supports Codex CLI and Claude Code through project-scoped
launchers and MCP configuration. Codex Desktop receives MCP tools where its
supported project configuration permits, but its inference networking is never
intercepted.

The system runs entirely on loopback and local workspace storage. It binds no
external listening interface and performs no telemetry upload.

## Verification and benchmarks

Unit tests cover:

- MCP request/response schemas and destructive-tool confirmation;
- endpoint failures, timeouts, and malformed local JSON;
- block-key changes for source, config, command, and tool version changes;
- allowlist rejection for unsafe commands;
- artifact line retrieval bounds and cleanup;
- token candidate rejection when savings are absent;
- streaming usage parsing.

Integration tests run a local gateway and MCP process with no provider
credentials. They verify that all tools work offline, no credentials are logged,
and no model request is made.

Evaluation uses a fixed coding-task corpus with the same model and acceptance
tests for baseline and ReToken runs. Publish: solved-task rate, total provider
cost, raw input/output tokens, cached input, number of provider calls, command
reuse count, retrieval count, and failures. The target is more than 25% lower
provider cost per solved task than the baseline without a lower solved-task rate.

## Delivery sequence

1. Implement the MCP binary and project-scoped configurations.
2. Correct streaming telemetry and cost accounting.
3. Add execution-block recording and a strict read-only allowlist.
4. Add artifact storage, structured projections, and bounded retrieval.
5. Add symbol-delta manifests and benchmark harness.
6. Enable features only after benchmark evidence confirms the non-regression
   and solved-task-quality gates.
