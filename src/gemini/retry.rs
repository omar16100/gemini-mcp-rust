//! Retry with exponential backoff and jitter for Gemini API calls.
//!
//! Policy (see docs/c4model.md, "Retry"):
//! - Retried: HTTP 408, 429, 500, 502, 503, 504, and transport errors where
//!   the connection could not be established (the request never reached the
//!   server).
//! - Not retried: other 4xx/5xx, client-side timeouts after the request was
//!   sent, body decode errors, empty responses. `generateContent` has no side
//!   effects beyond billing, but a timed-out request may still have been
//!   processed and billed, so it is not replayed.
//! - Delay: `initial_delay * multiplier^n` with +/-25% jitter, capped at
//!   `max_delay`. A server hint (`Retry-After` header or
//!   `google.rpc.RetryInfo.retryDelay` in the error body) is used as a floor;
//!   if the hint exceeds `max_delay` the error is returned immediately instead
//!   of retrying too early.

use std::time::{Duration, SystemTime};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::error::GeminiError;

/// Default number of retries after the first attempt.
pub const DEFAULT_MAX_RETRIES: u32 = 3;
/// Upper bound accepted from `GEMINI_MAX_RETRIES`.
pub const MAX_RETRIES_LIMIT: u32 = 10;
/// Environment variable that overrides the retry count (0 disables retries).
pub const MAX_RETRIES_ENV: &str = "GEMINI_MAX_RETRIES";

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries after the first attempt (default: 3)
    pub max_retries: u32,
    /// Base delay before the first retry (default: 1 s)
    pub initial_delay: Duration,
    /// Upper bound for any single delay, including server hints (default: 30 s)
    pub max_delay: Duration,
    /// Multiplier for exponential backoff (default: 2.0)
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Default config with `max_retries` taken from `GEMINI_MAX_RETRIES`.
    pub fn from_env() -> Self {
        let raw = std::env::var(MAX_RETRIES_ENV).ok();
        let config = Self {
            max_retries: parse_max_retries(raw.as_deref()),
            ..Self::default()
        };
        info!(
            "Retry policy: max_retries={}, initial_delay={:?}, max_delay={:?}",
            config.max_retries, config.initial_delay, config.max_delay
        );
        config
    }
}

/// Parse `GEMINI_MAX_RETRIES`; invalid or out-of-range values fall back to the default.
pub fn parse_max_retries(raw: Option<&str>) -> u32 {
    let Some(raw) = raw else {
        return DEFAULT_MAX_RETRIES;
    };
    match raw.trim().parse::<u32>() {
        Ok(n) if n <= MAX_RETRIES_LIMIT => n,
        _ => {
            warn!(
                "Ignoring invalid {}={:?} (expected 0-{}), using {}",
                MAX_RETRIES_ENV, raw, MAX_RETRIES_LIMIT, DEFAULT_MAX_RETRIES
            );
            DEFAULT_MAX_RETRIES
        }
    }
}

/// Execute an async operation with retry logic and exponential backoff.
///
/// Returns the first success, the first non-retryable error, or the last
/// error once `config.max_retries` retries are used up.
pub async fn retry_with_backoff<F, Fut, T>(
    mut operation: F,
    config: &RetryConfig,
) -> Result<T, GeminiError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, GeminiError>>,
{
    let mut attempt: u32 = 0;

    loop {
        let err = match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    info!("Gemini request succeeded after {} retries", attempt);
                }
                return Ok(result);
            }
            Err(err) => err,
        };

        if !is_retryable(&err) {
            debug!("Non-retryable error: {}", err);
            return Err(err);
        }

        if attempt >= config.max_retries {
            warn!(
                "Giving up after {} retries, last error: {}",
                config.max_retries, err
            );
            return Err(err);
        }

        let backoff = jittered_backoff(config, attempt, rand::random::<f64>());
        let delay = match retry_hint(&err) {
            Some(hint) if hint > config.max_delay => {
                warn!(
                    "Server asked to retry after {:?}, above max_delay {:?}; not retrying: {}",
                    hint, config.max_delay, err
                );
                return Err(err);
            }
            Some(hint) => hint.max(backoff),
            None => backoff,
        };

        attempt += 1;
        warn!(
            "Retry {}/{} in {} ms after error: {}",
            attempt,
            config.max_retries,
            delay.as_millis(),
            err
        );
        sleep(delay).await;
    }
}

/// Backoff for 0-based retry `attempt`: `initial * multiplier^attempt`, scaled
/// by a jitter factor in [0.75, 1.25) derived from `unit_random` in [0, 1),
/// then capped at `max_delay`.
fn jittered_backoff(config: &RetryConfig, attempt: u32, unit_random: f64) -> Duration {
    let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
    let base_ms =
        config.initial_delay.as_millis() as f64 * config.backoff_multiplier.powi(exponent);
    let jitter = (unit_random.clamp(0.0, 1.0) - 0.5) * 0.5;
    let max_ms = config.max_delay.as_millis() as f64;
    let ms = (base_ms * (1.0 + jitter)).clamp(0.0, max_ms);
    Duration::from_millis(ms as u64)
}

fn retry_hint(error: &GeminiError) -> Option<Duration> {
    match error {
        GeminiError::ApiError { retry_after, .. } => *retry_after,
        _ => None,
    }
}

/// Determine if an error is retryable
pub fn is_retryable(error: &GeminiError) -> bool {
    match error {
        // Only connection failures: the request never reached the server, so
        // replaying it cannot duplicate work. Timeouts after sending are not
        // retried (the generation may have completed and been billed).
        GeminiError::HttpClient(req_err) => req_err.is_connect(),
        GeminiError::ApiError { status, .. } => {
            matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
        }
        GeminiError::JsonParse(_) | GeminiError::EmptyResponse => false,
    }
}

/// Parse a `Retry-After` header value (delta-seconds or HTTP-date).
pub fn parse_retry_after_header(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = httpdate::parse_http_date(value).ok()?;
    Some(when.duration_since(now).unwrap_or(Duration::ZERO))
}

/// Extract `retryDelay` from a `google.rpc.RetryInfo` detail in a Google API
/// error body, e.g. `{"error": {"details": [{"@type":
/// "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "37s"}]}}`.
pub fn parse_retry_info_delay(body: &str) -> Option<Duration> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let details = json.get("error")?.get("details")?.as_array()?;
    details.iter().find_map(|detail| {
        let type_url = detail.get("@type")?.as_str()?;
        if !type_url.ends_with("google.rpc.RetryInfo") {
            return None;
        }
        parse_proto_duration(detail.get("retryDelay")?.as_str()?)
    })
}

/// Parse a protobuf JSON Duration such as `"37s"` or `"0.25s"`.
fn parse_proto_duration(value: &str) -> Option<Duration> {
    let secs: f64 = value.trim().strip_suffix('s')?.parse().ok()?;
    Duration::try_from_secs_f64(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    fn api_error(status: u16) -> GeminiError {
        GeminiError::ApiError {
            status,
            message: format!("status {status}"),
            retry_after: None,
        }
    }

    fn fast_config(max_retries: u32) -> RetryConfig {
        RetryConfig {
            max_retries,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(200),
            backoff_multiplier: 2.0,
        }
    }

    /// Runs `retry_with_backoff` over a scripted list of results and returns
    /// (final result, number of calls).
    async fn run_script(
        script: Vec<Result<i32, GeminiError>>,
        config: &RetryConfig,
    ) -> (Result<i32, GeminiError>, u32) {
        let calls = Arc::new(AtomicU32::new(0));
        let script = Arc::new(std::sync::Mutex::new(script.into_iter()));
        let counter = calls.clone();
        let result = retry_with_backoff(
            move || {
                let counter = counter.clone();
                let script = script.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    script
                        .lock()
                        .unwrap()
                        .next()
                        .expect("script exhausted: too many attempts")
                }
            },
            config,
        )
        .await;
        (result, calls.load(Ordering::SeqCst))
    }

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay, Duration::from_secs(1));
        assert_eq!(config.max_delay, Duration::from_secs(30));
        assert_eq!(config.backoff_multiplier, 2.0);
    }

    #[test]
    fn test_parse_max_retries() {
        assert_eq!(parse_max_retries(None), DEFAULT_MAX_RETRIES);
        assert_eq!(parse_max_retries(Some("0")), 0);
        assert_eq!(parse_max_retries(Some(" 5 ")), 5);
        assert_eq!(parse_max_retries(Some("10")), 10);
        assert_eq!(parse_max_retries(Some("11")), DEFAULT_MAX_RETRIES);
        assert_eq!(parse_max_retries(Some("-1")), DEFAULT_MAX_RETRIES);
        assert_eq!(parse_max_retries(Some("lots")), DEFAULT_MAX_RETRIES);
    }

    #[test]
    fn test_retryable_statuses() {
        for status in [408, 429, 500, 502, 503, 504] {
            assert!(is_retryable(&api_error(status)), "{status} should retry");
        }
        for status in [400, 401, 403, 404, 409, 422, 501, 505] {
            assert!(
                !is_retryable(&api_error(status)),
                "{status} should not retry"
            );
        }
    }

    #[test]
    fn test_is_not_retryable_empty_response() {
        assert!(!is_retryable(&GeminiError::EmptyResponse));
    }

    #[tokio::test]
    async fn test_connect_error_is_retryable() {
        // Bind then drop a listener to get a local port with nothing on it.
        // (A bound but non-listening socket is not usable here: on macOS the
        // connect times out instead of being refused.) Another process would
        // have to claim this exact ephemeral port in the meantime to break
        // the test.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap_err();
        assert!(err.is_connect(), "unexpected error: {err:?}");
        assert!(is_retryable(&GeminiError::HttpClient(err)));
    }

    #[test]
    fn test_jittered_backoff_bounds() {
        let config = RetryConfig::default();
        // attempt 0: 1000 ms scaled into [750, 1250)
        assert_eq!(
            jittered_backoff(&config, 0, 0.0),
            Duration::from_millis(750)
        );
        assert_eq!(
            jittered_backoff(&config, 0, 0.5),
            Duration::from_millis(1000)
        );
        assert!(jittered_backoff(&config, 0, 0.999) < Duration::from_millis(1250));
        // attempt 2: 4000 ms base
        assert_eq!(
            jittered_backoff(&config, 2, 0.5),
            Duration::from_millis(4000)
        );
        // Large attempts are capped at max_delay.
        assert_eq!(jittered_backoff(&config, 20, 0.5), config.max_delay);
        assert_eq!(jittered_backoff(&config, u32::MAX, 0.9), config.max_delay);
    }

    #[test]
    fn test_parse_retry_after_header_seconds() {
        let now = SystemTime::now();
        assert_eq!(
            parse_retry_after_header("120", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after_header(" 0 ", now), Some(Duration::ZERO));
        assert_eq!(parse_retry_after_header("soon", now), None);
        assert_eq!(parse_retry_after_header("-5", now), None);
    }

    #[test]
    fn test_parse_retry_after_header_http_date() {
        let now = httpdate::parse_http_date("Sun, 27 Sep 2026 10:00:00 GMT").unwrap();
        assert_eq!(
            parse_retry_after_header("Sun, 27 Sep 2026 10:00:45 GMT", now),
            Some(Duration::from_secs(45))
        );
        // A date in the past means "retry now".
        assert_eq!(
            parse_retry_after_header("Sun, 27 Sep 2026 09:00:00 GMT", now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn test_parse_retry_info_delay() {
        let body = r#"{"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "details": [
            {"@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": []},
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "37s"}
        ]}}"#;
        assert_eq!(parse_retry_info_delay(body), Some(Duration::from_secs(37)));

        let fractional = r#"{"error": {"details": [
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "0.25s"}
        ]}}"#;
        assert_eq!(
            parse_retry_info_delay(fractional),
            Some(Duration::from_millis(250))
        );

        assert_eq!(parse_retry_info_delay(r#"{"error": {"code": 429}}"#), None);
        assert_eq!(parse_retry_info_delay("not json"), None);
        let negative = r#"{"error": {"details": [
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "-1s"}
        ]}}"#;
        assert_eq!(parse_retry_info_delay(negative), None);
    }

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let (result, calls) = run_script(vec![Ok(42)], &fast_config(3)).await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_retries() {
        let script = vec![Err(api_error(503)), Err(api_error(429)), Ok(42)];
        let (result, calls) = run_script(script, &fast_config(3)).await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn test_retry_exhaust_attempts() {
        let script = vec![
            Err(api_error(503)),
            Err(api_error(503)),
            Err(api_error(503)),
        ];
        let (result, calls) = run_script(script, &fast_config(2)).await;
        assert!(matches!(
            result,
            Err(GeminiError::ApiError { status: 503, .. })
        ));
        assert_eq!(calls, 3); // initial + 2 retries
    }

    #[tokio::test]
    async fn test_zero_retries_disables_retry() {
        let (result, calls) = run_script(vec![Err(api_error(503))], &fast_config(0)).await;
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn test_retry_non_retryable_error() {
        let (result, calls) = run_script(vec![Err(api_error(400))], &fast_config(3)).await;
        assert!(matches!(
            result,
            Err(GeminiError::ApiError { status: 400, .. })
        ));
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn test_retry_waits_for_server_hint() {
        let hinted = GeminiError::ApiError {
            status: 429,
            message: "slow down".to_string(),
            retry_after: Some(Duration::from_millis(80)),
        };
        let started = Instant::now();
        let (result, calls) = run_script(vec![Err(hinted), Ok(7)], &fast_config(3)).await;
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls, 2);
        assert!(started.elapsed() >= Duration::from_millis(80));
    }

    #[tokio::test]
    async fn test_hint_above_max_delay_is_not_retried() {
        let hinted = GeminiError::ApiError {
            status: 429,
            message: "daily quota".to_string(),
            retry_after: Some(Duration::from_secs(3600)),
        };
        // The script holds one result, so a second attempt would panic.
        let (result, calls) = run_script(vec![Err(hinted)], &fast_config(3)).await;
        assert!(matches!(
            result,
            Err(GeminiError::ApiError { status: 429, .. })
        ));
        assert_eq!(calls, 1);
    }
}
