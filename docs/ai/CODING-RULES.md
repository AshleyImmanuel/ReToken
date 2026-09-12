# Coding Rules

- **Dependencies**: Do not introduce a dependency for functionality that can be implemented in the standard library unless there is a measurable reason.
- **Language**: Rust. Prefer safety. Avoid `unsafe` unless absolutely required for performance bottlenecks, heavily documented and reviewed.
- **Concurrency**: Hot path logic must minimize lock contention. Background work (Intelligence plane) should be async and cancellable.
- **Error Handling**: Fail open. If an optimization fails, fall back to passing the original, unmodified request through without breaking the user experience.
- **Privacy**: Never log API keys, tokens, passwords, private keys, session cookies, or environment secrets. Redact before observability storage.
- **Module Boundaries**: Strictly adhere to the layer segregation (e.g., analyzers vs compressors vs gateway).
