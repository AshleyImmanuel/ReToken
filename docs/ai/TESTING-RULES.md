# Testing Rules

ReToken must be heavily validated to ensure 100% correctness of optimization techniques against baseline agent behavior.

### Required Categories:
- Unit tests
- Integration tests
- Property tests
- Concurrency tests
- Cache invalidation tests
- Recovery tests
- Benchmark tests
- Provider adapter tests
- End-to-end agent tests

### Constraints:
Every optimization must include a regression test ensuring that it does not silently replace required exact data with an unsafe approximation. If tests fail due to token reduction optimizations, the optimization is considered a failure.
