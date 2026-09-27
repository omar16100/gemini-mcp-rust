# Plan: ship retry, search cache and token usage (27 Sep 2026)

## Goal

Ship the retry, query cache, token usage passthrough and consensus theme work written on 23 Dec 2025 but never committed, after fixing its correctness problems, and give the repository a CI workflow so "tests pass" is checked on every change.

## Scope

In scope:

- Import the 23 Dec 2025 working tree changes (`Cargo.toml`, `src/gemini/client.rs`, `src/gemini/mod.rs`, `src/main.rs`, `src/tools/{analyze,brainstorm,query}.rs`, `todo.md`) plus the untracked `src/cache/mod.rs` and `src/gemini/retry.rs`. `.env` is not copied.
- Fix the search cache key and cache-hit metadata.
- Harden retry (server hints, which errors are safe to replay).
- Verify default model IDs against Google's docs.
- README corrections, docs (`index.md`, `c4model.md`, this plan), `todo.md` update.
- Minimal CI: fmt check, clippy `-D warnings`, tests.

Out of scope: new tools (todo Phases 3 to 6), JSON-RPC notification handling, structured JSON output mode.

## Decisions

1. **Cache the raw Gemini answer, not the parsed result.** The 23 Dec cache stored the post-processed `SearchResult` keyed by `hash(query, source IDs)`. Same IDs with new content, a different model, different generation params, filters, ranking or `include_citations` all returned a stale or wrong result for up to 5 minutes. Now the key is a SHA-256 fingerprint of tool name, resolved model ID, the full prompt (query plus each selected source's ID, title and content) and the generation config, and parsing/filtering/ranking always run on the live request. Filters and ranking therefore do not need to be in the key.
2. **Cache hit metadata.** `ResponseMetadata` gained `cached: bool` (additive field). On a hit, `model_used` is the model that produced the answer (it is part of the key) instead of the literal `"cached"`, and token counts are 0 because no tokens were consumed. Callers summing usage do not double count.
3. **Cache bounds.** Capacity 100 by default (`GEMINI_CACHE_MAX_ENTRIES`), expired entries purged before evicting the oldest. TTL on the monotonic clock (`Instant`) instead of `SystemTime`. TTL or capacity 0 disables. Constructor arguments are clamped to 1 day and 10,000 entries; out-of-range or invalid environment values fall back to the defaults (logged).
4. **Retry hints.** `ApiError` now carries `retry_after`, parsed from `Retry-After` (delta-seconds or HTTP-date, via `httpdate`) or from a `google.rpc.RetryInfo` `retryDelay` in the body. The hint is a floor on the wait; a hint above `max_delay` (30 s) returns the error immediately, which covers exhausted daily quotas.
5. **What is replayed.** Status 408, 429, 500, 502, 503, 504 (not 501, 505 or other 5xx). Transport errors only when the connection could not be established. Client timeouts after sending are not retried: with a 60 s per-attempt timeout, 3 retries could turn one slow Pro call into 4 billed generations and several minutes of latency.
6. **API key in a header.** The key moved from the `?key=` query string to `x-goog-api-key`. reqwest includes the URL in error messages, and those were logged at warn level on every retry and returned to the MCP client.
7. **camelCase wire types.** The API returns `usageMetadata.promptTokenCount` etc.; the snake_case structs never matched, so "token usage passthrough" always reported 0. Request types now also serialize the documented camelCase names. Missing `candidates`/`content`/token fields and unknown part types no longer fail parsing.
8. **Startup check** uses `models.get` for the Pro model instead of a billed `generateContent("Test")`, and so also validates the configured model ID.
9. **Logs to stderr.** `tracing_subscriber` wrote to stdout, which is the MCP JSON-RPC channel.
10. **Model IDs** (checked 27 Sep 2026 on https://ai.google.dev/gemini-api/docs/models and https://ai.google.dev/gemini-api/docs/deprecations, both "Last updated 2026-09-24 UTC"):
    - `gemini-3-pro-preview`: deprecations table lists release 18 Nov 2025, shutdown **9 Mar 2026**, recommended replacement `gemini-3.1-pro-preview`. Default changed to `gemini-3.1-pro-preview` (the only Pro text model on the models page).
    - `gemini-3-flash-preview`: still listed, "No shutdown date announced", recommended replacement `gemini-3.6-flash`. The models page says "For any new projects, use our latest models: 3.5 Flash-Lite or 3.8 Flash." Default changed to `gemini-3.8-flash` (stable).
    - No API key was available in the environment or `~/.env`, so no `models.list` call was made; the IDs are verified from docs only.
11. **Analyzer params wired in.** `gemini-analyze-v2` accepted `params` and `options.detail_level` but ignored both (flagged by clippy as never read). They now set the generation config and add a brief/comprehensive instruction (standard adds nothing, so default prompts are unchanged).
12. **Removed dead code** flagged by `clippy -D warnings`: unused `generate_with_history`, unused `AuthError`/`ConfigError` variants, unused re-exports, the unread `jsonrpc` request field.
13. **Consensus clustering on word boundaries.** The imported clustering merged themes by substring (`"rain"` into `"training"`), inflating frequencies. It now matches whole words, and the weakened 23 Dec test was strengthened back to exact membership (ideas 1 to 3, frequency 3).

### Codex review 1 (gpt-6-astra, output kept outside the repository)

No blockers. Applied:

- Major: reqwest followed redirects and would forward `x-goog-api-key` to another host (it strips only standard auth headers) and could replay a POST. Redirects are now disabled; a 3xx is returned as an error. Test added.
- Major: an unreadable error body (stall until timeout, dropped connection) became `ApiError { status: 503, message: "Unknown error" }` and was retried. It is now returned as the transport error, which is not retried. Test with a raw TCP server that drops mid-body.
- Minor: retry hint precedence now takes the larger of header and body hints (a `Retry-After: 0` no longer overrides a 3 s `RetryInfo`). Unit tests via `http::Response`.
- Minor: clustering fix (13 above) confirmed.
- Minor: `GEMINI_MAX_RETRIES=0` does not stop reqwest's internal protocol-level retries. Kept (they only resend requests refused before processing) and documented.
- Minor: removed a wall-clock upper bound from a retry test, added a timeout to the connect-error test client, added tests for a nonzero body hint delaying the retry, startup-check retries, and `min_relevance` / `max_results` applying to cached answers.
- Minor: cache memory bounded by bytes, not just entries: answers over 256 KiB are not cached, expired entries are purged on every insert.
- Minor: docs: CI runs on pushes to `master` and PRs; c4model no longer claims rustls; README says v2 JSON is inside `content[0].text` and that search ranking scores are fixed at 0.7.
- Minor, pre-existing: search now logs query and source sizes instead of the query text at info level. The `--verbose` log of each raw JSON-RPC line is unchanged (debug only).
- Also: whitespace-only answers are `EmptyResponse`.

### Codex review 2 (gpt-6-astra)

No blocker or major; both earlier majors confirmed fixed; "approve with nits, contingent on green CI". Applied the nits: the truncated-body test now drains the full request, checks server I/O and asserts a body/decode error (not a send error); the redirect comment says redirects are unexpected rather than impossible; the cache limits wording distinguishes constructor clamping from env fallback. Not applied: holding a bound, non-listening socket for the connect-refused test, because on macOS the connect then times out (5 s) instead of being refused (tried locally); the test keeps the bind-then-release port, whose race needs another process to take that exact ephemeral port within microseconds.

## Status

| Step | Status |
| --- | --- |
| Worktree `feat/retry-and-cache` from `origin/master`, patch applied, `src/cache/mod.rs` and `src/gemini/retry.rs` copied | Done |
| Cache key and metadata fix with tests | Done |
| Retry hardening with tests | Done |
| Model ID check and default update | Done (docs only) |
| fmt, clippy `-D warnings`, tests | Done locally |
| README, docs, todo.md | Done |
| CI workflow | Added |
| Codex review 1 | Done, findings applied (see above) |
| Codex review 2 | Done, nits applied |
| PR, CI green, squash-merge | In the PR that adds this doc; merge only after CI is green |

## Verification

- `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (92 tests): all clean locally (rustc 1.95.0), test suite repeated 8 times without failure.
- Tests use mockito on localhost; none call the real Gemini API. Cache tests cover: same source IDs with changed content miss, title/model/params changes miss, a hit reports the original model with `cached: true` and zero tokens, post-processing options still apply to cached answers, capacity eviction, TTL expiry, disabled cache. Retry tests cover retryable statuses, connect errors, `Retry-After` seconds and HTTP-date, `RetryInfo` parsing, header/body hint precedence, hint floor, hint above cap not retried, zero retries, redirects not followed, truncated error bodies not retried, startup-check retries.
- Binary run without `GEMINI_API_KEY`: stdout empty, log line and error on stderr.
- Not tested live: any call to the real Gemini API (no key available), including whether `gemini-3.1-pro-preview` returns text within the tools' default `maxOutputTokens` (2048 for search/brainstorm, 256 for brief summaries) given Gemini 3 thinking.

## Deviations

- Beyond the listed task: API key header, camelCase wire types, stderr logging, `models.get` startup check, analyzer params wiring, dead code removal. Each fixes a defect found while making the imported work build cleanly under `clippy -D warnings` or while verifying that token usage and retry logging behave as documented.
- `Cargo.lock` stays gitignored (existing policy), so CI runs `cargo test --workspace` without `--locked`.
