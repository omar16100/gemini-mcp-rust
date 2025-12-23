use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::gemini::{client::GeminiClient, models::GeminiModel, types::GenerationConfig};
use crate::tools::types::{GenerationParams, ModelPreference, ResponseMetadata, ToolResponse};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SummarizeInput {
    #[schemars(description = "The text content to summarize")]
    pub content: String,

    #[schemars(description = "Summary length: brief, medium, or detailed")]
    #[serde(default = "default_length")]
    pub length: SummaryLength,

    #[schemars(description = "Summary format: paragraph, bullet_points, executive, or key_points")]
    #[serde(default = "default_format")]
    pub format: SummaryFormat,

    #[schemars(description = "Optional focus area for the summary")]
    #[serde(default)]
    pub focus: Option<String>,

    #[schemars(description = "Model preference")]
    #[serde(default)]
    pub model: Option<ModelPreference>,

    #[schemars(description = "Generation parameters")]
    #[serde(default)]
    pub params: Option<GenerationParams>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLength {
    Brief,
    Medium,
    Detailed,
}

fn default_length() -> SummaryLength {
    SummaryLength::Medium
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SummaryFormat {
    Paragraph,
    BulletPoints,
    Executive,
    KeyPoints,
}

fn default_format() -> SummaryFormat {
    SummaryFormat::Paragraph
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SummaryResult {
    pub summary: String,
    pub word_count: usize,
    pub key_topics: Vec<String>,
}

/// Legacy output for backward compatibility with existing server
#[derive(Debug, Serialize)]
pub struct SummarizeOutput {
    pub summary: String,
}

pub async fn execute(
    input: SummarizeInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<SummarizeOutput> {
    info!(
        "Summarize tool: length={:?}, format={:?}, content_len={}",
        input.length,
        input.format,
        input.content.len()
    );

    // Validate input
    if input.content.trim().is_empty() {
        anyhow::bail!("Content cannot be empty");
    }

    if input.content.len() > 1_000_000 {
        anyhow::bail!("Content too large (max 1M characters)");
    }

    let response = execute_v2(input, client).await?;

    // Convert to legacy format
    Ok(SummarizeOutput {
        summary: serde_json::to_string_pretty(&response)?,
    })
}

pub async fn execute_v2(
    input: SummarizeInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<ToolResponse<SummaryResult>> {
    debug!(
        "Summarize v2: length={:?}, format={:?}, content_len={}",
        input.length,
        input.format,
        input.content.len()
    );

    let (detail_instruction, max_tokens) = match input.length {
        SummaryLength::Brief => (
            "Provide a very brief, concise summary (2-3 sentences max).",
            256,
        ),
        SummaryLength::Detailed => (
            "Provide a comprehensive, detailed summary covering all key points and nuances.",
            2048,
        ),
        SummaryLength::Medium => (
            "Provide a balanced summary with key points and main themes.",
            1024,
        ),
    };

    let format_instruction = match input.format {
        SummaryFormat::BulletPoints => "\n\nFormat the summary as bullet points.",
        SummaryFormat::Executive => "\n\nFormat as an executive summary with clear sections.",
        SummaryFormat::KeyPoints => "\n\nExtract and list only the key takeaways.",
        SummaryFormat::Paragraph => "\n\nFormat the summary as coherent paragraphs.",
    };

    let focus_instruction = input
        .focus
        .as_ref()
        .map(|f| format!("\n\nFocus specifically on: {}", f))
        .unwrap_or_default();

    let prompt = format!(
        "Summarize the following content:\n\n{}\n\n{}{}{}",
        input.content, detail_instruction, format_instruction, focus_instruction
    );

    let model = match input.model {
        Some(ModelPreference::Pro) => GeminiModel::Pro,
        Some(ModelPreference::Flash) | None => GeminiModel::Flash,
    };

    let config = GenerationConfig {
        temperature: input.params.as_ref().and_then(|p| p.temperature).or(Some(0.4)),
        max_output_tokens: input.params.as_ref().and_then(|p| p.max_tokens).or(Some(max_tokens)),
        top_p: input.params.as_ref().and_then(|p| p.top_p),
        top_k: input.params.as_ref().and_then(|p| p.top_k),
    };

    let response = client
        .generate_content(&prompt, model, Some(config))
        .await?;

    debug!("Summary generated: {} chars", response.text.len());

    // Extract key topics (simple word frequency analysis)
    let key_topics = extract_key_topics(&response.text);

    // Count words
    let word_count = response.text.split_whitespace().count();

    let result = SummaryResult {
        summary: response.text,
        word_count,
        key_topics,
    };

    let metadata = ResponseMetadata::with_usage(model.as_str(), &response.usage);

    Ok(ToolResponse { result, metadata })
}

fn extract_key_topics(text: &str) -> Vec<String> {
    use std::collections::HashMap;

    // Simple keyword extraction: find frequently occurring words (4+ chars)
    let mut word_counts: HashMap<String, usize> = HashMap::new();

    for word in text.split_whitespace() {
        let clean = word
            .trim_matches(|c: char| !c.is_alphabetic())
            .to_lowercase();

        if clean.len() >= 4 {
            *word_counts.entry(clean).or_insert(0) += 1;
        }
    }

    // Stop words to filter out
    let stop_words = vec![
        "that", "this", "with", "from", "have", "will", "would", "could",
        "should", "about", "which", "their", "there", "these", "those",
        "been", "being", "were", "when", "where", "while", "after", "before",
    ];

    let mut topics: Vec<(String, usize)> = word_counts
        .into_iter()
        .filter(|(word, count)| *count >= 2 && !stop_words.contains(&word.as_str()))
        .collect();

    topics.sort_by(|a, b| b.1.cmp(&a.1));

    topics.into_iter().take(5).map(|(word, _)| word).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarize_input_defaults() {
        let json = r#"{"content": "Long text here..."}"#;
        let input: SummarizeInput = serde_json::from_str(json).unwrap();
        matches!(input.length, SummaryLength::Medium);
        matches!(input.format, SummaryFormat::Paragraph);
    }

    #[test]
    fn test_summarize_input_brief() {
        let json = r#"{"content": "Text", "length": "brief"}"#;
        let input: SummarizeInput = serde_json::from_str(json).unwrap();
        matches!(input.length, SummaryLength::Brief);
    }

    #[test]
    fn test_summarize_input_with_format() {
        let json = r#"{"content": "Text", "length": "detailed", "format": "bullet_points"}"#;
        let input: SummarizeInput = serde_json::from_str(json).unwrap();
        matches!(input.length, SummaryLength::Detailed);
        matches!(input.format, SummaryFormat::BulletPoints);
    }

    #[test]
    fn test_summarize_input_with_focus() {
        let json = r#"{"content": "Text", "focus": "main themes"}"#;
        let input: SummarizeInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.focus, Some("main themes".to_string()));
    }

    #[test]
    fn test_extract_key_topics() {
        let text = "Machine learning and artificial intelligence are important technologies. \
                   Learning algorithms enable intelligent systems.";
        let topics = extract_key_topics(text);

        assert!(!topics.is_empty());
        assert!(topics.contains(&"learning".to_string()) || topics.contains(&"machine".to_string()));
    }

    #[test]
    fn test_extract_key_topics_filters_stop_words() {
        let text = "This is a test with many common words that should be filtered.";
        let topics = extract_key_topics(text);

        // Stop words like "this", "that", "with", "should" should be filtered
        assert!(!topics.contains(&"this".to_string()));
        assert!(!topics.contains(&"that".to_string()));
        assert!(!topics.contains(&"with".to_string()));
    }

    #[test]
    fn test_summary_result_serialize() {
        let result = SummaryResult {
            summary: "Test summary".to_string(),
            word_count: 2,
            key_topics: vec!["test".to_string()],
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Test summary"));
        assert!(json.contains("word_count"));
    }
}
