use super::anthropic_messages::AnthropicMessagesHandler;
use super::error::TransportError;
use super::messages::StreamPart;
use super::openai_compatible::OpenAiCompatibleHandler;
use super::openai_responses::OpenAiResponsesHandler;
use super::sse::ParsedSseEvent;
use super::super::types::OpenCodeProtocol;

pub enum LiveStreamHandler {
    OpenAiCompatible(OpenAiCompatibleHandler),
    OpenAiResponses(OpenAiResponsesHandler),
    AnthropicMessages(AnthropicMessagesHandler),
}

impl LiveStreamHandler {
    pub fn new(protocol: OpenCodeProtocol, model_id: &str) -> Self {
        match protocol {
            OpenCodeProtocol::OpenAiCompatible => {
                Self::OpenAiCompatible(OpenAiCompatibleHandler::new(model_id))
            }
            OpenCodeProtocol::OpenAiResponses => {
                Self::OpenAiResponses(OpenAiResponsesHandler::new(model_id))
            }
            OpenCodeProtocol::AnthropicMessages => {
                Self::AnthropicMessages(AnthropicMessagesHandler::new(model_id))
            }
            OpenCodeProtocol::GoogleGenerativeAi => {
                Self::OpenAiCompatible(OpenAiCompatibleHandler::new(model_id))
            }
        }
    }

    pub fn push(&mut self, event: &ParsedSseEvent) -> Result<Vec<StreamPart>, TransportError> {
        match self {
            Self::OpenAiCompatible(handler) => handler.push(event),
            Self::OpenAiResponses(handler) => handler.push(event),
            Self::AnthropicMessages(handler) => handler.push(event),
        }
    }

    pub fn end(&mut self) -> Result<Vec<StreamPart>, TransportError> {
        match self {
            Self::OpenAiCompatible(handler) => handler.end(),
            Self::OpenAiResponses(handler) => handler.end(),
            Self::AnthropicMessages(handler) => handler.end(),
        }
    }
}
