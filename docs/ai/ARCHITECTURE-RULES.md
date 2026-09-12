# Architecture Rules

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
