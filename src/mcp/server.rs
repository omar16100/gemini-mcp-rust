// Simple stdio JSON-RPC MCP server implementation
// Direct protocol implementation without rust-mcp-sdk due to API complexity

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info};

use crate::gemini::client::GeminiClient;
use crate::tools;

pub struct McpGeminiServer {
    client: Arc<GeminiClient>,
}

impl McpGeminiServer {
    pub fn new(api_key: String) -> anyhow::Result<Self> {
        let client = GeminiClient::new(api_key)?;
        Ok(Self {
            client: Arc::new(client),
        })
    }

    pub async fn test_connection(&self) -> anyhow::Result<()> {
        self.client.test_connection().await?;
        Ok(())
    }

    pub async fn run(self) -> anyhow::Result<()> {
        info!("Starting MCP server (stdio JSON-RPC)");

        // Simple stdio message loop
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin);

        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    info!("EOF reached, shutting down");
                    break;
                }
                Ok(_) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }

                    debug!("Received: {}", line);

                    // Parse JSON-RPC request
                    let response = match serde_json::from_str::<JsonRpcRequest>(line) {
                        Ok(request) => self.handle_request(request).await,
                        Err(e) => {
                            error!("Invalid JSON: {}", e);
                            JsonRpcResponse::error(-32700, "Parse error", None)
                        }
                    };

                    // Send response
                    let response_json = serde_json::to_string(&response)?;
                    stdout.write_all(response_json.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }
                Err(e) => {
                    error!("Error reading stdin: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => {
                info!("Handling initialize request");
                JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": "Gemini MCP Server (Rust)",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }),
                )
            }
            "tools/list" => {
                info!("Handling tools/list request");
                self.list_tools(request.id)
            }
            "tools/call" => {
                info!("Handling tools/call request");
                self.call_tool(request.id, request.params).await
            }
            _ => JsonRpcResponse::error(-32601, "Method not found", Some(request.id)),
        }
    }

    fn list_tools(&self, id: serde_json::Value) -> JsonRpcResponse {
        let tools = serde_json::json!({
            "tools": [
                {
                    "name": "gemini-query",
                    "description": "Send direct queries to Gemini models",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "prompt": {"type": "string"},
                            "model": {"type": "string", "enum": ["pro", "flash"], "default": "pro"},
                            "temperature": {"type": "number"},
                            "max_output_tokens": {"type": "integer"}
                        },
                        "required": ["prompt"]
                    }
                },
                {
                    "name": "gemini-analyze-code",
                    "description": "Analyze code",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "code": {"type": "string"},
                            "language": {"type": "string"},
                            "focus": {"type": "string", "enum": ["general", "quality", "security", "performance", "bugs"]}
                        },
                        "required": ["code"]
                    }
                },
                {
                    "name": "gemini-analyze-text",
                    "description": "Analyze text",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "text": {"type": "string"},
                            "focus": {"type": "string"}
                        },
                        "required": ["text"]
                    }
                },
                {
                    "name": "gemini-summarize",
                    "description": "Summarize content",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string"},
                            "detail_level": {"type": "string", "enum": ["brief", "moderate", "detailed"]},
                            "format": {"type": "string", "enum": ["bullets", "paragraphs", "outline"]}
                        },
                        "required": ["content"]
                    }
                },
                {
                    "name": "gemini-brainstorm",
                    "description": "Collaborative brainstorming",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "prompt": {"type": "string"},
                            "claude_thoughts": {"type": "string"},
                            "max_rounds": {"type": "integer", "default": 3}
                        },
                        "required": ["prompt", "claude_thoughts"]
                    }
                },
                {
                    "name": "gemini-search-v2",
                    "description": "Multi-source semantic search with citations and ranking",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {"type": "string", "description": "The search query"},
                            "sources": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "id": {"type": "string"},
                                        "title": {"type": "string"},
                                        "content": {"type": "string"}
                                    },
                                    "required": ["id", "title", "content"]
                                }
                            },
                            "filters": {
                                "type": "object",
                                "properties": {
                                    "source_ids": {"type": "array", "items": {"type": "string"}},
                                    "min_relevance": {"type": "number"},
                                    "max_results": {"type": "integer"}
                                }
                            },
                            "ranking": {"type": "string", "enum": ["relevance", "recency", "popularity"], "default": "relevance"},
                            "include_citations": {"type": "boolean", "default": true},
                            "model": {"type": "string", "enum": ["pro", "flash"]},
                            "params": {
                                "type": "object",
                                "properties": {
                                    "temperature": {"type": "number"},
                                    "max_tokens": {"type": "integer"},
                                    "top_p": {"type": "number"},
                                    "top_k": {"type": "integer"}
                                }
                            }
                        },
                        "required": ["query", "sources"]
                    }
                },
                {
                    "name": "gemini-analyze-v2",
                    "description": "Unified analyzer with 5 types: text, code, document, sentiment, comparison",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string", "description": "The content to analyze"},
                            "analyzer_type": {
                                "type": "object",
                                "oneOf": [
                                    {"type": "object", "properties": {"type": {"const": "text"}}},
                                    {"type": "object", "properties": {"type": {"const": "code"}, "params": {"type": "object", "properties": {"language": {"type": "string"}}}}},
                                    {"type": "object", "properties": {"type": {"const": "document"}}},
                                    {"type": "object", "properties": {"type": {"const": "sentiment"}}},
                                    {"type": "object", "properties": {"type": {"const": "comparison"}, "params": {"type": "object", "properties": {"compare_with": {"type": "string"}}, "required": ["compare_with"]}}}
                                ]
                            },
                            "options": {
                                "type": "object",
                                "properties": {
                                    "focus_areas": {"type": "array", "items": {"type": "string"}},
                                    "detail_level": {"type": "string", "enum": ["brief", "standard", "comprehensive"], "default": "standard"}
                                }
                            },
                            "model": {"type": "string", "enum": ["pro", "flash"]},
                            "params": {"type": "object"}
                        },
                        "required": ["content", "analyzer_type"]
                    }
                },
                {
                    "name": "gemini-summarize-v2",
                    "description": "Enhanced summarization with key topics extraction and word count",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string"},
                            "length": {"type": "string", "enum": ["brief", "medium", "detailed"], "default": "medium"},
                            "format": {"type": "string", "enum": ["paragraph", "bullet_points", "executive", "key_points"], "default": "paragraph"},
                            "focus": {"type": "string"},
                            "model": {"type": "string", "enum": ["pro", "flash"]},
                            "params": {"type": "object"}
                        },
                        "required": ["content"]
                    }
                },
                {
                    "name": "gemini-brainstorm-v2",
                    "description": "Idea generation with consensus theme extraction",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "prompt": {"type": "string", "description": "The topic or problem to brainstorm"},
                            "num_ideas": {"type": "integer", "default": 10, "minimum": 1, "maximum": 50},
                            "constraints": {"type": "string"},
                            "extract_consensus": {"type": "boolean", "default": true},
                            "model": {"type": "string", "enum": ["pro", "flash"]},
                            "params": {"type": "object"}
                        },
                        "required": ["prompt"]
                    }
                }
            ]
        });

        JsonRpcResponse::success(id, tools)
    }

    async fn call_tool(
        &self,
        id: serde_json::Value,
        params: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        let params = match params {
            Some(p) => p,
            None => {
                return JsonRpcResponse::error(-32602, "Invalid params", Some(id));
            }
        };

        let tool_name = match params.get("name").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                return JsonRpcResponse::error(-32602, "Missing tool name", Some(id));
            }
        };

        let arguments = match params.get("arguments") {
            Some(args) => args.clone(),
            None => serde_json::json!({}),
        };

        debug!("Calling tool: {}", tool_name);

        let result = match tool_name {
            // V1 tools (legacy - backward compatibility)
            "gemini-query" => {
                match self.execute_query(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": r}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            "gemini-analyze-code" => {
                match self.execute_analyze_code(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": r}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            "gemini-analyze-text" => {
                match self.execute_analyze_text(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": r}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            "gemini-summarize" => {
                match self.execute_summarize(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": r}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            "gemini-brainstorm" => {
                match self.execute_brainstorm(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": r}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            // V2 tools (structured JSON responses)
            "gemini-search-v2" => {
                match self.execute_search_v2(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&r).unwrap_or_else(|_| "{}".to_string())}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            "gemini-analyze-v2" => {
                match self.execute_analyze_v2(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&r).unwrap_or_else(|_| "{}".to_string())}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            "gemini-summarize-v2" => {
                match self.execute_summarize_v2(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&r).unwrap_or_else(|_| "{}".to_string())}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            "gemini-brainstorm-v2" => {
                match self.execute_brainstorm_v2(arguments).await {
                    Ok(r) => serde_json::json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&r).unwrap_or_else(|_| "{}".to_string())}]}),
                    Err(e) => return JsonRpcResponse::error(-32603, &e.to_string(), Some(id)),
                }
            }
            _ => {
                return JsonRpcResponse::error(-32601, "Tool not found", Some(id));
            }
        };

        JsonRpcResponse::success(id, result)
    }

    async fn execute_query(&self, args: serde_json::Value) -> anyhow::Result<String> {
        let input: tools::query::QueryInput = serde_json::from_value(args)?;
        let output = tools::query::execute(input, Arc::clone(&self.client)).await?;
        Ok(output.text)
    }

    async fn execute_analyze_code(&self, args: serde_json::Value) -> anyhow::Result<String> {
        let input: tools::analyze::AnalyzeCodeInput = serde_json::from_value(args)?;
        let output = tools::analyze::execute_code(input, Arc::clone(&self.client)).await?;
        Ok(output.analysis)
    }

    async fn execute_analyze_text(&self, args: serde_json::Value) -> anyhow::Result<String> {
        let input: tools::analyze::AnalyzeTextInput = serde_json::from_value(args)?;
        let output = tools::analyze::execute_text(input, Arc::clone(&self.client)).await?;
        Ok(output.analysis)
    }

    async fn execute_summarize(&self, args: serde_json::Value) -> anyhow::Result<String> {
        let input: tools::summarize::SummarizeInput = serde_json::from_value(args)?;
        let output = tools::summarize::execute(input, Arc::clone(&self.client)).await?;
        Ok(output.summary)
    }

    async fn execute_brainstorm(&self, args: serde_json::Value) -> anyhow::Result<String> {
        let input: tools::brainstorm::BrainstormInput = serde_json::from_value(args)?;
        let output = tools::brainstorm::execute(input, Arc::clone(&self.client)).await?;
        Ok(format!(
            "# Synthesis\n\n{}\n\n# Conversation History\n\n{}",
            output.synthesis, output.conversation_history
        ))
    }

    // V2 API execute methods
    async fn execute_search_v2(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let input: tools::query::SearchInput = serde_json::from_value(args)?;
        let response = tools::query::execute_v2(input, Arc::clone(&self.client)).await?;

        // Serialize ToolResponse<SearchResult> to JSON
        Ok(serde_json::to_value(response)?)
    }

    async fn execute_analyze_v2(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let input: tools::analyze::AnalyzeInput = serde_json::from_value(args)?;
        let response = tools::analyze::execute_v2(input, Arc::clone(&self.client)).await?;

        // Serialize ToolResponse<AnalyzeResult> to JSON
        Ok(serde_json::to_value(response)?)
    }

    async fn execute_summarize_v2(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let input: tools::summarize::SummarizeInput = serde_json::from_value(args)?;
        let response = tools::summarize::execute_v2(input, Arc::clone(&self.client)).await?;

        // Serialize ToolResponse<SummaryResult> to JSON
        Ok(serde_json::to_value(response)?)
    }

    async fn execute_brainstorm_v2(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let input: tools::brainstorm::BrainstormInput = serde_json::from_value(args)?;
        let response = tools::brainstorm::execute_v2(input, Arc::clone(&self.client)).await?;

        // Serialize ToolResponse<BrainstormResult> to JSON
        Ok(serde_json::to_value(response)?)
    }
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(code: i32, message: &str, id: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: id.unwrap_or(serde_json::Value::Null),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
            }),
        }
    }
}

