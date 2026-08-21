use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use reqwest::Client;
use tokio_util::sync::CancellationToken;

use crate::backend::agent::{
    assistant_message_stream, AgentMessage, LlmContext, Model, StreamFn, StreamOptions,
};
use crate::backend::opencode::transport::{
    build_stream_request_body, stream_headers, stream_sse_request, stream_url, validate_language_model,
    LanguageModelIdentity, LiveStreamHandler, Message, MessageRole, ModelRequest, ModelTool,
    TransportError,
};
use crate::backend::opencode::stream_adapter::StreamPartEmitter;
use crate::backend::opencode::types::{OpenCodeProtocol, ProviderOptions};

#[derive(Clone)]
pub struct OpenCodeStreamConfig {
    pub client: Client,
    pub output_limit: u64,
}

pub fn protocol_from_api(api: &str) -> OpenCodeProtocol {
    match api {
        "openai-responses" => OpenCodeProtocol::OpenAiResponses,
        "anthropic-messages" => OpenCodeProtocol::AnthropicMessages,
        "google-generative-ai" => OpenCodeProtocol::GoogleGenerativeAi,
        _ => OpenCodeProtocol::OpenAiCompatible,
    }
}

pub fn convert_agent_messages(messages: Vec<AgentMessage>) -> Pin<Box<dyn Future<Output = Vec<crate::backend::agent::LlmMessage>> + Send>> {
    Box::pin(async move { adapter::agent_messages_to_llm(messages) })
}

pub fn llm_context_to_model_request(
    llm_context: &LlmContext,
    provider_options: ProviderOptions,
) -> Result<ModelRequest, TransportError> {
    adapter::llm_context_to_model_request(llm_context, provider_options)
}

pub fn opencode_stream_fn(config: OpenCodeStreamConfig) -> StreamFn {
    Arc::new(move |model, llm_context, options, cancel| {
        let config = config.clone();
        Box::pin(async move {
            let stream = assistant_message_stream();
            let worker = stream.clone();
            let model_for_task = model.clone();
            tokio::spawn(async move {
                let worker_for_task = worker.clone();
                if let Err(error) = run_live_stream(
                    worker,
                    config,
                    model_for_task.clone(),
                    llm_context,
                    options,
                    cancel,
                )
                .await
                {
                    let mut emitter = StreamPartEmitter::new(worker_for_task, model_for_task);
                    emitter.finish_transport_error(error);
                }
            });
            stream
        })
    })
}

async fn run_live_stream(
    stream: crate::backend::agent::AssistantMessageStream,
    config: OpenCodeStreamConfig,
    model: Model,
    llm_context: LlmContext,
    options: StreamOptions,
    cancel: CancellationToken,
) -> Result<(), TransportError> {
    let api_key = options
        .api_key
        .filter(|key| !key.is_empty())
        .ok_or_else(|| TransportError::new("invalid-api-key", "An API key is required.", false))?;

    let protocol = protocol_from_api(&model.api);
    let output_limit = options
        .max_output_tokens
        .unwrap_or(config.output_limit)
        .min(config.output_limit)
        .max(1);
    let identity = LanguageModelIdentity {
        id: model.id.clone(),
        protocol,
        output_limit,
    };
    validate_language_model(&api_key, &identity)?;

    let request = llm_context_to_model_request(&llm_context, ProviderOptions::default())?;
    let body = build_stream_request_body(protocol, &model.id, &request, output_limit)?;
    let plan = crate::config::plan_for_provider_id(&model.provider);
    let url = stream_url(plan, protocol);
    let headers = stream_headers(&api_key, protocol);

    let mut handler = LiveStreamHandler::new(protocol, &model.id);
    let mut emitter = StreamPartEmitter::new(stream.clone(), model);

    stream_sse_request(&config.client, &url, &headers, &body, &cancel, |event| {
        for part in handler.push(&event)? {
            emitter.push_part(part);
        }
        Ok(())
    })
    .await?;

    for part in handler.end()? {
        emitter.push_part(part);
    }
    Ok(())
}

pub mod adapter {
    use super::*;
    use crate::backend::agent::{
        AgentMessage, AssistantContent, LlmMessage, ToolResultMessage,
    };
    use serde_json::{json, Value};

    pub fn agent_messages_to_llm(messages: Vec<AgentMessage>) -> Vec<LlmMessage> {
        messages
            .into_iter()
            .filter_map(|message| match message {
                AgentMessage::User(user) => Some(LlmMessage {
                    role: user.role,
                    content: Some(user.content),
                    tool_call_id: None,
                    is_error: None,
                }),
            AgentMessage::Assistant(assistant) => {
                let mut parts = Vec::new();
                for content in &assistant.content {
                    match content {
                        AssistantContent::Text(text) => {
                            parts.push(json!({ "type": "text", "text": text.text }));
                        }
                        AssistantContent::ToolCall(tool_call) => {
                            parts.push(json!({
                                "type": "toolCall",
                                "id": tool_call.id,
                                "name": tool_call.name,
                                "arguments": tool_call.arguments,
                            }));
                        }
                    }
                }
                Some(LlmMessage {
                    role: assistant.role.clone(),
                    content: Some(Value::Array(parts)),
                    tool_call_id: None,
                    is_error: None,
                })
            }
            AgentMessage::ToolResult(tool) => {
                let text = tool_result_text(&tool);
                Some(LlmMessage {
                    role: tool.role.clone(),
                    content: Some(json!([{ "type": "text", "text": text }])),
                    tool_call_id: Some(tool.tool_call_id.clone()),
                    is_error: Some(tool.is_error),
                })
            }
            })
            .collect()
    }

    pub fn llm_context_to_model_request(
        llm_context: &LlmContext,
        provider_options: ProviderOptions,
    ) -> Result<ModelRequest, TransportError> {
        let mut messages = Vec::new();
        if let Some(system_prompt) = &llm_context.system_prompt {
            if !system_prompt.is_empty() {
                messages.push(Message {
                    role: MessageRole::System,
                    content_text: Some(system_prompt.clone()),
                    assistant_parts: Vec::new(),
                    tool_results: Vec::new(),
                    provider_metadata: None,
                });
            }
        }
        for llm_message in &llm_context.messages {
            match llm_message.role.as_str() {
                "user" => messages.push(Message {
                    role: MessageRole::User,
                    content_text: llm_message
                        .content
                        .as_ref()
                        .and_then(content_to_string),
                    assistant_parts: Vec::new(),
                    tool_results: Vec::new(),
                    provider_metadata: None,
                }),
                "assistant" => {
                    let mut assistant_parts = Vec::new();
                    if let Some(Value::Array(items)) = &llm_message.content {
                        for item in items {
                            match item.get("type").and_then(Value::as_str) {
                                Some("text") => {
                                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                                        assistant_parts.push(
                                            crate::backend::opencode::transport::AssistantPart::Text {
                                                text: text.to_string(),
                                            },
                                        );
                                    }
                                }
                                Some("toolCall") => {
                                    assistant_parts.push(
                                        crate::backend::opencode::transport::AssistantPart::ToolCall {
                                            tool_call_id: item
                                                .get("id")
                                                .and_then(Value::as_str)
                                                .unwrap_or("tool")
                                                .to_string(),
                                            tool_name: item
                                                .get("name")
                                                .and_then(Value::as_str)
                                                .unwrap_or("tool")
                                                .to_string(),
                                            input: item
                                                .get("arguments")
                                                .cloned()
                                                .unwrap_or(Value::Null),
                                        },
                                    );
                                }
                                _ => {}
                            }
                        }
                    } else if let Some(text) = llm_message
                        .content
                        .as_ref()
                        .and_then(content_to_string)
                    {
                        assistant_parts.push(
                            crate::backend::opencode::transport::AssistantPart::Text { text },
                        );
                    }
                    messages.push(Message {
                        role: MessageRole::Assistant,
                        content_text: None,
                        assistant_parts,
                        tool_results: Vec::new(),
                        provider_metadata: None,
                    });
                }
                "toolResult" => {
                    let tool_call_id = llm_message
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| "tool".into());
                    messages.push(Message {
                        role: MessageRole::Tool,
                        content_text: None,
                        assistant_parts: Vec::new(),
                        tool_results: vec![crate::backend::opencode::transport::ToolResultPart {
                            tool_call_id,
                            output: vec![crate::backend::opencode::transport::ToolResultOutput::Text {
                                text: llm_message
                                    .content
                                    .as_ref()
                                    .and_then(content_to_string)
                                    .unwrap_or_default(),
                            }],
                            is_error: llm_message.is_error.unwrap_or(false),
                        }],
                        provider_metadata: None,
                    });
                }
                _ => {}
            }
        }

        Ok(ModelRequest {
            messages,
            tools: llm_context
                .tools
                .iter()
                .map(|tool| ModelTool {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.parameters.clone(),
                })
                .collect(),
            provider_options,
        })
    }

    fn content_to_string(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        }
    }

    fn tool_result_text(message: &ToolResultMessage) -> String {
        message
            .content
            .iter()
            .filter_map(|part| part.as_text())
            .collect::<Vec<_>>()
            .join("")
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::adapter::{agent_messages_to_llm, llm_context_to_model_request};
    use crate::backend::agent::{
        assistant_message, AssistantContent, LlmContext, TextContent, ToolCall, ToolDefinition,
        ToolResultContent, ToolResultMessage, UserMessage,
    };
    use crate::backend::opencode::transport::{AssistantPart, MessageRole};
    use crate::backend::opencode::types::ProviderOptions;
    use serde_json::json;

    #[test]
    fn preserves_assistant_tool_calls_in_model_request() {
        let messages = vec![
            crate::backend::agent::AgentMessage::User(UserMessage::new("run it")),
            crate::backend::agent::AgentMessage::Assistant(assistant_message(
                vec![
                    AssistantContent::Text(TextContent::new("calling tool")),
                    AssistantContent::ToolCall(ToolCall::new(
                        "call-1",
                        "bash",
                        json!({ "command": "pwd" }),
                    )),
                ],
                crate::backend::agent::StopReason::ToolUse,
            )),
            crate::backend::agent::AgentMessage::ToolResult(ToolResultMessage {
                role: "toolResult".into(),
                tool_call_id: "call-1".into(),
                tool_name: "bash".into(),
                content: vec![ToolResultContent::text("/tmp")],
                details: None,
                usage: None,
                added_tool_names: None,
                is_error: false,
                timestamp: 0,
            }),
        ];
        let llm_messages = agent_messages_to_llm(messages);
        let request = llm_context_to_model_request(
            &LlmContext {
                system_prompt: None,
                messages: llm_messages,
                tools: vec![ToolDefinition {
                    name: "bash".into(),
                    description: "bash".into(),
                    parameters: json!({ "type": "object" }),
                }],
            },
            ProviderOptions::default(),
        )
        .expect("request");
        let assistant = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Assistant)
            .expect("assistant");
        assert!(matches!(
            assistant.assistant_parts.as_slice(),
            [
                AssistantPart::Text { text },
                AssistantPart::ToolCall { tool_name, .. }
            ] if text == "calling tool" && tool_name == "bash"
        ));
        let tool = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Tool)
            .expect("tool");
        assert_eq!(tool.tool_results[0].tool_call_id, "call-1");
        assert!(!tool.tool_results[0].is_error);
    }
}
