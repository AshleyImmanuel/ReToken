# ReToken Architecture

ReToken is a highly-optimized local proxy sitting between IDEs (like Cursor, Windsurf) and LLM providers. Its primary goal is to **cut API costs** and **reduce latency** through aggressive token compression, intelligent caching, and payload optimization.

## System Components

The workspace is divided into several highly-specialized Rust crates:

### 1. `proxy` / `gateway`
The core HTTP interceptor. It listens on `http://127.0.0.1:3000` and parses incoming API requests.
- **Provider Detection**: Automatically routes requests to OpenAI, Anthropic, or generic endpoints based on the path.
- **Optimization Pipeline (`optimizer.rs`)**: Applies all payload modifications before sending them over the network.
- **Bypass Mode**: Natively handles streaming and zero-copy bypass for requests marked with `X-ReToken-Bypass`.

### 2. `core`
Handles the fundamental state and configuration of ReToken.
- **State Engine**: A multi-tenant, concurrent map that tracks ongoing and historical requests.
- **Semantic Cache**: An in-memory cache that evaluates request hashes (including context modifications) to instantly serve duplicate requests.
- **Telemetry (`FlightRecord`)**: Records detailed observability metrics for every request (tokens saved, compression ratios, cache hits).

### 3. `retrieval`
Responsible for manipulating the structural JSON of LLM requests.
- **Packer**: Reorders message sequences to maximize Anthropic's native `cache_control` blocks.
- **Terse Injection**: Inserts high-priority system instructions compelling the LLM to output only raw code, drastically reducing generation time and output tokens.
- **Planner**: Ranks active files based on relevance and ensures the context package stays strictly within the user-defined token budget.

### 4. `compressors`
Dedicated parsers for stripping unnecessary data from requests.
- **Schema Compression**: Strips bloated human-readable `description` and `examples` fields from tool/function schemas.
- **JSON Compaction**: Removes nulls and redundant spacing.
- **Log Deduplication**: Truncates and slices massive error logs into manageable chunks before they reach the LLM context window.

### 5. `scheduler` & `analyzers`
*(Future Phase Features)*
- **Scheduler**: A DAG-based dependency resolver for queuing asynchronous task executions (e.g., fetching web content, running lints).
- **Analyzers**: AST-style scanners that rapidly walk the local repository to construct a Dependency Graph of symbols, functions, and imports.

## Data Flow (Hot Path)

1. **Request Intercepted**: IDE sends a POST to `http://127.0.0.1:3000/v1/messages`.
2. **Detection**: `gateway` identifies the target provider.
3. **Cache Lookup**: `core::cache` checks if an exact request hash has been seen. If YES -> return cached response (0ms latency, 0 tokens billed).
4. **Optimization**: 
   - Apply Terse mode instructions.
   - Compress tool schemas.
   - Reorder Anthropic messages for `cache_control`.
5. **Execution**: The modified payload is forwarded to the upstream provider via `reqwest`.
6. **Telemetry**: Token usage is extracted from the response headers/body, and the `FlightRecord` is logged to tracking.
7. **Cache Update**: The response is saved in the local cache for future lookups.
