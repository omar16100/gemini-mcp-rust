# Gemini MCP Server (Rust)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

High-performance Model Context Protocol (MCP) server for Google's Gemini API, written in Rust. Provides seamless integration between Claude Desktop/CLI and Gemini's powerful AI models.

## ✨ Features

- **🚀 High Performance**: 3.9MB binary, <200ms startup, <30MB memory usage
- **🔧 10 Built-in Tools**: 6 legacy + 4 enhanced v2 tools with structured responses
- **📊 Dual API**: V1 (plain text) and V2 (structured JSON with metadata)
- **🎯 Multiple Analyzers**: Text, code, document, sentiment, and comparison analysis
- **🔍 Multi-Source Search**: Semantic search with citations and ranking
- **💡 Smart Brainstorming**: Idea generation with consensus theme extraction
- **⚡ Zero Dependencies**: Standalone binary, no Node.js required
- **🛡️ Type Safe**: Full Rust type safety with comprehensive error handling

## 📦 Installation

### Prerequisites

- Rust 1.70+ ([Install Rust](https://rustup.rs/))
- Google Gemini API key ([Get one here](https://makersuite.google.com/app/apikey))

### Build from Source

```bash
git clone https://github.com/yourusername/gemini-mcp-rust.git
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
git clone https://github.com/yourusername/gemini-mcp-rust.git
cd gemini-mcp-rust
cargo build --release

# Install to PATH
sudo cp target/release/gemini-mcp /usr/local/bin/
```

## 🚀 Usage

### With Claude Desktop/CLI

Add to your MCP configuration:

```bash
# Using Claude CLI
claude mcp add gemini-rust \
  --command /path/to/gemini-mcp \
  --env GEMINI_API_KEY=your_api_key_here
```

Or manually edit your Claude config (`~/.config/claude/config.json`):

```json
{
  "mcpServers": {
    "gemini-rust": {
      "command": "/path/to/gemini-mcp",
      "env": {
        "GEMINI_API_KEY": "your_api_key_here",
        "GEMINI_PRO_MODEL": "gemini-3-pro-preview",
        "GEMINI_FLASH_MODEL": "gemini-3-flash-preview"
      }
    }
  }
}
```

### Standalone

```bash
# Run with default settings
GEMINI_API_KEY=your_key ./target/release/gemini-mcp

# Run with verbose logging
./target/release/gemini-mcp --verbose

# Run in quiet mode
./target/release/gemini-mcp --quiet
```

## 🛠️ Available Tools

### V1 Tools (Plain Text Responses)

| Tool | Description |
|------|-------------|
| `gemini-query` | Direct queries to Gemini models |
| `gemini-analyze-code` | Analyze code quality, security, performance |
| `gemini-analyze-text` | General text analysis |
| `gemini-summarize` | Content summarization |
| `gemini-brainstorm` | Collaborative brainstorming |
| `gemini-image-prompt` | Generate image prompts |

### V2 Tools (Structured JSON Responses)

| Tool | Description | Key Features |
|------|-------------|--------------|
| `gemini-search-v2` | Multi-source semantic search | Citations, relevance ranking, filters |
| `gemini-analyze-v2` | Unified analyzer | 5 types: text, code, document, sentiment, comparison |
| `gemini-summarize-v2` | Enhanced summarization | Key topics extraction, word count |
| `gemini-brainstorm-v2` | Idea generation | Numbered ideas, consensus themes |

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
| `GEMINI_API_KEY` | **Required**. Your Gemini API key | - |
| `GEMINI_PRO_MODEL` | Pro model identifier | `gemini-3-pro-preview` |
| `GEMINI_FLASH_MODEL` | Flash model identifier | `gemini-3-flash-preview` |
| `VERBOSE` | Enable verbose logging | `false` |

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
├── gemini/          # Gemini REST API client
│   ├── client.rs    # HTTP client with connection pooling
│   ├── types.rs     # Request/response types
│   └── models.rs    # Model enum (Pro/Flash)
├── mcp/             # MCP server implementation
│   └── server.rs    # JSON-RPC stdio server
├── tools/           # Tool implementations
│   ├── types.rs     # Shared types (ToolResponse, metadata)
│   ├── query.rs     # Query + multi-source search
│   ├── analyze.rs   # 5 analyzer types
│   ├── summarize.rs # Summarization with key topics
│   ├── brainstorm.rs# Idea generation + themes
│   └── image_gen.rs # Image prompt generation
├── error.rs         # Error types
└── main.rs          # Entry point
```

## 🧪 Development

### Running Tests

```bash
# Run all unit tests (34 tests)
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_search_v2
```

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

```bash
# Format code
cargo fmt

# Run linter
cargo clippy

# Run linter with strict warnings
cargo clippy -- -D warnings
```

## 📊 Performance

| Metric | Value |
|--------|-------|
| Binary Size | 3.9MB |
| Startup Time | <200ms |
| Memory (Idle) | <30MB |
| Memory (Active) | <50MB |
| Tests | 34/34 passing |

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
- Inspired by the TypeScript [@rlabs/gemini-mcp](https://github.com/yourusername/gemini-mcp)

## 🔗 Related Projects

- [TypeScript Gemini MCP](https://github.com/yourusername/gemini-mcp) - Node.js version
- [MCP Specification](https://spec.modelcontextprotocol.io/)
- [Claude Desktop](https://claude.ai/download)

## 📮 Support

- 🐛 [Report a Bug](https://github.com/yourusername/gemini-mcp-rust/issues)
- 💡 [Request a Feature](https://github.com/yourusername/gemini-mcp-rust/issues)
- 📧 [Contact](mailto:your-email@example.com)

---

**Note**: This is an unofficial community project and is not affiliated with Google or Anthropic.
