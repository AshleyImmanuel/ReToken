# ReToken Architecture

ReToken is a local proxy sitting between agentic IDEs (Cursor, Windsurf, RooCode) and LLM providers. Its primary goal is to **cut API costs** and **reduce latency** through structural payload compression, exact-hash caching, and prompt cache optimization.

## System Components

The workspace is divided into specialized Rust crates:

### 1. `gateway`
The core HTTP interceptor listening on `127.0.0.1:8888` (configurable):
- **Provider Detection & URL Normalization**: Routes requests to OpenAI, Anthropic, or generic upstream endpoints based on paths (`/v1/messages`, `/v1/chat/completions`) and headers, normalizing `/v1` path prefixes.
- **Upstream Validation**: Enforces valid `http`/`https` schemes with non-empty hosts on custom upstream URLs, preventing unsafe forwarding.
- **Optimization Pipeline (`optimizer.rs`)**: Applies provider-aware persona injection, tool-output compression (see `compressors`), and prompt caching breakpoints. It picks a recovery mode per request: a client-provided `*retoken_retrieve` tool (the `retoken mcp` server, which also works for streaming), proxy-side recovery (non-streaming Anthropic with `ccr_enabled`), or none (lossless transforms only).
- **Bypass Mode & Streaming**: Preserves upstream response headers (`content-type`, rate limits) and passes streaming chunked responses (`stream: true`) through natively without buffering or caching.

### 2. `core`
Handles configuration, caching, and state tracking:
- **Exact Request Cache (`ResponseCache`)**: In-memory cache keyed by SHA256 of HTTP method, endpoint path, provider name, request payload, and repo hash. Enforces TTL expiry and FIFO/LRU eviction.
- **State Engine (`StateEngine`)**: In-memory session tracking for objects and hashes (roadmap: persistent multi-tenant backend).
- **Telemetry (`FlightRecord`)**: Records detailed observability metrics for every request (token counts, compression ratios, cache hits/misses, duration).

### 3. `compressors`
Dedicated heuristic parsers for request payloads:
- **Content-Addressed CCR Store (`CcrStore`)**: Stores each original as its own file (`<data dir>/ccr/<sha256>`, written atomically before the elided request is forwarded), so the separate `retoken mcp` process can serve it immediately. Handles are validated strictly before any filesystem access; entries older than 7 days are purged at gateway startup.
- **Tool-Output Compression (`tool_output`)**: Attributes each tool result to the tool call that produced it (`calls`), classifies the call as a read/search or a command (`classify`), and applies the policy (`policy`): exact duplicates of earlier tool output are replaced losslessly; large command/log output and uniform JSON arrays are elided only when a retrieval route exists; file reads are verbatim while fresh (after the last assistant turn) and outlined by `code_outline` once stale; searches and git output are never elided. Transforms are stateless and depend only on the preceding history and freshness, so each tool result changes at most once and provider prompt caches keep hitting.
- **Log Slicing**: Truncates massive log outputs while preserving error lines and head/tail context.
- **JSON Compaction**: Collapses large uniform arrays with MAD anomaly preservation.
- **`retoken mcp` (cli)**: stdio MCP server exposing `retoken_retrieve(handle, start_line?, end_line?)` over the shared CCR store.

### 4. `retrieval`
Context budgeting, prioritization, and message formatting:
- **Context Package (`ContextPackage`)**: Manages token budgets and tracks included object IDs.
- **Context Planner (`ContextPlanner`)**: Ranks files using the dependency graph and active workspace files to prioritize relevant context within the token budget.
- **Packer (`packer.rs`)**: Reorganizes misplaced system messages into top-level parameters and injects Anthropic content block `cache_control` breakpoints without disturbing message sequence.

### 5. `scheduler` & `analyzers`
Modular scheduling and code intelligence:
- **Execution DAG (`ExecutionDag`)**: Generates topological execution batches, grouping independent read-only tool calls into parallel batches while enforcing sequential execution for mutating operations.
- **Prefetcher (`Prefetcher`)**: Predicts likely next files based on imports, reverse dependencies, and test patterns.
- **Virtualization (`slice_log`)**: Slices excessive command output into head and tail slices.
- **Task Manager (`TaskManager`)**: Tracks background task status and execution (roadmap: containerized virtualization).
- **Analyzers**: AST symbol extractor and file walker to construct the repository dependency graph.

## Data Flow (Hot Path)

1. **Request Intercepted**: IDE or CLI agent sends an HTTP request to `http://127.0.0.1:8888`.
2. **Provider Detection & Validation**: `gateway::provider` identifies the provider, validates custom upstreams, and normalizes target URLs.
3. **Cache Lookup**: If non-streaming and caching is enabled, `ResponseCache` evaluates the context-aware cache key. If hit -> returns cached response directly.
4. **Optimization Pipeline**:
   - Applies provider-safe persona prompt.
   - Removes exact duplicate tool outputs; when a retrieval route exists, stores large command/JSON output in CCR and elides it with a marker naming the retrieval tool.
   - Injects Anthropic prompt cache breakpoints.
5. **Upstream Forwarding**: Forwards the payload to the provider using `reqwest`, stripping `Host`, `Content-Length`, and hop-by-hop headers.
6. **Response Processing**:
   - Streaming responses pass through immediately.
   - Non-streaming Anthropic responses: intercepts `retoken_retrieve` tool calls, resolves original data from CCR, and updates `stop_reason` to `end_turn`.
   - Successful non-streaming responses are stored in `ResponseCache`.
7. **Telemetry**: Logs token usage, savings, and performance to `FlightRecord`.
