use super::super::endpoints::plan_endpoints;
use super::super::redact::redact_secret;
use super::super::types::{OpenCodePlan, OpenCodeProtocol};
use super::error::TransportError;
use super::http::unsupported_protocol_error;
use super::messages::{ModelRequest, StreamPart};
use super::openai_compatible::process_openai_compatible_events;
use super::openai_responses::process_openai_responses_events;
use super::anthropic_messages::process_anthropic_messages_events;
use super::sse::ParsedSseEvent;

pub struct LanguageModelIdentity {
    pub id: String,
    pub protocol: OpenCodeProtocol,
    pub output_limit: u64,
}

pub fn provider_for_protocol(protocol: OpenCodeProtocol) -> &'static str {
    match protocol {
        OpenCodeProtocol::OpenAiResponses => "openai.responses",
        OpenCodeProtocol::OpenAiCompatible => "openai-compatible.chat",
        OpenCodeProtocol::AnthropicMessages => "anthropic.messages",
        OpenCodeProtocol::GoogleGenerativeAi => "google.generative-ai",
    }
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
            "Anthropic model metadata must include a positive output limit.",
            false,
        ));
    }
    Ok(())
}

pub fn redact_stream_error(error: &TransportError, api_key: Option<&str>) -> TransportError {
    TransportError::new(
        redact_secret(error.message(), api_key),
        redact_secret(&error.code, api_key),
        error.retryable,
    )
}

pub fn process_protocol_events(
    plan: OpenCodePlan,
    model: &LanguageModelIdentity,
    _request: &ModelRequest,
    events: &[ParsedSseEvent],
    api_key: Option<&str>,
) -> Result<Vec<StreamPart>, TransportError> {
    validate_language_model(api_key.unwrap_or(""), model)?;
    match model.protocol {
        OpenCodeProtocol::GoogleGenerativeAi => {
            Err(unsupported_protocol_error("google-generative-ai", api_key))
        }
        OpenCodeProtocol::OpenAiResponses => {
            let _ = plan_endpoints(plan);
            process_openai_responses_events(&model.id, events)
        }
        OpenCodeProtocol::OpenAiCompatible => process_openai_compatible_events(&model.id, events),
        OpenCodeProtocol::AnthropicMessages => process_anthropic_messages_events(&model.id, events),
    }
}
