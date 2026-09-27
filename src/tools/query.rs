use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use tracing::{debug, info};

use crate::cache::{fingerprint, QueryCache};
use crate::gemini::{client::GeminiClient, models::GeminiModel, types::GenerationConfig};
use crate::tools::types::{GenerationParams, ModelPreference, ResponseMetadata, ToolResponse};

/// Process-wide cache of Gemini answers for `gemini-search-v2`, keyed by
/// [`search_cache_key`]. Configured from env on first use.
static SEARCH_CACHE: OnceLock<QueryCache<String>> = OnceLock::new();

/// Answers larger than this are not cached, which bounds cache memory to
/// roughly `GEMINI_CACHE_MAX_ENTRIES` x 256 KiB.
const MAX_CACHED_ANSWER_BYTES: usize = 256 * 1024;

// Legacy input/output for backward compatibility
#[derive(Debug, Deserialize)]
pub struct QueryInput {
    pub prompt: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

fn default_model() -> String {
    "pro".to_string()
}

#[derive(Debug, Serialize)]
pub struct QueryOutput {
    pub text: String,
}

// V2 multi-source search input
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    #[schemars(description = "The search query")]
    pub query: String,

    #[schemars(description = "Sources to search across")]
    pub sources: Vec<Source>,

    #[schemars(description = "Search filters")]
    #[serde(default)]
    pub filters: Option<SearchFilters>,

    #[schemars(description = "Ranking criteria")]
    #[serde(default = "default_ranking")]
    pub ranking: RankingCriteria,

    #[schemars(description = "Include citations in results")]
    #[serde(default = "default_include_citations")]
    pub include_citations: bool,

    #[schemars(description = "Model preference")]
    #[serde(default)]
    pub model: Option<ModelPreference>,

    #[schemars(description = "Generation parameters")]
    #[serde(default)]
    pub params: Option<GenerationParams>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Source {
    #[schemars(description = "Unique identifier for this source")]
    pub id: String,

    #[schemars(description = "Title of the source")]
    pub title: String,

    #[schemars(description = "Content to search")]
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchFilters {
    #[schemars(description = "Limit search to specific source IDs")]
    pub source_ids: Option<Vec<String>>,

    #[schemars(description = "Minimum relevance score (0-1)")]
    pub min_relevance: Option<f32>,

    #[schemars(description = "Maximum number of results")]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankingCriteria {
    Relevance,
    Recency,
    Popularity,
}

fn default_ranking() -> RankingCriteria {
    RankingCriteria::Relevance
}

fn default_include_citations() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResult {
    pub answer: String,
    pub results: Vec<SourceResult>,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SourceResult {
    pub source_id: String,
    pub source_title: String,
    pub excerpt: String,
    pub relevance_score: f32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Citation {
    pub source_id: String,
    pub source_title: String,
    pub quote: String,
}

// Legacy execute function
pub async fn execute(input: QueryInput, client: Arc<GeminiClient>) -> anyhow::Result<QueryOutput> {
    debug!(
        "Query tool (legacy): model={}, prompt_len={}",
        input.model,
        input.prompt.len()
    );

    let model = GeminiModel::from_str(&input.model);

    let config = if input.temperature.is_some() || input.max_output_tokens.is_some() {
        Some(GenerationConfig {
            temperature: input.temperature,
            max_output_tokens: input.max_output_tokens,
            top_p: None,
            top_k: None,
        })
    } else {
        None
    };

    let response = client
        .generate_content(&input.prompt, model, config)
        .await?;

    if response.text.trim().is_empty() {
        anyhow::bail!("Empty response from Gemini API");
    }

    debug!("Query tool (legacy): response_len={}", response.text.len());

    Ok(QueryOutput {
        text: response.text,
    })
}

// V2 multi-source search implementation
pub async fn execute_v2(
    input: SearchInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<ToolResponse<SearchResult>> {
    let cache = SEARCH_CACHE.get_or_init(QueryCache::from_env);
    execute_v2_with_cache(input, &client, cache).await
}

async fn execute_v2_with_cache(
    input: SearchInput,
    client: &GeminiClient,
    cache: &QueryCache<String>,
) -> anyhow::Result<ToolResponse<SearchResult>> {
    // Log sizes, not content: queries and sources may be private.
    info!(
        "Search v2: query_len={}, sources={}, include_citations={}",
        input.query.len(),
        input.sources.len(),
        input.include_citations
    );

    // Validate input
    if input.query.trim().is_empty() {
        anyhow::bail!("Query cannot be empty");
    }

    if input.sources.is_empty() {
        anyhow::bail!("At least one source is required");
    }

    // Filter sources if source_ids filter is provided
    let filtered_sources: Vec<&Source> =
        if let Some(filter_ids) = input.filters.as_ref().and_then(|f| f.source_ids.as_ref()) {
            input
                .sources
                .iter()
                .filter(|s| filter_ids.contains(&s.id))
                .collect()
        } else {
            input.sources.iter().collect()
        };

    if filtered_sources.is_empty() {
        anyhow::bail!("No sources match the filter criteria");
    }

    debug!("Filtered to {} sources", filtered_sources.len());

    let prompt = build_search_prompt(&input.query, &filtered_sources);

    let model = match input.model {
        Some(ModelPreference::Flash) => GeminiModel::Flash,
        Some(ModelPreference::Pro) | None => GeminiModel::Pro,
    };

    let config = GenerationParams::to_config(input.params.as_ref(), Some(0.3), Some(2048));

    // The cache stores the raw Gemini answer keyed by everything that
    // determines it (model ID, full prompt incl. source content, generation
    // config). Parsing, filtering and ranking below always run on the live
    // input, so filters/ranking/citation options never read stale results.
    let model_name = client.model_name(model).to_string();
    let cache_key = search_cache_key(&model_name, &prompt, &config);

    let (text, metadata) = match cache.get(&cache_key) {
        Some(text) => {
            info!(
                "Search v2 cache hit (model={}, key={})",
                model_name,
                &cache_key[..12]
            );
            (text, ResponseMetadata::from_cache(&model_name))
        }
        None => {
            let response = client
                .generate_content(&prompt, model, Some(config))
                .await?;
            if response.text.len() <= MAX_CACHED_ANSWER_BYTES {
                cache.insert(cache_key, response.text.clone());
            } else {
                debug!(
                    "Answer too large to cache ({} bytes > {})",
                    response.text.len(),
                    MAX_CACHED_ANSWER_BYTES
                );
            }
            let metadata = ResponseMetadata::with_usage(&model_name, &response.usage);
            (response.text, metadata)
        }
    };

    debug!("Search response: {} chars", text.len());

    // Parse response into structured results
    let answer = extract_answer(&text);
    let mut results = extract_results(&text, &filtered_sources);
    let citations = if input.include_citations {
        extract_citations(&text, &filtered_sources)
    } else {
        Vec::new()
    };

    // Apply filters
    if let Some(min_rel) = input.filters.as_ref().and_then(|f| f.min_relevance) {
        results.retain(|r| r.relevance_score >= min_rel);
    }

    // Apply ranking
    match input.ranking {
        RankingCriteria::Relevance => {
            results.sort_by(|a, b| b.relevance_score.total_cmp(&a.relevance_score));
        }
        RankingCriteria::Recency | RankingCriteria::Popularity => {
            // For now, keep relevance-based sorting
            // In production, would use metadata from sources
        }
    }

    // Apply max_results limit
    if let Some(max) = input.filters.as_ref().and_then(|f| f.max_results) {
        results.truncate(max);
    }

    info!(
        "Search complete: {} results, {} citations, cached={}",
        results.len(),
        citations.len(),
        metadata.cached
    );

    let result = SearchResult {
        answer,
        results,
        citations,
    };

    Ok(ToolResponse { result, metadata })
}

fn build_search_prompt(query: &str, sources: &[&Source]) -> String {
    let mut prompt = format!(
        "You are performing a semantic search across multiple sources.\n\n\
         Query: {}\n\n\
         Sources:\n\n",
        query
    );

    for source in sources {
        prompt.push_str(&format!(
            "--- Source: {} (ID: {}) ---\n{}\n\n",
            source.title, source.id, source.content
        ));
    }

    prompt.push_str(
        "Based on the query, provide:\n\
         1. A direct answer to the query\n\
         2. For each relevant source, provide:\n\
            - Source ID and title\n\
            - A brief excerpt showing relevance\n\
            - Relevance score (0.0-1.0)\n\
         3. If applicable, include direct quotes as citations\n\n\
         Format your response clearly with sections for Answer, Results, and Citations.",
    );

    prompt
}

/// Cache key for a search generation: tool name, resolved model ID, the full
/// prompt (query plus every selected source's ID, title and content) and the
/// generation config. `Debug` formatting of the config is stable within one
/// process, which is all an in-memory cache needs.
fn search_cache_key(model_name: &str, prompt: &str, config: &GenerationConfig) -> String {
    let config_repr = format!("{:?}", config);
    fingerprint(&["gemini-search-v2", model_name, prompt, &config_repr])
}

fn extract_answer(text: &str) -> String {
    // Look for answer section
    for line in text.lines() {
        if line.to_lowercase().starts_with("answer:") {
            return line.split(':').nth(1).unwrap_or("").trim().to_string();
        }
    }

    // Fallback: use first paragraph
    text.lines()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn extract_results(text: &str, sources: &[&Source]) -> Vec<SourceResult> {
    let mut results = Vec::new();

    // Simple extraction: look for source mentions
    for source in sources {
        if text.to_lowercase().contains(&source.title.to_lowercase()) || text.contains(&source.id) {
            // Extract excerpt around the mention
            let excerpt = extract_excerpt_for_source(text, source);

            results.push(SourceResult {
                source_id: source.id.clone(),
                source_title: source.title.clone(),
                excerpt,
                relevance_score: 0.7, // Default score
            });
        }
    }

    results
}

fn extract_excerpt_for_source(text: &str, source: &Source) -> String {
    // Find paragraph mentioning this source
    for para in text.split("\n\n") {
        if para.to_lowercase().contains(&source.title.to_lowercase()) {
            return para.chars().take(200).collect();
        }
    }

    // Fallback: first 100 chars of source content
    source.content.chars().take(100).collect()
}

fn extract_citations(text: &str, sources: &[&Source]) -> Vec<Citation> {
    let mut citations = Vec::new();

    // Look for quoted text
    use regex::Regex;
    let quote_regex = Regex::new(r#""([^"]{20,})""#).unwrap();

    for captures in quote_regex.captures_iter(text) {
        if let Some(quote_match) = captures.get(1) {
            let quote = quote_match.as_str().to_string();

            // Try to find which source this quote is from
            for source in sources {
                if source.content.contains(&quote)
                    || source
                        .content
                        .to_lowercase()
                        .contains(&quote.to_lowercase())
                {
                    citations.push(Citation {
                        source_id: source.id.clone(),
                        source_title: source.title.clone(),
                        quote,
                    });
                    break;
                }
            }
        }
    }

    citations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_input_defaults() {
        let json = r#"{"prompt": "test"}"#;
        let input: QueryInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.model, "pro");
        assert_eq!(input.temperature, None);
        assert_eq!(input.max_output_tokens, None);
    }

    #[test]
    fn test_search_input_deserialize() {
        let json = r#"{
            "query": "test query",
            "sources": [
                {"id": "1", "title": "Doc 1", "content": "Content 1"}
            ],
            "include_citations": true
        }"#;
        let input: SearchInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.query, "test query");
        assert_eq!(input.sources.len(), 1);
        assert!(input.include_citations);
    }

    #[test]
    fn test_extract_answer() {
        let text = "Answer: This is the answer\nMore text here";
        let answer = extract_answer(text);
        assert_eq!(answer, "This is the answer");
    }

    #[test]
    fn test_extract_results() {
        let sources = [
            source("1", "Document A", "Content A"),
            source("2", "Document B", "Content B"),
        ];

        let source_refs: Vec<&Source> = sources.iter().collect();
        let text = "The answer can be found in Document A which states...";
        let results = extract_results(text, &source_refs);

        assert!(!results.is_empty());
        assert_eq!(results[0].source_id, "1");
    }

    #[test]
    fn test_search_result_serialize() {
        let result = SearchResult {
            answer: "Test answer".to_string(),
            results: vec![SourceResult {
                source_id: "1".to_string(),
                source_title: "Doc".to_string(),
                excerpt: "Excerpt".to_string(),
                relevance_score: 0.9,
            }],
            citations: vec![],
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Test answer"));
        assert!(json.contains("relevance_score"));
    }

    fn source(id: &str, title: &str, content: &str) -> Source {
        Source {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
        }
    }

    fn search_input(sources: Vec<Source>) -> SearchInput {
        SearchInput {
            query: "what is the capital?".to_string(),
            sources,
            filters: None,
            ranking: RankingCriteria::Relevance,
            include_citations: true,
            model: None,
            params: None,
        }
    }

    const GEMINI_BODY: &str = r#"{
        "candidates": [{"content": {"role": "model", "parts": [{"text":
            "Answer: Paris\nDoc A says \"Paris is the capital of France.\""}]}}],
        "usageMetadata": {"promptTokenCount": 40, "candidatesTokenCount": 9, "totalTokenCount": 49}
    }"#;

    fn mock_client(url: String) -> GeminiClient {
        GeminiClient::with_config(
            "test-key".to_string(),
            url,
            "pro-model-x".to_string(),
            "flash-model-y".to_string(),
            crate::gemini::retry::RetryConfig {
                max_retries: 0,
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn test_search_cache_key_covers_all_inputs() {
        let config = GenerationParams::to_config(None, Some(0.3), Some(2048));
        let a = source("1", "Doc A", "Paris is the capital of France.");
        let prompt = build_search_prompt("q", &[&a]);
        let base = search_cache_key("m", &prompt, &config);

        assert_eq!(base, search_cache_key("m", &prompt, &config));

        // Same source ID, different content.
        let a2 = source("1", "Doc A", "Lyon is the capital of France.");
        let prompt2 = build_search_prompt("q", &[&a2]);
        assert_ne!(base, search_cache_key("m", &prompt2, &config));

        // Same ID and content, different title.
        let a3 = source("1", "Doc B", "Paris is the capital of France.");
        assert_ne!(
            base,
            search_cache_key("m", &build_search_prompt("q", &[&a3]), &config)
        );

        // Different model or generation params.
        assert_ne!(base, search_cache_key("m2", &prompt, &config));
        let hotter = GenerationParams::to_config(None, Some(0.9), Some(2048));
        assert_ne!(base, search_cache_key("m", &prompt, &hotter));
        let longer = GenerationParams::to_config(None, Some(0.3), Some(4096));
        assert_ne!(base, search_cache_key("m", &prompt, &longer));
    }

    #[tokio::test]
    async fn test_search_cache_hit_reports_original_model() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(200)
            .with_body(GEMINI_BODY)
            .expect(1)
            .create_async()
            .await;
        let client = mock_client(server.url());
        let cache = QueryCache::new(std::time::Duration::from_secs(300), 10);
        let docs = || vec![source("1", "Doc A", "Paris is the capital of France.")];

        let first = execute_v2_with_cache(search_input(docs()), &client, &cache)
            .await
            .unwrap();
        assert!(!first.metadata.cached);
        assert_eq!(first.metadata.model_used, "pro-model-x");
        assert_eq!(first.metadata.total_tokens, 49);

        let second = execute_v2_with_cache(search_input(docs()), &client, &cache)
            .await
            .unwrap();
        assert!(second.metadata.cached);
        assert_eq!(second.metadata.model_used, "pro-model-x");
        assert_eq!(second.metadata.total_tokens, 0);
        assert_eq!(second.result.answer, first.result.answer);
        assert_eq!(second.result.citations.len(), 1);

        // Post-processing options are not part of the key but still apply
        // to a cached answer.
        let mut no_citations = search_input(docs());
        no_citations.include_citations = false;
        let third = execute_v2_with_cache(no_citations, &client, &cache)
            .await
            .unwrap();
        assert!(third.metadata.cached);
        assert!(third.result.citations.is_empty());

        // min_relevance and max_results also apply to cached answers.
        assert_eq!(second.result.results.len(), 1);
        let mut strict = search_input(docs());
        strict.filters = Some(SearchFilters {
            source_ids: None,
            min_relevance: Some(0.9),
            max_results: None,
        });
        let fourth = execute_v2_with_cache(strict, &client, &cache)
            .await
            .unwrap();
        assert!(fourth.metadata.cached);
        assert!(fourth.result.results.is_empty());

        let mut capped = search_input(docs());
        capped.filters = Some(SearchFilters {
            source_ids: None,
            min_relevance: None,
            max_results: Some(0),
        });
        let fifth = execute_v2_with_cache(capped, &client, &cache)
            .await
            .unwrap();
        assert!(fifth.metadata.cached);
        assert!(fifth.result.results.is_empty());

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_search_does_not_cache_oversized_answers() {
        let big_text = "x".repeat(MAX_CACHED_ANSWER_BYTES + 1);
        let body = serde_json::json!({
            "candidates": [{"content": {"role": "model", "parts": [{"text": big_text}]}}]
        })
        .to_string();
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(200)
            .with_body(body)
            .expect(2)
            .create_async()
            .await;
        let client = mock_client(server.url());
        let cache = QueryCache::new(std::time::Duration::from_secs(300), 10);
        let docs = || vec![source("1", "Doc A", "Paris is the capital of France.")];

        for _ in 0..2 {
            let response = execute_v2_with_cache(search_input(docs()), &client, &cache)
                .await
                .unwrap();
            assert!(!response.metadata.cached);
        }
        assert_eq!(cache.len(), 0);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_search_cache_misses_when_source_content_changes() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(200)
            .with_body(GEMINI_BODY)
            .expect(2)
            .create_async()
            .await;
        let client = mock_client(server.url());
        let cache = QueryCache::new(std::time::Duration::from_secs(300), 10);

        let v1 = search_input(vec![source("1", "Doc A", "Paris is the capital.")]);
        let v2 = search_input(vec![source("1", "Doc A", "Lyon is the capital.")]);

        let first = execute_v2_with_cache(v1, &client, &cache).await.unwrap();
        let second = execute_v2_with_cache(v2, &client, &cache).await.unwrap();

        assert!(!first.metadata.cached);
        assert!(!second.metadata.cached, "same ID, new content must not hit");
        assert_eq!(cache.len(), 2);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_search_cache_misses_when_model_changes() {
        let mut server = mockito::Server::new_async().await;
        let pro = server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(200)
            .with_body(GEMINI_BODY)
            .expect(1)
            .create_async()
            .await;
        let flash = server
            .mock("POST", "/models/flash-model-y:generateContent")
            .with_status(200)
            .with_body(GEMINI_BODY)
            .expect(1)
            .create_async()
            .await;
        let client = mock_client(server.url());
        let cache = QueryCache::new(std::time::Duration::from_secs(300), 10);
        let docs = || vec![source("1", "Doc A", "Paris is the capital of France.")];

        execute_v2_with_cache(search_input(docs()), &client, &cache)
            .await
            .unwrap();
        let mut flash_input = search_input(docs());
        flash_input.model = Some(ModelPreference::Flash);
        let flash_result = execute_v2_with_cache(flash_input, &client, &cache)
            .await
            .unwrap();

        assert!(!flash_result.metadata.cached);
        assert_eq!(flash_result.metadata.model_used, "flash-model-y");
        pro.assert_async().await;
        flash.assert_async().await;
    }

    #[test]
    fn test_citation_serialize() {
        let citation = Citation {
            source_id: "1".to_string(),
            source_title: "Source".to_string(),
            quote: "This is a quote".to_string(),
        };

        let json = serde_json::to_string(&citation).unwrap();
        assert!(json.contains("This is a quote"));
    }
}
