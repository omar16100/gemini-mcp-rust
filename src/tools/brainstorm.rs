use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

use crate::gemini::{client::GeminiClient, models::GeminiModel};
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

#[derive(Debug, Clone, Serialize, JsonSchema)]
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
        input.prompt, input.num_ideas, input.extract_consensus
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

    let config = GenerationParams::to_config(input.params.as_ref(), Some(0.9), Some(2048));

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

    let metadata = ResponseMetadata::with_usage(client.model_name(model), &response.usage);

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
    use std::collections::HashSet;

    // Expanded stop words list
    let stop_words: HashSet<&str> = [
        "that", "this", "with", "from", "have", "will", "would", "could", "should", "about",
        "which", "their", "there", "these", "those", "been", "being", "were", "when", "where",
        "while", "after", "before", "using", "make", "more", "into", "over", "such", "also",
        "some", "than", "them", "then", "very", "well", "only", "just", "even", "what", "your",
        "each", "other", "does", "said", "many", "much", "here", "like", "back", "down", "used",
        "through", "same", "both",
    ]
    .iter()
    .copied()
    .collect();

    let mut keyword_to_ideas: HashMap<String, Vec<usize>> = HashMap::new();
    let mut phrase_to_ideas: HashMap<String, Vec<usize>> = HashMap::new();

    // Extract single words and multi-word phrases
    let word_regex = Regex::new(r"\b[a-zA-Z]{4,}\b").unwrap();

    for idea in ideas {
        let lowercased = idea.text.to_lowercase();
        let words: Vec<&str> = word_regex
            .find_iter(&lowercased)
            .map(|m| m.as_str())
            .collect();

        let mut seen_keywords: HashSet<String> = HashSet::new();
        let mut seen_phrases: HashSet<String> = HashSet::new();

        // Extract single keywords (4+ chars, not stop words)
        for word in &words {
            if !stop_words.contains(word) {
                let word_str = word.to_string();
                if seen_keywords.insert(word_str.clone()) {
                    keyword_to_ideas.entry(word_str).or_default().push(idea.id);
                }
            }
        }

        // Extract bigrams (2-word phrases)
        for window in words.windows(2) {
            if window.len() == 2 && !stop_words.contains(window[1]) {
                let phrase = format!("{} {}", window[0], window[1]);
                if seen_phrases.insert(phrase.clone()) {
                    phrase_to_ideas.entry(phrase).or_default().push(idea.id);
                }
            }
        }

        // Extract trigrams (3-word phrases)
        for window in words.windows(3) {
            if window.len() == 3 {
                let phrase = format!("{} {} {}", window[0], window[1], window[2]);
                if seen_phrases.insert(phrase.clone()) {
                    phrase_to_ideas.entry(phrase).or_default().push(idea.id);
                }
            }
        }
    }

    // Filter by threshold (appears in at least 25% of ideas, lowered from 30%)
    let threshold = (ideas.len() as f32 * 0.25).ceil() as usize;

    // Combine keywords and phrases with relevance scoring
    let mut all_themes: Vec<ConsensusTheme> = Vec::new();

    // Process single keywords
    for (word, idea_ids) in keyword_to_ideas {
        if idea_ids.len() >= threshold && !stop_words.contains(word.as_str()) {
            all_themes.push(ConsensusTheme {
                theme: word,
                frequency: idea_ids.len(),
                related_ideas: idea_ids,
            });
        }
    }

    // Process phrases (with higher threshold for phrases)
    let phrase_threshold = (ideas.len() as f32 * 0.2).ceil() as usize;
    for (phrase, idea_ids) in phrase_to_ideas {
        if idea_ids.len() >= phrase_threshold && phrase.len() > 8 {
            all_themes.push(ConsensusTheme {
                theme: phrase,
                frequency: idea_ids.len(),
                related_ideas: idea_ids,
            });
        }
    }

    // Enhanced scoring: frequency * distribution_score
    // Distribution score is higher when ideas are spread across different positions
    all_themes.sort_by(|a, b| {
        let score_a = calculate_theme_score(a, ideas.len());
        let score_b = calculate_theme_score(b, ideas.len());
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Semantic clustering: group similar themes
    let clustered = cluster_similar_themes(all_themes);

    // Return top 10 themes
    clustered.into_iter().take(10).collect()
}

/// Calculate relevance score for a theme
fn calculate_theme_score(theme: &ConsensusTheme, total_ideas: usize) -> f32 {
    let frequency_score = theme.frequency as f32;

    // Distribution score: higher when ideas are spread throughout the list
    let min_id = *theme.related_ideas.iter().min().unwrap_or(&1) as f32;
    let max_id = *theme.related_ideas.iter().max().unwrap_or(&total_ideas) as f32;
    let spread = (max_id - min_id) / total_ideas as f32;
    let distribution_score = 1.0 + spread;

    frequency_score * distribution_score
}

/// Simple semantic clustering: merge themes with shared words
fn cluster_similar_themes(themes: Vec<ConsensusTheme>) -> Vec<ConsensusTheme> {
    use std::collections::HashSet;

    let mut result: Vec<ConsensusTheme> = Vec::new();
    let mut used_indices: HashSet<usize> = HashSet::new();

    for (i, theme) in themes.iter().enumerate() {
        if used_indices.contains(&i) {
            continue;
        }

        let mut merged_theme = theme.clone();
        used_indices.insert(i);

        // Look for similar themes (single words that are part of phrases, or vice versa)
        for (j, other) in themes.iter().enumerate().skip(i + 1) {
            if used_indices.contains(&j) {
                continue;
            }

            // Merge if one theme contains the other as whole words
            if contains_words(&theme.theme, &other.theme)
                || contains_words(&other.theme, &theme.theme)
            {
                // Prefer the longer/more specific theme
                if other.theme.len() > merged_theme.theme.len() {
                    merged_theme.theme = other.theme.clone();
                }
                // Merge idea lists
                for id in &other.related_ideas {
                    if !merged_theme.related_ideas.contains(id) {
                        merged_theme.related_ideas.push(*id);
                    }
                }
                merged_theme.frequency = merged_theme.related_ideas.len();
                used_indices.insert(j);
            }
        }

        result.push(merged_theme);
    }

    result
}

/// True if `needle` occurs in `haystack` on word boundaries, so "machine
/// learning" contains "learning" but "training" does not contain "rain".
fn contains_words(haystack: &str, needle: &str) -> bool {
    format!(" {} ", haystack).contains(&format!(" {} ", needle))
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
    let conversation_history = format!(
        "Round 1\nClaude: {}\nGemini: {}",
        claude_thoughts, synthesis
    );

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
    fn test_contains_words_respects_word_boundaries() {
        assert!(contains_words("machine learning", "learning"));
        assert!(contains_words(
            "machine learning models",
            "machine learning"
        ));
        assert!(contains_words("learning", "learning"));
        assert!(!contains_words("training", "rain"));
        assert!(!contains_words("restraint systems", "train"));
    }

    #[test]
    fn test_cluster_does_not_merge_substrings_of_words() {
        let themes = vec![
            ConsensusTheme {
                theme: "training".to_string(),
                frequency: 2,
                related_ideas: vec![1, 2],
            },
            ConsensusTheme {
                theme: "rain".to_string(),
                frequency: 2,
                related_ideas: vec![3, 4],
            },
        ];
        let clustered = cluster_similar_themes(themes);
        assert_eq!(clustered.len(), 2);
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

        // "learning" is in 3 of 4 ideas and scores highest, so the first theme
        // mentioning it absorbs every phrase containing the word and covers
        // exactly ideas 1-3 (idea 4 never mentions it).
        let learning_theme = themes
            .iter()
            .find(|t| contains_words(&t.theme, "learning"))
            .expect("Expected a theme containing 'learning'");
        let mut ids = learning_theme.related_ideas.clone();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(learning_theme.frequency, 3);
        assert_eq!(
            themes
                .iter()
                .filter(|t| contains_words(&t.theme, "learning"))
                .count(),
            1,
            "all 'learning' phrases are clustered into one theme"
        );
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
