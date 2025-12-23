use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::gemini::{client::GeminiClient, models::GeminiModel, types::GenerationConfig};
use crate::tools::types::{GenerationParams, ModelPreference, ResponseMetadata, ToolResponse};

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

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResult {
    pub answer: String,
    pub results: Vec<SourceResult>,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceResult {
    pub source_id: String,
    pub source_title: String,
    pub excerpt: String,
    pub relevance_score: f32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Citation {
    pub source_id: String,
    pub source_title: String,
    pub quote: String,
}

// Legacy execute function
pub async fn execute(
    input: QueryInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<QueryOutput> {
    debug!("Query tool (legacy): model={}, prompt_len={}", input.model, input.prompt.len());

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

    let response = client.generate_content(&input.prompt, model, config).await?;

    if response.text.trim().is_empty() {
        anyhow::bail!("Empty response from Gemini API");
    }

    debug!("Query tool (legacy): response_len={}", response.text.len());

    Ok(QueryOutput { text: response.text })
}

// V2 multi-source search implementation
pub async fn execute_v2(
    input: SearchInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<ToolResponse<SearchResult>> {
    info!(
        "Search v2: query='{}', sources={}, include_citations={}",
        input.query,
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
    let filtered_sources: Vec<&Source> = if let Some(filter_ids) = input.filters.as_ref().and_then(|f| f.source_ids.as_ref()) {
        input.sources.iter()
            .filter(|s| filter_ids.contains(&s.id))
            .collect()
    } else {
        input.sources.iter().collect()
    };

    if filtered_sources.is_empty() {
        anyhow::bail!("No sources match the filter criteria");
    }

    debug!("Filtered to {} sources", filtered_sources.len());

    // Build search prompt with all sources
    let mut prompt = format!(
        "You are performing a semantic search across multiple sources.\n\n\
         Query: {}\n\n\
         Sources:\n\n",
        input.query
    );

    for source in &filtered_sources {
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
         Format your response clearly with sections for Answer, Results, and Citations."
    );

    let model = match input.model {
        Some(ModelPreference::Flash) => GeminiModel::Flash,
        Some(ModelPreference::Pro) | None => GeminiModel::Pro,
    };

    let config = GenerationConfig {
        temperature: input.params.as_ref().and_then(|p| p.temperature).or(Some(0.3)),
        max_output_tokens: input.params.as_ref().and_then(|p| p.max_tokens).or(Some(2048)),
        top_p: input.params.as_ref().and_then(|p| p.top_p),
        top_k: input.params.as_ref().and_then(|p| p.top_k),
    };

    let response = client
        .generate_content(&prompt, model, Some(config))
        .await?;

    debug!("Search response: {} chars", response.text.len());

    // Parse response into structured results
    let answer = extract_answer(&response.text);
    let mut results = extract_results(&response.text, &filtered_sources);
    let citations = if input.include_citations {
        extract_citations(&response.text, &filtered_sources)
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
            results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());
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

    info!("Search complete: {} results, {} citations", results.len(), citations.len());

    let result = SearchResult {
        answer,
        results,
        citations,
    };

    let metadata = ResponseMetadata::with_usage(model.as_str(), &response.usage);

    Ok(ToolResponse { result, metadata })
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
                if source.content.contains(&quote) || source.content.to_lowercase().contains(&quote.to_lowercase()) {
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
        let sources = vec![
            Source {
                id: "1".to_string(),
                title: "Document A".to_string(),
                content: "Content A".to_string(),
            },
            Source {
                id: "2".to_string(),
                title: "Document B".to_string(),
                content: "Content B".to_string(),
            },
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
