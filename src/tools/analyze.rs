use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::gemini::{client::GeminiClient, models::GeminiModel};
use crate::tools::types::{GenerationParams, ModelPreference, ResponseMetadata, ToolResponse};

// Shared analyze output for backward compatibility
#[derive(Debug, Serialize)]
pub struct AnalyzeOutput {
    pub analysis: String,
}

// Legacy inputs for backward compatibility
#[derive(Debug, Deserialize)]
pub struct AnalyzeCodeInput {
    pub code: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_focus")]
    pub focus: String,
}

#[derive(Debug, Deserialize)]
pub struct AnalyzeTextInput {
    pub text: String,
    #[serde(default)]
    pub focus: Option<String>,
}

fn default_focus() -> String {
    "general".to_string()
}

// V2 unified analyze input
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeInput {
    #[schemars(description = "The content to analyze")]
    pub content: String,

    #[schemars(description = "Type of analyzer to use")]
    pub analyzer_type: AnalyzerType,

    #[schemars(description = "Analyzer options")]
    #[serde(default)]
    pub options: Option<AnalyzerOptions>,

    #[schemars(description = "Model preference")]
    #[serde(default)]
    pub model: Option<ModelPreference>,

    #[schemars(description = "Generation parameters")]
    #[serde(default)]
    pub params: Option<GenerationParams>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "params")]
pub enum AnalyzerType {
    #[serde(rename = "text")]
    Text,

    #[serde(rename = "code")]
    Code {
        #[serde(default)]
        language: Option<String>,
    },

    #[serde(rename = "document")]
    Document,

    #[serde(rename = "sentiment")]
    Sentiment,

    #[serde(rename = "comparison")]
    Comparison {
        compare_with: String,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzerOptions {
    #[schemars(description = "Specific aspects to focus on")]
    pub focus_areas: Option<Vec<String>>,

    #[schemars(description = "Level of detail in analysis")]
    #[serde(default = "default_detail_level")]
    pub detail_level: DetailLevel,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DetailLevel {
    Brief,
    Standard,
    Comprehensive,
}

fn default_detail_level() -> DetailLevel {
    DetailLevel::Standard
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "type")]
pub enum AnalyzeResult {
    #[serde(rename = "text")]
    Text(TextAnalysis),

    #[serde(rename = "code")]
    Code(CodeAnalysis),

    #[serde(rename = "document")]
    Document(DocumentAnalysis),

    #[serde(rename = "sentiment")]
    Sentiment(SentimentAnalysis),

    #[serde(rename = "comparison")]
    Comparison(ComparisonAnalysis),
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TextAnalysis {
    pub sentiment: String,
    pub themes: Vec<String>,
    pub tone: String,
    pub key_points: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CodeAnalysis {
    pub quality_score: f32,
    pub issues: Vec<CodeIssue>,
    pub patterns: Vec<String>,
    pub complexity: String,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CodeIssue {
    pub severity: String,
    pub category: String,
    pub description: String,
    pub location: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentAnalysis {
    pub structure: String,
    pub readability_score: f32,
    pub sections: Vec<String>,
    pub key_points: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SentimentAnalysis {
    pub overall_sentiment: String,
    pub confidence: f32,
    pub emotions: Vec<Emotion>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Emotion {
    pub name: String,
    pub intensity: f32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ComparisonAnalysis {
    pub similarities: Vec<String>,
    pub differences: Vec<String>,
    pub verdict: String,
}

// Legacy execute_code for backward compatibility
pub async fn execute_code(
    input: AnalyzeCodeInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<AnalyzeOutput> {
    info!(
        "Analyze code (legacy): language={:?}, focus={}, code_len={}",
        input.language,
        input.focus,
        input.code.len()
    );

    let focus_instruction = match input.focus.as_str() {
        "quality" => "Focus on code quality, readability, and best practices.",
        "security" => "Focus on security vulnerabilities and potential exploits.",
        "performance" => "Focus on performance optimizations and bottlenecks.",
        "bugs" => "Focus on identifying bugs and logical errors.",
        _ => "Provide a general comprehensive analysis.",
    };

    let lang_info = input
        .language
        .as_ref()
        .map(|l| format!("Language: {}\n", l))
        .unwrap_or_default();

    let prompt = format!(
        "Analyze the following code:\n\n{}```\n{}\n```\n\n{}",
        lang_info, input.code, focus_instruction
    );

    let response = client
        .generate_content(&prompt, GeminiModel::Pro, None)
        .await?;

    debug!("Analyze code (legacy): analysis_len={}", response.text.len());

    Ok(AnalyzeOutput { analysis: response.text })
}

// Legacy execute_text for backward compatibility
pub async fn execute_text(
    input: AnalyzeTextInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<AnalyzeOutput> {
    info!(
        "Analyze text (legacy): focus={:?}, text_len={}",
        input.focus,
        input.text.len()
    );

    let focus_instruction = input
        .focus
        .as_ref()
        .map(|f| format!("\n\nFocus on: {}", f))
        .unwrap_or_default();

    let prompt = format!(
        "Analyze the following text:{}\n\n{}",
        focus_instruction, input.text
    );

    let response = client
        .generate_content(&prompt, GeminiModel::Pro, None)
        .await?;

    debug!("Analyze text (legacy): analysis_len={}", response.text.len());

    Ok(AnalyzeOutput { analysis: response.text })
}

// V2 unified analyze implementation
pub async fn execute_v2(
    input: AnalyzeInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<ToolResponse<AnalyzeResult>> {
    info!(
        "Analyze v2: type={:?}, content_len={}",
        input.analyzer_type,
        input.content.len()
    );

    // Validate input
    if input.content.trim().is_empty() {
        anyhow::bail!("Content cannot be empty");
    }

    let model = match input.model {
        Some(ModelPreference::Flash) => GeminiModel::Flash,
        Some(ModelPreference::Pro) | None => GeminiModel::Pro,
    };

    let (result, usage) = match &input.analyzer_type {
        AnalyzerType::Text => {
            let (analysis, usage) = analyze_text(&input, &client, model).await?;
            (AnalyzeResult::Text(analysis), usage)
        }
        AnalyzerType::Code { language } => {
            let (analysis, usage) = analyze_code(&input, language.clone(), &client, model).await?;
            (AnalyzeResult::Code(analysis), usage)
        }
        AnalyzerType::Document => {
            let (analysis, usage) = analyze_document(&input, &client, model).await?;
            (AnalyzeResult::Document(analysis), usage)
        }
        AnalyzerType::Sentiment => {
            let (analysis, usage) = analyze_sentiment(&input, &client, model).await?;
            (AnalyzeResult::Sentiment(analysis), usage)
        }
        AnalyzerType::Comparison { compare_with } => {
            let (analysis, usage) = analyze_comparison(&input, compare_with, &client, model).await?;
            (AnalyzeResult::Comparison(analysis), usage)
        }
    };

    let metadata = ResponseMetadata::with_usage(model.as_str(), &usage);

    Ok(ToolResponse { result, metadata })
}

async fn analyze_text(
    input: &AnalyzeInput,
    client: &GeminiClient,
    model: GeminiModel,
) -> anyhow::Result<(TextAnalysis, crate::gemini::types::UsageMetadata)> {
    debug!("Running text analyzer");

    let focus = input
        .options
        .as_ref()
        .and_then(|o| o.focus_areas.as_ref())
        .map(|f| format!("\nFocus on: {}", f.join(", ")))
        .unwrap_or_default();

    let prompt = format!(
        "Analyze the following text and provide:\n\
         1. Overall sentiment (positive, negative, neutral, mixed)\n\
         2. Main themes (3-5 themes)\n\
         3. Tone (formal, informal, technical, conversational, etc.)\n\
         4. Key points (3-5 bullet points){}\n\n\
         Text:\n{}\n\n\
         Provide analysis in a structured format.",
        focus, input.content
    );

    let response = client.generate_content(&prompt, model, None).await?;

    // Parse the response (simplified - in production, use JSON mode)
    let sentiment = extract_field(&response.text, "sentiment").unwrap_or_else(|| "neutral".to_string());
    let themes = extract_list(&response.text, "theme");
    let tone = extract_field(&response.text, "tone").unwrap_or_else(|| "neutral".to_string());
    let key_points = extract_list(&response.text, "key point");

    let analysis = TextAnalysis {
        sentiment,
        themes,
        tone,
        key_points,
    };

    Ok((analysis, response.usage))
}

async fn analyze_code(
    input: &AnalyzeInput,
    language: Option<String>,
    client: &GeminiClient,
    model: GeminiModel,
) -> anyhow::Result<(CodeAnalysis, crate::gemini::types::UsageMetadata)> {
    debug!("Running code analyzer for language: {:?}", language);

    let lang_info = language
        .as_ref()
        .map(|l| format!("Language: {}\n", l))
        .unwrap_or_default();

    let prompt = format!(
        "Analyze this code and provide:\n\
         1. Quality score (0-10)\n\
         2. List of issues with severity (critical/high/medium/low) and category\n\
         3. Design patterns used\n\
         4. Complexity assessment\n\
         5. Improvement suggestions\n\n\
         {}```\n{}\n```\n\n\
         Be specific and actionable.",
        lang_info, input.content
    );

    let response = client.generate_content(&prompt, model, None).await?;

    // Parse response (simplified)
    let quality_score = extract_score(&response.text).unwrap_or(5.0);
    let issues = extract_issues(&response.text);
    let patterns = extract_list(&response.text, "pattern");
    let complexity = extract_field(&response.text, "complexity").unwrap_or_else(|| "moderate".to_string());
    let suggestions = extract_list(&response.text, "suggestion");

    let analysis = CodeAnalysis {
        quality_score,
        issues,
        patterns,
        complexity,
        suggestions,
    };

    Ok((analysis, response.usage))
}

async fn analyze_document(
    input: &AnalyzeInput,
    client: &GeminiClient,
    model: GeminiModel,
) -> anyhow::Result<(DocumentAnalysis, crate::gemini::types::UsageMetadata)> {
    debug!("Running document analyzer");

    let prompt = format!(
        "Analyze this document's structure and readability:\n\
         1. Overall structure (how it's organized)\n\
         2. Readability score (0-10, where 10 is most readable)\n\
         3. Main sections\n\
         4. Key points\n\n\
         Document:\n{}\n\n\
         Provide structured analysis.",
        input.content
    );

    let response = client.generate_content(&prompt, model, None).await?;

    let structure = extract_field(&response.text, "structure").unwrap_or_else(|| "linear".to_string());
    let readability_score = extract_score(&response.text).unwrap_or(7.0);
    let sections = extract_list(&response.text, "section");
    let key_points = extract_list(&response.text, "key point");

    let analysis = DocumentAnalysis {
        structure,
        readability_score,
        sections,
        key_points,
    };

    Ok((analysis, response.usage))
}

async fn analyze_sentiment(
    input: &AnalyzeInput,
    client: &GeminiClient,
    model: GeminiModel,
) -> anyhow::Result<(SentimentAnalysis, crate::gemini::types::UsageMetadata)> {
    debug!("Running sentiment analyzer");

    let prompt = format!(
        "Perform detailed sentiment analysis:\n\
         1. Overall sentiment (very negative, negative, neutral, positive, very positive)\n\
         2. Confidence level (0-1)\n\
         3. Detected emotions with intensity (0-1): joy, sadness, anger, fear, surprise, etc.\n\n\
         Text:\n{}\n\n\
         Be precise and nuanced.",
        input.content
    );

    let response = client.generate_content(&prompt, model, None).await?;

    let overall_sentiment = extract_field(&response.text, "sentiment")
        .unwrap_or_else(|| "neutral".to_string());
    let confidence = extract_score(&response.text).unwrap_or(0.5);
    let emotions = extract_emotions(&response.text);

    let analysis = SentimentAnalysis {
        overall_sentiment,
        confidence,
        emotions,
    };

    Ok((analysis, response.usage))
}

async fn analyze_comparison(
    input: &AnalyzeInput,
    compare_with: &str,
    client: &GeminiClient,
    model: GeminiModel,
) -> anyhow::Result<(ComparisonAnalysis, crate::gemini::types::UsageMetadata)> {
    debug!("Running comparison analyzer");

    let prompt = format!(
        "Compare these two texts:\n\n\
         Text A:\n{}\n\n\
         Text B:\n{}\n\n\
         Provide:\n\
         1. Key similarities\n\
         2. Key differences\n\
         3. Overall verdict on how similar they are",
        input.content, compare_with
    );

    let response = client.generate_content(&prompt, model, None).await?;

    let similarities = extract_list(&response.text, "similar");
    let differences = extract_list(&response.text, "differ");
    let verdict = extract_field(&response.text, "verdict")
        .unwrap_or_else(|| "moderately similar".to_string());

    Ok(ComparisonAnalysis {
        similarities,
        differences,
        verdict,
    })
}

// Helper parsing functions (simplified - in production, use structured JSON output)
fn extract_field(text: &str, field: &str) -> Option<String> {
    text.lines()
        .find(|line| line.to_lowercase().contains(field))
        .map(|line| {
            line.split(':')
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string()
        })
}

fn extract_score(text: &str) -> Option<f32> {
    use regex::Regex;
    let score_regex = Regex::new(r"(\d+\.?\d*)/10|score[:\s]+(\d+\.?\d*)").ok()?;
    score_regex.captures(text).and_then(|cap| {
        cap.get(1)
            .or_else(|| cap.get(2))
            .and_then(|m| m.as_str().parse().ok())
    })
}

fn extract_list(text: &str, keyword: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.to_lowercase().contains(keyword) && (line.starts_with('-') || line.starts_with('*') || line.contains('•')))
        .map(|line| {
            line.trim_start_matches('-')
                .trim_start_matches('*')
                .trim_start_matches('•')
                .trim()
                .to_string()
        })
        .collect()
}

fn extract_issues(text: &str) -> Vec<CodeIssue> {
    // Simplified extraction
    let mut issues = Vec::new();
    for line in text.lines() {
        if line.to_lowercase().contains("issue") || line.to_lowercase().contains("problem") {
            issues.push(CodeIssue {
                severity: "medium".to_string(),
                category: "general".to_string(),
                description: line.trim().to_string(),
                location: None,
            });
        }
    }
    issues
}

fn extract_emotions(text: &str) -> Vec<Emotion> {
    let emotion_keywords = ["joy", "sadness", "anger", "fear", "surprise", "trust"];
    let mut emotions = Vec::new();

    for keyword in &emotion_keywords {
        if text.to_lowercase().contains(keyword) {
            emotions.push(Emotion {
                name: keyword.to_string(),
                intensity: 0.5,
            });
        }
    }

    emotions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_code_input_defaults() {
        let json = r#"{"code": "fn main() {}"}"#;
        let input: AnalyzeCodeInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.focus, "general");
        assert_eq!(input.language, None);
    }

    #[test]
    fn test_extract_field() {
        let text = "Sentiment: positive\nTone: formal";
        assert_eq!(extract_field(text, "sentiment"), Some("positive".to_string()));
        assert_eq!(extract_field(text, "tone"), Some("formal".to_string()));
    }

    #[test]
    fn test_extract_score() {
        let text = "Quality score: 8.5/10";
        assert_eq!(extract_score(text), Some(8.5));

        let text2 = "Readability score 7.2";
        assert_eq!(extract_score(text2), Some(7.2));
    }

    #[test]
    fn test_extract_list() {
        let text = "Themes:\n- Theme 1\n- Theme 2\n* Theme 3";
        let themes = extract_list(text, "theme");
        assert!(themes.len() >= 2);
    }

    #[test]
    fn test_code_issue_serialize() {
        let issue = CodeIssue {
            severity: "high".to_string(),
            category: "security".to_string(),
            description: "SQL injection risk".to_string(),
            location: Some("line 42".to_string()),
        };

        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("high"));
        assert!(json.contains("security"));
    }

    #[test]
    fn test_emotion_serialize() {
        let emotion = Emotion {
            name: "joy".to_string(),
            intensity: 0.8,
        };

        let json = serde_json::to_string(&emotion).unwrap();
        assert!(json.contains("joy"));
        assert!(json.contains("0.8"));
    }
}
