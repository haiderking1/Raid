use serde_json::{Map, Value};

use super::error::TransportError;
use super::super::json::read_string;

pub fn parse_sse_json(data: &str, retryable: bool) -> Result<Map<String, Value>, TransportError> {
    let parsed: Value = serde_json::from_str(data).map_err(|cause| {
        TransportError::with_cause(
            "malformed-stream",
            "Provider stream contained malformed JSON.",
            retryable,
            cause,
        )
    })?;
    parsed.as_object().cloned().ok_or_else(|| {
        TransportError::new(
            "malformed-stream",
            "Provider stream contained a non-object event.",
            retryable,
        )
    })
}

pub fn provider_stream_error(payload: &Map<String, Value>, retryable: bool) -> TransportError {
    let error = payload
        .get("error")
        .and_then(|value| value.as_object())
        .unwrap_or(payload);
    let message = error
        .get("message")
        .and_then(read_string)
        .unwrap_or("Provider reported a stream error.");
    let code = error
        .get("code")
        .and_then(read_string)
        .or_else(|| error.get("type").and_then(read_string))
        .unwrap_or("provider-error");
    let retryable_code = matches!(
        code,
        "overloaded_error" | "server_error" | "api_error" | "rate_limit_exceeded"
    );
    TransportError::new(code, message, retryable && retryable_code)
}

pub fn anthropic_provider_stream_error(payload: &Map<String, Value>, retryable: bool) -> TransportError {
    let error = payload
        .get("error")
        .and_then(|value| value.as_object())
        .unwrap_or(payload);
    let message = error
        .get("message")
        .and_then(read_string)
        .unwrap_or("Provider reported a stream error.");
    let code = error
        .get("type")
        .and_then(read_string)
        .or_else(|| error.get("code").and_then(read_string))
        .unwrap_or("provider-error");
    let retryable_code = matches!(code, "overloaded_error" | "api_error" | "rate_limit_error");
    TransportError::new(code, message, retryable && retryable_code)
}

pub fn responses_provider_stream_error(payload: &Map<String, Value>, retryable: bool) -> TransportError {
    let response = payload.get("response").and_then(|value| value.as_object());
    let error = payload
        .get("error")
        .and_then(|value| value.as_object())
        .or_else(|| response.and_then(|value| value.get("error")).and_then(|value| value.as_object()))
        .unwrap_or(payload);
    provider_stream_error(error, retryable)
}
