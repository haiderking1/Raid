use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::complete_tool_call::complete_tool_call;
use super::error::TransportError;
use super::messages::StreamPart;
use super::sse::{is_sse_terminal_sentinel, ParsedSseEvent};
use super::stream_json::{anthropic_provider_stream_error, parse_sse_json};
use super::super::json::{is_record, read_string, snapshot_safe_json};
use super::super::malformed_tool_call::malformed_tool_call_input;
use super::usage::{
    anthropic_finish_reason, anthropic_usage, merge_usage, FinishReason, TokenUsage,
};

#[derive(Clone)]
enum ContentBlock {
    Text {
        text: String,
        stopped: bool,
    },
    Thinking {
        thinking: String,
        signature: String,
        stopped: bool,
    },
    RedactedThinking {
        data: String,
        stopped: bool,
    },
    ToolUse {
        id: String,
        name: String,
        json: String,
        input: Option<Value>,
        emitted: bool,
        stopped: bool,
    },
}

pub struct AnthropicMessagesHandler {
    model_id: String,
    blocks: BTreeMap<i64, ContentBlock>,
    usage: Option<TokenUsage>,
    finish_reason: Option<FinishReason>,
    finished: bool,
    emitted: u64,
}

impl AnthropicMessagesHandler {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            blocks: BTreeMap::new(),
            usage: None,
            finish_reason: None,
            finished: false,
            emitted: 0,
        }
    }

    pub fn push(&mut self, event: &ParsedSseEvent) -> Result<Vec<StreamPart>, TransportError> {
        if event.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        if self.finished {
            return Err(TransportError::new(
                "invalid-stream-order",
                "Anthropic stream emitted data after message_stop.",
                false,
            ));
        }
        let retryable = self.emitted == 0;
        let payload = parse_sse_json(&event.data, retryable)?;
        let event_type = payload
            .get("type")
            .and_then(read_string)
            .unwrap_or(event.event.as_str());
        self.handle(event_type, &payload, retryable)
    }

    pub fn end(&mut self) -> Result<Vec<StreamPart>, TransportError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if self.emitted == 0 && !self.has_emittable_tool_calls() {
            return Err(TransportError::new(
                "incomplete-stream",
                "Model stream ended without message_stop.",
                true,
            ));
        }
        if self.finish_reason.is_none() {
            self.finish_reason = Some(if self.has_emittable_tool_calls() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            });
        }
        Ok([self.flush_tool_calls(), self.finish_parts()].concat())
    }

    fn handle(
        &mut self,
        event_type: &str,
        payload: &Map<String, Value>,
        retryable: bool,
    ) -> Result<Vec<StreamPart>, TransportError> {
        match event_type {
            "message_start" => {
                if let Some(message) = payload.get("message").and_then(|value| value.as_object()) {
                    if let Some(usage) = message.get("usage") {
                        self.usage = merge_usage(self.usage.clone(), anthropic_usage(usage));
                    }
                }
                Ok(Vec::new())
            }
            "content_block_start" => {
                self.start_block(payload);
                Ok(Vec::new())
            }
            "content_block_delta" => Ok(self.delta(payload)),
            "content_block_stop" => self.stop_block(payload),
            "message_delta" => {
                self.usage = merge_usage(
                    self.usage.clone(),
                    payload.get("usage").and_then(anthropic_usage),
                );
                if let Some(delta) = payload.get("delta").and_then(|value| value.as_object()) {
                    if let Some(reason) = delta.get("stop_reason").and_then(read_string) {
                        self.finish_reason = Some(anthropic_finish_reason(
                            Some(reason),
                            self.has_emittable_tool_calls(),
                        ));
                    }
                }
                Ok(Vec::new())
            }
            "message_stop" => {
                if self.finish_reason.is_none() {
                    if self.emitted == 0 && !self.has_emittable_tool_calls() {
                        return Err(TransportError::new(
                            "incomplete-stream",
                            "Anthropic stream ended without a stop reason.",
                            true,
                        ));
                    }
                    self.finish_reason = Some(if self.has_emittable_tool_calls() {
                        FinishReason::ToolCalls
                    } else {
                        FinishReason::Stop
                    });
                }
                Ok([self.flush_tool_calls(), self.finish_parts()].concat())
            }
            "ping" => Ok(Vec::new()),
            "error" => Err(anthropic_provider_stream_error(payload, retryable)),
            _ => {
                if payload.get("error").filter(|value| value.is_object()).is_some() {
                    Err(anthropic_provider_stream_error(payload, retryable))
                } else {
                    Ok(Vec::new())
                }
            }
        }
    }

    fn start_block(&mut self, payload: &Map<String, Value>) {
        let index = payload
            .get("index")
            .and_then(|value| value.as_i64())
            .unwrap_or(self.blocks.len() as i64);
        let block = payload.get("content_block").and_then(|value| value.as_object());
        let block_type = block.and_then(|value| value.get("type")).and_then(read_string);
        match block_type {
            Some("tool_use") => {
                let initial_input = block.and_then(|value| value.get("input")).and_then(|input| {
                    snapshot_safe_json(input)
                        .ok()
                        .or_else(|| {
                            Some(malformed_tool_call_input(
                                input.as_str().unwrap_or(""),
                            ))
                        })
                });
                self.blocks.insert(
                    index,
                    ContentBlock::ToolUse {
                        id: block
                            .and_then(|value| value.get("id"))
                            .and_then(read_string)
                            .unwrap_or("")
                            .to_string(),
                        name: block
                            .and_then(|value| value.get("name"))
                            .and_then(read_string)
                            .unwrap_or("")
                            .to_string(),
                        json: block
                            .and_then(|value| value.get("input"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .to_string(),
                        input: initial_input,
                        emitted: false,
                        stopped: false,
                    },
                );
            }
            Some("thinking") => {
                self.blocks.insert(
                    index,
                    ContentBlock::Thinking {
                        thinking: block
                            .and_then(|value| value.get("thinking"))
                            .and_then(read_string)
                            .unwrap_or("")
                            .to_string(),
                        signature: block
                            .and_then(|value| value.get("signature"))
                            .and_then(read_string)
                            .unwrap_or("")
                            .to_string(),
                        stopped: false,
                    },
                );
            }
            Some("redacted_thinking") => {
                self.blocks.insert(
                    index,
                    ContentBlock::RedactedThinking {
                        data: block
                            .and_then(|value| value.get("data"))
                            .and_then(read_string)
                            .unwrap_or("")
                            .to_string(),
                        stopped: false,
                    },
                );
            }
            Some("text") => {
                self.blocks.insert(
                    index,
                    ContentBlock::Text {
                        text: block
                            .and_then(|value| value.get("text"))
                            .and_then(read_string)
                            .unwrap_or("")
                            .to_string(),
                        stopped: false,
                    },
                );
            }
            _ => {}
        }
    }

    fn delta(&mut self, payload: &Map<String, Value>) -> Vec<StreamPart> {
        let index = payload.get("index").and_then(|value| value.as_i64()).unwrap_or(0);
        let Some(delta) = payload.get("delta").and_then(|value| value.as_object()) else {
            return Vec::new();
        };
        match delta.get("type").and_then(read_string) {
            Some("text_delta") => {
                let Some(text) = delta.get("text").and_then(read_string) else {
                    return Vec::new();
                };
                if text.is_empty() {
                    return Vec::new();
                }
                match self.blocks.get_mut(&index) {
                    Some(ContentBlock::Text { text: existing, .. }) => existing.push_str(text),
                    _ => {
                        self.blocks.insert(
                            index,
                            ContentBlock::Text {
                                text: text.to_string(),
                                stopped: false,
                            },
                        );
                    }
                }
                self.emitted += 1;
                vec![StreamPart::TextDelta {
                    text: text.to_string(),
                }]
            }
            Some("thinking_delta") => {
                let Some(text) = delta.get("thinking").and_then(read_string) else {
                    return Vec::new();
                };
                if text.is_empty() {
                    return Vec::new();
                }
                match self.blocks.get_mut(&index) {
                    Some(ContentBlock::Thinking { thinking, .. }) => thinking.push_str(text),
                    _ => {
                        self.blocks.insert(
                            index,
                            ContentBlock::Thinking {
                                thinking: text.to_string(),
                                signature: String::new(),
                                stopped: false,
                            },
                        );
                    }
                }
                self.emitted += 1;
                vec![StreamPart::ReasoningDelta {
                    text: text.to_string(),
                }]
            }
            Some("signature_delta") => {
                if let Some(ContentBlock::Thinking { signature, .. }) = self.blocks.get_mut(&index) {
                    if let Some(chunk) = delta.get("signature").and_then(read_string) {
                        signature.push_str(chunk);
                    }
                }
                Vec::new()
            }
            Some("input_json_delta") => {
                let partial = delta
                    .get("partial_json")
                    .and_then(read_string)
                    .unwrap_or("");
                match self.blocks.get_mut(&index) {
                    Some(ContentBlock::ToolUse { json, .. }) => json.push_str(partial),
                    _ => {
                        self.blocks.insert(
                            index,
                            ContentBlock::ToolUse {
                                id: String::new(),
                                name: String::new(),
                                json: partial.to_string(),
                                input: None,
                                emitted: false,
                                stopped: false,
                            },
                        );
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn stop_block(&mut self, payload: &Map<String, Value>) -> Result<Vec<StreamPart>, TransportError> {
        let index = payload.get("index").and_then(|value| value.as_i64()).unwrap_or(0);
        let Some(block) = self.blocks.get_mut(&index) else {
            return Ok(Vec::new());
        };
        match block {
            ContentBlock::ToolUse {
                emitted: false,
                ..
            } => {
                let block = block.clone();
                if let ContentBlock::ToolUse { .. } = block {
                    return self.emit_tool_call(block, index as usize);
                }
                Ok(Vec::new())
            }
            other => {
                match other {
                    ContentBlock::Text { stopped, .. }
                    | ContentBlock::Thinking { stopped, .. }
                    | ContentBlock::RedactedThinking { stopped, .. }
                    | ContentBlock::ToolUse { stopped, .. } => *stopped = true,
                }
                Ok(Vec::new())
            }
        }
    }

    fn emit_tool_call(
        &mut self,
        block: ContentBlock,
        fallback_index: usize,
    ) -> Result<Vec<StreamPart>, TransportError> {
        let ContentBlock::ToolUse {
            id,
            name,
            json,
            input,
            emitted,
            ..
        } = block
        else {
            return Ok(Vec::new());
        };
        if emitted {
            return Ok(Vec::new());
        }
        let arguments = if json.trim().is_empty() { "" } else { json.as_str() };
        if let Some(call) = complete_tool_call(&id, &name, arguments, fallback_index) {
            let tool_call_id = if id.is_empty() {
                call.tool_call_id.clone()
            } else {
                id.clone()
            };
            let tool_name = if name.is_empty() {
                call.tool_name.clone()
            } else {
                name.clone()
            };
            let mut final_input = call.input;
            if json.trim().is_empty() {
                if let Some(initial) = input {
                    final_input = initial;
                }
            }
            self.blocks.insert(
                fallback_index as i64,
                ContentBlock::ToolUse {
                    id: tool_call_id.clone(),
                    name: tool_name.clone(),
                    json,
                    input: Some(final_input.clone()),
                    emitted: true,
                    stopped: true,
                },
            );
            self.emitted += 1;
            return Ok(vec![StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input: final_input,
            }]);
        }
        if let Some(initial) = input {
            let tool_call_id = if id.is_empty() {
                format!("incomplete-call-{fallback_index}")
            } else {
                id.clone()
            };
            let tool_name = if name.is_empty() {
                "unknown".into()
            } else {
                name.clone()
            };
            self.blocks.insert(
                fallback_index as i64,
                ContentBlock::ToolUse {
                    id: tool_call_id.clone(),
                    name: tool_name.clone(),
                    json,
                    input: Some(initial.clone()),
                    emitted: true,
                    stopped: true,
                },
            );
            self.emitted += 1;
            return Ok(vec![StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input: initial,
            }]);
        }
        Ok(Vec::new())
    }

    fn flush_tool_calls(&mut self) -> Vec<StreamPart> {
        let indexes: Vec<_> = self.blocks.keys().copied().collect();
        let mut parts = Vec::new();
        for index in indexes {
            if let Some(block @ ContentBlock::ToolUse { emitted: false, .. }) =
                self.blocks.get(&index).cloned()
            {
                if let Ok(chunk) = self.emit_tool_call(block, index as usize) {
                    parts.extend(chunk);
                }
            }
        }
        parts
    }

    fn has_emittable_tool_calls(&self) -> bool {
        self.blocks.values().any(|block| match block {
            ContentBlock::ToolUse {
                emitted,
                id,
                name,
                json,
                input,
                ..
            } => {
                *emitted
                    || !id.is_empty()
                    || !name.is_empty()
                    || !json.trim().is_empty()
                    || input.is_some()
            }
            _ => false,
        })
    }

    fn finish_parts(&mut self) -> Vec<StreamPart> {
        if self.finished {
            return Vec::new();
        }
        let content = self.native_content();
        self.finished = true;
        self.emitted += 1;
        vec![StreamPart::Finish {
            reason: self.finish_reason.unwrap_or(if self.has_emittable_tool_calls() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }),
            usage: self.usage.clone(),
            provider_metadata: if content.is_empty() {
                None
            } else {
                Some(json!({
                    "protocol": "anthropic-messages",
                    "modelId": self.model_id,
                    "content": content,
                }))
            },
        }]
    }

    fn native_content(&self) -> Vec<Value> {
        let mut content = Vec::new();
        for block in self.blocks.values() {
            match block {
                ContentBlock::Text { text, .. } => {
                    content.push(json!({ "type": "text", "text": text }));
                }
                ContentBlock::Thinking {
                    thinking,
                    signature,
                    ..
                } if !signature.is_empty() => {
                    content.push(json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": signature,
                    }));
                }
                ContentBlock::RedactedThinking { data, .. } => {
                    content.push(json!({ "type": "redacted_thinking", "data": data }));
                }
                ContentBlock::ToolUse {
                    emitted: true,
                    id,
                    name,
                    input,
                    ..
                } => {
                    content.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input.clone().unwrap_or_else(|| json!({})),
                    }));
                }
                _ => {}
            }
        }
        content
    }
}

pub fn process_anthropic_messages_events(
    model_id: &str,
    events: &[ParsedSseEvent],
) -> Result<Vec<StreamPart>, TransportError> {
    let mut handler = AnthropicMessagesHandler::new(model_id);
    let mut parts = Vec::new();
    for event in events {
        if is_sse_terminal_sentinel(event) {
            break;
        }
        parts.extend(handler.push(event)?);
    }
    parts.extend(handler.end()?);
    Ok(parts)
}
