# DO NOT DO

These are strict limitations for AI Coding Agents working on ReToken.

**DO NOT:**
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
