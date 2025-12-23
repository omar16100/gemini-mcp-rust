use thiserror::Error;

#[derive(Error, Debug)]
pub enum GeminiError {
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    ApiError { status: u16, message: String },

    #[error("JSON parsing error: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("Empty response from API")]
    EmptyResponse,

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

pub type Result<T> = std::result::Result<T, GeminiError>;
