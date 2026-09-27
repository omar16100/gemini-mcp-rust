# Week 2 Implementation TODO

## Setup Phase
- [x] Add schemars dependency to Cargo.toml
- [x] Remove image_gen tool
- [x] Create types.rs with shared JSON response types

## Tool Implementations
- [x] Query tool: Specialized multi-source search with filtering, ranking, citations
- [x] Analyze tool: Specialized analyzers (text, code, document, sentiment, comparison)
- [x] Summarize tool: Enhanced with JSON responses and metadata
- [x] Brainstorm tool: Idea generation with regex consensus extraction

## MCP Server Updates
- [x] Remove image_gen from server.rs
- [ ] Update tool schemas in list_tools() (optional for v2 API)
- [ ] Update tool execution methods (optional for v2 API)

## Testing
- [x] Unit tests added for all tools
- [x] Schema validation tests
- [x] Helper function tests (consensus extraction, filtering, etc.)
- [x] Run tests (34 tests passing)
- [x] Verify build (succeeded with minor warnings)

## Implementation Summary
All Week 2 tools upgraded with backward compatibility:
- types.rs: Shared JSON response types with metadata (110 lines)
- summarize.rs: JSON responses with key topic extraction (280 lines)
- brainstorm.rs: Idea generation with regex consensus themes (408 lines)
- analyze.rs: 5 specialized analyzers - text, code, document, sentiment, comparison (606 lines)
- query.rs: Multi-source search with filtering, ranking, citations (442 lines)

Total: ~1846 lines of production code + tests
All files under 2000 lines as required

---

# Week 3 Implementation TODO

## Phase 1: Foundation (shipped 27 Sep 2026, branch feat/retry-and-cache)
Written 23 Dec 2025, shipped after fixes. Plan: docs/27092026_retry_cache_plan.md

### 1.1 Token Counting
- [x] Add UsageMetadata & GenerationResponse to gemini/types.rs
- [x] Modify GeminiClient::generate_content() return type
- [x] Update existing tools: query, analyze, summarize, brainstorm
- [x] ResponseMetadata::with_usage (token counts), from_cache (cached flag)
- [x] Fix: wire types use camelCase so usageMetadata is actually parsed (was always 0)

### 1.2 Retry Logic
- [x] gemini/retry.rs: RetryConfig, retry_with_backoff (exponential backoff + jitter)
- [x] Integrate retry into GeminiClient (generateContent and startup models.get)
- [x] Honour Retry-After header and google.rpc.RetryInfo retryDelay; hint above 30 s is not retried
- [x] Retry only 408/429/500/502/503/504 and connect failures (no replay after timeout)
- [x] GEMINI_MAX_RETRIES env var (0-10)
- [x] Tests for statuses, hints, backoff bounds, connect errors, mock-server retries

## Phase 2: Enhanced Features (shipped 27 Sep 2026, branch feat/retry-and-cache)
### 2.1 Query Caching
- [x] cache/mod.rs: bounded QueryCache (TTL on Instant, capacity with eviction)
- [x] Integrate into gemini-search-v2
- [x] Fix: key = SHA-256 of model ID + full prompt (incl. source content) + generation config
- [x] Fix: cache hit reports original model, cached=true, zero tokens
- [x] GEMINI_CACHE_TTL_SECS / GEMINI_CACHE_MAX_ENTRIES env vars
- [x] Tests: changed content misses, model/params change misses, hit metadata, eviction, expiry

### 2.2 Improved Consensus
- [x] Enhance extract_consensus_themes() in brainstorm.rs
- [x] Add semantic clustering (merge related themes)
- [x] Add multi-word phrase extraction (bigrams + trigrams)
- [x] Relevance scoring (frequency x distribution)
- [x] Expanded stop words list
- [x] Thresholds: 25% for keywords, 20% for phrases

### 2.3 Maintenance done alongside (27 Sep 2026)
- [x] API key sent in x-goog-api-key header instead of URL query string
- [x] Logs to stderr (stdout is the MCP JSON-RPC channel)
- [x] Startup check uses models.get (generates no content) instead of generateContent
- [x] Default models: gemini-3.1-pro-preview, gemini-3.8-flash (gemini-3-pro-preview shut down 9 Mar 2026)
- [x] gemini-analyze-v2 honours params and options.detail_level
- [x] cargo fmt, clippy -D warnings clean, dead code removed
- [x] CI workflow (.github/workflows/ci.yml): fmt, clippy, test
- [x] README corrections (tool count, removed tool, claude mcp add syntax, config paths, performance table removed)
- [x] docs/index.md, docs/c4model.md, docs/27092026_retry_cache_plan.md

## Phase 3-6: New Tools (not started)
- [ ] Create generate.rs (code generation tool)
- [ ] Create translate.rs (translation tool)
- [ ] Create qa.rs (Q&A with context tool)
- [ ] Create extract.rs (data extraction tool)

## Phase 7: MCP Server Integration (not started)
- [ ] Add 4 new tool schemas to server.rs
- [ ] Add execute methods for new tools
- [ ] Update tool mappings

## Phase 8: Testing & Verification (not started)
- [ ] Unit tests for all new components
- [ ] Integration tests
- [ ] Run all tests (target: 54+ tests passing)
- [ ] Verify build succeeds
- [ ] Verify all files under 2000 LOC
