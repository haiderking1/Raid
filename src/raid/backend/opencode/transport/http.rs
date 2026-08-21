use reqwest::StatusCode;

use super::error::TransportError;
use super::super::redact::{redact_error, redact_secret};
use super::super::json::stringify_json;
use serde_json::Value;

pub fn classify_http_status(status: StatusCode) -> (&'static str, bool) {
    let code = status.as_u16();
    match code {
        401 | 403 => ("authentication", false),
        408 => ("request-timeout", true),
        409 => ("conflict", true),
        429 => ("rate-limit", true),
        529 => ("overloaded", true),
        400 | 404 | 413 | 415 | 422 => ("invalid-request", false),
        500..=599 => ("server-error", true),
        400..=499 => ("invalid-request", false),
        _ => ("http-error", false),
    }
}

pub fn parse_retry_after_ms(header: Option<&str>) -> Option<u64> {
    let trimmed = header?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(seconds) = trimmed.parse::<f64>() {
        if seconds.is_finite() && seconds >= 0.0 {
            return Some((seconds * 1000.0) as u64);
        }
    }
    None
}

pub fn extract_provider_error_message(body_text: &str) -> Option<String> {
    let trimmed = body_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: Value = serde_json::from_str(trimmed).ok()?;
    message_from_unknown(&parsed)
}

fn message_from_unknown(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().filter(|text| !text.is_empty()) {
        return Some(text.to_string());
    }
    let record = value.as_object()?;
    if let Some(text) = record.get("message").and_then(|v| v.as_str()).filter(|t| !t.is_empty()) {
        return Some(text.to_string());
    }
    if let Some(text) = record.get("error").and_then(|v| v.as_str()).filter(|t| !t.is_empty()) {
        return Some(text.to_string());
    }
    record
        .get("error")
        .and_then(|nested| message_from_unknown(nested))
}

pub fn join_endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let suffix = path.trim_start_matches('/');
    format!("{base}/{suffix}")
}

pub fn unsupported_protocol_error(protocol: &str, api_key: Option<&str>) -> TransportError {
    TransportError::new(
        "unsupported-protocol",
        redact_secret(
            &format!("Protocol '{protocol}' is not supported by native transports."),
            api_key,
        ),
        false,
    )
}

pub fn classify_fetch_error(
    error: &(dyn std::error::Error + 'static),
    api_key: Option<&str>,
) -> TransportError {
    let message = redact_error(error, api_key);
    TransportError::with_cause(
        "network-error",
        if message.is_empty() {
            "Network request failed.".into()
        } else {
            message
        },
        true,
        std::io::Error::new(std::io::ErrorKind::Other, error.to_string()),
    )
}

pub fn json_headers(api_key: &str) -> Vec<(String, String)> {
    vec![
        ("accept".into(), "text/event-stream".into()),
        ("content-type".into(), "application/json".into()),
        ("authorization".into(), format!("Bearer {api_key}")),
    ]
}

pub fn stringify_request_body(body: &Value) -> Result<String, TransportError> {
    stringify_json(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_auth_errors() {
        let (code, retryable) = classify_http_status(StatusCode::UNAUTHORIZED);
        assert_eq!(code, "authentication");
        assert!(!retryable);
    }

    #[test]
    fn parses_retry_after_seconds() {
        assert_eq!(parse_retry_after_ms(Some("2")), Some(2000));
    }

    #[test]
    fn joins_endpoints() {
        assert_eq!(
            join_endpoint("https://opencode.ai/zen/go/v1/", "/chat/completions"),
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
    }
}
