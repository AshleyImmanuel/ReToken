# ReToken — Product Requirements Document (PRD)

**Document Status:** Draft / Engineering Baseline  
**Version:** 0.1  
**Project:** ReToken  
**Document Type:** Product Requirements + Technical Architecture Specification  
**Primary Goal:** Reduce AI coding-agent cost and latency by eliminating unnecessary context, computation, tool calls, and model round trips.

---

## 1. Executive Summary

ReToken is a **local-first Agent Runtime Optimizer** designed for AI coding agents such as Claude Code, Codex-style agents, Gemini-based coding agents, and custom agent runtimes.

ReToken is not primarily a prompt compressor.

Its central idea is:

> **Do not compress context that can be avoided. Do not resend state that can be referenced. Do not execute work that can be reused. Do not retrieve data that can be predicted. Do not serialize independent operations that can run in parallel. Compress only what remains necessary.**

The runtime sits between an AI coding agent and its tools/model providers. It maintains a persistent local representation of repository state, tool results, context, caches, dependencies, and execution history.

The provider should receive the **minimum sufficient context** required to complete the current step rather than the entire accumulated agent state.

### North-Star Objective

ReToken targets **100× efficiency improvement on suitable workloads**, where:

- A task costing approximately `$0.10` in an unoptimized agent flow could target approximately `$0.001`.
- This is an engineering target, not a universal guarantee.
- Quality/correctness must remain at or above an agreed baseline.
- Runtime overhead must be near-zero on the hot path.
- Total wall-clock latency should ideally decrease rather than increase.

The 100× objective comes from combining multiple optimizations:

1. API-call elimination
2. exact and semantic reuse
3. deterministic tool-result caching
4. context virtualization
5. repository-aware retrieval
6. delta transmission
7. parallel tool execution
8. speculative prefetch
9. cache-aware request construction
10. structural compression
11. output minimization
12. agent-turn reduction

---

# 2. Problem Statement

Modern coding agents repeatedly transmit and regenerate information that already exists locally.

A typical agent task can involve:

```text
User request
    ↓
Model
    ↓
Read file
    ↓
Model
    ↓
Read another file
    ↓
Model
    ↓
Search repository
    ↓
Model
    ↓
Run tests
    ↓
Model
    ↓
Read test output
    ↓
Model
    ↓
Modify code
    ↓
Model
    ↓
Run tests again
    ↓
Model
```

The same repository information, tool definitions, tool outputs, history, and unchanged files may appear repeatedly.

This creates several forms of waste:

### 2.1 Context waste

- Repeated file contents
- Repeated tool results
- Repeated repository structure
- Repeated conversation state
- Large logs
- Large JSON responses
- Duplicate search results
- Unchanged configuration

### 2.2 Execution waste

- Repeated deterministic commands
- Duplicate tool calls
- Repeated repository searches
- Sequential independent reads
- Recomputing unchanged analysis

### 2.3 Latency waste

- Model round trips
- Tool round trips
- Network latency
- Repository scanning
- Repeated file parsing
- Context preparation
- Cache misses

### 2.4 Cost waste

Agentic coding workloads can accumulate very large amounts of input context over multiple turns. Raw token count therefore does not tell the whole story; the runtime must optimize the complete execution path.

---

# 3. Product Vision

ReToken should make a coding agent behave as though it has a very large persistent working memory while actually sending only small, relevant representations to the model.

### Vision

> **A virtualized execution context for AI agents where the local runtime owns state and the model receives only the minimum sufficient information needed for the next decision.**

The long-term product should feel invisible.

The user should continue using their preferred coding agent normally.

```text
                 USER
                  │
                  ▼
            AI CODING AGENT
                  │
                  ▼
        ┌─────────────────────┐
        │       ReToken       │
        │ Agent Runtime Layer │
        └─────────────────────┘
          │       │       │
          ▼       ▼       ▼
       State    Tools   Provider
       Engine   Runtime   API
```

---

# 4. Goals

## 4.1 Primary Goals

### G1 — Reduce provider input tokens

Target:

- 50–80% reduction initially
- 80–95% reduction in highly repetitive workloads

### G2 — Reduce provider output tokens

Target:

- 30–70% reduction where concise machine-oriented output is safe

### G3 — Reduce repeated context

Target:

- 80–95% reduction in repeated state transmission

### G4 — Reduce tool round trips

Target:

- 30–70% reduction through caching, fusion, prediction, and scheduling

### G5 — Reduce total latency

ReToken should aim to reduce:

- time-to-first-token
- model prefill
- tool waiting
- unnecessary agent turns
- serial tool execution

### G6 — Preserve correctness

The runtime must never silently replace required exact data with an unsafe approximation.

### G7 — Maintain provider flexibility

Support multiple providers without coupling the core architecture to a single model vendor.

### G8 — Be useful without an external AI dependency

Core optimization must work locally.

External AI APIs must never be mandatory for the hot path.

---

# 5. Non-Goals

ReToken is not initially intended to:

- train foundation models
- replace coding agents
- build a general-purpose LLM
- guarantee 100× savings on every task
- modify provider model weights
- require a cloud-hosted ReToken service
- perform expensive LLM compression for every request
- replace developer IDEs
- become a full autonomous coding agent in v1

---

# 6. Core Design Principles

## P1 — Eliminate Before Compressing

Priority order:

```text
REUSE
  ↓
PREDICT
  ↓
PARALLELIZE
  ↓
SELECT
  ↓
DELTA
  ↓
COMPRESS
  ↓
CACHE-AWARE PACK
  ↓
LLM
```

Compression is not the first optimization.

## P2 — Local State Is the Source of Truth

ReToken maintains local state for:

- files
- symbols
- repository structure
- tool results
- commands
- errors
- test results
- previous context
- decisions
- cache entries
- dependency relationships

## P3 — Hot Path Must Be Cheap

The request path must prefer:

- hashing
- cache lookup
- local indexes
- deterministic transforms
- lightweight ranking
- reference resolution

Avoid:

- external embeddings
- remote LLM calls
- large model inference
- expensive repository-wide scans

## P4 — Exact Recovery

Lossy representations must have an exact recovery path.

Every compressed or summarized object should retain:

- original content
- content hash
- representation type
- version
- metadata

## P5 — Cache Awareness

Optimization must account for provider prompt caching.

A smaller request that destroys a reusable cache prefix may be worse than a slightly larger request that hits the provider cache.

## P6 — Correctness Before Savings

Every optimization must have a fallback:

```text
Optimization unsafe?
        ↓
Pass through original data
```

---

# 7. Target Users

## Primary Users

- AI-assisted software developers
- Users of Claude Code-style terminal agents
- Users of Codex-style coding agents
- AI IDE users
- AI coding-agent developers
- Teams operating high-volume coding agents

## Secondary Users

- Agent framework developers
- AI infrastructure engineers
- LLM application developers
- Researchers studying agent efficiency

---

# 8. User Experience

ReToken should require minimal behavioral change.

Example:

```bash
re-token wrap claude
```

or:

```bash
re-token run -- claude
```

The user continues working normally.

ReToken operates transparently.

Optional diagnostics:

```bash
re-token stats
re-token trace
re-token benchmark
re-token inspect
re-token cache
re-token doctor
```

Example:

```text
ReToken
────────────────────────────────────
Task                     Fix auth bug
Provider                 Claude
Baseline input           184,200 tokens
Optimized input           31,400 tokens
Input reduction              82.9%

Tool calls
Baseline                       24
ReToken                         11

Tool latency
Baseline                     18.4s
ReToken                       7.1s

Estimated provider cost
Baseline                     $0.104
ReToken                       $0.006

Correctness                    PASS
────────────────────────────────────
```

---

# 9. High-Level Architecture

```text
                         ┌───────────────┐
                         │     USER      │
                         └───────┬───────┘
                                 │
                                 ▼
                       ┌───────────────────┐
                       │   AI CODE AGENT   │
                       │ Claude / Codex /  │
                       │ Gemini / Custom   │
                       └─────────┬─────────┘
                                 │
                                 ▼
                ┌────────────────────────────────┐
                │          RETOKEN GATEWAY        │
                │                                  │
                │  Request Interception           │
                │  Provider Adapter               │
                │  Policy Engine                  │
                └───────────────┬────────────────┘
                                │
             ┌──────────────────┼──────────────────┐
             ▼                  ▼                  ▼
      ┌────────────┐     ┌──────────────┐   ┌──────────────┐
      │ Exact      │     │ State Engine │   │ Tool Runtime │
      │ Cache      │     │              │   │              │
      └────────────┘     └──────────────┘   └──────┬───────┘
             │                  │                   │
             └──────────────────┼───────────────────┘
                                ▼
                       ┌─────────────────┐
                       │ Context Planner │
                       └────────┬────────┘
                                │
             ┌──────────────────┼──────────────────┐
             ▼                  ▼                  ▼
        Relevance            Delta            Retrieval
         Ranking             Engine             Engine
             │                  │                  │
             └──────────────────┼──────────────────┘
                                ▼
                    ┌────────────────────────┐
                    │ Compression Planner    │
                    │                        │
                    │ Structural             │
                    │ Deterministic          │
                    │ Semantic (optional)    │
                    └────────────┬───────────┘
                                 ▼
                     ┌────────────────────────┐
                     │ Cache-Aware Packer     │
                     └────────────┬───────────┘
                                  ▼
                              PROVIDER
                                  │
                                  ▼
                               MODEL
```

---

# 10. Hot Plane and Intelligence Plane

This separation is a fundamental architectural requirement.

## 10.1 Hot Plane

Runs during an active model/tool request.

Responsibilities:

- request interception
- hashing
- cache lookup
- state lookup
- reference resolution
- delta generation
- lightweight ranking
- deterministic compression
- scheduling
- provider request construction

Target overhead:

> **Sub-10 ms for common local transformations, with a long-term target approaching single-digit milliseconds or lower.**

The target is measured, not assumed.

## 10.2 Intelligence Plane

Runs asynchronously in the background.

Responsibilities:

- AST parsing
- repository indexing
- dependency graph construction
- semantic indexing
- compression profiling
- cache warming
- next-file prediction
- next-tool prediction
- historical trace analysis
- benchmark analysis

The intelligence plane must never become a mandatory synchronous dependency for ordinary requests.

---

# 11. Component Requirements

## 11.1 ReToken Gateway

The gateway is the entry point.

### Requirements

- Support local proxy operation
- Support OpenAI-compatible APIs
- Support Anthropic-compatible flows
- Provide provider abstraction
- Preserve authentication behavior
- Preserve streaming where supported
- Record request metadata
- Allow bypass mode
- Allow per-provider policies

### Example

```text
Agent
  ↓
localhost:REToken
  ↓
Provider API
```

---

# 12. Context State Engine

The Context State Engine is the core memory layer.

Everything important becomes a versioned state object.

Example:

```text
file:src/auth/login.ts
tool:test-result:73a91
error:jwt-expired:9921
search:auth-symbols:44b2
decision:use-refresh-token:9fa2
```

Each object should include:

```text
object_id
content_hash
version
type
size
created_at
updated_at
source
dependencies
confidence
recovery_location
```

### Requirements

- Content-addressable storage
- Versioning
- Exact recovery
- Incremental updates
- Fast lookup
- Garbage collection
- Session persistence

---

# 13. Repository Intelligence

Code must not be treated as ordinary text.

ReToken should maintain:

```text
Repository
   │
   ├── Files
   │
   ├── Symbols
   │
   ├── Imports
   │
   ├── Exports
   │
   ├── Calls
   │
   ├── Types
   │
   └── Dependencies
```

### Required capabilities

- AST parsing
- Symbol extraction
- Import/export graph
- Function/class relationships
- Reverse dependency lookup
- Incremental indexing
- Git-aware changes
- File relevance ranking

### Example

User:

> Fix the JWT refresh bug.

Instead of sending the entire repository:

```text
Relevant:
src/auth/jwt.ts
src/auth/refresh.ts
src/middleware/auth.ts
src/api/session.ts
tests/auth/refresh.test.ts
```

Potentially relevant supporting symbols:

```text
validateToken()
refreshToken()
getSession()
AuthMiddleware
TokenPayload
```

---

# 14. Context Planner

The Context Planner determines the minimum sufficient context.

### Inputs

- user request
- current model state
- repository state
- recent actions
- active errors
- dependency graph
- previous tool results
- token budget
- provider cache state

### Output

A ranked context package.

### Priority score

```text
Score =
    task relevance
  + dependency proximity
  + explicit user reference
  + error relevance
  + recency
  + active-file relevance
  + user priority
  - redundancy
  - stale state
```

The exact scoring model may evolve.

---

# 15. Tool Runtime

ReToken should understand tool properties.

Each tool should expose metadata such as:

```text
read_only
mutating
deterministic
cacheable
parallel_safe
prefetchable
latency_estimate
result_size_estimate
side_effect_level
```

Example:

```text
read_file
  read_only=true
  deterministic=true
  cacheable=true
  parallel_safe=true

git_commit
  read_only=false
  mutating=true
  parallel_safe=false
```

---

# 16. Tool Scheduling

Independent read-only operations should run concurrently.

Example:

```text
Before:

read A → wait
read B → wait
read C → wait

After:

read A ─┐
read B ─┼──→ continue
read C ─┘
```

For complex workflows, ReToken should construct a dependency DAG:

```text
        Read A
       /      \
   Read B     Read C
       \      /
        Analyze
           │
         Test
```

Mutating operations remain ordered where required.

---

# 17. Tool Result Virtualization

Large tool results should be stored locally instead of automatically injected in full.

Example:

```text
Tool:
npm test

Actual output:
8,000 lines

ReToken sends:

TEST_RESULT id=t_93af
status=FAILED
failed=2
passed=417
errors=2
files=[
  auth.test.ts,
  session.test.ts
]
```

The model can request:

```text
FETCH t_93af errors
FETCH t_93af lines 510-570
FETCH t_93af stacktrace 2
```

This creates a virtualized result space.

---

# 18. Delta Engine

If state changes only slightly, transmit the change.

Example:

```text
Previous:
file hash = A

Current:
file hash = B

Delta:
lines 104-109 changed
```

Applicable to:

- code
- JSON
- YAML
- configuration
- logs
- test output
- tool results
- conversation state

---

# 19. Compression Engine

Compression is a final-stage optimization.

### Initial compressors

```text
JSON
CSV
YAML
HTML
Logs
Terminal output
Diffs
Code
Tool schemas
Search results
Tables
Repeated content
```

### Compression policy

```text
if exact cache hit:
    reuse

else if reference is sufficient:
    send reference

else if delta is sufficient:
    send delta

else if structural compression is beneficial:
    compress

else:
    pass through original
```

### Safety requirement

Never compress if:

- parser fails
- recovery storage fails
- representation is larger
- confidence is insufficient
- exact details are required

---

# 20. Semantic Compression

Semantic compression is optional.

It may use:

- local models
- classifiers
- extractive ranking
- lightweight encoders

It must not be required for basic operation.

External API-based compression is prohibited from the normal hot path.

Reason:

```text
External compressor
       ↓
network
       ↓
model
       ↓
compression
       ↓
main model
```

This can erase the latency/cost benefit.

---

# 21. Caching Architecture

Caching should operate at multiple levels.

```text
L1  Exact request cache
L2  Normalized request cache
L3  Tool result cache
L4  Repository state cache
L5  Context object cache
L6  Semantic cache
L7  Provider prompt cache
L8  Provider call
```

### Cache correctness

Cache keys must account for relevant state such as:

- repository revision
- file content hashes
- environment
- command
- command arguments
- tool version
- configuration
- relevant dependencies

Never return stale state merely because a string key matches.

---

# 22. Speculative Prefetch

ReToken should predict likely next operations.

Example:

```text
Model reads:
src/auth/login.ts

Dependency graph predicts:
src/auth/session.ts
src/auth/jwt.ts
tests/auth/login.test.ts
```

ReToken can prepare these locally while the model is thinking.

Another example:

```text
npm test starts
      ↓
background parser watches output
      ↓
likely failing files indexed
      ↓
relevant symbols prepared
      ↓
model receives failure context immediately
```

Prefetch must be cheap and must not cause dangerous side effects.

Only safe, read-only operations may be speculatively executed by default.

---

# 23. API Call Elimination

The cheapest model call is a call that never happens.

ReToken should eliminate:

- duplicate requests
- deterministic repeated tool calls
- unnecessary searches
- redundant reads
- repeated model queries
- repeated analysis

Possible decision:

```text
Can local state answer this?
       │
      YES ──→ no model call
       │
       NO
       ↓
Can cached result answer this?
       │
      YES ──→ reuse
       │
       NO
       ↓
Can tool execution answer this?
       │
      YES ──→ execute locally
       │
       NO
       ↓
Call model
```

---

# 24. Agent Turn Reduction

A major optimization target is reducing the number of model turns.

Example:

```text
Bad:

model → read A
model → read B
model → read C
model → search X
model → test
model → inspect error

Optimized:

model → request context
ReToken → fetch A+B+C+X in parallel
ReToken → run deterministic checks
model → receive sufficient result
```

The agent should make fewer decisions because ReToken performs mechanical work automatically.

---

# 25. Provider Cache Optimization

Provider caching must be treated as a first-class constraint.

Context should be arranged approximately as:

```text
STATIC PREFIX
  system instructions
  stable tool definitions
  repository map
  stable project metadata

VARIABLE SUFFIX
  current task
  changed files
  latest tool results
```

ReToken must avoid unnecessary rewriting of stable prefixes.

The optimization objective is therefore:

> **Minimize effective billed work and latency, not merely raw token count.**

---

# 26. ReToken Objective Function

The runtime optimizer should eventually optimize a weighted objective:

```text
Score =
    α × API_COST
  + β × TTFT
  + γ × TOTAL_LATENCY
  + δ × INPUT_TOKENS
  + ε × OUTPUT_TOKENS
  + ζ × TOOL_ROUND_TRIPS
  + η × CACHE_MISSES
```

Subject to:

```text
task correctness ≥ baseline
tool correctness = 100%
context fidelity ≥ defined threshold
```

Weights should be configurable.

---

# 27. Observability — Agent Flight Recorder

Before advanced optimization, ReToken must measure reality.

Every task should optionally record:

```text
task_id
session_id
provider
model
timestamp
request_size
input_tokens
output_tokens
cached_tokens
tool_calls
tool_duration
network_duration
TTFT
generation_duration
cache_hits
cache_misses
compression_ratio
context_reduction
agent_turns
errors
final_success
```

The flight recorder is essential for proving that an optimization actually works.

---

# 28. Benchmarking

ReToken must maintain reproducible benchmarks.

## 28.1 Baselines

Every benchmark compares:

```text
Baseline agent
vs
ReToken enabled
```

### Metrics

- input tokens
- output tokens
- total tokens
- estimated provider cost
- TTFT
- total wall-clock time
- tool latency
- number of model turns
- number of tool calls
- cache hit rate
- context reduction
- correctness
- failure rate

## 28.2 Benchmark Classes

### A. Code navigation

- locate symbol
- trace dependency
- find implementation

### B. Bug fixing

- isolated bug
- multi-file bug
- dependency bug

### C. Testing

- failing unit tests
- integration failures
- repeated test cycles

### D. Logs

- large logs
- repeated logs
- stack traces

### E. JSON/API data

- large structured responses
- repeated responses
- nested JSON

### F. Repository-wide tasks

- refactoring
- feature implementation
- test generation
- dependency updates

---

# 29. Correctness Evaluation

Every optimization must be evaluated against an unoptimized baseline.

Required checks:

```text
Does the final patch compile?
Does the test suite pass?
Does the agent solve the task?
Does the optimized agent make unsafe assumptions?
Does exact recovery work?
Does cache invalidation work?
```

A token reduction is considered a failure if it causes a meaningful correctness regression.

---

# 30. 100× Efficiency Target

The project should use the following framing:

> **ReToken 100× Efficiency Target**

Not:

> “ReToken compresses everything by 100×.”

The 100× target is achieved through a stack.

Illustrative path:

```text
Baseline
$0.100

API elimination          → $0.070
Tool-result reuse        → $0.050
Context selection        → $0.025
Delta state              → $0.015
Structural compression   → $0.010
Provider caching         → $0.004
Turn reduction           → $0.001–$0.003
```

These are design examples, not guaranteed measurements.

---

# 31. Performance Requirements

## Hot Path

Target:

- local overhead normally <10 ms
- no mandatory external network calls
- no mandatory external model inference
- asynchronous persistence where safe
- lock contention minimized

## Background

May perform:

- AST parsing
- indexing
- graph analysis
- embeddings
- prediction
- benchmark analysis

Background work must be cancellable and resource-aware.

---

# 32. Privacy and Security

ReToken should default to local processing.

### Requirements

- No project source code uploaded to ReToken servers by default
- Provider credentials pass through securely
- Secrets must never enter logs
- Sensitive tool outputs must be protected
- Local state storage must support permissions/encryption where appropriate
- Cache entries must respect repository boundaries
- Multi-project data must not leak across contexts

### Secret handling

Never log:

```text
API keys
tokens
passwords
private keys
session cookies
environment secrets
```

Redaction must occur before observability storage.

---

# 33. Failure Handling

ReToken must fail open where possible.

If an optimization component fails:

```text
Optimizer failure
      ↓
Disable optimization
      ↓
Pass through original request
```

The user's coding agent should continue working.

Examples:

- parser failure → raw file
- cache corruption → provider/tool execution
- compression failure → original output
- index unavailable → direct search
- predictor unavailable → normal scheduling

---

# 34. Technology Stack

## Recommended Initial Stack

### Core

**Rust**

Reason:

- low overhead
- concurrency
- predictable performance
- strong memory safety
- suitable for local infrastructure

Go is acceptable for an initial prototype if development speed is more important.

### Storage

Initial:

```text
SQLite
+
content-addressed filesystem/object store
```

### Parsing

```text
tree-sitter
```

### Search

Start with:

```text
lexical search
symbol search
dependency search
```

Add local semantic indexing later.

### Interfaces

```text
CLI
OpenAI-compatible gateway
Anthropic adapter
Python SDK
TypeScript SDK
```

---

# 35. Repository Structure

```text
re-token/
│
├── core/
│   ├── state/
│   ├── cache/
│   ├── hashing/
│   ├── budget/
│   ├── protocol/
│   └── errors/
│
├── analyzers/
│   ├── code/
│   ├── repository/
│   ├── logs/
│   └── structured/
│
├── retrieval/
│   ├── lexical/
│   ├── symbol/
│   ├── dependency/
│   └── semantic/
│
├── scheduler/
│   ├── dag/
│   ├── parallel/
│   ├── prefetch/
│   └── fusion/
│
├── compressors/
│   ├── json/
│   ├── csv/
│   ├── yaml/
│   ├── html/
│   ├── logs/
│   ├── terminal/
│   ├── code/
│   ├── diff/
│   ├── tables/
│   └── schemas/
│
├── gateway/
│   ├── openai/
│   ├── anthropic/
│   ├── google/
│   └── openrouter/
│
├── proxy/
├── cli/
├── sdk/
│   ├── python/
│   └── typescript/
│
├── benchmarks/
├── tests/
├── docs/
└── examples/
```

---

# 36. Development Roadmap

## Phase 0 — Agent Flight Recorder

Build:

- request tracing
- token accounting
- tool timing
- cache measurement
- latency measurement
- baseline benchmark suite

**Exit criterion:** We can explain where tokens and milliseconds are being spent.

---

## Phase 1 — Zero-Cost Runtime Optimizations

Build:

- exact cache
- request deduplication
- hashes
- content-addressed storage
- deterministic tool cache
- parallel read execution
- provider-cache-aware packing

**Exit criterion:** measurable improvement without ML.

---

## Phase 2 — Repository Intelligence

Build:

- AST indexing
- symbol graph
- dependency graph
- repository map
- incremental indexing

**Exit criterion:** relevant code can be located without reading the whole repository.

---

## Phase 3 — Tool Runtime

Build:

- tool metadata
- scheduler
- DAG execution
- parallelization
- tool deduplication
- safe prefetch
- tool fusion
- latency estimator

**Exit criterion:** fewer tool round trips and lower tool latency.

---

## Phase 4 — Context Runtime

Build:

- persistent state objects
- references
- retrieval
- relevance ranking
- context budgets
- tool-result virtualization
- delta engine

**Exit criterion:** major reduction in repeated context.

---

## Phase 5 — Structural Compression

Build:

- JSON
- logs
- CSV
- YAML
- HTML
- terminal
- diffs
- tables
- schemas
- search results

**Exit criterion:** compression improves token/cost metrics without correctness regressions.

---

## Phase 6 — Provider Optimization

Build provider-specific adapters for:

- prompt caching
- cache boundaries
- token accounting
- streaming
- provider-specific limits

**Exit criterion:** ReToken improves effective cost rather than merely token count.

---

## Phase 7 — Predictive Runtime

Build:

- next-file predictor
- next-tool predictor
- cache usefulness predictor
- compression usefulness predictor
- context sufficiency predictor

Use local traces as the primary learning source.

**Exit criterion:** prediction provides measurable latency reduction without unsafe speculative behavior.

---

# 37. Documentation Requirements

Documentation is a first-class part of ReToken.

The project must not depend on the AI coding agent remembering architectural decisions from previous sessions.

## Required documentation tree

```text
docs/
│
├── README.md
├── ARCHITECTURE.md
├── PRODUCT.md
├── PRINCIPLES.md
├── ROADMAP.md
├── GLOSSARY.md
│
├── architecture/
│   ├── system-overview.md
│   ├── hot-path.md
│   ├── intelligence-plane.md
│   ├── state-engine.md
│   ├── context-runtime.md
│   ├── tool-runtime.md
│   ├── scheduler.md
│   ├── retrieval.md
│   ├── compression.md
│   ├── caching.md
│   └── provider-adapters.md
│
├── protocols/
│   ├── retoken-wire-format.md
│   ├── state-object.md
│   ├── tool-contract.md
│   ├── reference-format.md
│   └── delta-format.md
│
├── decisions/
│   ├── ADR-0001-rust-core.md
│   ├── ADR-0002-local-first.md
│   ├── ADR-0003-state-model.md
│   └── ...
│
├── research/
│   ├── prior-art.md
│   ├── benchmarks.md
│   ├── token-economics.md
│   └── experiments/
│
├── development/
│   ├── setup.md
│   ├── coding-standards.md
│   ├── testing.md
│   ├── debugging.md
│   ├── performance.md
│   └── release-process.md
│
└── ai/
    ├── AI-CODING-GUIDE.md
    ├── PROJECT-CONTEXT.md
    ├── ARCHITECTURE-RULES.md
    ├── CODING-RULES.md
    ├── TESTING-RULES.md
    ├── DO-NOT-DO.md
    ├── TASK-PROTOCOL.md
    ├── CHANGE-PROTOCOL.md
    └── SESSION-HANDOFF.md
```

---

# 38. AI-Assisted Development Documentation

This section is mandatory.

Because ReToken itself is intended to be developed heavily with AI coding agents, the repository must contain a dedicated **AI Engineering Context Layer**.

The purpose is to prevent an AI coding agent from:

- misunderstanding the architecture
- rewriting existing components
- introducing duplicate systems
- violating hot-path constraints
- breaking provider caching
- adding unnecessary dependencies
- optimizing token count while increasing latency
- accidentally changing architectural contracts

## 38.1 AI-CODING-GUIDE.md

This is the entry point for AI coding agents.

It should contain:

```text
1. What ReToken is
2. What ReToken is not
3. Architecture overview
4. Core principles
5. Repository structure
6. Current implementation status
7. Important invariants
8. How to run tests
9. How to run benchmarks
10. How to make changes
11. Documentation rules
12. Forbidden architectural shortcuts
```

The AI agent should read this before substantial work.

---

# 39. PROJECT-CONTEXT.md

This is the compact persistent project memory for AI agents.

It should describe:

```text
Project:
ReToken

Mission:
Optimize AI coding-agent cost and latency.

Primary principle:
Minimum sufficient context.

Core architecture:
Gateway → State → Planner → Runtime → Provider.

Performance rule:
No expensive synchronous work on the hot path.

Current phase:
<updated automatically/manually>

Implemented:
...

In progress:
...

Known limitations:
...

Open architectural questions:
...
```

Keep this document concise.

Do not turn it into a giant history file.

---

# 40. ARCHITECTURE-RULES.md

This file contains non-negotiable invariants.

Example rules:

```text
RULE-001
Never add external network calls to the hot path without explicit architectural approval.

RULE-002
Never use an LLM to perform deterministic local transformations.

RULE-003
Never discard original data when using lossy compression.

RULE-004
Never bypass cache invalidation rules.

RULE-005
Never parallelize state-mutating operations unless dependencies explicitly permit it.

RULE-006
Never optimize token count at the expense of measurable latency without documenting the tradeoff.

RULE-007
Provider-specific behavior belongs in provider adapters, not core logic.

RULE-008
Background intelligence must not be required for basic correctness.

RULE-009
All new optimizers require a benchmark.

RULE-010
Every architectural change requires documentation.
```

---

# 41. CODING-RULES.md

This should define implementation conventions.

Include:

- naming
- module boundaries
- error handling
- concurrency
- logging
- configuration
- dependency policy
- API stability
- performance conventions
- unsafe-code policy
- testing expectations

Example:

```text
Do not introduce a dependency for functionality that can be implemented
in the standard library unless there is a measurable reason.
```

---

# 42. TESTING-RULES.md

AI agents must know how ReToken is validated.

Required categories:

```text
unit tests
integration tests
property tests
concurrency tests
cache invalidation tests
recovery tests
benchmark tests
provider adapter tests
end-to-end agent tests
```

Every optimization must include a regression test.

---

# 43. DO-NOT-DO.md

This is especially important for AI coding.

It should list known dangerous patterns.

Examples:

```text
DO NOT:
- rewrite the architecture to make a local task easier
- introduce a global singleton without approval
- add an external API dependency to the hot path
- silently drop tool output
- use lossy compression without recovery
- assume a cache entry is valid forever
- mix provider-specific logic into core
- optimize benchmarks only
- delete failing tests
- disable correctness checks to improve token numbers
- replace a deterministic algorithm with an LLM
- perform speculative writes
```

---

# 44. TASK-PROTOCOL.md

AI agents should follow a standard task lifecycle.

```text
1. Read PROJECT-CONTEXT.md
2. Read relevant ARCHITECTURE documents
3. Inspect current implementation
4. Identify existing abstractions
5. State intended change
6. Implement smallest correct change
7. Run focused tests
8. Run relevant integration tests
9. Run benchmark if performance-related
10. Update documentation
11. Record architectural decision if necessary
12. Provide concise handoff
```

This reduces AI-induced architectural drift.

---

# 45. CHANGE-PROTOCOL.md

Every significant AI-generated change should contain:

```text
Problem
Why existing behavior is insufficient
Chosen solution
Alternatives considered
Performance impact
Correctness impact
Cache impact
Hot-path impact
Tests added
Documentation updated
```

---

# 46. SESSION-HANDOFF.md

AI coding agents frequently operate across multiple sessions.

This file should contain only current state:

```text
Current objective:
...

Completed:
...

Current implementation:
...

Tests:
...

Known failures:
...

Next recommended task:
...

Do not repeat:
...
```

It should be updated at the end of substantial AI coding sessions.

---

# 47. Architecture Decision Records

Use ADRs for irreversible or high-impact decisions.

Format:

```text
# ADR-XXXX — Decision Title

Status:
Accepted / Proposed / Superseded

Context:
...

Decision:
...

Alternatives:
...

Consequences:
...

Performance implications:
...

Security implications:
...
```

Examples:

- Rust vs Go
- SQLite vs embedded database
- content-addressed storage
- state-reference protocol
- provider abstraction
- compression policy
- scheduler semantics
- cache invalidation strategy

---

# 48. Research Log

Every major experiment should be recorded.

```text
research/experiments/YYYY-MM-DD-name.md
```

Template:

```text
# Experiment

Hypothesis:
...

Baseline:
...

Method:
...

Dataset/workload:
...

Results:
...

Latency:
...

Token usage:
...

Cost:
...

Correctness:
...

Conclusion:
...

Next experiment:
...
```

This prevents the project from repeatedly testing the same idea.

---

# 49. Benchmark Record Format

Store machine-readable benchmark results where possible.

Example:

```json
{
  "task": "fix-auth-bug",
  "baseline": {
    "input_tokens": 184200,
    "output_tokens": 8200,
    "tool_calls": 24,
    "latency_ms": 42100
  },
  "retoken": {
    "input_tokens": 31400,
    "output_tokens": 5100,
    "tool_calls": 11,
    "latency_ms": 19100
  },
  "correctness": true
}
```

This enables historical comparison.

---

# 50. AI Agent Operating Model

The repository should treat AI coding agents as contributors with strict interfaces.

### Before coding

AI reads:

```text
AI-CODING-GUIDE.md
PROJECT-CONTEXT.md
ARCHITECTURE-RULES.md
relevant module docs
```

### During coding

AI must:

```text
inspect
plan
modify
test
measure
document
```

### After coding

AI must report:

```text
what changed
why
tests
performance impact
known limitations
docs changed
```

---

# 51. Context Budget for AI Developers

The project documentation itself should be optimized.

Do not give an AI coding agent every document on every task.

Use progressive disclosure:

```text
Level 0
AI-CODING-GUIDE.md

Level 1
PROJECT-CONTEXT.md
ARCHITECTURE-RULES.md

Level 2
Relevant component documentation

Level 3
Specific protocol/API documentation

Level 4
Source code

Level 5
Research/ADRs only when needed
```

This mirrors ReToken's own architecture.

---

# 52. Documentation as Machine-Readable Architecture

Important architectural metadata should eventually be represented in structured files.

Potential:

```text
architecture/components.yaml
architecture/dependencies.yaml
architecture/invariants.yaml
tools/tool-registry.yaml
benchmarks/workloads.yaml
providers/providers.yaml
```

This allows AI agents and developer tooling to query architecture without reading huge documents.

---

# 53. AI Coding Prompt Contract

Every substantial AI coding task should be framed as:

```text
TASK
<exact objective>

SCOPE
<allowed files/modules>

CONSTRAINTS
<architectural constraints>

SUCCESS CRITERIA
<tests/benchmarks>

DO NOT CHANGE
<protected interfaces>

RELEVANT DOCUMENTATION
<links/paths>

EXPECTED OUTPUT
<implementation + tests + docs>
```

This is preferable to vague prompts such as:

> “Build the scheduler.”

---

# 54. Versioning

ReToken should use semantic versioning where applicable.

Architecture documents should include:

```text
version
status
last updated
owner
related components
```

Protocol changes require explicit versioning.

---

# 55. Security Boundaries

Components should have explicit trust levels.

```text
USER INPUT
   ↓
AGENT
   ↓
RETOKEN
   ↓
LOCAL STATE
   ↓
TOOLS
   ↓
PROVIDER
```

Potentially dangerous actions must be clearly classified.

Speculative execution is restricted to safe read-only operations by default.

---

# 56. Configuration

Configuration should allow:

```yaml
runtime:
  mode: balanced

optimization:
  caching: true
  delta: true
  compression: true
  speculative_prefetch: true

performance:
  hot_path_budget_ms: 10

safety:
  speculative_writes: false

providers:
  anthropic:
    enabled: true
  openai:
    enabled: true
```

Exact configuration format may change during implementation.

---

# 57. Observability Requirements

Provide:

```text
re-token stats
re-token trace
re-token benchmark
re-token cache
re-token doctor
```

Metrics should distinguish:

```text
raw tokens
estimated tokens
provider-reported tokens
cached tokens
effective tokens
```

Never present inferred local estimates as provider billing data.

---

# 58. MVP Definition

The MVP should NOT attempt to build every intelligent feature.

### MVP includes

```text
✓ Gateway
✓ Flight recorder
✓ Content-addressed state
✓ Exact cache
✓ Deterministic tool cache
✓ Repository indexing
✓ Symbol lookup
✓ Basic context selection
✓ Tool-result virtualization
✓ Delta engine
✓ Parallel read scheduling
✓ Basic structural compression
✓ Benchmark harness
✓ Provider adapter
✓ AI development documentation
```

### MVP excludes

```text
✗ advanced semantic predictor
✗ autonomous model routing
✗ complex reinforcement learning
✗ cloud control plane
✗ speculative writes
✗ mandatory local LLM
```

---

# 59. MVP Success Criteria

ReToken MVP is successful if it demonstrates, on reproducible coding workloads:

1. Meaningful input-token reduction.
2. Lower or equal total latency.
3. Fewer tool round trips.
4. No meaningful correctness regression.
5. Safe cache invalidation.
6. Exact recovery of optimized state.
7. Measurable benefit without requiring an external AI service in the hot path.

---

# 60. Long-Term Architecture

The eventual system should evolve toward:

```text
             ┌──────────────────────────────┐
             │        AI CODING AGENT       │
             └──────────────┬───────────────┘
                            │
                            ▼
                  ┌──────────────────┐
                  │ ReToken Runtime  │
                  └────────┬─────────┘
                           │
       ┌───────────────────┼────────────────────┐
       │                   │                    │
       ▼                   ▼                    ▼
  Context OS          Tool Execution       Cache OS
       │                   │                    │
       ▼                   ▼                    ▼
  Repository          Scheduler            Provider
  Graph               Prefetch             Cache
       │                   │                    │
       └───────────────────┼────────────────────┘
                           │
                           ▼
                         MODEL
```

The model should increasingly become the **decision engine**, while ReToken handles mechanical state management, retrieval, execution, reuse, and representation.

---

# 61. Competitive / Prior-Art Positioning

ReToken should learn from existing systems without becoming a fork or rebrand.

Relevant ideas include:

- prompt caching
- repository maps
- prefix/KV caching
- prompt compression
- structural serialization
- retrieval compression
- tool execution optimization
- agent-loop optimization

ReToken's differentiator is the combination:

> **State virtualization + execution optimization + context minimization + cache-aware packing + predictive prefetch in a local-first runtime.**

The project should explicitly document which ideas are prior art and which implementation/design decisions are original.

---

# 62. Licensing and Intellectual Property

ReToken must be independently implemented.

Do not copy source code from projects whose licenses or terms are incompatible with the intended ReToken license.

Prior-art research may inform architecture, but implementation should be original unless a compatible license is deliberately adopted and tracked.

Maintain:

```text
docs/research/prior-art.md
```

with:

- project
- relevant concept
- license
- inspiration
- implementation differences

---

# 63. Open Research Questions

The following are intentionally unresolved:

### RQ1
How can ReToken determine the minimum sufficient context reliably?

### RQ2
When is compression beneficial after accounting for CPU latency?

### RQ3
How can speculative prefetch be made highly accurate?

### RQ4
How should semantic cache similarity be measured safely?

### RQ5
How should provider-specific cache economics affect context planning?

### RQ6
Can agent turn count be reduced without reducing reasoning quality?

### RQ7
Can a local trace model predict the next tool/file with useful accuracy?

### RQ8
What workloads can realistically approach the 100× efficiency target?

---

# 64. Key Engineering Mantra

Every major implementation decision should be evaluated with:

```text
Can we avoid it?
    ↓
Can we reuse it?
    ↓
Can we predict it?
    ↓
Can we parallelize it?
    ↓
Can we select only the relevant part?
    ↓
Can we send only the delta?
    ↓
Can we compress it?
    ↓
Can we cache the resulting request?
```

If the answer is yes earlier in the chain, do not use a later and more expensive technique.

---

# 65. Final Product Definition

ReToken is:

> **A local-first runtime for AI coding agents that minimizes the amount of computation, context, tool execution, and provider interaction required to complete a software-engineering task while preserving correctness.**

The product is not defined by compression.

It is defined by **elimination**.

The ultimate goal is:

```text
Same task
    ↓
Less context
    ↓
Less computation
    ↓
Fewer tool calls
    ↓
Fewer model turns
    ↓
More cache reuse
    ↓
Lower cost
    ↓
Lower latency
    ↓
Same or better result
```

**ReToken's North Star:**

> **Make the agent's virtual context large, while making the provider's actual context small.**
