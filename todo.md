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

## Phase 1: Foundation
### 1.1 Token Counting
- [ ] Add UsageMetadata & GenerationResponse to gemini/types.rs
- [ ] Modify GeminiClient::generate_content() return type
- [ ] Update existing tools: query, analyze, summarize, brainstorm
- [ ] Update ResponseMetadata::new() to accept token counts

### 1.2 Retry Logic
- [ ] Create gemini/retry.rs with RetryConfig
- [ ] Implement retry_with_backoff() function
- [ ] Integrate retry into GeminiClient
- [ ] Add tests for retry logic

## Phase 2: Enhanced Features
### 2.1 Query Caching
- [ ] Create cache/mod.rs with CacheEntry & QueryCache
- [ ] Integrate caching into query.rs execute_v2()
- [ ] Add cache tests

### 2.2 Improved Consensus
- [ ] Enhance extract_consensus_themes() in brainstorm.rs
- [ ] Add semantic clustering
- [ ] Add multi-word phrase extraction

## Phase 3-6: New Tools
- [ ] Create generate.rs (code generation tool)
- [ ] Create translate.rs (translation tool)
- [ ] Create qa.rs (Q&A with context tool)
- [ ] Create extract.rs (data extraction tool)

## Phase 7: MCP Server Integration
- [ ] Add 4 new tool schemas to server.rs
- [ ] Add execute methods for new tools
- [ ] Update tool mappings

## Phase 8: Testing & Verification
- [ ] Unit tests for all new components
- [ ] Integration tests
- [ ] Run all tests (target: 54+ tests passing)
- [ ] Verify build succeeds
- [ ] Verify all files under 2000 LOC
