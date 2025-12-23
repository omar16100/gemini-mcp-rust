use reqwest::{Client, StatusCode};
use std::time::Duration;
use tracing::{debug, info};

use crate::error::{GeminiError, Result};
use crate::gemini::{models::GeminiModel, types::*};

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiClient {
    http_client: Client,
    api_key: String,
    pro_model: String,
    flash_model: String,
}

impl GeminiClient {
    pub fn new(api_key: String) -> Result<Self> {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .build()
            .map_err(GeminiError::HttpClient)?;

        let pro_model = std::env::var("GEMINI_PRO_MODEL")
            .unwrap_or_else(|_| GeminiModel::Pro.as_str().to_string());
        let flash_model = std::env::var("GEMINI_FLASH_MODEL")
            .unwrap_or_else(|_| GeminiModel::Flash.as_str().to_string());

        info!("Gemini client initialized");
        debug!("Pro model: {}", pro_model);
        debug!("Flash model: {}", flash_model);

        Ok(Self {
            http_client,
            api_key,
            pro_model,
            flash_model,
        })
    }

    pub async fn generate_content(
        &self,
        prompt: &str,
        model: GeminiModel,
        config: Option<GenerationConfig>,
    ) -> Result<GenerationResponse> {
        let model_name = match model {
            GeminiModel::Pro => &self.pro_model,
            GeminiModel::Flash => &self.flash_model,
        };

        let request = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![Part::Text {
                    text: prompt.to_string(),
                }],
            }],
            generation_config: config,
            safety_settings: None,
        };

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            BASE_URL, model_name, self.api_key
        );

        debug!("Sending request to {}", model_name);

        let response = self
            .http_client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(GeminiError::HttpClient)?;

        match response.status() {
            StatusCode::OK => {
                let resp: GenerateContentResponse = response.json().await?;

                let usage = resp.usage_metadata.clone().unwrap_or_default();

                debug!(
                    "Tokens - prompt: {}, response: {}, total: {}",
                    usage.prompt_token_count,
                    usage.candidates_token_count,
                    usage.total_token_count
                );

                let text = resp.candidates
                    .first()
                    .and_then(|c| c.content.parts.first())
                    .and_then(|p| match p {
                        Part::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .ok_or(GeminiError::EmptyResponse)?;

                Ok(GenerationResponse { text, usage })
            }
            status => {
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                Err(GeminiError::ApiError {
                    status: status.as_u16(),
                    message: error_body,
                })
            }
        }
    }

    pub async fn generate_with_history(
        &self,
        messages: Vec<(String, String)>, // (role, content)
        model: GeminiModel,
        config: Option<GenerationConfig>,
    ) -> Result<String> {
        let model_name = match model {
            GeminiModel::Pro => &self.pro_model,
            GeminiModel::Flash => &self.flash_model,
        };

        let contents: Vec<Content> = messages
            .into_iter()
            .map(|(role, text)| Content {
                role,
                parts: vec![Part::Text { text }],
            })
            .collect();

        let request = GenerateContentRequest {
            contents,
            generation_config: config,
            safety_settings: None,
        };

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            BASE_URL, model_name, self.api_key
        );

        let response = self
            .http_client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(GeminiError::HttpClient)?;

        match response.status() {
            StatusCode::OK => {
                let resp: GenerateContentResponse = response.json().await?;

                resp.candidates
                    .first()
                    .and_then(|c| c.content.parts.first())
                    .and_then(|p| match p {
                        Part::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .ok_or(GeminiError::EmptyResponse)
            }
            status => {
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                Err(GeminiError::ApiError {
                    status: status.as_u16(),
                    message: error_body,
                })
            }
        }
    }

    pub async fn test_connection(&self) -> Result<()> {
        info!("Testing connection to Gemini API...");
        self.generate_content("Test", GeminiModel::Pro, None)
            .await?;
        info!("Connection test successful");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = GeminiClient::new("test_key".to_string());
        assert!(client.is_ok());
    }
}
