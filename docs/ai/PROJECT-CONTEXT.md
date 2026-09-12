# Project Context

**Project:** ReToken
**Mission:** Optimize AI coding-agent cost and latency.
**Primary principle:** Minimum sufficient context.

**Core architecture:**
Gateway → State → Planner → Runtime → Provider.

**Performance rule:**
No expensive synchronous work on the hot path (target < 10ms overhead).

**Current phase:**
Phase 0 — Agent Flight Recorder (Observability & Tracing).

**Implemented:**
- Scaffolded workspace and basic telemetry skeleton.
- AI-Assisted Documentation Layer.

**In progress:**
- CLI setup and Gateway API observability.

**Known limitations:**
- Currently in prototyping phase; no actual provider interceptors implemented yet.

**Open architectural questions:**
- RQ1: How to determine minimum sufficient context reliably.
- RQ2: When compression is beneficial after CPU latency accounting.
- RQ3: How to make speculative prefetch accurate.
