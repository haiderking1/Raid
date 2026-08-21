use crate::backend::agent::{
    assistant_message, assistant_message_stream, empty_usage, AssistantContent, AssistantMessage,
    AssistantMessageEvent, AssistantMessageStream, Model, StopReason, TextContent, ToolCall,
};
use crate::backend::opencode::transport::{FinishReason, StreamPart, TokenUsage, TransportError};

pub struct StreamPartEmitter {
    stream: AssistantMessageStream,
    model: Model,
    partial: AssistantMessage,
    text_index: Option<u64>,
    thinking_index: Option<u64>,
    tool_index: Option<u64>,
    started: bool,
    text: String,
    thinking: String,
}

impl StreamPartEmitter {
    pub fn new(stream: AssistantMessageStream, model: Model) -> Self {
        let partial = empty_assistant(&model);
        Self {
            stream,
            model,
            partial,
            text_index: None,
            thinking_index: None,
            tool_index: None,
            started: false,
            text: String::new(),
            thinking: String::new(),
        }
    }

    pub fn push_part(&mut self, part: StreamPart) {
        match part {
            StreamPart::TextDelta { text } => {
                self.ensure_started();
                if self.text_index.is_none() {
                    self.text_index = Some(0);
                    self.emit(AssistantMessageEvent::TextStart {
                        content_index: 0,
                        partial: self.partial.clone(),
                    });
                }
                self.text.push_str(&text);
                self.partial.content = vec![AssistantContent::Text(TextContent::new(&self.text))];
                self.emit(AssistantMessageEvent::TextDelta {
                    content_index: self.text_index.unwrap_or(0),
                    delta: text,
                    partial: self.partial.clone(),
                });
            }
            StreamPart::ReasoningDelta { text } => {
                self.ensure_started();
                if self.thinking_index.is_none() {
                    self.thinking_index = Some(0);
                    self.emit(AssistantMessageEvent::ThinkingStart {
                        content_index: 0,
                        partial: self.partial.clone(),
                    });
                }
                self.thinking.push_str(&text);
                self.emit(AssistantMessageEvent::ThinkingDelta {
                    content_index: self.thinking_index.unwrap_or(0),
                    delta: text,
                    partial: self.partial.clone(),
                });
            }
            StreamPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
            } => {
                self.ensure_started();
                let index = self.tool_index.unwrap_or(0);
                self.tool_index = Some(index + 1);
                let tool_call = ToolCall::new(tool_call_id, tool_name, input);
                self.partial.content.push(AssistantContent::ToolCall(tool_call.clone()));
                self.emit(AssistantMessageEvent::ToolcallStart {
                    content_index: index,
                    partial: self.partial.clone(),
                });
                self.emit(AssistantMessageEvent::ToolcallEnd {
                    content_index: index,
                    tool_call,
                    partial: self.partial.clone(),
                });
            }
            StreamPart::Finish {
                reason,
                usage,
                provider_metadata: _,
            } => {
                self.finish_success(reason, usage);
            }
        }
    }

    pub fn finish_success(&mut self, reason: FinishReason, usage: Option<TokenUsage>) {
        self.ensure_started();
        let stop_reason = map_finish_reason(reason);
        let mut message = self.partial.clone();
        message.stop_reason = stop_reason;
        message.usage = usage.map(map_usage).unwrap_or_else(empty_usage);
        if !self.started {
            self.stream.push(AssistantMessageEvent::Start {
                partial: message.clone(),
            });
        }
        self.stream.push(AssistantMessageEvent::Done {
            reason: stop_reason,
            message: message.clone(),
        });
        self.stream.end(Some(message));
    }

    pub fn finish_error(&mut self, message: impl Into<String>) {
        let stop_reason = StopReason::Error;
        let error = assistant_message(
            vec![AssistantContent::Text(TextContent::new(message.into()))],
            stop_reason,
        );
        if !self.started {
            self.stream.push(AssistantMessageEvent::Start {
                partial: error.clone(),
            });
        }
        self.stream.push(AssistantMessageEvent::Error {
            reason: stop_reason,
            error: error.clone(),
        });
        self.stream.end(Some(error));
    }

    pub fn finish_transport_error(&mut self, error: TransportError) {
        self.finish_error(error.message());
    }

    fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.stream.push(AssistantMessageEvent::Start {
            partial: self.partial.clone(),
        });
    }

    fn emit(&self, event: AssistantMessageEvent) {
        self.stream.push(event);
    }
}

fn empty_assistant(model: &Model) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content: Vec::new(),
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        usage: empty_usage(),
        stop_reason: StopReason::Pending,
        error_message: None,
        timestamp: crate::backend::agent::now_ms(),
    }
}

fn map_finish_reason(reason: FinishReason) -> StopReason {
    match reason {
        FinishReason::Stop => StopReason::Stop,
        FinishReason::Length => StopReason::Length,
        FinishReason::ToolCalls => StopReason::ToolUse,
        FinishReason::Error => StopReason::Error,
        FinishReason::ContentFilter | FinishReason::Other => StopReason::Stop,
    }
}

fn map_usage(usage: TokenUsage) -> crate::backend::agent::Usage {
    crate::backend::agent::Usage {
        input: usage.input_tokens.unwrap_or(0.0) as u64,
        output: usage.output_tokens.unwrap_or(0.0) as u64,
        cache_read: usage.cache_read_tokens.unwrap_or(0.0) as u64,
        cache_write: usage.cache_write_tokens.unwrap_or(0.0) as u64,
        total_tokens: usage
            .input_tokens
            .unwrap_or(0.0) as u64
            + usage.output_tokens.unwrap_or(0.0) as u64,
        cost: Default::default(),
    }
}
