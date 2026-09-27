use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GeminiError {
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),

    /// Non-2xx response from the Gemini API.
    ///
    /// `retry_after` carries the server's retry hint, taken from the
    /// `Retry-After` header or a `google.rpc.RetryInfo` detail in the body.
    #[error("API error ({status}): {message}")]
    ApiError {
        status: u16,
        message: String,
        retry_after: Option<Duration>,
    },

    #[error("JSON parsing error: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("Empty response from API")]
    EmptyResponse,
}

pub type Result<T> = std::result::Result<T, GeminiError>;
