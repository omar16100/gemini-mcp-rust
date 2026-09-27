use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::gemini::types::{GenerationConfig, UsageMetadata};

/// Shared JSON response wrapper with metadata
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolResponse<T> {
    pub result: T,
    pub metadata: ResponseMetadata,
}

/// Response metadata including model info and token usage
#[derive(Debug, Serialize, JsonSchema)]
pub struct ResponseMetadata {
    /// Model ID that produced the result (for cached results, the model that
    /// produced the original response).
    pub model_used: String,
    pub prompt_tokens: u32,
    pub response_tokens: u32,
    pub total_tokens: u32,
    /// True when the result was served from the in-memory cache. Token counts
    /// are 0 in that case because no API call was made.
    pub cached: bool,
}

impl ResponseMetadata {
    pub fn with_usage(model: &str, usage: &UsageMetadata) -> Self {
        Self {
            model_used: model.to_string(),
            prompt_tokens: usage.prompt_token_count,
            response_tokens: usage.candidates_token_count,
            total_tokens: usage.total_token_count,
            cached: false,
        }
    }

    /// Metadata for a cache hit: original model, zero tokens consumed.
    pub fn from_cache(model: &str) -> Self {
        Self {
            model_used: model.to_string(),
            prompt_tokens: 0,
            response_tokens: 0,
            total_tokens: 0,
            cached: true,
        }
    }
}

/// Model preference for tool requests
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModelPreference {
    #[default]
    Pro,
    Flash,
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

impl GenerationParams {
    /// Build a request config from optional caller params, falling back to
    /// the tool's defaults for temperature and max output tokens.
    pub fn to_config(
        params: Option<&GenerationParams>,
        default_temperature: Option<f32>,
        default_max_tokens: Option<u32>,
    ) -> GenerationConfig {
        GenerationConfig {
            temperature: params.and_then(|p| p.temperature).or(default_temperature),
            max_output_tokens: params.and_then(|p| p.max_tokens).or(default_max_tokens),
            top_p: params.and_then(|p| p.top_p),
            top_k: params.and_then(|p| p.top_k),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_with_usage() {
        let usage = UsageMetadata {
            prompt_token_count: 10,
            candidates_token_count: 5,
            total_token_count: 15,
        };
        let meta = ResponseMetadata::with_usage("gemini-x", &usage);
        assert_eq!(meta.model_used, "gemini-x");
        assert_eq!(meta.prompt_tokens, 10);
        assert_eq!(meta.response_tokens, 5);
        assert_eq!(meta.total_tokens, 15);
        assert!(!meta.cached);
    }

    #[test]
    fn test_metadata_from_cache() {
        let meta = ResponseMetadata::from_cache("gemini-x");
        assert_eq!(meta.model_used, "gemini-x");
        assert_eq!(meta.total_tokens, 0);
        assert!(meta.cached);
    }

    #[test]
    fn test_model_preference_default() {
        assert!(matches!(ModelPreference::default(), ModelPreference::Pro));
    }

    #[test]
    fn test_model_preference_deserialize() {
        let pref: ModelPreference = serde_json::from_str(r#""pro""#).unwrap();
        assert!(matches!(pref, ModelPreference::Pro));

        let pref2: ModelPreference = serde_json::from_str(r#""flash""#).unwrap();
        assert!(matches!(pref2, ModelPreference::Flash));
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
    fn test_generation_params_to_config() {
        let defaults = GenerationParams::to_config(None, Some(0.3), Some(2048));
        assert_eq!(defaults.temperature, Some(0.3));
        assert_eq!(defaults.max_output_tokens, Some(2048));
        assert_eq!(defaults.top_p, None);

        let params = GenerationParams {
            temperature: Some(0.9),
            max_tokens: None,
            top_p: Some(0.8),
            top_k: Some(20),
        };
        let config = GenerationParams::to_config(Some(&params), Some(0.3), Some(2048));
        assert_eq!(config.temperature, Some(0.9));
        assert_eq!(config.max_output_tokens, Some(2048));
        assert_eq!(config.top_p, Some(0.8));
        assert_eq!(config.top_k, Some(20));
    }

    #[test]
    fn test_tool_response_serialize() {
        let response = ToolResponse {
            result: "test result".to_string(),
            metadata: ResponseMetadata::from_cache("gemini-flash"),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["result"], "test result");
        assert_eq!(json["metadata"]["model_used"], "gemini-flash");
        assert_eq!(json["metadata"]["cached"], true);
    }
}
