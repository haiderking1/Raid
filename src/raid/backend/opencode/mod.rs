mod cache;
mod catalog;
mod endpoints;
mod error;
mod json;
mod malformed_tool_call;
mod reasoning;
mod redact;
mod transport;
mod types;
mod validate;
mod wire;

pub use cache::{memory_cache, MetadataCache};
pub use catalog::{effective_input_limit, load_catalog, CatalogHttp, ReqwestCatalogHttp};
pub use endpoints::{
    metadata_provider_id, plan_endpoints, DEFAULT_REQUEST_TIMEOUT, METADATA_URL, PLAN_IDS,
};
pub use error::CatalogError;
pub use json::{
    assert_json_value, clone_safe_json, describe_json_value_problem, is_record, parse_json_object,
    parse_tool_call_arguments, read_finite_number, read_string, snapshot_safe_json, stringify_json,
};
pub use reasoning::{
    derive_reasoning_variants, parse_reasoning_options, ReasoningOption,
};
pub use redact::{
    redact_error, redact_secret, redact_unknown, CIRCULAR_PLACEHOLDER, REDACTED_PLACEHOLDER,
    UNREADABLE_PLACEHOLDER,
};
pub use transport::{
    anthropic_finish_reason, anthropic_request_messages, anthropic_tools, anthropic_usage,
    chat_completion_messages, chat_completion_tools, chat_finish_reason, classify_fetch_error,
    classify_http_status, complete_tool_call, extract_provider_error_message, join_endpoint,
    json_headers, merge_usage, parse_retry_after_ms, process_anthropic_messages_events,
    process_openai_compatible_events, process_openai_responses_events, process_protocol_events,
    provider_for_protocol, provider_options_record, redact_stream_error, responses_finish_reason,
    responses_input_from_messages, responses_tools, stringify_request_body, token_usage_from_openai,
    unsupported_protocol_error, validate_language_model, FinishReason, LanguageModelIdentity,
    Message, MessageRole, ModelRequest, ModelTool, ParsedSseEvent, SseParser, StreamPart,
    TokenUsage, TransportError,
};
pub use types::{
    CatalogDiagnostic, CatalogSource, InterleavedFieldState, ModelCost, ModelModality,
    ModelStatus, OpenCodeCatalog, OpenCodePlan, OpenCodeProtocol, ReasoningVariant,
    ResolvedModel, SdkPackage,
};
pub use malformed_tool_call::{
    is_malformed_tool_call_input, malformed_tool_call_input, MALFORMED_TOOL_ARGUMENTS_FLAG,
    MALFORMED_TOOL_ARGUMENTS_MESSAGE,
};
pub use validate::{parse_cached_catalog, serialize_catalog, validate_catalog_invariants, CatalogValidationError};
