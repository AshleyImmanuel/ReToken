# Contributing to ReToken

Thanks for helping make coding agents cheaper to run. Bug reports, benchmarks from your own workloads, docs fixes and new compression strategies are all welcome.

## Ways to contribute

- **Report a bug**: open an issue with the agent, provider, ReToken version and the proxy log lines around the failure. Remove API keys and private code first.
- **Share a benchmark**: cost, turns and correctness for your agent and task, with and without ReToken. Real workloads are the most useful evidence the project has.
- **Fix or improve code**: pick an issue labelled `good first issue` or `help wanted`, or open an issue to discuss a larger idea first.
- **Improve docs**: unclear setup steps in `README.md` or `AI_INSTALL.md` are bugs too.

## Set up

You need a stable Rust toolchain (`rustup`), with `rustfmt` and `clippy`.

```bash
git clone https://github.com/AshleyImmanuel/ReToken.git
cd ReToken
cargo build --workspace
cargo test --workspace
```

Run your build against a real agent:

```bash
cargo run --release -p cli -- run                  # proxy on 127.0.0.1:8888
ANTHROPIC_BASE_URL=http://127.0.0.1:8888 claude    # in another terminal
```

## Where things live

| Crate | What it owns |
|---|---|
| `gateway` | HTTP proxy: routing, request handling, caching, streaming, admin endpoints |
| `compressors` | Tool-output compression (`tool_output/`), log and JSON compression, code outlines, the CCR store |
| `retrieval` | Prompt-cache breakpoint placement (`packer.rs`) and context planning |
| `core` | Configuration (`AppConfig`), response cache, telemetry |
| `cli` | The `retoken` command: `run`, `wrap`, `mcp`, `doctor`, `stats`, `cache` |
| `mcp` | The `retoken-mcp` admin MCP server |

`ARCHITECTURE.md` describes the request flow. The module docs at the top of `compressors/src/tool_output/mod.rs` state the compression invariants; read them before changing what reaches the model.

## Rules for changes that touch model input

ReToken is only useful if agents stay as accurate as without it. Changes to compression must keep these invariants:

1. Never shorten a tool result the model has not had a turn with yet, except as the module docs allow.
2. Every lossy change must be recoverable through `retoken_retrieve`; without a retrieval route, only lossless changes apply.
3. A tool result may change at most once across turns, so provider prompt caches stay valid.
4. Never modify user prompts, system prompts or assistant turns.

Add a test that shows the new behaviour and a test that shows what must stay untouched.

## Before you open a pull request

CI runs these on Linux, macOS and Windows; run them locally first:

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

- Keep each pull request to one change, with a test.
- Use [Conventional Commits](https://www.conventionalcommits.org/) messages, such as `fix(tool_output): ...` or `docs(readme): ...`.
- Explain in the description why the change is needed and how you verified it. For cost changes, include measured numbers.

## Security issues

Do not open a public issue for a vulnerability. Report it privately through GitHub's **Security > Report a vulnerability** form for this repository.

## License

By contributing, you agree that your contributions are licensed under the MIT License in `LICENSE`.
