# ReToken AI Coding Guide

## 1. What ReToken is
ReToken is a local-first Agent Runtime Optimizer designed for AI coding agents. It sits between an AI coding agent and its tools/model providers. It minimizes context, tool execution, and provider interaction by reusing state, eliminating redundant calls, predicting next steps, and performing cache-aware scheduling.

## 2. What ReToken is not
It is not primarily a prompt compressor, a general-purpose LLM, an autonomous coding agent, or a mandatory cloud-hosted service.

## 3. Architecture overview
Gateway → State Engine / Tool Runtime → Context Planner → Retrieval / Compression Planner → Cache-Aware Packer → Provider API

## 4. Core principles
1. Eliminate before compressing (Reuse -> Predict -> Parallelize -> Select -> Delta -> Compress)
2. Local state is the source of truth
3. Hot path must be cheap (<10ms overhead, no external LLM calls)
4. Exact recovery of any lossy representations
5. Cache awareness (optimize for provider prompt caches)
6. Correctness before savings

## 5. Repository structure
- `core/`: runtime logic, state engine, cache
- `analyzers/`: codebase indexing, AST parsing
- `retrieval/`: local context retrieval
- `scheduler/`: tool DAG scheduling and prefetching
- `compressors/`: structural compression and diffing
- `gateway/`: provider adapter and interceptor
- `proxy/`: local networking
- `cli/`: terminal entry points

## 6. Current implementation status
Bootstrapping Phase 0 (Agent Flight Recorder) for request tracing and observability.

## 7. Important invariants
No synchronous external network calls on the hot path. Background intelligence must not block the hot path. All new optimizations require benchmarks and must fallback to exact data if unsure.

## 8. How to run tests
`cargo test`

## 9. How to run benchmarks
`cargo bench` (to be implemented)

## 10. How to make changes
Follow `TASK-PROTOCOL.md` and `CHANGE-PROTOCOL.md`. Smallest correct changes are preferred. Add tests and update docs.

## 11. Documentation rules
Docs are our AI architectural state. Update `PROJECT-CONTEXT.md` on state changes. Use ADRs for major decisions.

## 12. Forbidden architectural shortcuts
See `DO-NOT-DO.md` for strict boundaries on AI agent behavior.
