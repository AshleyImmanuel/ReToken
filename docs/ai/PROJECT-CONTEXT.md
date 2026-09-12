# Project Context

**Project:** ReToken
**Mission:** Optimize AI coding-agent cost and latency.
**Primary principle:** Minimum sufficient context.

**Core architecture:**
Gateway → State → Planner → Runtime → Provider.

**Performance rule:**
No expensive synchronous work on the hot path (target < 10ms overhead).

**Current phase:**
Phase 2 — Optimization, Caching & Integration.

**Implemented:**
- Core proxy daemon with `axum` routing.
- Multi-provider support (OpenAI, Anthropic, generic).
- Zero-config Provider caching optimizations (Message reordering, `cache_control` injection).
- Terse Mode Injection for massive output token reduction.
- Semantic response caching with in-memory store.
- Tool Schema compression (stripping descriptions to save input tokens).
- AI Agent installer protocol (`AI_INSTALL.md`).

**In progress:**
- Zero-copy bypass streaming and SSE (`stream: true`) support for LLM responses.
- SQLite-backed persistent cache storage.

**Known limitations:**
- Currently using an in-memory `ResponseCache`. Needs persistence (SQLite).
- Does not yet support native streaming proxying; currently buffers the entire response.

**Open architectural questions:**
- RQ1: How to determine minimum sufficient context reliably across large workspaces.
- RQ2: When compression is beneficial after CPU latency accounting.
- RQ3: How to securely parse and track SSE streams on the fly without breaking connection streams.
