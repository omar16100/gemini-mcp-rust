//! Request and response types for the Gemini `generateContent` REST API.
//!
//! The REST API uses camelCase JSON field names (`generationConfig`,
//! `usageMetadata`, `promptTokenCount`, ...), so every type here renames its
//! fields to camelCase.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentRequest {
    pub contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<SafetySetting>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
    /// Any part type this client does not model (function calls, code
    /// execution results, ...). Kept so one unknown part does not fail the
    /// whole response.
    Other(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineData {
    pub mime_type: String,
    pub data: String, // base64
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SafetySetting {
    pub category: String,
    pub threshold: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    /// Absent when the prompt itself was blocked.
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Absent when generation stopped before producing content (for example
    /// a safety block).
    #[serde(default)]
    pub content: Option<Content>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageMetadata {
    pub prompt_token_count: u32,
    pub candidates_token_count: u32,
    pub total_token_count: u32,
}

/// Response from generate_content that tools will use
#[derive(Debug, Clone)]
pub struct GenerationResponse {
    pub text: String,
    pub usage: UsageMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serializes_camel_case() {
        let request = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![Part::Text {
                    text: "hi".to_string(),
                }],
            }],
            generation_config: Some(GenerationConfig {
                temperature: Some(0.5),
                max_output_tokens: Some(64),
                top_p: None,
                top_k: Some(4),
            }),
            safety_settings: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        let config = &json["generationConfig"];
        assert_eq!(config["maxOutputTokens"], 64);
        assert_eq!(config["topK"], 4);
        assert!(config.get("topP").is_none());
        assert!(json.get("safetySettings").is_none());
        assert_eq!(json["contents"][0]["parts"][0]["text"], "hi");
    }

    #[test]
    fn test_response_deserializes_camel_case_usage() {
        let body = r#"{
            "candidates": [{
                "content": {"role": "model", "parts": [{"text": "hello"}]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 11,
                "candidatesTokenCount": 7,
                "totalTokenCount": 18
            }
        }"#;

        let resp: GenerateContentResponse = serde_json::from_str(body).unwrap();
        let usage = resp.usage_metadata.unwrap();
        assert_eq!(usage.prompt_token_count, 11);
        assert_eq!(usage.candidates_token_count, 7);
        assert_eq!(usage.total_token_count, 18);
        assert_eq!(resp.candidates[0].finish_reason.as_deref(), Some("STOP"));
    }

    #[test]
    fn test_response_tolerates_missing_fields_and_unknown_parts() {
        // Missing candidatesTokenCount, a part type we do not model, and a
        // candidate without content must not fail deserialization.
        let body = r#"{
            "candidates": [
                {"content": {"role": "model", "parts": [
                    {"functionCall": {"name": "f", "args": {}}},
                    {"text": "ok", "thoughtSignature": "abc"}
                ]}},
                {"finishReason": "SAFETY"}
            ],
            "usageMetadata": {"promptTokenCount": 3, "totalTokenCount": 3}
        }"#;

        let resp: GenerateContentResponse = serde_json::from_str(body).unwrap();
        let usage = resp.usage_metadata.unwrap();
        assert_eq!(usage.candidates_token_count, 0);
        let parts = &resp.candidates[0].content.as_ref().unwrap().parts;
        assert!(matches!(parts[0], Part::Other(_)));
        assert!(matches!(&parts[1], Part::Text { text } if text == "ok"));
        assert!(resp.candidates[1].content.is_none());
    }

    #[test]
    fn test_response_without_candidates() {
        let body = r#"{"promptFeedback": {"blockReason": "SAFETY"}}"#;
        let resp: GenerateContentResponse = serde_json::from_str(body).unwrap();
        assert!(resp.candidates.is_empty());
        assert!(resp.usage_metadata.is_none());
    }
}
