use super::super::types::OpenCodeProtocol;
use super::error::TransportError;

pub struct LanguageModelIdentity {
    pub id: String,
    pub protocol: OpenCodeProtocol,
    pub output_limit: u64,
}

pub fn validate_language_model(api_key: &str, model: &LanguageModelIdentity) -> Result<(), TransportError> {
    if api_key.is_empty() {
        return Err(TransportError::new(
            "invalid-api-key",
            "An API key is required to create an OpenCode language model.",
            false,
        ));
    }
    if model.protocol == OpenCodeProtocol::AnthropicMessages && model.output_limit == 0 {
        return Err(TransportError::new(
            "invalid-model-metadata",
            format!(
                "Anthropic model '{}' metadata must include a positive output limit.",
                model.id
            ),
            false,
        ));
    }
    Ok(())
}

#[cfg(test)]
pub fn redact_stream_error(error: &TransportError, api_key: Option<&str>) -> TransportError {
    use super::super::redact::redact_secret;

    TransportError::new(
        redact_secret(error.message(), api_key),
        redact_secret(&error.code, api_key),
        error.retryable,
    )
}
