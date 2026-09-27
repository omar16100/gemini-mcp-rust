use reqwest::{header::RETRY_AFTER, Client, Response};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

use crate::error::{GeminiError, Result};
use crate::gemini::{
    models::GeminiModel,
    retry::{self, retry_with_backoff, RetryConfig},
    types::*,
};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
/// The API key goes in a header, not the `?key=` query string, so it never
/// appears in URLs that reqwest includes in error messages and logs.
const API_KEY_HEADER: &str = "x-goog-api-key";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub struct GeminiClient {
    http_client: Client,
    api_key: String,
    base_url: String,
    pro_model: String,
    flash_model: String,
    retry_config: RetryConfig,
}

impl GeminiClient {
    pub fn new(api_key: String) -> Result<Self> {
        let pro_model = std::env::var("GEMINI_PRO_MODEL")
            .unwrap_or_else(|_| GeminiModel::Pro.as_str().to_string());
        let flash_model = std::env::var("GEMINI_FLASH_MODEL")
            .unwrap_or_else(|_| GeminiModel::Flash.as_str().to_string());

        Self::with_config(
            api_key,
            DEFAULT_BASE_URL.to_string(),
            pro_model,
            flash_model,
            RetryConfig::from_env(),
        )
    }

    /// Fully explicit constructor (used by `new` and by tests against a local
    /// mock server).
    pub fn with_config(
        api_key: String,
        base_url: String,
        pro_model: String,
        flash_model: String,
        retry_config: RetryConfig,
    ) -> Result<Self> {
        let http_client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // Google documents these endpoints as direct HTTPS calls, so a
            // redirect is unexpected and is returned as an error. Following
            // one would forward the x-goog-api-key header (reqwest only
            // strips standard auth headers across hosts) and could replay a
            // POST.
            .redirect(reqwest::redirect::Policy::none())
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .build()
            .map_err(GeminiError::HttpClient)?;

        info!(
            "Gemini client initialized: pro_model={}, flash_model={}, max_retries={}",
            pro_model, flash_model, retry_config.max_retries
        );

        Ok(Self {
            http_client,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            pro_model,
            flash_model,
            retry_config,
        })
    }

    /// Model ID actually sent to the API for `model`, including any
    /// `GEMINI_PRO_MODEL` / `GEMINI_FLASH_MODEL` override.
    pub fn model_name(&self, model: GeminiModel) -> &str {
        match model {
            GeminiModel::Pro => &self.pro_model,
            GeminiModel::Flash => &self.flash_model,
        }
    }

    pub async fn generate_content(
        &self,
        prompt: &str,
        model: GeminiModel,
        config: Option<GenerationConfig>,
    ) -> Result<GenerationResponse> {
        let model_name = self.model_name(model);

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

        let url = format!("{}/models/{}:generateContent", self.base_url, model_name);

        debug!(
            "Sending generateContent to {} (prompt_len={})",
            model_name,
            prompt.len()
        );

        retry_with_backoff(|| self.send_generate(&url, &request), &self.retry_config).await
    }

    /// One HTTP attempt of `generateContent`.
    async fn send_generate(
        &self,
        url: &str,
        request: &GenerateContentRequest,
    ) -> Result<GenerationResponse> {
        let response = self
            .http_client
            .post(url)
            .header(API_KEY_HEADER, &self.api_key)
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(api_error(response, SystemTime::now()).await);
        }

        let resp: GenerateContentResponse = response.json().await?;
        parse_generation(resp)
    }

    /// Verify the API key and the configured Pro model with a `models.get`
    /// call, which generates no content (the old check sent a real prompt).
    pub async fn test_connection(&self) -> Result<()> {
        let model_name = self.model_name(GeminiModel::Pro);
        info!(
            "Testing connection to Gemini API (models.get {})",
            model_name
        );
        let url = format!("{}/models/{}", self.base_url, model_name);

        retry_with_backoff(
            || async {
                let response = self
                    .http_client
                    .get(&url)
                    .header(API_KEY_HEADER, &self.api_key)
                    .send()
                    .await?;
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(api_error(response, SystemTime::now()).await)
                }
            },
            &self.retry_config,
        )
        .await?;

        info!("Connection test successful");
        Ok(())
    }
}

/// Build an `ApiError` from a non-success response, keeping any retry hint.
///
/// The hint is the larger of the `Retry-After` header and a `RetryInfo`
/// `retryDelay` in the body. If the body cannot be read (for example it
/// stalls until the request timeout or the connection drops), the transport
/// error is returned instead, which the retry policy does not replay.
async fn api_error(response: Response, now: SystemTime) -> GeminiError {
    let status = response.status().as_u16();
    let header_hint = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| retry::parse_retry_after_header(value, now));
    let message = match response.text().await {
        Ok(message) => message,
        Err(err) => {
            warn!(
                "Gemini API returned status {} but the error body could not be read: {}",
                status, err
            );
            return GeminiError::HttpClient(err);
        }
    };
    let body_hint = retry::parse_retry_info_delay(&message);
    let retry_after = match (header_hint, body_hint) {
        (Some(header), Some(body)) => Some(header.max(body)),
        (header, body) => header.or(body),
    };

    debug!(
        "Gemini API error: status={}, retry_after={:?}",
        status, retry_after
    );

    GeminiError::ApiError {
        status,
        message,
        retry_after,
    }
}

/// Concatenate the text parts of the first candidate.
fn parse_generation(resp: GenerateContentResponse) -> Result<GenerationResponse> {
    let usage = resp.usage_metadata.unwrap_or_default();
    debug!(
        "Tokens - prompt: {}, response: {}, total: {}",
        usage.prompt_token_count, usage.candidates_token_count, usage.total_token_count
    );

    let candidate = resp
        .candidates
        .into_iter()
        .next()
        .ok_or(GeminiError::EmptyResponse)?;
    debug!("Finish reason: {:?}", candidate.finish_reason);

    let text: String = candidate
        .content
        .map(|content| content.parts)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text),
            _ => None,
        })
        .collect();

    if text.trim().is_empty() {
        return Err(GeminiError::EmptyResponse);
    }

    Ok(GenerationResponse { text, usage })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;

    const OK_BODY: &str = r#"{
        "candidates": [{
            "content": {"role": "model", "parts": [{"text": "Hello"}, {"text": " world"}]},
            "finishReason": "STOP"
        }],
        "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 2, "totalTokenCount": 7}
    }"#;

    fn test_client(base_url: String) -> GeminiClient {
        GeminiClient::with_config(
            "test-key".to_string(),
            base_url,
            "pro-model-x".to_string(),
            "flash-model-y".to_string(),
            RetryConfig {
                max_retries: 2,
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(500),
                backoff_multiplier: 2.0,
            },
        )
        .unwrap()
    }

    #[test]
    fn test_client_creation() {
        let client = GeminiClient::new("test_key".to_string());
        assert!(client.is_ok());
    }

    #[test]
    fn test_model_name_uses_configured_ids() {
        let client = test_client("http://localhost".to_string());
        assert_eq!(client.model_name(GeminiModel::Pro), "pro-model-x");
        assert_eq!(client.model_name(GeminiModel::Flash), "flash-model-y");
    }

    #[tokio::test]
    async fn test_generate_content_sends_key_header_and_parses_usage() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/models/flash-model-y:generateContent")
            .match_header(API_KEY_HEADER, "test-key")
            .match_query(Matcher::Missing)
            .match_body(Matcher::PartialJson(serde_json::json!({
                "generationConfig": {"maxOutputTokens": 32}
            })))
            .with_status(200)
            .with_body(OK_BODY)
            .expect(1)
            .create_async()
            .await;

        let client = test_client(server.url());
        let config = GenerationConfig {
            max_output_tokens: Some(32),
            ..Default::default()
        };
        let response = client
            .generate_content("hi", GeminiModel::Flash, Some(config))
            .await
            .unwrap();

        assert_eq!(response.text, "Hello world");
        assert_eq!(response.usage.prompt_token_count, 5);
        assert_eq!(response.usage.candidates_token_count, 2);
        assert_eq!(response.usage.total_token_count, 7);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_generate_content_retries_429_with_retry_after() {
        let mut server = mockito::Server::new_async().await;
        let path = "/models/pro-model-x:generateContent";
        let limited = server
            .mock("POST", path)
            .with_status(429)
            .with_header("retry-after", "0")
            .with_body(r#"{"error": {"code": 429, "status": "RESOURCE_EXHAUSTED"}}"#)
            .expect(1)
            .create_async()
            .await;
        // mockito serves the first-created matching mock that still has
        // expected hits left, so the 429 is returned once, then the 200.
        let ok = server
            .mock("POST", path)
            .with_status(200)
            .with_body(OK_BODY)
            .expect(1)
            .create_async()
            .await;

        let client = test_client(server.url());
        let response = client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap();

        assert_eq!(response.text, "Hello world");
        limited.assert_async().await;
        ok.assert_async().await;
    }

    #[tokio::test]
    async fn test_generate_content_does_not_retry_400() {
        let mut server = mockito::Server::new_async().await;
        let bad = server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(400)
            .with_body(r#"{"error": {"code": 400, "message": "bad"}}"#)
            .expect(1)
            .create_async()
            .await;

        let client = test_client(server.url());
        let err = client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap_err();

        assert!(matches!(err, GeminiError::ApiError { status: 400, .. }));
        bad.assert_async().await;
    }

    #[tokio::test]
    async fn test_long_retry_info_hint_is_not_retried() {
        let mut server = mockito::Server::new_async().await;
        let limited = server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(429)
            .with_body(
                r#"{"error": {"code": 429, "details": [
                    {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "40s"}
                ]}}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let client = test_client(server.url());
        let err = client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap_err();

        match err {
            GeminiError::ApiError {
                status,
                retry_after,
                ..
            } => {
                assert_eq!(status, 429);
                assert_eq!(retry_after, Some(Duration::from_secs(40)));
            }
            other => panic!("unexpected error: {other:?}"),
        }
        limited.assert_async().await;
    }

    #[tokio::test]
    async fn test_empty_candidate_is_empty_response() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(200)
            .with_body(r#"{"candidates": [{"finishReason": "SAFETY"}]}"#)
            .create_async()
            .await;

        let client = test_client(server.url());
        let err = client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap_err();
        assert!(matches!(err, GeminiError::EmptyResponse));
    }

    #[tokio::test]
    async fn test_redirects_are_not_followed() {
        let mut server = mockito::Server::new_async().await;
        let redirect = server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(307)
            .with_header("location", "/elsewhere")
            .expect(1)
            .create_async()
            .await;
        let elsewhere = server
            .mock("POST", "/elsewhere")
            .with_status(200)
            .with_body(OK_BODY)
            .expect(0)
            .create_async()
            .await;

        let client = test_client(server.url());
        let err = client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap_err();

        assert!(matches!(err, GeminiError::ApiError { status: 307, .. }));
        redirect.assert_async().await;
        elsewhere.assert_async().await;
    }

    #[tokio::test]
    async fn test_unreadable_error_body_is_not_retried() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // A server that reads the whole request, sends 503 headers promising
        // a 1000-byte body, writes 8 bytes of it, then closes the connection.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicU32::new(0));
        let counter = connections.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                read_full_request(&mut socket).await;
                socket
                    .write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 1000\r\n\r\n{\"error\"",
                    )
                    .await
                    .unwrap();
                socket.shutdown().await.unwrap();
            }
        });

        async fn read_full_request(socket: &mut tokio::net::TcpStream) {
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0, "client closed before sending the full request");
                data.extend_from_slice(&buf[..n]);
                let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
                let body_len: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map(|v| v.trim().parse().unwrap())
                    .unwrap_or(0);
                if data.len() >= end + 4 + body_len {
                    return;
                }
            }
        }

        let client = test_client(format!("http://{addr}"));
        let err = client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap_err();

        // The failure must come from reading the response body (inside
        // api_error), not from sending the request.
        match &err {
            GeminiError::HttpClient(e) => {
                assert!(e.is_body() || e.is_decode(), "unexpected error: {e:?}");
                assert!(
                    !e.is_connect() && !e.is_request(),
                    "unexpected error: {e:?}"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(connections.load(Ordering::SeqCst), 1, "must not retry");
    }

    fn http_response(status: u16, retry_after: Option<&str>, body: &str) -> Response {
        let mut builder = http::Response::builder().status(status);
        if let Some(value) = retry_after {
            builder = builder.header("retry-after", value);
        }
        Response::from(builder.body(body.to_string()).unwrap())
    }

    const RETRY_INFO_3S: &str = r#"{"error": {"details": [
        {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "3s"}
    ]}}"#;

    #[tokio::test]
    async fn test_api_error_uses_larger_of_header_and_body_hints() {
        let now = SystemTime::now();
        let cases = [
            (Some("7"), RETRY_INFO_3S, Some(Duration::from_secs(7))),
            (Some("0"), RETRY_INFO_3S, Some(Duration::from_secs(3))),
            (
                Some("not-a-date"),
                RETRY_INFO_3S,
                Some(Duration::from_secs(3)),
            ),
            (Some("5"), "{}", Some(Duration::from_secs(5))),
            (None, "{}", None),
        ];
        for (header, body, expected) in cases {
            match api_error(http_response(429, header, body), now).await {
                GeminiError::ApiError {
                    status,
                    retry_after,
                    message,
                } => {
                    assert_eq!(status, 429);
                    assert_eq!(retry_after, expected, "header={header:?}");
                    assert_eq!(message, body);
                }
                other => panic!("unexpected error: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_body_retry_hint_delays_retry() {
        let mut server = mockito::Server::new_async().await;
        let path = "/models/pro-model-x:generateContent";
        let limited = server
            .mock("POST", path)
            .with_status(429)
            .with_body(
                r#"{"error": {"details": [
                    {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "0.2s"}
                ]}}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let ok = server
            .mock("POST", path)
            .with_status(200)
            .with_body(OK_BODY)
            .expect(1)
            .create_async()
            .await;

        let client = test_client(server.url());
        let started = std::time::Instant::now();
        client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap();

        assert!(started.elapsed() >= Duration::from_millis(200));
        limited.assert_async().await;
        ok.assert_async().await;
    }

    #[tokio::test]
    async fn test_whitespace_only_text_is_empty_response() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/models/pro-model-x:generateContent")
            .with_status(200)
            .with_body(r#"{"candidates": [{"content": {"parts": [{"text": "  \n"}]}}]}"#)
            .create_async()
            .await;

        let client = test_client(server.url());
        let err = client
            .generate_content("hi", GeminiModel::Pro, None)
            .await
            .unwrap_err();
        assert!(matches!(err, GeminiError::EmptyResponse));
    }

    #[tokio::test]
    async fn test_connection_retries_transient_errors() {
        let mut server = mockito::Server::new_async().await;
        let unavailable = server
            .mock("GET", "/models/pro-model-x")
            .with_status(503)
            .with_body("{}")
            .expect(1)
            .create_async()
            .await;
        let ok = server
            .mock("GET", "/models/pro-model-x")
            .with_status(200)
            .with_body(r#"{"name": "models/pro-model-x"}"#)
            .expect(1)
            .create_async()
            .await;

        let client = test_client(server.url());
        client.test_connection().await.unwrap();
        unavailable.assert_async().await;
        ok.assert_async().await;
    }

    #[tokio::test]
    async fn test_connection_uses_models_get() {
        let mut server = mockito::Server::new_async().await;
        let get = server
            .mock("GET", "/models/pro-model-x")
            .match_header(API_KEY_HEADER, "test-key")
            .with_status(200)
            .with_body(r#"{"name": "models/pro-model-x"}"#)
            .expect(1)
            .create_async()
            .await;

        let client = test_client(server.url());
        client.test_connection().await.unwrap();
        get.assert_async().await;
    }

    #[tokio::test]
    async fn test_connection_reports_unknown_model() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models/pro-model-x")
            .with_status(404)
            .with_body(r#"{"error": {"code": 404, "status": "NOT_FOUND"}}"#)
            .create_async()
            .await;

        let client = test_client(server.url());
        let err = client.test_connection().await.unwrap_err();
        assert!(matches!(err, GeminiError::ApiError { status: 404, .. }));
    }
}
