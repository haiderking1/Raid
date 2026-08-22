use serde_json::{Map, Value};

use super::super::json::stringify_json;
use super::super::types::ProviderOptions;
use super::usage::TokenUsage;

#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum AssistantPart {
    Text { text: String },
    Reasoning { text: String },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: Value,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResultPart {
    pub tool_call_id: String,
    pub output: Vec<ToolResultOutput>,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum ToolResultOutput {
    Text { text: String },
    Json { value: Value },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: MessageRole,
    pub content_text: Option<String>,
    pub assistant_parts: Vec<AssistantPart>,
    pub tool_results: Vec<ToolResultPart>,
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamPart {
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: Value,
    },
    Finish {
        reason: super::usage::FinishReason,
        usage: Option<TokenUsage>,
        provider_metadata: Option<Value>,
    },
}

pub fn serialize_tool_result_output(result: &ToolResultPart) -> Result<String, super::error::TransportError> {
    if result.output.len() == 1 {
        match &result.output[0] {
            ToolResultOutput::Text { text } => return Ok(text.clone()),
            ToolResultOutput::Json { value } => return stringify_json(value),
        }
    }
    let array: Vec<Value> = result
        .output
        .iter()
        .map(|part| match part {
            ToolResultOutput::Text { text } => Value::String(text.clone()),
            ToolResultOutput::Json { value } => value.clone(),
        })
        .collect();
    stringify_json(&Value::Array(array))
}

pub fn responses_input_from_messages(
    messages: &[Message],
    model_id: &str,
) -> Result<Vec<Value>, super::error::TransportError> {
    let mut input = Vec::new();
    for message in messages {
        match message.role {
            MessageRole::System => {
                input.push(serde_json::json!({
                    "role": "system",
                    "content": message.content_text.as_deref().unwrap_or(""),
                }));
            }
            MessageRole::User => {
                input.push(serde_json::json!({
                    "role": "user",
                    "content": message.content_text.as_deref().unwrap_or(""),
                }));
            }
            MessageRole::Assistant => {
                if let Some(native) = native_array(&message.provider_metadata, "openai-responses", model_id, "output") {
                    input.extend(native);
                    continue;
                }
                for part in &message.assistant_parts {
                    match part {
                        AssistantPart::Text { text } => {
                            input.push(serde_json::json!({ "role": "assistant", "content": text }));
                        }
                        AssistantPart::ToolCall {
                            tool_call_id,
                            tool_name,
                            input: args,
                        } => {
                            input.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": stringify_json(args)?,
                            }));
                        }
                        AssistantPart::Reasoning { .. } => {}
                    }
                }
            }
            MessageRole::Tool => {
                for result in &message.tool_results {
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": result.tool_call_id,
                        "output": serialize_tool_result_output(result)?,
                    }));
                }
            }
        }
    }
    Ok(input)
}

pub fn responses_tools(tools: &[ModelTool]) -> Option<Vec<Value>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect(),
    )
}

pub fn chat_completion_messages(
    messages: &[Message],
    model_id: &str,
) -> Result<Vec<Value>, super::error::TransportError> {
    let mut output = Vec::new();
    for message in messages {
        match message.role {
            MessageRole::System => {
                output.push(serde_json::json!({
                    "role": "system",
                    "content": message.content_text.as_deref().unwrap_or(""),
                }));
            }
            MessageRole::User => {
                output.push(serde_json::json!({
                    "role": "user",
                    "content": message.content_text.as_deref().unwrap_or(""),
                }));
            }
            MessageRole::Assistant => {
                if let Some(native) =
                    native_record(&message.provider_metadata, "openai-compatible", model_id, "message")
                {
                    output.push(native);
                    continue;
                }
                let mut text = String::new();
                let mut reasoning = String::new();
                let mut tool_calls = Vec::new();
                for part in &message.assistant_parts {
                    match part {
                        AssistantPart::Text { text: chunk } => text.push_str(chunk),
                        AssistantPart::Reasoning { text: chunk } => reasoning.push_str(chunk),
                        AssistantPart::ToolCall {
                            tool_call_id,
                            tool_name,
                            input,
                        } => tool_calls.push(serde_json::json!({
                            "id": tool_call_id,
                            "type": "function",
                            "function": {
                                "name": tool_name,
                                "arguments": stringify_json(input)?,
                            }
                        })),
                    }
                }
                // Aborted or empty responses can remain in session history. Sending one back
                // without content or tool calls is invalid for OpenAI-compatible providers.
                if text.is_empty() && tool_calls.is_empty() {
                    continue;
                }
                let mut assistant = Map::new();
                assistant.insert("role".into(), Value::String("assistant".into()));
                assistant.insert(
                    "content".into(),
                    if text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(text)
                    },
                );
                if !reasoning.is_empty() {
                    assistant.insert("reasoning_content".into(), Value::String(reasoning));
                }
                if !tool_calls.is_empty() {
                    assistant.insert("tool_calls".into(), Value::Array(tool_calls));
                }
                output.push(Value::Object(assistant));
            }
            MessageRole::Tool => {
                for result in &message.tool_results {
                    output.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": result.tool_call_id,
                        "content": serialize_tool_result_output(result)?,
                    }));
                }
            }
        }
    }
    Ok(output)
}

pub fn chat_completion_tools(tools: &[ModelTool]) -> Option<Vec<Value>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnthropicMessages {
    pub system: Option<String>,
    pub messages: Vec<Value>,
}

pub fn anthropic_request_messages(
    messages: &[Message],
    model_id: &str,
) -> Result<AnthropicMessages, super::error::TransportError> {
    let mut system_parts = Vec::new();
    let mut output: Vec<(String, Vec<Value>)> = Vec::new();

    let mut push = |role: &str, content: Vec<Value>| {
        if let Some((last_role, last_content)) = output.last_mut() {
            if last_role == role {
                last_content.extend(content);
                return;
            }
        }
        output.push((role.to_string(), content));
    };

    for message in messages {
        match message.role {
            MessageRole::System => {
                if let Some(text) = &message.content_text {
                    system_parts.push(text.clone());
                }
            }
            MessageRole::User => {
                push(
                    "user",
                    vec![serde_json::json!({
                        "type": "text",
                        "text": message.content_text.as_deref().unwrap_or(""),
                    })],
                );
            }
            MessageRole::Assistant => {
                if let Some(native) =
                    native_array(&message.provider_metadata, "anthropic-messages", model_id, "content")
                {
                    push("assistant", native);
                    continue;
                }
                let mut content = Vec::new();
                for part in &message.assistant_parts {
                    match part {
                        AssistantPart::Text { text } => {
                            content.push(serde_json::json!({ "type": "text", "text": text }));
                        }
                        AssistantPart::ToolCall {
                            tool_call_id,
                            tool_name,
                            input,
                        } => {
                            content.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tool_call_id,
                                "name": tool_name,
                                "input": input,
                            }));
                        }
                        AssistantPart::Reasoning { .. } => {}
                    }
                }
                if !content.is_empty() {
                    push("assistant", content);
                }
            }
            MessageRole::Tool => {
                push(
                    "user",
                    message
                        .tool_results
                        .iter()
                        .map(|result| {
                            Ok(serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": result.tool_call_id,
                                "content": serialize_tool_result_output(result)?,
                                "is_error": result.is_error,
                            }))
                        })
                        .collect::<Result<Vec<_>, super::error::TransportError>>()?,
                );
            }
        }
    }

    Ok(AnthropicMessages {
        system: if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        },
        messages: output
            .into_iter()
            .map(|(role, content)| {
                serde_json::json!({
                    "role": role,
                    "content": content,
                })
            })
            .collect(),
    })
}

pub fn anthropic_tools(tools: &[ModelTool]) -> Option<Vec<Value>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect(),
    )
}

fn native_array(
    metadata: &Option<Value>,
    protocol: &str,
    model_id: &str,
    key: &str,
) -> Option<Vec<Value>> {
    let record = native_metadata(metadata, protocol, model_id)?;
    let value = record.get(key)?;
    value.as_array().filter(|array| !array.is_empty()).cloned()
}

fn native_record(
    metadata: &Option<Value>,
    protocol: &str,
    model_id: &str,
    key: &str,
) -> Option<Value> {
    let record = native_metadata(metadata, protocol, model_id)?;
    record.get(key).filter(|value| value.is_object()).cloned()
}

fn native_metadata(metadata: &Option<Value>, protocol: &str, model_id: &str) -> Option<Map<String, Value>> {
    let record = metadata.as_ref()?.as_object()?;
    if record.get("protocol")?.as_str()? != protocol {
        return None;
    }
    if record.get("modelId")?.as_str()? != model_id {
        return None;
    }
    Some(record.clone())
}

pub struct ModelRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ModelTool>,
    pub provider_options: ProviderOptions,
}

#[cfg(test)]
mod tests {
    use super::{chat_completion_messages, AssistantPart, Message, MessageRole};
    use serde_json::json;

    fn assistant(parts: Vec<AssistantPart>) -> Message {
        Message {
            role: MessageRole::Assistant,
            content_text: None,
            assistant_parts: parts,
            tool_results: Vec::new(),
            provider_metadata: None,
        }
    }

    #[test]
    fn skips_empty_assistant_messages_for_chat_completions() {
        let messages = vec![
            Message {
                role: MessageRole::User,
                content_text: Some("first".into()),
                assistant_parts: Vec::new(),
                tool_results: Vec::new(),
                provider_metadata: None,
            },
            assistant(Vec::new()),
            Message {
                role: MessageRole::User,
                content_text: Some("second".into()),
                assistant_parts: Vec::new(),
                tool_results: Vec::new(),
                provider_metadata: None,
            },
        ];

        let wire = chat_completion_messages(&messages, "test-model").expect("wire messages");

        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0], json!({ "role": "user", "content": "first" }));
        assert_eq!(wire[1], json!({ "role": "user", "content": "second" }));
    }

    #[test]
    fn preserves_assistant_tool_calls_without_text() {
        let messages = vec![assistant(vec![AssistantPart::ToolCall {
            tool_call_id: "call-1".into(),
            tool_name: "bash".into(),
            input: json!({ "command": "pwd" }),
        }])];

        let wire = chat_completion_messages(&messages, "test-model").expect("wire messages");

        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "assistant");
        assert!(wire[0]["content"].is_null());
        assert_eq!(wire[0]["tool_calls"][0]["id"], "call-1");
    }
}
