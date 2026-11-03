//! Shared HTTP client and error type for provider connectors.

use serde_json::Value;
use std::time::Duration;

const USER_AGENT: &str = concat!("viberooms/", env!("CARGO_PKG_VERSION"));

#[derive(Debug)]
pub struct ApiError {
    /// HTTP status, or 0 for a network error.
    pub status: u16,
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ApiError {}

impl ApiError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), retry_after: None }
    }
    pub fn is_unauthorized(&self) -> bool {
        self.status == 401
    }
    /// A refused refresh means the grant is dead: the user has to sign in again.
    pub fn is_grant_dead(&self) -> bool {
        matches!(self.status, 400 | 401 | 403)
    }
}

fn parse_retry_after(h: &reqwest::header::HeaderMap) -> Option<Duration> {
    let v = h.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(s) = v.trim().parse::<u64>() {
        return Some(Duration::from_secs(s));
    }
    let when = chrono::DateTime::parse_from_rfc2822(v).ok()?;
    let ms = when.timestamp_millis() - crate::now_ms();
    Some(Duration::from_millis(ms.max(0) as u64))
}

#[derive(Clone)]
pub struct Http {
    pub inner: reqwest::Client,
}

impl Http {
    pub fn new() -> Self {
        let inner = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client");
        Self { inner }
    }

    /// Sends a request and returns its JSON body, mapping any non-2xx status to an `ApiError`.
    pub async fn json(&self, req: reqwest::RequestBuilder) -> Result<Value, ApiError> {
        let res = req.header("Accept", "application/json").send().await.map_err(|e| ApiError {
            status: 0,
            message: format!("network: {}", if e.is_timeout() { "timeout".into() } else { e.to_string() }),
            retry_after: None,
        })?;
        let status = res.status();
        let retry_after = parse_retry_after(res.headers());
        let text = res.text().await.unwrap_or_default();
        let json: Option<Value> = serde_json::from_str(&text).ok();
        if !status.is_success() {
            let detail = json
                .as_ref()
                .and_then(|j| {
                    j.pointer("/error/message")
                        .or_else(|| j.get("error_description"))
                        .or_else(|| j.get("error"))
                        .or_else(|| j.get("detail"))
                        .map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))
                })
                .unwrap_or_else(|| text.chars().take(200).collect());
            return Err(ApiError { status: status.as_u16(), message: format!("HTTP {} {}", status.as_u16(), detail), retry_after });
        }
        json.ok_or_else(|| ApiError::new(status.as_u16(), "response is not JSON"))
    }
}

/// Parses an RFC 3339 timestamp into epoch milliseconds.
pub fn parse_rfc3339(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.timestamp_millis())
}
