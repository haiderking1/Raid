use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{
    agent_loop, agent_loop_continue, set_default_stream_fn, AgentContext, AgentEvent, AgentLoopConfig,
    AgentMessage, AgentTool, AssistantContent, AssistantMessageEvent, Model, StopReason,
    TextContent, ToolCall, ToolResultContent, UserMessage, TOOL_EXECUTION_PARALLEL, StreamFn,
};
use super::{
    assistant_message, assistant_message_stream, identity_convert_async, AgentLoopHandle,
    AgentToolResult,
};

fn echo_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    })
}

struct EchoTool {
    name: &'static str,
    execution_mode: Option<&'static str>,
    executed: Arc<Mutex<Vec<String>>>,
    prepare: Option<Arc<dyn Fn(Value) -> Value + Send + Sync>>,
    terminate: bool,
}

#[async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.name
    }

    fn parameters_schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(echo_schema)
    }

    fn execution_mode(&self) -> Option<&'static str> {
        self.execution_mode
    }

    fn prepare_arguments(&self, args: Value) -> Value {
        self.prepare
            .as_ref()
            .map(|prepare| prepare(args.clone()))
            .unwrap_or(args)
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        _cancel: &CancellationToken,
        _on_update: Option<Box<dyn Fn(AgentToolResult) + Send + Sync>>,
    ) -> AgentToolResult {
        let value = args
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.executed.lock().expect("executed lock").push(value.clone());
        AgentToolResult {
            content: vec![ToolResultContent::text(format!("echoed: {value}"))],
            details: json!({ "value": value }),
            usage: None,
            added_tool_names: None,
            terminate: self.terminate,
            is_error: false,
        }
    }
}

fn mock_model() -> Model {
    Model {
        id: "mock".into(),
        name: "mock".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
    }
}

fn identity_config() -> AgentLoopConfig {
    AgentLoopConfig::new(
        mock_model(),
        Arc::new(|messages| Box::pin(identity_convert_async(messages))),
    )
}

fn mock_stream_fn(
    responses: Arc<Mutex<Vec<super::AssistantMessage>>>,
) -> StreamFn {
    Arc::new(move |_model, _context, _options, _cancel| {
        let responses = responses.clone();
        Box::pin(async move {
            let message = responses.lock().expect("responses lock").remove(0);
            let stream = assistant_message_stream();
            let done = AssistantMessageEvent::Done {
                reason: message.stop_reason,
                message: message.clone(),
            };
            stream.push(done);
            stream.end(Some(message));
            stream
        })
    })
}

async fn collect_loop(mut handle: AgentLoopHandle) -> (Vec<AgentEvent>, Vec<AgentMessage>) {
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
    let messages = result.await;
    (events, messages)
}

#[tokio::test]
async fn uses_configured_default_when_stream_fn_omitted() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_default = calls.clone();
    set_default_stream_fn(Some(Arc::new(move |_model, _context, _options, _cancel| {
        calls_for_default.fetch_add(1, Ordering::SeqCst);
        let stream = assistant_message_stream();
        let message = assistant_message(
            vec![AssistantContent::Text(TextContent::new("fallback"))],
            StopReason::Stop,
        );
        stream.push(AssistantMessageEvent::Done {
            reason: StopReason::Stop,
            message: message.clone(),
        });
        stream.end(Some(message));
        Box::pin(async move { stream })
    })));

    let context = AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
    };
    let handle = agent_loop(
        vec![AgentMessage::User(UserMessage::new("Hello"))],
        context,
        identity_config(),
        CancellationToken::new(),
        None,
    );
    let _ = collect_loop(handle).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    set_default_stream_fn(None);
}

#[tokio::test]
async fn emits_agent_message_events() {
    let responses = Arc::new(Mutex::new(vec![assistant_message(
        vec![AssistantContent::Text(TextContent::new("Hi there!"))],
        StopReason::Stop,
    )]));
    let context = AgentContext {
        system_prompt: "You are helpful.".into(),
        messages: Vec::new(),
        tools: Vec::new(),
    };
    let handle = agent_loop(
        vec![AgentMessage::User(UserMessage::new("Hello"))],
        context,
        identity_config(),
        CancellationToken::new(),
        Some(mock_stream_fn(responses)),
    );
    let (events, messages) = collect_loop(handle).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role(), "user");
    assert_eq!(messages[1].role(), "assistant");
    let event_types = events
        .iter()
        .map(|event| match event {
            AgentEvent::AgentStart => "agent_start",
            AgentEvent::AgentEnd { .. } => "agent_end",
            AgentEvent::TurnStart => "turn_start",
            AgentEvent::TurnEnd { .. } => "turn_end",
            AgentEvent::MessageStart { .. } => "message_start",
            AgentEvent::MessageEnd { .. } => "message_end",
            _ => "other",
        })
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"agent_start"));
    assert!(event_types.contains(&"turn_start"));
    assert!(event_types.contains(&"message_start"));
    assert!(event_types.contains(&"message_end"));
    assert!(event_types.contains(&"turn_end"));
    assert!(event_types.contains(&"agent_end"));
}

#[tokio::test]
async fn executes_tool_calls_and_emits_tool_events() {
    let executed = Arc::new(Mutex::new(Vec::new()));
    let tool = Arc::new(EchoTool {
        name: "echo",
        execution_mode: None,
        executed: executed.clone(),
        prepare: None,
        terminate: false,
    });
    let responses = Arc::new(Mutex::new(vec![
        assistant_message(
            vec![AssistantContent::ToolCall(ToolCall::new(
                "tool-1",
                "echo",
                json!({ "value": "hello" }),
            ))],
            StopReason::ToolUse,
        ),
        assistant_message(
            vec![AssistantContent::Text(TextContent::new("done"))],
            StopReason::Stop,
        ),
    ]));
    let context = AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: vec![tool],
    };
    let handle = agent_loop(
        vec![AgentMessage::User(UserMessage::new("echo something"))],
        context,
        identity_config(),
        CancellationToken::new(),
        Some(mock_stream_fn(responses)),
    );
    let (events, _messages) = collect_loop(handle).await;
    assert_eq!(*executed.lock().expect("executed lock"), vec!["hello"]);
    assert!(events.iter().any(|event| matches!(event, AgentEvent::ToolExecutionStart { .. })));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolExecutionEnd { is_error: false, .. }
    )));
}

#[tokio::test]
async fn does_not_execute_length_truncated_tool_calls() {
    let executed = Arc::new(Mutex::new(Vec::<String>::new()));
    let tool = Arc::new(EchoTool {
        name: "echo",
        execution_mode: None,
        executed: executed.clone(),
        prepare: None,
        terminate: false,
    });
    let responses = Arc::new(Mutex::new(vec![
        assistant_message(
            vec![AssistantContent::ToolCall(ToolCall::new(
                "tool-1",
                "echo",
                json!({ "value": "hel" }),
            ))],
            StopReason::Length,
        ),
        assistant_message(
            vec![AssistantContent::Text(TextContent::new("done"))],
            StopReason::Stop,
        ),
    ]));
    let context = AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: vec![tool],
    };
    let handle = agent_loop(
        vec![AgentMessage::User(UserMessage::new("echo something"))],
        context,
        identity_config(),
        CancellationToken::new(),
        Some(mock_stream_fn(responses)),
    );
    let (events, messages) = collect_loop(handle).await;
    assert!(executed.lock().expect("executed lock").is_empty());
    let tool_end = events.iter().find_map(|event| match event {
        AgentEvent::ToolExecutionEnd { is_error, result, .. } if *is_error => Some(result.content.clone()),
        _ => None,
    });
    assert!(tool_end.is_some_and(|content| {
        content
            .first()
            .and_then(|part| part.as_text())
            .is_some_and(|text| text.contains("output token limit"))
    }));
    assert_eq!(messages.last().map(|message| message.role()), Some("assistant"));
}

#[tokio::test]
async fn agent_loop_continue_rejects_empty_context() {
    let context = AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
    };
    assert!(agent_loop_continue(
        context,
        identity_config(),
        CancellationToken::new(),
        Some(mock_stream_fn(Arc::new(Mutex::new(Vec::new())))),
    )
    .is_err());
}

#[tokio::test]
async fn agent_loop_continue_returns_only_new_messages() {
    let responses = Arc::new(Mutex::new(vec![assistant_message(
        vec![AssistantContent::Text(TextContent::new("Response"))],
        StopReason::Stop,
    )]));
    let context = AgentContext {
        system_prompt: "You are helpful.".into(),
        messages: vec![AgentMessage::User(UserMessage::new("Hello"))],
        tools: Vec::new(),
    };
    let handle = agent_loop_continue(
        context,
        identity_config(),
        CancellationToken::new(),
        Some(mock_stream_fn(responses)),
    )
    .expect("continue");
    let (events, messages) = collect_loop(handle).await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role(), "assistant");
    let message_ends = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::MessageEnd { .. }))
        .count();
    assert_eq!(message_ends, 1);
}

#[tokio::test]
async fn stops_after_should_stop_after_turn() {
    let executed = Arc::new(Mutex::new(Vec::new()));
    let tool = Arc::new(EchoTool {
        name: "echo",
        execution_mode: None,
        executed: executed.clone(),
        prepare: None,
        terminate: false,
    });
    let responses = Arc::new(Mutex::new(vec![assistant_message(
        vec![AssistantContent::ToolCall(ToolCall::new(
            "tool-1",
            "echo",
            json!({ "value": "hello" }),
        ))],
        StopReason::ToolUse,
    )]));
    let mut config = identity_config();
    config.should_stop_after_turn = Some(Arc::new(|_context| Box::pin(async { true })));
    let context = AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: vec![tool],
    };
    let handle = agent_loop(
        vec![AgentMessage::User(UserMessage::new("echo something"))],
        context,
        config,
        CancellationToken::new(),
        Some(mock_stream_fn(responses)),
    );
    let (events, messages) = collect_loop(handle).await;
    assert_eq!(*executed.lock().expect("executed lock"), vec!["hello"]);
    assert_eq!(
        messages.iter().map(|message| message.role()).collect::<Vec<_>>(),
        vec!["user", "assistant", "toolResult"]
    );
    assert!(events.iter().any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
}

#[tokio::test]
async fn parallel_tool_execution_can_overlap() {
    let executed = Arc::new(Mutex::new(Vec::<String>::new()));
    let first_resolved = Arc::new(Mutex::new(false));
    let parallel_observed = Arc::new(Mutex::new(false));
    let release_first = Arc::new(tokio::sync::Notify::new());

    struct SlowEcho {
        executed: Arc<Mutex<Vec<String>>>,
        first_resolved: Arc<Mutex<bool>>,
        parallel_observed: Arc<Mutex<bool>>,
        release_first: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl AgentTool for SlowEcho {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echo"
        }
        fn parameters_schema(&self) -> &Value {
            static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
            SCHEMA.get_or_init(echo_schema)
        }
        fn execution_mode(&self) -> Option<&'static str> {
            Some(TOOL_EXECUTION_PARALLEL)
        }
        async fn execute(
            &self,
            _tool_call_id: &str,
            args: Value,
            _cancel: &CancellationToken,
            _on_update: Option<Box<dyn Fn(AgentToolResult) + Send + Sync>>,
        ) -> AgentToolResult {
            let value = args["value"].as_str().unwrap_or_default().to_string();
            if value == "first" {
                self.release_first.notified().await;
                *self.first_resolved.lock().expect("first_resolved") = true;
            }
            if value == "second" && !*self.first_resolved.lock().expect("first_resolved") {
                *self.parallel_observed.lock().expect("parallel_observed") = true;
            }
            self.executed.lock().expect("executed").push(value.clone());
            AgentToolResult {
                content: vec![ToolResultContent::text(format!("echoed: {value}"))],
                details: json!({ "value": value }),
                usage: None,
                added_tool_names: None,
                terminate: false,
                is_error: false,
            }
        }
    }

    let tool = Arc::new(SlowEcho {
        executed: executed.clone(),
        first_resolved,
        parallel_observed: parallel_observed.clone(),
        release_first: release_first.clone(),
    });
    let responses = Arc::new(Mutex::new(vec![
        assistant_message(
            vec![
                AssistantContent::ToolCall(ToolCall::new("tool-1", "echo", json!({ "value": "first" }))),
                AssistantContent::ToolCall(ToolCall::new("tool-2", "echo", json!({ "value": "second" }))),
            ],
            StopReason::ToolUse,
        ),
        assistant_message(
            vec![AssistantContent::Text(TextContent::new("done"))],
            StopReason::Stop,
        ),
    ]));
    let mut config = identity_config();
    config.tool_execution = TOOL_EXECUTION_PARALLEL;
    let context = AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: vec![tool],
    };
    let handle = agent_loop(
        vec![AgentMessage::User(UserMessage::new("echo both"))],
        context,
        config,
        CancellationToken::new(),
        Some(mock_stream_fn(responses)),
    );
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        release_first.notify_waiters();
    });
    let _ = collect_loop(handle).await;
    assert!(*parallel_observed.lock().expect("parallel_observed"));
}
