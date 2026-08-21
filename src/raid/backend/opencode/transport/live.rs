use futures::StreamExt;
use reqwest::Client;
use tokio_util::sync::CancellationToken;

use super::error::TransportError;
use super::http::{classify_fetch_error, classify_http_status, extract_provider_error_message, parse_retry_after_ms, stringify_request_body};
use super::sse::{is_sse_terminal_sentinel, ParsedSseEvent, SseParser};
use serde_json::Value;

pub async fn stream_sse_request<F>(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
    body: &Value,
    cancel: &CancellationToken,
    mut on_event: F,
) -> Result<(), TransportError>
where
    F: FnMut(ParsedSseEvent) -> Result<(), TransportError>,
{
    let payload = stringify_request_body(body)?;
    let mut request = client.post(url).body(payload);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }

    let response = tokio::select! {
        response = request.send() => response,
        _ = cancel.cancelled() => {
            return Err(TransportError::new("aborted", "Request was cancelled.", false));
        }
    }
    .map_err(|error| classify_fetch_error(&error, None))?;

    let status = response.status();
    if !status.is_success() {
        let retry_after_ms = parse_retry_after_ms(response.headers().get("retry-after").and_then(|value| value.to_str().ok()));
        let body_text = response.text().await.unwrap_or_default();
        let (code, retryable) = classify_http_status(status);
        let message = extract_provider_error_message(&body_text)
            .unwrap_or_else(|| format!("HTTP {status} from provider."));
        let mut error = TransportError::new(code, message, retryable);
        if let Some(retry_after_ms) = retry_after_ms {
            error = error.with_retry_after(retry_after_ms);
        }
        return Err(error);
    }

    let mut parser = SseParser::new();
    let mut byte_stream = response.bytes_stream();
    while let Some(chunk) = tokio::select! {
        chunk = byte_stream.next() => chunk,
        _ = cancel.cancelled() => {
            return Err(TransportError::new("aborted", "Stream was cancelled.", false));
        }
    } {
        let chunk = chunk.map_err(|error| classify_fetch_error(&error, None))?;
        for event in parser.push(&chunk) {
            if is_sse_terminal_sentinel(&event) {
                return Ok(());
            }
            on_event(event)?;
        }
    }

    for event in parser.finish() {
        if is_sse_terminal_sentinel(&event) {
            return Ok(());
        }
        on_event(event)?;
    }
    Ok(())
}
