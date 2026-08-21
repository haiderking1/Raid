use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::complete_tool_call::complete_tool_call;
use super::error::TransportError;
use super::messages::StreamPart;
use super::sse::{is_sse_terminal_sentinel, ParsedSseEvent};
use super::stream_json::{parse_sse_json, responses_provider_stream_error};
use super::super::json::{is_record, read_string, snapshot_safe_json};
use super::usage::{
    merge_usage, responses_finish_reason, token_usage_from_openai, FinishReason, TokenUsage,
};

#[derive(Clone)]
struct FunctionCallBuffer {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    emitted: bool,
}

pub struct OpenAiResponsesHandler {
    model_id: String,
    calls: BTreeMap<String, FunctionCallBuffer>,
    calls_by_index: BTreeMap<i64, FunctionCallBuffer>,
    output_items: BTreeMap<i64, Value>,
    usage: Option<TokenUsage>,
    finish_reason: Option<FinishReason>,
    finished: bool,
    emitted: u64,
}

impl OpenAiResponsesHandler {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            calls: BTreeMap::new(),
            calls_by_index: BTreeMap::new(),
            output_items: BTreeMap::new(),
            usage: None,
            finish_reason: None,
            finished: false,
            emitted: 0,
        }
    }

    pub fn push(&mut self, event: &ParsedSseEvent) -> Result<Vec<StreamPart>, TransportError> {
        if event.data.trim().is_empty() || self.finished {
            return Ok(Vec::new());
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
                "Model stream ended without a terminal response event.",
                true,
            ));
        }
        self.finish_reason = Some(
            self.finish_reason.unwrap_or(if self.has_emittable_tool_calls() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }),
        );
        Ok(self.finish_parts())
    }

    fn handle(
        &mut self,
        event_type: &str,
        payload: &Map<String, Value>,
        retryable: bool,
    ) -> Result<Vec<StreamPart>, TransportError> {
        match event_type {
            "response.output_text.delta" => Ok(self.text_delta(payload, "text-delta")),
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                Ok(self.text_delta(payload, "reasoning-delta"))
            }
            "response.output_item.added" => {
                self.add_output_item(payload);
                Ok(Vec::new())
            }
            "response.function_call_arguments.delta" => {
                self.append_arguments(payload, payload.get("delta").and_then(read_string).unwrap_or(""));
                Ok(Vec::new())
            }
            "response.function_call_arguments.done" => self.complete_arguments(payload),
            "response.output_item.done" => self.complete_output_item(payload),
            "response.completed" | "response.incomplete" => {
                self.complete_response(payload, event_type == "response.incomplete", retryable)
            }
            "response.failed" | "error" => Err(responses_provider_stream_error(payload, retryable)),
            _ => {
                if payload.get("error").filter(|value| value.is_object()).is_some() {
                    Err(responses_provider_stream_error(payload, retryable))
                } else {
                    Ok(Vec::new())
                }
            }
        }
    }

    fn text_delta(&mut self, payload: &Map<String, Value>, kind: &str) -> Vec<StreamPart> {
        let Some(text) = payload.get("delta").and_then(read_string) else {
            return Vec::new();
        };
        if text.is_empty() {
            return Vec::new();
        }
        self.emitted += 1;
        vec![match kind {
            "reasoning-delta" => StreamPart::ReasoningDelta { text: text.to_string() },
            _ => StreamPart::TextDelta { text: text.to_string() },
        }]
    }

    fn add_output_item(&mut self, payload: &Map<String, Value>) {
        let Some(item) = payload.get("item").and_then(|value| value.as_object()) else {
            return;
        };
        if item.get("type").and_then(read_string) != Some("function_call") {
            return;
        }
        let item_id = item
            .get("id")
            .and_then(read_string)
            .or_else(|| payload.get("item_id").and_then(read_string))
            .unwrap_or("")
            .to_string();
        let output_index = payload
            .get("output_index")
            .and_then(|value| value.as_i64())
            .unwrap_or(self.calls_by_index.len() as i64);
        let existing = if !item_id.is_empty() {
            self.calls.get(&item_id).cloned()
        } else {
            None
        }
        .or_else(|| self.calls_by_index.get(&output_index).cloned());
        let call_id = item.get("call_id").and_then(read_string).unwrap_or("").to_string();
        let name = item.get("name").and_then(read_string).unwrap_or("").to_string();
        let args = item
            .get("arguments")
            .and_then(read_string)
            .unwrap_or("")
            .to_string();
        if let Some(mut existing) = existing {
            existing.call_id.push_str(&call_id);
            existing.name.push_str(&name);
            existing.arguments.push_str(&args);
            if !item_id.is_empty() {
                self.calls.insert(item_id.clone(), existing.clone());
            }
            self.calls_by_index.insert(output_index, existing);
            return;
        }
        let buffer = FunctionCallBuffer {
            item_id: item_id.clone(),
            call_id,
            name,
            arguments: args,
            emitted: false,
        };
        if !buffer.item_id.is_empty() {
            self.calls.insert(buffer.item_id.clone(), buffer.clone());
        }
        self.calls_by_index.insert(output_index, buffer);
    }

    fn lookup(&self, payload: &Map<String, Value>) -> Option<FunctionCallBuffer> {
        if let Some(item_id) = payload.get("item_id").and_then(read_string) {
            if let Some(existing) = self.calls.get(item_id) {
                return Some(existing.clone());
            }
        }
        payload
            .get("output_index")
            .and_then(|value| value.as_i64())
            .and_then(|index| self.calls_by_index.get(&index).cloned())
    }

    fn append_arguments(&mut self, payload: &Map<String, Value>, delta: &str) {
        if let Some(mut buffer) = self.lookup(payload) {
            buffer.arguments.push_str(delta);
            if let Some(name) = payload.get("name").and_then(read_string) {
                if !name.is_empty() {
                    buffer.name = name.to_string();
                }
            }
            self.store_buffer(payload, buffer);
            return;
        }
        let buffer = FunctionCallBuffer {
            item_id: payload
                .get("item_id")
                .and_then(read_string)
                .unwrap_or("")
                .to_string(),
            call_id: String::new(),
            name: payload.get("name").and_then(read_string).unwrap_or("").to_string(),
            arguments: delta.to_string(),
            emitted: false,
        };
        self.store_buffer(payload, buffer);
    }

    fn store_buffer(&mut self, payload: &Map<String, Value>, buffer: FunctionCallBuffer) {
        if !buffer.item_id.is_empty() {
            self.calls.insert(buffer.item_id.clone(), buffer.clone());
        }
        if let Some(index) = payload.get("output_index").and_then(|value| value.as_i64()) {
            self.calls_by_index.insert(index, buffer);
        }
    }

    fn complete_arguments(&mut self, payload: &Map<String, Value>) -> Result<Vec<StreamPart>, TransportError> {
        self.append_arguments(payload, "");
        let Some(mut buffer) = self.lookup(payload) else {
            return Ok(Vec::new());
        };
        if let Some(name) = payload.get("name").and_then(read_string) {
            if !name.is_empty() {
                buffer.name = name.to_string();
            }
        }
        if let Some(arguments) = payload.get("arguments").and_then(read_string) {
            buffer.arguments = arguments.to_string();
        }
        self.emit_tool_call(&mut buffer, 0)
    }

    fn complete_output_item(&mut self, payload: &Map<String, Value>) -> Result<Vec<StreamPart>, TransportError> {
        let Some(item) = payload.get("item").and_then(|value| value.as_object()) else {
            return Ok(Vec::new());
        };
        if let Some(index) = payload.get("output_index").and_then(|value| value.as_i64()) {
            if let Ok(snapshot) = snapshot_safe_json(&Value::Object(item.clone())) {
                self.output_items.insert(index, snapshot);
            }
        }
        if item.get("type").and_then(read_string) != Some("function_call") {
            return Ok(Vec::new());
        }
        let mut buffer = self.lookup(payload).unwrap_or_else(|| FunctionCallBuffer {
            item_id: item.get("id").and_then(read_string).unwrap_or("").to_string(),
            call_id: item.get("call_id").and_then(read_string).unwrap_or("").to_string(),
            name: item.get("name").and_then(read_string).unwrap_or("").to_string(),
            arguments: item
                .get("arguments")
                .and_then(read_string)
                .unwrap_or("")
                .to_string(),
            emitted: false,
        });
        if buffer.call_id.is_empty() {
            buffer.call_id = item.get("call_id").and_then(read_string).unwrap_or("").to_string();
        }
        if buffer.name.is_empty() {
            buffer.name = item.get("name").and_then(read_string).unwrap_or("").to_string();
        }
        if let Some(arguments) = item.get("arguments").and_then(read_string) {
            if !arguments.is_empty() {
                buffer.arguments = arguments.to_string();
            }
        }
        self.emit_tool_call(&mut buffer, payload.get("output_index").and_then(|v| v.as_i64()).unwrap_or(0) as usize)
    }

    fn complete_response(
        &mut self,
        payload: &Map<String, Value>,
        incomplete: bool,
        retryable: bool,
    ) -> Result<Vec<StreamPart>, TransportError> {
        let response = payload
            .get("response")
            .and_then(|value| value.as_object())
            .unwrap_or(payload);
        self.usage = merge_usage(
            self.usage.clone(),
            token_usage_from_openai(&Value::Object(response.clone())),
        );
        let mut completed = Vec::new();
        if let Some(output) = response.get("output").and_then(|value| value.as_array()) {
            self.output_items.clear();
            for (index, item) in output.iter().enumerate() {
                let Some(item) = item.as_object() else {
                    continue;
                };
                let mut map = Map::new();
                map.extend(item.clone());
                let mut payload = Map::new();
                payload.insert("item".into(), Value::Object(map));
                payload.insert("output_index".into(), json!(index));
                completed.extend(self.complete_output_item(&payload)?);
            }
        }
        let incomplete_reason = response
            .get("incomplete_details")
            .and_then(|value| value.as_object())
            .and_then(|details| details.get("reason"))
            .and_then(read_string);
        self.finish_reason = Some(responses_finish_reason(
            Some(if incomplete {
                "incomplete"
            } else {
                response.get("status").and_then(read_string).unwrap_or("completed")
            }),
            incomplete_reason,
            self.has_tool_calls(),
        ));
        if self.finish_reason == Some(FinishReason::Error) {
            return Err(responses_provider_stream_error(response, retryable));
        }
        Ok([completed, self.finish_parts()].concat())
    }

    fn emit_tool_call(
        &mut self,
        buffer: &mut FunctionCallBuffer,
        fallback_index: usize,
    ) -> Result<Vec<StreamPart>, TransportError> {
        if buffer.emitted {
            return Ok(Vec::new());
        }
        let Some(call) = complete_tool_call(
            &buffer.call_id,
            &buffer.name,
            &buffer.arguments,
            fallback_index,
        ) else {
            return Ok(Vec::new());
        };
        buffer.emitted = true;
        if buffer.call_id.is_empty() {
            buffer.call_id = call.tool_call_id.clone();
        }
        if buffer.name.is_empty() {
            buffer.name = call.tool_name.clone();
        }
        self.emitted += 1;
        Ok(vec![StreamPart::ToolCall {
            tool_call_id: call.tool_call_id,
            tool_name: call.tool_name,
            input: call.input,
        }])
    }

    fn flush_pending_tool_calls(&mut self) -> Vec<StreamPart> {
        let mut parts = Vec::new();
        let mut index = 0usize;
        let by_index: Vec<_> = self.calls_by_index.values().cloned().collect();
        for mut buffer in by_index {
            if let Ok(chunk) = self.emit_tool_call(&mut buffer, index) {
                parts.extend(chunk);
            }
            index += 1;
        }
        let by_id: Vec<_> = self.calls.values().cloned().collect();
        for mut buffer in by_id {
            if let Ok(chunk) = self.emit_tool_call(&mut buffer, index) {
                parts.extend(chunk);
            }
            index += 1;
        }
        parts
    }

    fn has_tool_calls(&self) -> bool {
        self.has_emittable_tool_calls()
    }

    fn has_emittable_tool_calls(&self) -> bool {
        self.calls.values().any(|buffer| self.buffer_is_emittable(buffer))
            || self
                .calls_by_index
                .values()
                .any(|buffer| self.buffer_is_emittable(buffer))
    }

    fn buffer_is_emittable(&self, buffer: &FunctionCallBuffer) -> bool {
        buffer.emitted
            || !buffer.call_id.is_empty()
            || !buffer.name.is_empty()
            || !buffer.arguments.trim().is_empty()
    }

    fn finish_parts(&mut self) -> Vec<StreamPart> {
        if self.finished {
            return Vec::new();
        }
        let pending = self.flush_pending_tool_calls();
        self.finished = true;
        self.emitted += 1;
        let output: Vec<_> = self
            .output_items
            .iter()
            .map(|(_, item)| item.clone())
            .collect();
        let finish = StreamPart::Finish {
            reason: self.finish_reason.unwrap_or(if self.has_emittable_tool_calls() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }),
            usage: self.usage.clone(),
            provider_metadata: if output.is_empty() {
                None
            } else {
                Some(json!({
                    "protocol": "openai-responses",
                    "modelId": self.model_id,
                    "output": output,
                }))
            },
        };
        [pending, vec![finish]].concat()
    }
}

pub fn process_openai_responses_events(
    model_id: &str,
    events: &[ParsedSseEvent],
) -> Result<Vec<StreamPart>, TransportError> {
    let mut handler = OpenAiResponsesHandler::new(model_id);
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
