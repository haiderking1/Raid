mod anthropic_messages;
mod complete_tool_call;
mod error;
mod http;
mod language_model;
mod live;
mod live_handler;
mod messages;
mod openai_compatible;
mod openai_responses;
mod request;
mod sse;
mod stream_json;
mod usage;
mod wire_options;

#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod tests;

pub use live::stream_sse_request;
pub use live_handler::LiveStreamHandler;
pub use request::{build_stream_request_body, stream_headers, stream_url};
pub use error::TransportError;
pub use language_model::{validate_language_model, LanguageModelIdentity};
pub use messages::{
    AssistantPart, Message, MessageRole, ModelRequest, ModelTool, StreamPart, ToolResultOutput,
    ToolResultPart,
};
pub use usage::{FinishReason, TokenUsage};
