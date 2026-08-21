mod anthropic_messages;
mod complete_tool_call;
mod error;
mod http;
mod language_model;
mod messages;
mod openai_compatible;
mod openai_responses;
mod sse;
mod stream_json;
mod usage;
mod wire_options;

#[cfg(test)]
mod tests;

pub use anthropic_messages::process_anthropic_messages_events;
pub use complete_tool_call::complete_tool_call;
pub use error::TransportError;
pub use http::{
    classify_fetch_error, classify_http_status, extract_provider_error_message, join_endpoint,
    json_headers, parse_retry_after_ms, stringify_request_body, unsupported_protocol_error,
};
pub use language_model::{
    process_protocol_events, provider_for_protocol, redact_stream_error, validate_language_model,
    LanguageModelIdentity,
};
pub use messages::{
    anthropic_request_messages, anthropic_tools, chat_completion_messages, chat_completion_tools,
    responses_input_from_messages, responses_tools, AnthropicMessages, AssistantPart, Message,
    MessageRole, ModelRequest, ModelTool, StreamPart, ToolResultOutput, ToolResultPart,
};
pub use openai_compatible::process_openai_compatible_events;
pub use openai_responses::process_openai_responses_events;
pub use sse::{is_sse_terminal_sentinel, ParsedSseEvent, SseParser};
pub use usage::{
    anthropic_finish_reason, anthropic_usage, chat_finish_reason, merge_usage,
    responses_finish_reason, token_usage_from_openai, FinishReason, TokenUsage,
};
pub use wire_options::{
    anthropic_effort, anthropic_max_tokens, anthropic_thinking, openai_compatible_max_tokens,
    openai_compatible_reasoning_effort, openai_responses_max_output_tokens,
    openai_responses_reasoning, provider_options_record,
};
