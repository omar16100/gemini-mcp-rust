# C4 model: gemini-mcp-rust

Architecture source of truth. Read before architecture changes; update in the same change as any change to containers, components, dependencies or data flows.

## Context

```
+-------------------+   stdio JSON-RPC    +-------------------+   HTTPS (REST)   +---------------------------+
| MCP client        | ------------------> | gemini-mcp        | ---------------> | Gemini API                |
| (Claude Code or   | <------------------ | (this repo)       | <--------------- | generativelanguage.       |
|  other MCP host)  |   tool results      |                   |   JSON           | googleapis.com/v1beta     |
+-------------------+                     +-------------------+                  +---------------------------+
```

- The MCP client launches the binary as a child process and talks JSON-RPC over stdin/stdout.
- The server calls the Gemini REST API with the user's `GEMINI_API_KEY` (sent in the `x-goog-api-key` header).
- No other external systems. No database, no disk writes.

## Containers

| Container | Tech | Responsibility |
| --- | --- | --- |
| `gemini-mcp` binary | Rust, tokio, reqwest (default TLS backend) | Single process: MCP stdio server, tool logic, Gemini HTTP client, in-memory search cache. Logs to stderr. |

Configuration is environment only: `GEMINI_API_KEY`, `GEMINI_PRO_MODEL`, `GEMINI_FLASH_MODEL`, `GEMINI_MAX_RETRIES`, `GEMINI_CACHE_TTL_SECS`, `GEMINI_CACHE_MAX_ENTRIES`, plus `--verbose` / `--quiet` flags. A `.env` file in the working directory is loaded if present.

## Components

| Component | File | Responsibility |
| --- | --- | --- |
| Entry point | `src/main.rs` | Parse CLI flags, configure logging (stderr), read `GEMINI_API_KEY`, run the startup check, start the server. |
| MCP server | `src/mcp/server.rs` | Line-delimited JSON-RPC loop on stdin/stdout. Handles `initialize`, `tools/list` (9 tools) and `tools/call`, dispatching to tools. Requests are processed one at a time. |
| Tools | `src/tools/{query,analyze,summarize,brainstorm}.rs` | Build prompts from tool arguments, call the Gemini client, parse text into v1 strings or v2 `ToolResponse { result, metadata }`. |
| Tool types | `src/tools/types.rs` | `ToolResponse`, `ResponseMetadata` (`model_used`, token counts, `cached`), `ModelPreference`, `GenerationParams::to_config`. |
| Gemini client | `src/gemini/client.rs` | `generateContent` requests, `models.get` startup check, API key header, redirects disabled (a redirect would forward the key header and could replay a POST). Maps non-2xx responses to `ApiError` with the larger of the header and body retry hints; an unreadable error body becomes a non-retried transport error. Concatenates the first candidate's text parts (whitespace-only is `EmptyResponse`). Resolves `pro` / `flash` to configured model IDs. |
| Retry | `src/gemini/retry.rs` | `retry_with_backoff`: retries 408, 429, 500, 502, 503, 504 and connection failures, up to `GEMINI_MAX_RETRIES` (default 3). Delay 1 s doubling with +/-25% jitter, capped at 30 s. `Retry-After` (seconds or HTTP date) or `google.rpc.RetryInfo.retryDelay` sets a floor; hints above 30 s stop retrying. Timeouts after the request was sent are not retried (possible duplicate billing). reqwest's own protocol-level retries (requests refused before processing) are left at their defaults. |
| Search cache | `src/cache/mod.rs` | `QueryCache<T>`: mutex-guarded map with monotonic (`Instant`) TTL (default 300 s) and a capacity bound (default 100). Every insert purges expired entries, then evicts the oldest if still full. `fingerprint` gives length-prefixed SHA-256 keys. Used only by `gemini-search-v2`, which caches the raw Gemini answer text and skips answers over 256 KiB, bounding memory to about capacity x 256 KiB. |
| Models | `src/gemini/models.rs` | Default IDs: `gemini-3.1-pro-preview` (Pro), `gemini-3.8-flash` (Flash). |
| Wire types | `src/gemini/types.rs` | camelCase request/response structs; tolerant of missing fields and unknown part types. |
| Errors | `src/error.rs` | `GeminiError`: `HttpClient`, `ApiError { status, message, retry_after }`, `JsonParse`, `EmptyResponse`. |

## Data flows

### Startup

1. `main` loads `.env`, configures logging to stderr, reads `GEMINI_API_KEY`.
2. `GeminiClient::new` resolves model IDs and the retry policy from env.
3. `test_connection` sends `GET /models/{pro_model}` (with retries). Failure exits the process.
4. The stdio loop starts.

### Tool call (all tools)

1. Client sends `tools/call` on stdin. The server deserializes arguments into the tool's input type.
2. The tool builds a prompt and a `GenerationConfig` (tool defaults overridden by `params`).
3. `GeminiClient::generate_content` POSTs `/models/{model}:generateContent` inside `retry_with_backoff`.
4. The response's text parts and `usageMetadata` become `GenerationResponse { text, usage }`.
5. The tool parses the text into its result; v2 tools attach `ResponseMetadata::with_usage(model_id, usage)`.
6. The server writes one JSON-RPC response line to stdout.

### `gemini-search-v2` with cache

1. Validate input, apply `filters.source_ids`, build the prompt from the query and the selected sources (ID, title, content).
2. Key = `fingerprint(["gemini-search-v2", model_id, prompt, Debug(config)])`.
3. Hit: reuse the cached answer text; metadata is `ResponseMetadata::from_cache(model_id)` (`cached: true`, zero tokens).
4. Miss: call Gemini as above; store the answer text under the key unless it exceeds 256 KiB.
5. On both paths, parse answer, results and citations, then apply `min_relevance`, `ranking`, `max_results` and `include_citations` from the live request.

Concurrency: the cache is shared process-wide behind a `Mutex` (poisoning tolerated). There is no request coalescing, so concurrent identical misses each call the API; with the current sequential stdio loop this does not occur.

## Change log

| Date | Change |
| --- | --- |
| 27 Sep 2026 | Created. Added retry component (`retry.rs`) and search cache component (`cache/`), shipped from 23 Dec 2025 work with fixes: complete cache key (model, full prompt incl. source content, config), bounded capacity, `Instant` TTL, `cached` metadata flag; retry hints from `Retry-After` / `RetryInfo`, connect-only transport retries. API key moved from URL query to header. Logs moved to stderr. Startup check switched from a billed `generateContent` to `models.get`. Response parsing switched to camelCase so token usage is populated. Default models updated to `gemini-3.1-pro-preview` / `gemini-3.8-flash`. After codex review: redirects disabled, unreadable error bodies not retried, retry hint = max(header, body), cache purges expired entries on every insert and skips answers over 256 KiB, search logs sizes instead of query text. Plan: [27092026_retry_cache_plan.md](27092026_retry_cache_plan.md). |
