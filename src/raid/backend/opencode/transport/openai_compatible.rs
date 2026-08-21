use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::complete_tool_call::complete_tool_call;
use super::error::TransportError;
use super::messages::StreamPart;
use super::sse::{is_sse_terminal_sentinel, ParsedSseEvent};
use super::stream_json::{parse_sse_json, provider_stream_error};
use super::super::json::{is_record, read_string};
use super::usage::{chat_finish_reason, merge_usage, token_usage_from_openai, FinishReason, TokenUsage};

const INTERLEAVED_REASONING_FIELDS: [&str; 4] =
    ["reasoning", "reasoning_content", "reasoning_details", "reasoning_text"];

#[derive(Default)]
struct ToolCallBuffer {
    id: String,
    name: String,
    arguments: String,
    emitted: bool,
}

pub struct OpenAiCompatibleHandler {
    model_id: String,
    calls: BTreeMap<i64, ToolCallBuffer>,
    text: String,
    reasoning: BTreeMap<String, String>,
    usage: Option<TokenUsage>,
    finish_reason: Option<FinishReason>,
    finished: bool,
    emitted: u64,
}

impl OpenAiCompatibleHandler {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            calls: BTreeMap::new(),
            text: String::new(),
            reasoning: BTreeMap::new(),
            usage: None,
            finish_reason: None,
            finished: false,
            emitted: 0,
        }
    }

    pub fn push(&mut self, event: &ParsedSseEvent) -> Result<Vec<StreamPart>, TransportError> {
        if self.finished || event.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let retryable = self.emitted == 0;
        let payload = parse_sse_json(&event.data, retryable)?;
        if payload.get("error").filter(|value| value.is_object()).is_some() {
            return Err(provider_stream_error(&payload, retryable));
        }
        Ok(self.handle_chunk(&payload))
    }

    pub fn end(&mut self) -> Result<Vec<StreamPart>, TransportError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if self.finish_reason.is_none() {
            if self.emitted == 0 && !self.has_emittable_tool_calls() {
                return Err(TransportError::new(
                    "incomplete-stream",
                    "Chat Completions stream ended without finish_reason.",
                    true,
                ));
            }
            self.finish_reason = Some(if self.has_emittable_tool_calls() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            });
        }
        let pending = self.flush_tool_calls();
        Ok([pending, self.finish_parts()].concat())
    }

    fn handle_chunk(&mut self, payload: &Map<String, Value>) -> Vec<StreamPart> {
        let mut parts = Vec::new();
        self.usage = merge_usage(
            self.usage.clone(),
            token_usage_from_openai(&Value::Object(payload.clone())),
        );
        let choices = payload
            .get("choices")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        for choice in choices {
            let Some(choice) = choice.as_object() else {
                continue;
            };
            if let Some(delta) = choice.get("delta").and_then(|value| value.as_object()) {
                if let Some(content) = delta.get("content").and_then(read_string) {
                    if !content.is_empty() {
                        self.text.push_str(content);
                        self.emitted += 1;
                        parts.push(StreamPart::TextDelta {
                            text: content.to_string(),
                        });
                    }
                }
                if let Some(reasoning) = interleaved_reasoning(delta) {
                    if !reasoning.text.is_empty() {
                        self.reasoning
                            .entry(reasoning.field.clone())
                            .and_modify(|existing| existing.push_str(&reasoning.text))
                            .or_insert_with(|| reasoning.text.clone());
                        self.emitted += 1;
                        parts.push(StreamPart::ReasoningDelta {
                            text: reasoning.text,
                        });
                    }
                }
                self.append_tool_calls(delta.get("tool_calls"));
            }
            if let Some(reason) = choice.get("finish_reason").and_then(read_string) {
                self.finish_reason = Some(chat_finish_reason(
                    Some(reason),
                    self.has_emittable_tool_calls(),
                ));
            }
        }
        parts
    }

    fn append_tool_calls(&mut self, value: Option<&Value>) {
        let Some(entries) = value.and_then(|value| value.as_array()) else {
            return;
        };
        for entry in entries {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            let index = entry
                .get("index")
                .and_then(|value| value.as_i64())
                .unwrap_or(self.calls.len() as i64);
            let buffer = self.calls.entry(index).or_default();
            if let Some(id) = entry.get("id").and_then(read_string) {
                buffer.id.push_str(id);
            }
            if let Some(function) = entry.get("function").and_then(|value| value.as_object()) {
                if let Some(name) = function.get("name").and_then(read_string) {
                    buffer.name.push_str(name);
                }
                if let Some(args) = function.get("arguments").and_then(read_string) {
                    buffer.arguments.push_str(args);
                }
            }
        }
    }

    fn flush_tool_calls(&mut self) -> Vec<StreamPart> {
        let mut parts = Vec::new();
        for (index, buffer) in self.calls.iter_mut() {
            if buffer.emitted {
                continue;
            }
            let Some(call) = complete_tool_call(
                &buffer.id,
                &buffer.name,
                &buffer.arguments,
                *index as usize,
            ) else {
                continue;
            };
            buffer.emitted = true;
            if buffer.id.is_empty() {
                buffer.id = call.tool_call_id.clone();
            }
            if buffer.name.is_empty() {
                buffer.name = call.tool_name.clone();
            }
            self.emitted += 1;
            parts.push(StreamPart::ToolCall {
                tool_call_id: call.tool_call_id,
                tool_name: call.tool_name,
                input: call.input,
            });
        }
        parts
    }

    fn has_emittable_tool_calls(&self) -> bool {
        self.calls.values().any(|buffer| {
            buffer.emitted
                || !buffer.id.is_empty()
                || !buffer.name.is_empty()
                || !buffer.arguments.trim().is_empty()
        })
    }

    fn finish_parts(&mut self) -> Vec<StreamPart> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        self.emitted += 1;
        let mut message = Map::new();
        message.insert(
            "role".into(),
            Value::String("assistant".into()),
        );
        message.insert(
            "content".into(),
            if self.text.is_empty() {
                Value::Null
            } else {
                Value::String(self.text.clone())
            },
        );
        for (field, text) in &self.reasoning {
            message.insert(field.clone(), Value::String(text.clone()));
        }
        let emitted_calls: Vec<_> = self
            .calls
            .iter()
            .filter(|(_, call)| call.emitted)
            .map(|(_, call)| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": call.arguments,
                    }
                })
            })
            .collect();
        if !emitted_calls.is_empty() {
            message.insert("tool_calls".into(), Value::Array(emitted_calls));
        }
        vec![StreamPart::Finish {
            reason: self.finish_reason.unwrap_or(if self.has_emittable_tool_calls() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }),
            usage: self.usage.clone(),
            provider_metadata: Some(json!({
                "protocol": "openai-compatible",
                "modelId": self.model_id,
                "message": message,
            })),
        }]
    }
}

struct InterleavedReasoning {
    field: String,
    text: String,
}

fn interleaved_reasoning(delta: &Map<String, Value>) -> Option<InterleavedReasoning> {
    for field in INTERLEAVED_REASONING_FIELDS {
        if let Some(text) = delta.get(field).and_then(read_string) {
            if !text.is_empty() {
                return Some(InterleavedReasoning {
                    field: field.to_string(),
                    text: text.to_string(),
                });
            }
        }
    }
    None
}

pub fn process_openai_compatible_events(
    model_id: &str,
    events: &[ParsedSseEvent],
) -> Result<Vec<StreamPart>, TransportError> {
    let mut handler = OpenAiCompatibleHandler::new(model_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::opencode::transport::SseParser;

    #[test]
    fn streams_text_and_finish() {
        let mut parser = SseParser::new();
        let events = parser.push(
            br#"data: {"choices":[{"delta":{"content":"hi"}}]}

data: {"choices":[{"finish_reason":"stop"}]}

data: [DONE]

"#,
        );
        let parts = process_openai_compatible_events("test-model", &events).expect("parts");
        assert!(parts.iter().any(|part| matches!(part, StreamPart::TextDelta { .. })));
        assert!(parts.iter().any(|part| matches!(part, StreamPart::Finish { .. })));
    }
}
