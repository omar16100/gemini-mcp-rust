# Gemini MCP Server (Rust)

[![CI](https://github.com/omar16100/gemini-mcp-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/omar16100/gemini-mcp-rust/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Model Context Protocol (MCP) server for Google's Gemini API, written in Rust. It exposes Gemini text generation, analysis, summarization, brainstorming and multi-source search as MCP tools for Claude Code and other MCP clients.

## ✨ Features

- **9 tools**: 5 v1 tools (plain text responses) and 4 v2 tools (structured JSON with metadata)
- **Multiple analyzers**: text, code, document, sentiment, and comparison analysis
- **Multi-source search**: answers over caller-supplied sources, with citations and source filters
- **Brainstorming**: idea generation with consensus theme extraction
- **Retries**: transient Gemini errors are retried with exponential backoff and jitter, honouring server retry hints
- **Search cache**: repeated identical `gemini-search-v2` requests reuse the previous Gemini answer for 5 minutes
- **Token usage**: v2 responses report prompt, response and total token counts
- **Standalone binary**: no Node.js runtime required

## 📦 Installation

### Prerequisites

- A stable Rust toolchain ([Install Rust](https://rustup.rs/))
- A Google Gemini API key ([Get one here](https://aistudio.google.com/apikey))

### Build from Source

```bash
git clone https://github.com/omar16100/gemini-mcp-rust.git
cd gemini-mcp-rust

# Set up environment
cp .env.example .env
# Edit .env and add your GEMINI_API_KEY

# Build release binary
cargo build --release

# Binary will be at: target/release/gemini-mcp
```

### Quick Install

```bash
# Clone and build
git clone https://github.com/omar16100/gemini-mcp-rust.git
cd gemini-mcp-rust
cargo build --release

# Install to PATH
sudo cp target/release/gemini-mcp /usr/local/bin/
```

## 🚀 Usage

### With Claude Code

Register the server with `claude mcp add`. Options such as `--scope` and `-e/--env` go before `--`; the server command goes after it:

```bash
# Local scope (default): this project only, private to you
claude mcp add gemini-rust -e GEMINI_API_KEY=your_api_key_here -- /path/to/gemini-mcp

# User scope: available in all your projects
claude mcp add --scope user gemini-rust -e GEMINI_API_KEY=your_api_key_here -- /path/to/gemini-mcp

# Project scope: writes .mcp.json in the project root, to share via version control
claude mcp add --scope project gemini-rust -- /path/to/gemini-mcp
```

Local and user scope entries are stored in `~/.claude.json`. For project scope, keep the key out of the repository by referencing an environment variable in `.mcp.json` (Claude Code expands `${VAR}`):

```json
{
  "mcpServers": {
    "gemini-rust": {
      "type": "stdio",
      "command": "/path/to/gemini-mcp",
      "args": [],
      "env": {
        "GEMINI_API_KEY": "${GEMINI_API_KEY}"
      }
    }
  }
}
```

Check the result with `claude mcp list` or `claude mcp get gemini-rust`. See the [Claude Code MCP docs](https://code.claude.com/docs/en/mcp) for details. Other MCP clients that launch stdio servers need the same command and environment variables.

### Standalone

```bash
# Run with default settings
GEMINI_API_KEY=your_key ./target/release/gemini-mcp

# Run with verbose logging
./target/release/gemini-mcp --verbose

# Run in quiet mode
./target/release/gemini-mcp --quiet
```

The server speaks JSON-RPC over stdin/stdout; logs go to stderr. On startup it checks the API key and the Pro model ID with a `models.get` call (no content is generated) and exits if that fails.

## 🛠️ Available Tools

### V1 Tools (Plain Text Responses)

| Tool | Description |
|------|-------------|
| `gemini-query` | Direct queries to Gemini models |
| `gemini-analyze-code` | Analyze code quality, security, performance |
| `gemini-analyze-text` | General text analysis |
| `gemini-summarize` | Content summarization |
| `gemini-brainstorm` | Collaborative brainstorming |

### V2 Tools (Structured JSON Responses)

| Tool | Description | Key Features |
|------|-------------|--------------|
| `gemini-search-v2` | Multi-source semantic search | Citations, source filters, response cache |
| `gemini-analyze-v2` | Unified analyzer | 5 types: text, code, document, sentiment, comparison |
| `gemini-summarize-v2` | Enhanced summarization | Key topics extraction, word count |
| `gemini-brainstorm-v2` | Idea generation | Numbered ideas, consensus themes |

V2 tools return a JSON object of the form `{"result": {...}, "metadata": {...}}`, serialized as text inside the MCP tool result's `content[0].text` (not as MCP `structuredContent`). `metadata` has:

| Field | Meaning |
|-------|---------|
| `model_used` | Model ID actually sent to the API, including any `GEMINI_PRO_MODEL` / `GEMINI_FLASH_MODEL` override |
| `prompt_tokens`, `response_tokens`, `total_tokens` | Token counts from the API's `usageMetadata` (0 if the API omits them) |
| `cached` | `true` only for `gemini-search-v2` answers served from the cache (token counts are then 0) |

## 📖 Usage Examples

### Basic Query

```json
{
  "tool": "gemini-query",
  "arguments": {
    "prompt": "Explain Rust ownership",
    "model": "pro",
    "temperature": 0.7
  }
}
```

### Multi-Source Search (V2)

```json
{
  "tool": "gemini-search-v2",
  "arguments": {
    "query": "best practices for async Rust",
    "sources": [
      {"id": "1", "title": "Tokio Guide", "content": "..."},
      {"id": "2", "title": "Async Book", "content": "..."}
    ],
    "include_citations": true,
    "ranking": "relevance"
  }
}
```

### Code Analysis (V2)

```json
{
  "tool": "gemini-analyze-v2",
  "arguments": {
    "content": "fn main() { println!(\"Hello\"); }",
    "analyzer_type": {
      "type": "code",
      "params": {"language": "rust"}
    },
    "options": {
      "detail_level": "comprehensive",
      "focus_areas": ["performance", "security"]
    }
  }
}
```

Note: `gemini-search-v2` currently gives every matched source the same `relevance_score` (0.7), and `ranking: "recency"` / `"popularity"` do not reorder results yet, so result order is not meaningful.

### Idea Generation (V2)

```json
{
  "tool": "gemini-brainstorm-v2",
  "arguments": {
    "prompt": "Ways to improve application performance",
    "num_ideas": 15,
    "constraints": "Focus on low-hanging fruit",
    "extract_consensus": true
  }
}
```

## ⚙️ Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `GEMINI_API_KEY` | **Required**. Your Gemini API key (sent in the `x-goog-api-key` header) | - |
| `GEMINI_PRO_MODEL` | Model used for `pro` requests | `gemini-3.1-pro-preview` |
| `GEMINI_FLASH_MODEL` | Model used for `flash` requests | `gemini-3.8-flash` |
| `GEMINI_MAX_RETRIES` | Retries after the first attempt for transient errors (0 to 10, 0 disables) | `3` |
| `GEMINI_CACHE_TTL_SECS` | `gemini-search-v2` cache lifetime in seconds (0 to 86400, 0 disables) | `300` |
| `GEMINI_CACHE_MAX_ENTRIES` | Maximum cached search answers (0 to 10000, 0 disables) | `100` |

Invalid or out-of-range values are logged and replaced by the default.

Default model IDs were checked against Google's [models](https://ai.google.dev/gemini-api/docs/models) and [deprecations](https://ai.google.dev/gemini-api/docs/deprecations) pages (both last updated 24 Sep 2026) as of 27 Sep 2026: the previous default `gemini-3-pro-preview` was shut down on 9 Mar 2026 with `gemini-3.1-pro-preview` as its replacement, and the models page recommends 3.8 Flash for new projects. Set the variables above to pin other models.

### Retries

- Applies to every Gemini request and to the startup connection check.
- Retried: HTTP 408, 429, 500, 502, 503, 504, and connection failures (the request never reached the server).
- Backoff: 1 s, 2 s, 4 s, ... with +/-25% jitter, each wait capped at 30 s.
- Server hints: the larger of a `Retry-After` header (seconds or HTTP date) and a `google.rpc.RetryInfo` `retryDelay` in the error body sets the minimum wait. If the hint is longer than 30 s (for example an exhausted daily quota), the error is returned immediately instead of retrying early.
- Not retried: other 4xx and 5xx errors, requests that time out (60 s per attempt) after being sent, and error responses whose body cannot be read, because the generation may already have run and been billed. Redirects are not followed.
- A retried 5xx can still repeat work the backend had started; set `GEMINI_MAX_RETRIES=0` if that matters more than availability.
- `GEMINI_MAX_RETRIES` controls this server's retry loop. The HTTP library (reqwest) may separately resend a request the server refused at the protocol level before processing it.

### Search cache (`gemini-search-v2` only)

- In memory, per server process, never written to disk.
- Key: SHA-256 of the model ID, the full prompt (query plus the ID, title and content of every selected source) and the generation parameters. Changing any of these, including a source's content under the same ID, is a cache miss.
- `filters` (`min_relevance`, `max_results`), `ranking` and `include_citations` are applied after the cache lookup, so they always reflect the current request.
- On a hit, `metadata.cached` is `true`, `model_used` is the model that produced the cached answer, and token counts are `0` because no API call was made.
- Entries expire after `GEMINI_CACHE_TTL_SECS`. Each insert drops expired entries; when `GEMINI_CACHE_MAX_ENTRIES` is still reached, the oldest entry is evicted.
- Answers over 256 KiB are not cached, so the cache holds at most about `GEMINI_CACHE_MAX_ENTRIES` x 256 KiB of answer text.
- Two identical requests running at the same time can both miss and both call the API.

### CLI Options

```bash
Options:
  -v, --verbose    Enable verbose logging
  -q, --quiet      Run in quiet mode (errors only)
  -h, --help       Print help information
```

## 🏗️ Architecture

```
src/
├── cache/
│   └── mod.rs       # Bounded in-memory TTL cache (search-v2)
├── gemini/          # Gemini REST API client
│   ├── client.rs    # HTTP client, API key header, error mapping
│   ├── retry.rs     # Retry policy: backoff, jitter, Retry-After
│   ├── types.rs     # Request/response types (camelCase JSON)
│   └── models.rs    # Model enum (Pro/Flash) and default IDs
├── mcp/             # MCP server implementation
│   └── server.rs    # JSON-RPC stdio server, tool registry
├── tools/           # Tool implementations
│   ├── types.rs     # Shared types (ToolResponse, metadata)
│   ├── query.rs     # Query + multi-source search
│   ├── analyze.rs   # 5 analyzer types
│   ├── summarize.rs # Summarization with key topics
│   └── brainstorm.rs# Idea generation + themes
├── error.rs         # Error types
└── main.rs          # Entry point
```

More detail: [docs/c4model.md](docs/c4model.md) and the [docs index](docs/index.md).

## 🧪 Development

### Running Tests

```bash
# Run all unit tests
cargo test

# Run with output
cargo test -- --nocapture

# Run one module's tests
cargo test cache
```

Tests use a local mock HTTP server ([mockito](https://crates.io/crates/mockito)); none call the real Gemini API or need an API key.

### Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Check without building
cargo check
```

### Code Quality

CI runs these on pushes to `master` and on pull requests:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/AmazingFeature`)
3. Commit your changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

### Development Guidelines

- Follow Rust naming conventions
- Add tests for new features
- Update documentation
- Run `cargo fmt` and `cargo clippy` before committing
- Keep binary size under 5MB

## 📝 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built with the [Model Context Protocol](https://modelcontextprotocol.io/)
- Powered by [Google Gemini API](https://ai.google.dev/)
- Inspired by the TypeScript [@rlabs/gemini-mcp](https://github.com/RLabs-Inc/gemini-mcp)

## 🔗 Related Projects

- [TypeScript Gemini MCP](https://github.com/RLabs-Inc/gemini-mcp) - Node.js version
- [MCP Specification](https://modelcontextprotocol.io/specification)
- [Claude Code MCP docs](https://code.claude.com/docs/en/mcp)

## 📮 Support

- 🐛 [Report a Bug](https://github.com/omar16100/gemini-mcp-rust/issues)
- 💡 [Request a Feature](https://github.com/omar16100/gemini-mcp-rust/issues)

---

**Note**: This is an unofficial community project and is not affiliated with Google or Anthropic.
