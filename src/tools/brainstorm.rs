use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

use crate::gemini::{client::GeminiClient, models::GeminiModel, types::GenerationConfig};
use crate::tools::types::{GenerationParams, ModelPreference, ResponseMetadata, ToolResponse};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrainstormInput {
    #[schemars(description = "The topic or problem to brainstorm about")]
    pub prompt: String,

    #[schemars(description = "Number of ideas to generate (1-50)")]
    #[serde(default = "default_num_ideas")]
    pub num_ideas: u32,

    #[schemars(description = "Optional constraints or context for brainstorming")]
    #[serde(default)]
    pub constraints: Option<String>,

    #[schemars(description = "Extract consensus themes from generated ideas")]
    #[serde(default = "default_extract_consensus")]
    pub extract_consensus: bool,

    #[schemars(description = "Model preference")]
    #[serde(default)]
    pub model: Option<ModelPreference>,

    #[schemars(description = "Generation parameters")]
    #[serde(default)]
    pub params: Option<GenerationParams>,

    // Legacy field for backward compatibility
    #[serde(default)]
    pub claude_thoughts: Option<String>,

    #[serde(default = "default_max_rounds")]
    pub max_rounds: Option<u32>,
}

fn default_num_ideas() -> u32 {
    10
}

fn default_extract_consensus() -> bool {
    true
}

fn default_max_rounds() -> Option<u32> {
    Some(3)
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BrainstormResult {
    pub ideas: Vec<Idea>,
    pub consensus_themes: Option<Vec<ConsensusTheme>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Idea {
    pub id: usize,
    pub text: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConsensusTheme {
    pub theme: String,
    pub frequency: usize,
    pub related_ideas: Vec<usize>,
}

/// Legacy output for backward compatibility
#[derive(Debug, Serialize)]
pub struct BrainstormOutput {
    pub synthesis: String,
    pub conversation_history: String,
}

pub async fn execute(
    input: BrainstormInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<BrainstormOutput> {
    info!(
        "Brainstorm tool: topic_len={}, num_ideas={}",
        input.prompt.len(),
        input.num_ideas
    );

    // Check if this is a legacy request (has claude_thoughts)
    if input.claude_thoughts.is_some() {
        return execute_legacy(input, client).await;
    }

    let response = execute_v2(input, client).await?;

    // Convert to legacy format
    let synthesis = serde_json::to_string_pretty(&response)?;
    Ok(BrainstormOutput {
        synthesis,
        conversation_history: String::new(),
    })
}

pub async fn execute_v2(
    input: BrainstormInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<ToolResponse<BrainstormResult>> {
    debug!(
        "Brainstorm v2: topic={}, num_ideas={}, extract_consensus={}",
        input.prompt,
        input.num_ideas,
        input.extract_consensus
    );

    // Validate input
    if input.num_ideas == 0 || input.num_ideas > 50 {
        anyhow::bail!("num_ideas must be between 1 and 50");
    }

    if input.prompt.trim().is_empty() {
        anyhow::bail!("Topic cannot be empty");
    }

    let mut prompt = format!(
        "Generate {} creative, diverse ideas for the following topic:\n\n{}\n\n",
        input.num_ideas, input.prompt
    );

    if let Some(constraints) = &input.constraints {
        prompt.push_str(&format!("Constraints: {}\n\n", constraints));
    }

    prompt.push_str("List each idea on a new line, numbered (1., 2., 3., etc.).\n");
    prompt.push_str("Make ideas specific, actionable, and varied in approach.");

    let model = match input.model {
        Some(ModelPreference::Flash) => GeminiModel::Flash,
        Some(ModelPreference::Pro) | None => GeminiModel::Pro,
    };

    let config = GenerationConfig {
        temperature: input.params.as_ref().and_then(|p| p.temperature).or(Some(0.9)),
        max_output_tokens: input.params.as_ref().and_then(|p| p.max_tokens).or(Some(2048)),
        top_p: input.params.as_ref().and_then(|p| p.top_p),
        top_k: input.params.as_ref().and_then(|p| p.top_k),
    };

    let response = client
        .generate_content(&prompt, model, Some(config))
        .await?;

    debug!("Ideas generated: {} chars", response.text.len());

    // Parse ideas into structured list
    let ideas = parse_ideas(&response.text);

    info!("Parsed {} ideas", ideas.len());

    // Extract consensus themes if requested
    let consensus_themes = if input.extract_consensus {
        Some(extract_consensus_themes(&ideas))
    } else {
        None
    };

    let result = BrainstormResult {
        ideas,
        consensus_themes,
    };

    let metadata = ResponseMetadata::with_usage(model.as_str(), &response.usage);

    Ok(ToolResponse { result, metadata })
}

fn parse_ideas(text: &str) -> Vec<Idea> {
    let line_regex = Regex::new(r"^\s*(\d+)\.?\s*(.+)$").unwrap();
    let mut ideas = Vec::new();
    let mut id = 1;

    for line in text.lines() {
        if let Some(captures) = line_regex.captures(line) {
            if let Some(text_match) = captures.get(2) {
                ideas.push(Idea {
                    id,
                    text: text_match.as_str().trim().to_string(),
                });
                id += 1;
            }
        } else if !line.trim().is_empty() && !ideas.is_empty() {
            // Continuation of previous idea
            if let Some(last) = ideas.last_mut() {
                last.text.push(' ');
                last.text.push_str(line.trim());
            }
        }
    }

    ideas
}

fn extract_consensus_themes(ideas: &[Idea]) -> Vec<ConsensusTheme> {
    // Extract keywords from each idea (4+ character words)
    let word_regex = Regex::new(r"\b[a-zA-Z]{4,}\b").unwrap();
    let mut keyword_to_ideas: HashMap<String, Vec<usize>> = HashMap::new();

    for idea in ideas {
        let lowercased = idea.text.to_lowercase();
        let mut seen_in_idea = std::collections::HashSet::new();

        for cap in word_regex.captures_iter(&lowercased) {
            if let Some(word_match) = cap.get(0) {
                let word = word_match.as_str().to_string();
                if seen_in_idea.insert(word.clone()) {
                    keyword_to_ideas
                        .entry(word)
                        .or_insert_with(Vec::new)
                        .push(idea.id);
                }
            }
        }
    }

    // Stop words to filter out
    let stop_words = vec![
        "that", "this", "with", "from", "have", "will", "would", "could",
        "should", "about", "which", "their", "there", "these", "those",
        "been", "being", "were", "when", "where", "while", "after", "before",
        "using", "make", "more", "into", "over", "such", "also", "some",
        "than", "them", "then", "very", "well", "only", "just", "even",
    ];

    // Filter by threshold (appears in at least 30% of ideas)
    let threshold = (ideas.len() as f32 * 0.3).ceil() as usize;

    let mut themes: Vec<ConsensusTheme> = keyword_to_ideas
        .into_iter()
        .filter(|(word, idea_ids)| {
            idea_ids.len() >= threshold && !stop_words.contains(&word.as_str())
        })
        .map(|(word, idea_ids)| ConsensusTheme {
            theme: word,
            frequency: idea_ids.len(),
            related_ideas: idea_ids,
        })
        .collect();

    // Sort by frequency (descending)
    themes.sort_by(|a, b| b.frequency.cmp(&a.frequency));

    // Return top 10 themes
    themes.into_iter().take(10).collect()
}

// Legacy implementation for backward compatibility
async fn execute_legacy(
    input: BrainstormInput,
    client: Arc<GeminiClient>,
) -> anyhow::Result<BrainstormOutput> {
    info!("Using legacy brainstorm implementation");

    let claude_thoughts = input.claude_thoughts.unwrap_or_default();
    let _max_rounds = input.max_rounds.unwrap_or(3);

    // Simple legacy implementation: just get Gemini's response
    let prompt = format!(
        "Collaborative brainstorm on: {}\n\nClaude's thoughts: {}\n\nRespond with your insights.",
        input.prompt, claude_thoughts
    );

    let response = client
        .generate_content(&prompt, GeminiModel::Pro, None)
        .await?;

    let synthesis = response.text;
    let conversation_history = format!("Round 1\nClaude: {}\nGemini: {}", claude_thoughts, &synthesis);

    Ok(BrainstormOutput {
        synthesis,
        conversation_history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brainstorm_input_defaults() {
        let json = r#"{"prompt": "Topic"}"#;
        let input: BrainstormInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.num_ideas, 10);
        assert!(input.extract_consensus);
    }

    #[test]
    fn test_brainstorm_input_custom() {
        let json = r#"{
            "prompt": "Topic",
            "num_ideas": 20,
            "constraints": "Must be innovative",
            "extract_consensus": false
        }"#;
        let input: BrainstormInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.num_ideas, 20);
        assert_eq!(input.constraints, Some("Must be innovative".to_string()));
        assert!(!input.extract_consensus);
    }

    #[test]
    fn test_parse_ideas() {
        let text = "1. First idea\n2. Second idea\n3. Third idea with\nextra line";
        let ideas = parse_ideas(text);

        assert_eq!(ideas.len(), 3);
        assert_eq!(ideas[0].id, 1);
        assert_eq!(ideas[0].text, "First idea");
        assert_eq!(ideas[1].id, 2);
        assert_eq!(ideas[1].text, "Second idea");
        assert_eq!(ideas[2].id, 3);
        assert!(ideas[2].text.contains("Third idea"));
    }

    #[test]
    fn test_parse_ideas_with_dots() {
        let text = "1. Idea one\n2. Idea two\n3. Idea three";
        let ideas = parse_ideas(text);
        assert_eq!(ideas.len(), 3);
    }

    #[test]
    fn test_extract_consensus_themes() {
        let ideas = vec![
            Idea {
                id: 1,
                text: "Use machine learning for automation".to_string(),
            },
            Idea {
                id: 2,
                text: "Implement machine learning algorithms".to_string(),
            },
            Idea {
                id: 3,
                text: "Apply learning techniques to data".to_string(),
            },
            Idea {
                id: 4,
                text: "Create automated workflows".to_string(),
            },
        ];

        let themes = extract_consensus_themes(&ideas);

        assert!(!themes.is_empty());

        // "learning" should appear in themes as it's in 3/4 ideas (75% > 30% threshold)
        let has_learning = themes.iter().any(|t| t.theme == "learning");
        assert!(has_learning, "Expected 'learning' in consensus themes");

        // Verify theme structure
        if let Some(learning_theme) = themes.iter().find(|t| t.theme == "learning") {
            assert_eq!(learning_theme.frequency, 3);
            assert!(learning_theme.related_ideas.contains(&1));
            assert!(learning_theme.related_ideas.contains(&2));
            assert!(learning_theme.related_ideas.contains(&3));
        }
    }

    #[test]
    fn test_extract_consensus_filters_stop_words() {
        let ideas = vec![
            Idea {
                id: 1,
                text: "This is a test with common words".to_string(),
            },
            Idea {
                id: 2,
                text: "This has some words that should filter".to_string(),
            },
            Idea {
                id: 3,
                text: "These words are very common".to_string(),
            },
        ];

        let themes = extract_consensus_themes(&ideas);

        // Stop words should be filtered out
        assert!(!themes.iter().any(|t| t.theme == "this"));
        assert!(!themes.iter().any(|t| t.theme == "that"));
        assert!(!themes.iter().any(|t| t.theme == "with"));
        assert!(!themes.iter().any(|t| t.theme == "should"));
    }

    #[test]
    fn test_consensus_theme_serialize() {
        let theme = ConsensusTheme {
            theme: "innovation".to_string(),
            frequency: 5,
            related_ideas: vec![1, 2, 3, 4, 5],
        };

        let json = serde_json::to_string(&theme).unwrap();
        assert!(json.contains("innovation"));
        assert!(json.contains("frequency"));
        assert!(json.contains("related_ideas"));
    }
}
