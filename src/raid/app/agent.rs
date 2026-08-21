use std::collections::HashMap;
use std::sync::Arc;

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::backend::agent::{
    agent_loop, set_default_stream_fn, AgentContext, AgentEvent, AgentLoopConfig, AgentLoopHandle,
    AgentMessage, AssistantContent, AssistantMessage, Model, StreamFn, UserMessage,
};
use crate::config::{resolve_api_key, RaidSettings, DEFAULT_MODEL, DEFAULT_PROVIDER};
use crate::frontend::chat::ViewportState;
use crate::frontend::tools::ToolStatus;
use serde_json::Value;

pub struct AgentSession {
    handle: Option<AgentLoopHandle>,
    context: AgentContext,
    config: AgentLoopConfig,
    cancel: CancellationToken,
    tool_indices: HashMap<String, usize>,
    assistant_index: Option<usize>,
    runtime: Handle,
    stream_fn: Option<StreamFn>,
}

impl AgentSession {
    pub fn new(runtime: Handle) -> Self {
        let settings = RaidSettings::load();
        let provider_id = settings.provider_id();
        let model_id = settings.model_id().to_string();
        let mut config = AgentLoopConfig::new(
            Model {
                id: model_id.clone(),
                name: model_id,
                api: settings.api().into(),
                provider: provider_id.to_string(),
            },
            Arc::new(crate::backend::opencode::convert_agent_messages),
        );
        config.api_key = resolve_api_key(provider_id);
        Self {
            handle: None,
            context: AgentContext {
                system_prompt: String::from("You are a helpful coding agent in a terminal UI."),
                messages: Vec::new(),
                tools: Vec::new(),
            },
            config,
            cancel: CancellationToken::new(),
            tool_indices: HashMap::new(),
            assistant_index: None,
            runtime,
            stream_fn: None,
        }
    }

    pub fn reload_credentials(&mut self) {
        let settings = RaidSettings::load();
        let provider_id = settings.provider_id();
        let model_id = settings.model_id().to_string();
        self.config.model.id = model_id.clone();
        self.config.model.name = model_id;
        self.config.model.api = settings.api().into();
        self.config.model.provider = provider_id.to_string();
        self.config.api_key = resolve_api_key(provider_id);
    }

    #[cfg(test)]
    pub fn with_stream_fn(mut self, stream_fn: StreamFn) -> Self {
        self.stream_fn = Some(stream_fn);
        self
    }

    pub fn is_running(&self) -> bool {
        self.handle.is_some()
    }

    pub fn submit(&mut self, message: String) {
        if self.is_running() {
            return;
        }
        self.cancel = CancellationToken::new();
        self.tool_indices.clear();
        self.assistant_index = None;
        let prompt = AgentMessage::User(UserMessage::new(message));
        self.context.messages.push(prompt.clone());
        let handle = agent_loop(
            vec![prompt],
            self.context.clone(),
            self.config.clone(),
            self.cancel.clone(),
            self.stream_fn.clone(),
        );
        self.handle = Some(handle);
    }

    pub fn poll(&mut self, chat: &mut ViewportState) {
        let Some(mut handle) = self.handle.take() else {
            return;
        };

        loop {
            match handle.events.try_recv() {
                Ok(event) => self.apply_event(chat, event),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    self.handle = Some(handle);
                    return;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    let _ = self.runtime.block_on(handle.result);
                    self.handle = None;
                    return;
                }
            }
        }
    }

    fn apply_event(&mut self, chat: &mut ViewportState, event: AgentEvent) {
        match event {
            AgentEvent::MessageStart { message } => self.on_message_start(chat, message),
            AgentEvent::MessageUpdate { message, .. } => self.on_message_update(chat, message),
            AgentEvent::MessageEnd { message } => self.on_message_end(message),
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let index = chat.start_tool(tool_name, format_tool_args(&args));
                self.tool_indices.insert(tool_call_id, index);
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name: _,
                result,
                is_error,
            } => {
                if let Some(index) = self.tool_indices.get(&tool_call_id).copied() {
                    let summary = result
                        .content
                        .first()
                        .map(|part| part.text.clone())
                        .unwrap_or_else(|| "Done".into());
                    chat.finish_tool(
                        index,
                        if is_error {
                            ToolStatus::Failed
                        } else {
                            ToolStatus::Success
                        },
                        summary,
                    );
                }
            }
            AgentEvent::AgentEnd { messages } => {
                for message in messages {
                    if !matches!(message, AgentMessage::User(_)) {
                        self.context.messages.push(message);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_message_start(&mut self, chat: &mut ViewportState, message: AgentMessage) {
        if let AgentMessage::Assistant(assistant) = message {
            self.assistant_index = Some(chat.append_assistant(assistant_text(&assistant)));
        }
    }

    fn on_message_update(&mut self, chat: &mut ViewportState, message: AgentMessage) {
        if let AgentMessage::Assistant(assistant) = message {
            if let Some(index) = self.assistant_index {
                chat.update_assistant(index, assistant_text(&assistant));
            }
        }
    }

    fn on_message_end(&mut self, message: AgentMessage) {
        if matches!(message, AgentMessage::Assistant(_)) {
            self.assistant_index = None;
        }
    }

    pub fn drive_to_completion(&mut self, chat: &mut ViewportState) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        let (events, _messages) = self.runtime.block_on(collect_agent_events(handle));
        for event in events {
            self.apply_event(chat, event);
        }
        self.handle = None;
    }
}

async fn collect_agent_events(
    mut handle: AgentLoopHandle,
) -> (Vec<AgentEvent>, Vec<AgentMessage>) {
    let mut events = Vec::new();
    let mut result = handle.result;
    loop {
        tokio::select! {
            event = handle.events.recv() => {
                match event {
                    Some(event) => events.push(event),
                    None => break,
                }
            }
            messages = &mut result => {
                while let Some(event) = handle.events.recv().await {
                    events.push(event);
                }
                return (events, messages);
            }
        }
    }
    (events, result.await)
}

fn assistant_text(message: &AssistantMessage) -> String {
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

fn format_tool_args(args: &Value) -> String {
    serde_json::to_string(args).unwrap_or_else(|_| "{}".into())
}

pub fn install_default_stream_fn() {
    use crate::backend::opencode::{opencode_stream_fn, OpenCodeStreamConfig};
    let client = reqwest::Client::new();
    let config = OpenCodeStreamConfig {
        plan: crate::config::plan_for_provider_id(DEFAULT_PROVIDER),
        client,
        output_limit: 8192,
    };
    set_default_stream_fn(Some(opencode_stream_fn(config)));
}

#[cfg(test)]
pub fn test_stream_fn() -> StreamFn {
    use crate::backend::agent::{
        assistant_message, assistant_message_stream, AssistantMessageEvent, StopReason, TextContent,
    };
    Arc::new(|_model, _context, _options, _cancel| {
        Box::pin(async move {
            let stream = assistant_message_stream();
            let message = assistant_message(
                vec![AssistantContent::Text(TextContent::new("mock reply"))],
                StopReason::Stop,
            );
            stream.push(AssistantMessageEvent::Done {
                reason: StopReason::Stop,
                message: message.clone(),
            });
            stream.end(Some(message));
            stream
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{test_stream_fn, AgentSession};
    use crate::frontend::chat::{Role, ViewportState};

    #[test]
    fn session_collects_assistant_reply() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut chat = ViewportState::default();
        let mut session = AgentSession::new(rt.handle().clone()).with_stream_fn(test_stream_fn());
        session.submit("hello".into());
        session.drive_to_completion(&mut chat);
        assert_eq!(chat.last_role(), Some(Role::Assistant));
    }
}

#[cfg(test)]
pub fn install_test_stream_fn() {
    set_default_stream_fn(Some(test_stream_fn()));
}
