use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::event_stream::AssistantMessageStream;

pub type ToolExecutionMode = &'static str;
pub const TOOL_EXECUTION_SEQUENTIAL: ToolExecutionMode = "sequential";
pub const TOOL_EXECUTION_PARALLEL: ToolExecutionMode = "parallel";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Pending,
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    #[serde(rename = "cacheRead")]
    pub cache_read: u64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: u64,
    #[serde(rename = "totalTokens", default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cost: UsageCost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

impl TextContent {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            kind: "text".into(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            kind: "toolCall".into(),
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AssistantContent {
    Text(TextContent),
    ToolCall(ToolCall),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    pub role: String,
    pub content: Value,
    pub timestamp: u64,
}

impl UserMessage {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Value::String(text.into()),
            timestamp: now_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub role: String,
    pub content: Vec<AssistantContent>,
    pub api: String,
    pub provider: String,
    pub model: String,
    pub usage: Usage,
    #[serde(rename = "stopReason")]
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "errorMessage")]
    pub error_message: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub role: String,
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub content: Vec<TextContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "addedToolNames")]
    pub added_tool_names: Option<Vec<String>>,
    #[serde(rename = "isError")]
    pub is_error: bool,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

impl AgentMessage {
    pub fn role(&self) -> &str {
        match self {
            Self::User(message) => &message.role,
            Self::Assistant(message) => &message.role,
            Self::ToolResult(message) => &message.role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "toolCallId")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmContext {
    pub system_prompt: Option<String>,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    Start { partial: AssistantMessage },
    TextStart {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        partial: AssistantMessage,
    },
    TextDelta {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        delta: String,
        partial: AssistantMessage,
    },
    TextEnd {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        content: String,
        partial: AssistantMessage,
    },
    ThinkingStart {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        partial: AssistantMessage,
    },
    ThinkingDelta {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        delta: String,
        partial: AssistantMessage,
    },
    ThinkingEnd {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        content: String,
        partial: AssistantMessage,
    },
    ToolcallStart {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        partial: AssistantMessage,
    },
    ToolcallDelta {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        delta: String,
        partial: AssistantMessage,
    },
    ToolcallEnd {
        #[serde(rename = "contentIndex")]
        content_index: u64,
        #[serde(rename = "toolCall")]
        tool_call: ToolCall,
        partial: AssistantMessage,
    },
    Done {
        reason: StopReason,
        message: AssistantMessage,
    },
    Error {
        reason: StopReason,
        error: AssistantMessage,
    },
}

pub type AgentToolCall = ToolCall;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AgentToolResult {
    pub content: Vec<TextContent>,
    pub details: Value,
    pub usage: Option<Usage>,
    pub added_tool_names: Option<Vec<String>>,
    pub terminate: bool,
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn parameters_schema(&self) -> &Value;
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        None
    }
    fn prepare_arguments(&self, args: Value) -> Value {
        args
    }
    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
        cancel: &CancellationToken,
        on_update: Option<Box<dyn Fn(AgentToolResult) + Send + Sync>>,
    ) -> AgentToolResult;
}

#[derive(Clone)]
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<Arc<dyn AgentTool>>,
}

#[derive(Debug, Clone, Default)]
pub struct BeforeToolCallResult {
    pub block: bool,
    pub reason: Option<String>,
    pub terminate: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AfterToolCallResult {
    pub content: Option<Vec<TextContent>>,
    pub details: Option<Value>,
    pub is_error: Option<bool>,
    pub usage: Option<Usage>,
    pub terminate: Option<bool>,
}

#[derive(Clone)]
pub struct BeforeToolCallContext {
    pub assistant_message: AssistantMessage,
    pub tool_call: AgentToolCall,
    pub args: Value,
    pub context: AgentContext,
}

#[derive(Clone)]
pub struct AfterToolCallContext {
    pub assistant_message: AssistantMessage,
    pub tool_call: AgentToolCall,
    pub args: Value,
    pub result: AgentToolResult,
    pub is_error: bool,
    pub context: AgentContext,
}

#[derive(Clone)]
pub struct ShouldStopAfterTurnContext {
    pub message: AssistantMessage,
    pub tool_results: Vec<ToolResultMessage>,
    pub context: AgentContext,
    pub new_messages: Vec<AgentMessage>,
}

#[derive(Clone, Default)]
pub struct AgentLoopTurnUpdate {
    pub context: Option<AgentContext>,
    pub model: Option<Model>,
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StreamOptions {
    pub api_key: Option<String>,
}

pub type ConvertToLlmFn =
    Arc<dyn Fn(Vec<AgentMessage>) -> Pin<Box<dyn Future<Output = Vec<LlmMessage>> + Send>> + Send + Sync>;
pub type TransformContextFn = Arc<
    dyn Fn(Vec<AgentMessage>, CancellationToken) -> Pin<Box<dyn Future<Output = Vec<AgentMessage>> + Send>>
        + Send
        + Sync,
>;
pub type GetApiKeyFn =
    Arc<dyn Fn(&str) -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;
pub type ShouldStopAfterTurnFn = Arc<
    dyn Fn(ShouldStopAfterTurnContext) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync,
>;
pub type PrepareNextTurnFn = Arc<
    dyn Fn(ShouldStopAfterTurnContext) -> Pin<Box<dyn Future<Output = Option<AgentLoopTurnUpdate>> + Send>>
        + Send
        + Sync,
>;
pub type GetSteeringMessagesFn =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Vec<AgentMessage>> + Send>> + Send + Sync>;
pub type GetFollowUpMessagesFn =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Vec<AgentMessage>> + Send>> + Send + Sync>;
pub type BeforeToolCallFn = Arc<
    dyn Fn(BeforeToolCallContext, CancellationToken) -> Pin<Box<dyn Future<Output = Option<BeforeToolCallResult>> + Send>>
        + Send
        + Sync,
>;
pub type AfterToolCallFn = Arc<
    dyn Fn(AfterToolCallContext, CancellationToken) -> Pin<Box<dyn Future<Output = Option<AfterToolCallResult>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct AgentLoopConfig {
    pub model: Model,
    pub convert_to_llm: ConvertToLlmFn,
    pub api_key: Option<String>,
    pub transform_context: Option<TransformContextFn>,
    pub get_api_key: Option<GetApiKeyFn>,
    pub should_stop_after_turn: Option<ShouldStopAfterTurnFn>,
    pub prepare_next_turn: Option<PrepareNextTurnFn>,
    pub get_steering_messages: Option<GetSteeringMessagesFn>,
    pub get_follow_up_messages: Option<GetFollowUpMessagesFn>,
    pub tool_execution: ToolExecutionMode,
    pub before_tool_call: Option<BeforeToolCallFn>,
    pub after_tool_call: Option<AfterToolCallFn>,
    pub reasoning: Option<String>,
}

impl AgentLoopConfig {
    pub fn new(model: Model, convert_to_llm: ConvertToLlmFn) -> Self {
        Self {
            model,
            convert_to_llm,
            api_key: None,
            transform_context: None,
            get_api_key: None,
            should_stop_after_turn: None,
            prepare_next_turn: None,
            get_steering_messages: None,
            get_follow_up_messages: None,
            tool_execution: TOOL_EXECUTION_PARALLEL,
            before_tool_call: None,
            after_tool_call: None,
            reasoning: None,
        }
    }
}

pub type StreamFn = Arc<
    dyn Fn(
            Model,
            LlmContext,
            StreamOptions,
            CancellationToken,
        ) -> Pin<Box<dyn Future<Output = AssistantMessageStream> + Send>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    AgentStart,
    AgentEnd { messages: Vec<AgentMessage> },
    TurnStart,
    TurnEnd {
        message: AgentMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart { message: AgentMessage },
    MessageUpdate {
        message: AgentMessage,
        assistant_message_event: AssistantMessageEvent,
    },
    MessageEnd { message: AgentMessage },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        partial_result: AgentToolResult,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: AgentToolResult,
        is_error: bool,
    },
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn empty_usage() -> Usage {
    Usage {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        total_tokens: 0,
        cost: UsageCost::default(),
    }
}

pub fn tool_calls(message: &AssistantMessage) -> Vec<&ToolCall> {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            AssistantContent::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect()
}

pub fn assistant_message(content: Vec<AssistantContent>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content,
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "mock".into(),
        usage: empty_usage(),
        stop_reason,
        error_message: None,
        timestamp: now_ms(),
    }
}

pub fn identity_convert(messages: Vec<AgentMessage>) -> Vec<LlmMessage> {
    messages
        .into_iter()
        .filter_map(|message| match message {
            AgentMessage::User(user) => Some(LlmMessage {
                role: user.role,
                content: Some(user.content),
                tool_call_id: None,
            }),
            AgentMessage::Assistant(assistant) => Some(LlmMessage {
                role: assistant.role,
                content: Some(Value::String(format!("{:?}", assistant.content))),
                tool_call_id: None,
            }),
            AgentMessage::ToolResult(tool) => Some(LlmMessage {
                role: tool.role,
                content: Some(Value::Array(
                    tool.content.iter().map(|part| serde_json::to_value(part).unwrap_or(Value::Null)).collect(),
                )),
                tool_call_id: Some(tool.tool_call_id),
            }),
        })
        .collect()
}

pub async fn identity_convert_async(messages: Vec<AgentMessage>) -> Vec<LlmMessage> {
    identity_convert(messages)
}

pub fn agent_loop_turn_update_from(
    current_context: &mut AgentContext,
    config: &mut AgentLoopConfig,
    snapshot: AgentLoopTurnUpdate,
) {
    if let Some(context) = snapshot.context {
        *current_context = context;
    }
    if let Some(model) = snapshot.model {
        config.model = model;
    }
    if let Some(thinking_level) = snapshot.thinking_level {
        config.reasoning = if thinking_level == "off" {
            None
        } else {
            Some(thinking_level)
        };
    }
}
