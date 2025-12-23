use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Shared JSON response wrapper with metadata
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolResponse<T> {
    pub result: T,
    pub metadata: ResponseMetadata,
}

/// Response metadata including model info and token usage
#[derive(Debug, Serialize, JsonSchema)]
pub struct ResponseMetadata {
    pub model_used: String,
    pub prompt_tokens: u32,
    pub response_tokens: u32,
    pub total_tokens: u32,
}

impl ResponseMetadata {
    pub fn new(model: &str) -> Self {
        Self {
            model_used: model.to_string(),
            prompt_tokens: 0,
            response_tokens: 0,
            total_tokens: 0,
        }
    }

    pub fn with_usage(model: &str, usage: &crate::gemini::types::UsageMetadata) -> Self {
        Self {
            model_used: model.to_string(),
            prompt_tokens: usage.prompt_token_count,
            response_tokens: usage.candidates_token_count,
            total_tokens: usage.total_token_count,
        }
    }
}

/// Model preference for tool requests
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModelPreference {
    Pro,
    Flash,
}

impl Default for ModelPreference {
    fn default() -> Self {
        ModelPreference::Pro
    }
}

/// Generation parameters for customizing model behavior
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GenerationParams {
    #[schemars(description = "Temperature for generation (0.0-2.0)")]
    pub temperature: Option<f32>,

    #[schemars(description = "Maximum tokens in response")]
    pub max_tokens: Option<u32>,

    #[schemars(description = "Top-p sampling parameter")]
    pub top_p: Option<f32>,

    #[schemars(description = "Top-k sampling parameter")]
    pub top_k: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_creation() {
        let meta = ResponseMetadata::new("gemini-pro");
        assert_eq!(meta.model_used, "gemini-pro");
        assert_eq!(meta.total_tokens, 0);
    }

    #[test]
    fn test_model_preference_default() {
        let pref = ModelPreference::default();
        matches!(pref, ModelPreference::Pro);
    }

    #[test]
    fn test_model_preference_deserialize() {
        let json = r#""pro""#;
        let pref: ModelPreference = serde_json::from_str(json).unwrap();
        matches!(pref, ModelPreference::Pro);

        let json2 = r#""flash""#;
        let pref2: ModelPreference = serde_json::from_str(json2).unwrap();
        matches!(pref2, ModelPreference::Flash);
    }

    #[test]
    fn test_generation_params_deserialize() {
        let json = r#"{"temperature": 0.7, "max_tokens": 1024}"#;
        let params: GenerationParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.temperature, Some(0.7));
        assert_eq!(params.max_tokens, Some(1024));
        assert_eq!(params.top_p, None);
        assert_eq!(params.top_k, None);
    }

    #[test]
    fn test_tool_response_serialize() {
        let response = ToolResponse {
            result: "test result".to_string(),
            metadata: ResponseMetadata::new("gemini-flash"),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("test result"));
        assert!(json.contains("gemini-flash"));
    }
}
