use serde_json::{json, Map, Value};

use super::error::TransportError;
use super::http::join_endpoint;
use super::messages::{
    anthropic_request_messages, anthropic_tools, chat_completion_messages, chat_completion_tools,
    responses_input_from_messages, responses_tools, ModelRequest,
};
use super::wire_options::{
    anthropic_effort, anthropic_max_tokens, anthropic_thinking, openai_compatible_max_tokens,
    openai_compatible_reasoning_effort, openai_responses_max_output_tokens,
    openai_responses_reasoning,
};
use super::super::endpoints::plan_endpoints;
use super::super::types::{OpenCodePlan, OpenCodeProtocol};

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub fn stream_path(protocol: OpenCodeProtocol) -> &'static str {
    match protocol {
        OpenCodeProtocol::OpenAiCompatible => "/chat/completions",
        OpenCodeProtocol::OpenAiResponses => "/responses",
        OpenCodeProtocol::AnthropicMessages => "/messages",
        OpenCodeProtocol::GoogleGenerativeAi => "/chat/completions",
    }
}

pub fn stream_url(plan: OpenCodePlan, protocol: OpenCodeProtocol) -> String {
    join_endpoint(plan_endpoints(plan).base_url, stream_path(protocol))
}

pub fn stream_headers(api_key: &str, protocol: OpenCodeProtocol) -> Vec<(String, String)> {
    let mut headers = super::http::json_headers(api_key);
    if protocol == OpenCodeProtocol::AnthropicMessages {
        headers.push(("anthropic-version".into(), ANTHROPIC_VERSION.into()));
    }
    headers
}

pub fn build_stream_request_body(
    protocol: OpenCodeProtocol,
    model_id: &str,
    request: &ModelRequest,
    output_limit: u64,
) -> Result<Value, TransportError> {
    match protocol {
        OpenCodeProtocol::OpenAiCompatible | OpenCodeProtocol::GoogleGenerativeAi => {
            let reasoning_effort = openai_compatible_reasoning_effort(&request.provider_options);
            let tools = chat_completion_tools(&request.tools);
            let max_tokens = openai_compatible_max_tokens(&request.provider_options);
            let mut body = Map::new();
            body.insert("model".into(), Value::String(model_id.into()));
            body.insert(
                "messages".into(),
                Value::Array(chat_completion_messages(&request.messages, model_id)?),
            );
            body.insert("stream".into(), Value::Bool(true));
            body.insert(
                "stream_options".into(),
                json!({ "include_usage": true }),
            );
            if let Some(tools) = tools {
                body.insert("tools".into(), Value::Array(tools));
            }
            if let Some(reasoning_effort) = reasoning_effort {
                body.insert("reasoning_effort".into(), Value::String(reasoning_effort));
            }
            if let Some(max_tokens) = max_tokens {
                body.insert("max_tokens".into(), json!(max_tokens));
                body.insert("max_completion_tokens".into(), json!(max_tokens));
            }
            Ok(Value::Object(body))
        }
        OpenCodeProtocol::OpenAiResponses => {
            let reasoning = openai_responses_reasoning(&request.provider_options);
            let tools = responses_tools(&request.tools);
            let max_output_tokens = openai_responses_max_output_tokens(&request.provider_options);
            let mut body = Map::new();
            body.insert("model".into(), Value::String(model_id.into()));
            body.insert(
                "input".into(),
                Value::Array(responses_input_from_messages(&request.messages, model_id)?),
            );
            body.insert("stream".into(), Value::Bool(true));
            if let Some(tools) = tools {
                body.insert("tools".into(), Value::Array(tools));
            }
            if let Some(max_output_tokens) = max_output_tokens {
                body.insert("max_output_tokens".into(), json!(max_output_tokens));
            }
            if let Some(reasoning) = reasoning {
                body.insert("reasoning".into(), Value::Object(reasoning));
                body.insert(
                    "include".into(),
                    Value::Array(vec![Value::String("reasoning.encrypted_content".into())]),
                );
            }
            Ok(Value::Object(body))
        }
        OpenCodeProtocol::AnthropicMessages => {
            let converted = anthropic_request_messages(&request.messages, model_id)?;
            let thinking = anthropic_thinking(&request.provider_options);
            let effort = anthropic_effort(&request.provider_options);
            let tools = anthropic_tools(&request.tools);
            let configured_max_tokens = anthropic_max_tokens(&request.provider_options);
            let max_tokens = configured_max_tokens
                .map(|value| value.min(output_limit))
                .unwrap_or(output_limit);
            if let Some(ref thinking) = thinking {
                if let Some(budget) = thinking.get("budget_tokens").and_then(|value| value.as_i64()) {
                    if budget >= max_tokens as i64 {
                        return Err(TransportError::new(
                            "invalid-provider-options",
                            "Anthropic thinking budget must be smaller than the model output limit.",
                            false,
                        ));
                    }
                }
            }
            let mut body = Map::new();
            body.insert("model".into(), Value::String(model_id.into()));
            body.insert("messages".into(), Value::Array(converted.messages));
            body.insert("max_tokens".into(), json!(max_tokens));
            body.insert("stream".into(), Value::Bool(true));
            if let Some(system) = converted.system {
                body.insert("system".into(), Value::String(system));
            }
            if let Some(tools) = tools {
                body.insert("tools".into(), Value::Array(tools));
            }
            if let Some(thinking) = thinking {
                body.insert("thinking".into(), Value::Object(thinking));
            }
            if let Some(effort) = effort {
                body.insert("output_config".into(), json!({ "effort": effort }));
            }
            Ok(Value::Object(body))
        }
    }
}
