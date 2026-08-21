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
    let identity = LanguageModelIdentity {
        id: model.id.clone(),
        protocol,
        output_limit: config.output_limit,
    };
    validate_language_model(&api_key, &identity)?;

    let request = llm_context_to_model_request(&llm_context, ProviderOptions::default())?;
    let body = build_stream_request_body(protocol, &model.id, &request, config.output_limit)?;
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
    use crate::backend::agent::{AgentMessage, AssistantContent, LlmMessage, ToolResultMessage};
    use serde_json::Value;

    pub fn agent_messages_to_llm(messages: Vec<AgentMessage>) -> Vec<LlmMessage> {
        messages
            .into_iter()
            .filter_map(|message| match message {
                AgentMessage::User(user) => Some(LlmMessage {
                    role: user.role,
                    content: Some(user.content),
                    tool_call_id: None,
                }),
            AgentMessage::Assistant(assistant) => {
                let text = assistant_text(&assistant);
                Some(LlmMessage {
                    role: assistant.role.clone(),
                    content: Some(Value::String(text)),
                    tool_call_id: None,
                })
            }
            AgentMessage::ToolResult(tool) => {
                let text = tool_result_text(&tool);
                Some(LlmMessage {
                    role: tool.role.clone(),
                    content: Some(Value::String(text)),
                    tool_call_id: Some(tool.tool_call_id.clone()),
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
                "assistant" => messages.push(Message {
                    role: MessageRole::Assistant,
                    content_text: None,
                    assistant_parts: vec![crate::backend::opencode::transport::AssistantPart::Text {
                        text: llm_message
                            .content
                            .as_ref()
                            .and_then(content_to_string)
                            .unwrap_or_default(),
                    }],
                    tool_results: Vec::new(),
                    provider_metadata: None,
                }),
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
                            is_error: false,
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

    fn assistant_text(message: &crate::backend::agent::AssistantMessage) -> String {
        message
            .content
            .iter()
            .filter_map(|part| match part {
                AssistantContent::Text(text) => Some(text.text.clone()),
                AssistantContent::ToolCall(_) => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn tool_result_text(message: &ToolResultMessage) -> String {
        message
            .content
            .iter()
            .map(|part| part.text.clone())
            .collect::<Vec<_>>()
            .join("")
    }
}
