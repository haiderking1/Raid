use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures::future::join_all;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::stream_fn::get_default_stream_fn;
use super::types::{
    agent_loop_turn_update_from, AfterToolCallContext, AfterToolCallResult, AgentContext,
    AgentEvent, AgentLoopConfig, AgentMessage, AgentTool, AgentToolCall, AgentToolResult,
    AssistantMessage, AssistantMessageEvent, BeforeToolCallContext,
    LlmContext, ShouldStopAfterTurnContext, StopReason, ToolResultContent,
    ToolCall, ToolResultMessage, TOOL_EXECUTION_SEQUENTIAL,
};
use super::validation::validate_tool_arguments;
use serde_json::Value;

pub type AgentEventSink = Arc<dyn Fn(AgentEvent) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub struct AgentLoopHandle {
    pub events: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    pub result: Pin<Box<dyn Future<Output = Vec<AgentMessage>> + Send>>,
}

pub fn agent_loop(
    prompts: Vec<AgentMessage>,
    context: AgentContext,
    config: AgentLoopConfig,
    cancel: CancellationToken,
    stream_fn: Option<super::types::StreamFn>,
) -> AgentLoopHandle {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let emit: AgentEventSink = Arc::new(move |event| {
        let tx = tx.clone();
        Box::pin(async move {
            let _ = tx.send(event);
        })
    });
    let stream_fn = stream_fn.unwrap_or_else(get_default_stream_fn);
    let result = {
        let emit = emit.clone();
        Box::pin(async move {
            tokio::spawn(async move {
                run_agent_loop(prompts, context, config, emit, cancel, stream_fn).await
            })
            .await
            .expect("agent loop task")
        })
    };
    AgentLoopHandle {
        events: rx,
        result,
    }
}

#[cfg(test)]
pub fn agent_loop_continue(
    context: AgentContext,
    config: AgentLoopConfig,
    cancel: CancellationToken,
    stream_fn: Option<super::types::StreamFn>,
) -> Result<AgentLoopHandle, String> {
    if context.messages.is_empty() {
        return Err("Cannot continue: no messages in context".into());
    }
    if context.messages.last().is_some_and(|message| message.role() == "assistant") {
        return Err("Cannot continue from message role: assistant".into());
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let emit: AgentEventSink = Arc::new(move |event| {
        let tx = tx.clone();
        Box::pin(async move {
            let _ = tx.send(event);
        })
    });
    let stream_fn = stream_fn.unwrap_or_else(get_default_stream_fn);
    let result = {
        let emit = emit.clone();
        Box::pin(async move {
            tokio::spawn(async move {
                run_agent_loop_continue(context, config, emit, cancel, stream_fn).await
            })
            .await
            .expect("agent loop task")
        })
    };
    Ok(AgentLoopHandle {
        events: rx,
        result,
    })
}

pub async fn run_agent_loop(
    prompts: Vec<AgentMessage>,
    context: AgentContext,
    config: AgentLoopConfig,
    emit: AgentEventSink,
    cancel: CancellationToken,
    stream_fn: super::types::StreamFn,
) -> Vec<AgentMessage> {
    let mut new_messages = prompts.clone();
    let current_context = AgentContext {
        system_prompt: context.system_prompt,
        messages: {
            let mut messages = context.messages;
            messages.extend(prompts);
            messages
        },
        tools: context.tools,
    };

    emit(AgentEvent::AgentStart).await;
    emit(AgentEvent::TurnStart).await;
    for prompt in &new_messages {
        emit(AgentEvent::MessageStart {
            message: prompt.clone(),
        })
        .await;
        emit(AgentEvent::MessageEnd {
            message: prompt.clone(),
        })
        .await;
    }

    run_loop(
        current_context,
        &mut new_messages,
        config,
        cancel,
        emit,
        stream_fn,
    )
    .await;
    new_messages
}

#[cfg(test)]
pub async fn run_agent_loop_continue(
    context: AgentContext,
    config: AgentLoopConfig,
    emit: AgentEventSink,
    cancel: CancellationToken,
    stream_fn: super::types::StreamFn,
) -> Vec<AgentMessage> {
    if context.messages.is_empty() {
        panic!("Cannot continue: no messages in context");
    }
    if context.messages.last().is_some_and(|message| message.role() == "assistant") {
        panic!("Cannot continue from message role: assistant");
    }

    let mut new_messages = Vec::new();
    let current_context = context;

    emit(AgentEvent::AgentStart).await;
    emit(AgentEvent::TurnStart).await;

    run_loop(
        current_context,
        &mut new_messages,
        config,
        cancel,
        emit,
        stream_fn,
    )
    .await;
    new_messages
}

async fn run_loop(
    mut current_context: AgentContext,
    new_messages: &mut Vec<AgentMessage>,
    mut config: AgentLoopConfig,
    cancel: CancellationToken,
    emit: AgentEventSink,
    stream_fn: super::types::StreamFn,
) {
    let mut first_turn = true;
    let mut pending_messages = if let Some(get_steering) = &config.get_steering_messages {
        get_steering().await
    } else {
        Vec::new()
    };

    loop {
        let mut has_more_tool_calls = true;

        while has_more_tool_calls || !pending_messages.is_empty() {
            if !first_turn {
                emit(AgentEvent::TurnStart).await;
            } else {
                first_turn = false;
            }

            if !pending_messages.is_empty() {
                for message in pending_messages.drain(..) {
                    emit(AgentEvent::MessageStart {
                        message: message.clone(),
                    })
                    .await;
                    emit(AgentEvent::MessageEnd {
                        message: message.clone(),
                    })
                    .await;
                    current_context.messages.push(message.clone());
                    new_messages.push(message);
                }
            }

            let message = stream_assistant_response(
                &mut current_context,
                &config,
                &cancel,
                emit.clone(),
                stream_fn.clone(),
            )
            .await;
            new_messages.push(AgentMessage::Assistant(message.clone()));

            if matches!(message.stop_reason, StopReason::Error | StopReason::Aborted) {
                emit(AgentEvent::TurnEnd {
                    message: AgentMessage::Assistant(message),
                    tool_results: Vec::new(),
                })
                .await;
                emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                })
                .await;
                return;
            }

            let tool_call_list = super::types::tool_calls(&message)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            let mut tool_results = Vec::new();
            has_more_tool_calls = false;
            if !tool_call_list.is_empty() {
                let executed = if message.stop_reason == StopReason::Length {
                    fail_tool_calls_from_truncated_message(tool_call_list, emit.clone()).await
                } else {
                    execute_tool_calls(
                        &current_context,
                        &message,
                        &config,
                        &cancel,
                        emit.clone(),
                    )
                    .await
                };
                tool_results = executed.messages;
                has_more_tool_calls = !executed.terminate;

                for result in &tool_results {
                    let agent = AgentMessage::ToolResult(result.clone());
                    current_context.messages.push(agent.clone());
                    new_messages.push(agent);
                }
            }

            emit(AgentEvent::TurnEnd {
                message: AgentMessage::Assistant(message.clone()),
                tool_results: tool_results.clone(),
            })
            .await;

            let next_turn_context = ShouldStopAfterTurnContext {
                message: message.clone(),
                tool_results: tool_results.clone(),
                context: current_context.clone(),
                new_messages: new_messages.clone(),
            };
            if let Some(prepare_next_turn) = &config.prepare_next_turn {
                if let Some(snapshot) = prepare_next_turn(next_turn_context.clone()).await {
                    agent_loop_turn_update_from(&mut current_context, &mut config, snapshot);
                }
            }

            if let Some(should_stop) = &config.should_stop_after_turn {
                if should_stop(next_turn_context.clone()).await {
                    emit(AgentEvent::AgentEnd {
                        messages: new_messages.clone(),
                    })
                    .await;
                    return;
                }
            }

            pending_messages = if let Some(get_steering) = &config.get_steering_messages {
                get_steering().await
            } else {
                Vec::new()
            };
        }

        let follow_up_messages = if let Some(get_follow_up) = &config.get_follow_up_messages {
            get_follow_up().await
        } else {
            Vec::new()
        };
        if !follow_up_messages.is_empty() {
            pending_messages = follow_up_messages;
            continue;
        }
        break;
    }

    emit(AgentEvent::AgentEnd {
        messages: new_messages.clone(),
    })
    .await;
}

async fn stream_assistant_response(
    context: &mut AgentContext,
    config: &AgentLoopConfig,
    cancel: &CancellationToken,
    emit: AgentEventSink,
    stream_fn: super::types::StreamFn,
) -> AssistantMessage {
    let mut messages = context.messages.clone();
    if let Some(transform) = &config.transform_context {
        messages = transform(messages, cancel.clone()).await;
    }

    let llm_messages = (config.convert_to_llm)(messages).await;
    let llm_context = LlmContext {
        system_prompt: Some(context.system_prompt.clone()),
        messages: llm_messages,
        tools: context
            .tools
            .iter()
            .map(|tool| super::types::ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters_schema().clone(),
            })
            .collect(),
    };

    let resolved_api_key = if let Some(get_api_key) = &config.get_api_key {
        get_api_key(&config.model.provider).await
    } else {
        None
    };
    let api_key = resolved_api_key.or_else(|| config.api_key.clone());

    let stream = stream_fn(
        config.model.clone(),
        llm_context,
        super::types::StreamOptions {
            api_key,
            max_output_tokens: None,
            provider_options: None,
        },
        cancel.clone(),
    )
    .await;

    let mut partial_message: Option<AssistantMessage> = None;
    let mut added_partial = false;
    let mut stream = stream.into_stream();

    while let Some(event) = stream.next().await {
        match event {
            AssistantMessageEvent::Start { partial } => {
                partial_message = Some(partial.clone());
                context.messages.push(AgentMessage::Assistant(partial.clone()));
                added_partial = true;
                emit(AgentEvent::MessageStart {
                    message: AgentMessage::Assistant(partial),
                })
                .await;
            }
            AssistantMessageEvent::TextStart { ref partial, .. }
            | AssistantMessageEvent::TextDelta { ref partial, .. }
            | AssistantMessageEvent::TextEnd { ref partial, .. }
            | AssistantMessageEvent::ThinkingStart { ref partial, .. }
            | AssistantMessageEvent::ThinkingDelta { ref partial, .. }
            | AssistantMessageEvent::ThinkingEnd { ref partial, .. }
            | AssistantMessageEvent::ToolcallStart { ref partial, .. }
            | AssistantMessageEvent::ToolcallDelta { ref partial, .. }
            | AssistantMessageEvent::ToolcallEnd { ref partial, .. } => {
                if partial_message.is_some() {
                    partial_message = Some(partial.clone());
                    if let Some(last) = context.messages.last_mut() {
                        *last = AgentMessage::Assistant(partial.clone());
                    }
                    emit(AgentEvent::MessageUpdate {
                        message: AgentMessage::Assistant(partial.clone()),
                        assistant_message_event: event,
                    })
                    .await;
                }
            }
            AssistantMessageEvent::Done { message, .. } => {
                if added_partial {
                    if let Some(last) = context.messages.last_mut() {
                        *last = AgentMessage::Assistant(message.clone());
                    }
                } else {
                    context.messages.push(AgentMessage::Assistant(message.clone()));
                }
                if !added_partial {
                    emit(AgentEvent::MessageStart {
                        message: AgentMessage::Assistant(message.clone()),
                    })
                    .await;
                }
                emit(AgentEvent::MessageEnd {
                    message: AgentMessage::Assistant(message.clone()),
                })
                .await;
                return message;
            }
            AssistantMessageEvent::Error { reason, error } => {
                let message = error;
                if added_partial {
                    if let Some(last) = context.messages.last_mut() {
                        *last = AgentMessage::Assistant(message.clone());
                    }
                    emit(AgentEvent::MessageUpdate {
                        message: AgentMessage::Assistant(message.clone()),
                        assistant_message_event: AssistantMessageEvent::Error {
                            reason,
                            error: message.clone(),
                        },
                    })
                    .await;
                } else {
                    context.messages.push(AgentMessage::Assistant(message.clone()));
                    emit(AgentEvent::MessageStart {
                        message: AgentMessage::Assistant(message.clone()),
                    })
                    .await;
                }
                emit(AgentEvent::MessageEnd {
                    message: AgentMessage::Assistant(message.clone()),
                })
                .await;
                return message;
            }
        }
    }

    unreachable!("assistant stream ended without terminal event")
}

struct ExecutedToolCallBatch {
    messages: Vec<ToolResultMessage>,
    terminate: bool,
}

async fn fail_tool_calls_from_truncated_message(
    tool_calls: Vec<AgentToolCall>,
    emit: AgentEventSink,
) -> ExecutedToolCallBatch {
    let mut messages = Vec::new();
    for tool_call in tool_calls {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.arguments.clone(),
        })
        .await;
        let finalized = FinalizedToolCallOutcome {
            tool_call: tool_call.clone(),
            result: error_tool_result(format!(
                "Tool call \"{}\" was not executed: the response hit the output token limit, so its arguments may be truncated. Re-issue the tool call with complete arguments.",
                tool_call.name
            )),
            is_error: true,
        };
        emit_tool_execution_end(&finalized, emit.clone()).await;
        let tool_result_message = create_tool_result_message(&finalized);
        emit_tool_result_message(&tool_result_message, emit.clone()).await;
        messages.push(tool_result_message);
    }
    ExecutedToolCallBatch {
        messages,
        terminate: false,
    }
}

async fn execute_tool_calls(
    current_context: &AgentContext,
    assistant_message: &AssistantMessage,
    config: &AgentLoopConfig,
    cancel: &CancellationToken,
    emit: AgentEventSink,
) -> ExecutedToolCallBatch {
    let tool_calls = super::types::tool_calls(assistant_message)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let has_sequential_tool_call = tool_calls.iter().any(|tool_call| {
        current_context
            .tools
            .iter()
            .find(|tool| tool.name() == tool_call.name)
            .and_then(|tool| tool.execution_mode())
            == Some(TOOL_EXECUTION_SEQUENTIAL)
    });
    if config.tool_execution == TOOL_EXECUTION_SEQUENTIAL || has_sequential_tool_call {
        execute_tool_calls_sequential(current_context, assistant_message, tool_calls, config, cancel, emit).await
    } else {
        execute_tool_calls_parallel(current_context, assistant_message, tool_calls, config, cancel, emit).await
    }
}

struct FinalizedToolCallOutcome {
    tool_call: AgentToolCall,
    result: AgentToolResult,
    is_error: bool,
}

enum FinalizedToolCallEntry {
    Ready(FinalizedToolCallOutcome),
    Pending(Pin<Box<dyn Future<Output = FinalizedToolCallOutcome> + Send>>),
}

fn should_terminate_tool_batch(finalized_calls: &[FinalizedToolCallOutcome]) -> bool {
    !finalized_calls.is_empty() && finalized_calls.iter().all(|entry| entry.result.terminate)
}

async fn execute_tool_calls_sequential(
    current_context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_calls: Vec<AgentToolCall>,
    config: &AgentLoopConfig,
    cancel: &CancellationToken,
    emit: AgentEventSink,
) -> ExecutedToolCallBatch {
    let mut finalized_calls = Vec::new();
    let mut messages = Vec::new();

    for tool_call in tool_calls {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.arguments.clone(),
        })
        .await;

        let preparation = prepare_tool_call(current_context, assistant_message, &tool_call, config, cancel).await;
        let finalized = match preparation {
            ToolPreparation::Immediate { result, is_error } => FinalizedToolCallOutcome {
                tool_call: tool_call.clone(),
                result,
                is_error,
            },
            ToolPreparation::Prepared(prepared) => {
                let executed = execute_prepared_tool_call(prepared, cancel, emit.clone()).await;
                finalize_executed_tool_call(
                    current_context,
                    assistant_message,
                    executed,
                    config,
                    cancel,
                )
                .await
            }
        };

        emit_tool_execution_end(&finalized, emit.clone()).await;
        let tool_result_message = create_tool_result_message(&finalized);
        emit_tool_result_message(&tool_result_message, emit.clone()).await;
        finalized_calls.push(finalized);
        messages.push(tool_result_message);

        if cancel.is_cancelled() {
            break;
        }
    }

    ExecutedToolCallBatch {
        messages,
        terminate: should_terminate_tool_batch(&finalized_calls),
    }
}

async fn execute_tool_calls_parallel(
    current_context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_calls: Vec<AgentToolCall>,
    config: &AgentLoopConfig,
    cancel: &CancellationToken,
    emit: AgentEventSink,
) -> ExecutedToolCallBatch {
    let mut entries: Vec<FinalizedToolCallEntry> = Vec::new();

    for tool_call in tool_calls {
        emit(AgentEvent::ToolExecutionStart {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.arguments.clone(),
        })
        .await;

        let preparation = prepare_tool_call(current_context, assistant_message, &tool_call, config, cancel).await;
        match preparation {
            ToolPreparation::Immediate { result, is_error } => {
                let finalized = FinalizedToolCallOutcome {
                    tool_call: tool_call.clone(),
                    result,
                    is_error,
                };
                emit_tool_execution_end(&finalized, emit.clone()).await;
                entries.push(FinalizedToolCallEntry::Ready(finalized));
                if cancel.is_cancelled() {
                    break;
                }
            }
            ToolPreparation::Prepared(prepared) => {
                let current_context = current_context.clone();
                let assistant_message = assistant_message.clone();
                let config = config.clone();
                let cancel_for_task = cancel.clone();
                let emit = emit.clone();
                entries.push(FinalizedToolCallEntry::Pending(Box::pin(async move {
                    let executed =
                        execute_prepared_tool_call(prepared, &cancel_for_task, emit.clone()).await;
                    let finalized = finalize_executed_tool_call(
                        &current_context,
                        &assistant_message,
                        executed,
                        &config,
                        &cancel_for_task,
                    )
                    .await;
                    emit_tool_execution_end(&finalized, emit).await;
                    finalized
                })));
                if cancel.is_cancelled() {
                    break;
                }
            }
        }
    }

    let ordered = join_all(entries.into_iter().map(|entry| async move {
        match entry {
            FinalizedToolCallEntry::Ready(finalized) => finalized,
            FinalizedToolCallEntry::Pending(future) => future.await,
        }
    }))
    .await;

    let mut messages = Vec::new();
    for finalized in &ordered {
        let tool_result_message = create_tool_result_message(finalized);
        emit_tool_result_message(&tool_result_message, emit.clone()).await;
        messages.push(tool_result_message);
    }

    ExecutedToolCallBatch {
        messages,
        terminate: should_terminate_tool_batch(&ordered),
    }
}

struct PreparedToolCall {
    tool_call: AgentToolCall,
    tool: Arc<dyn AgentTool>,
    args: serde_json::Value,
}

enum ToolPreparation {
    Prepared(PreparedToolCall),
    Immediate {
        result: AgentToolResult,
        is_error: bool,
    },
}

struct ExecutedPreparedToolCall {
    tool_call: AgentToolCall,
    args: serde_json::Value,
    result: AgentToolResult,
    is_error: bool,
}

fn prepare_tool_call_arguments(tool: &dyn AgentTool, tool_call: &AgentToolCall) -> AgentToolCall {
    let prepared_arguments = tool.prepare_arguments(tool_call.arguments.clone());
    if prepared_arguments == tool_call.arguments {
        return tool_call.clone();
    }
    ToolCall {
        kind: tool_call.kind.clone(),
        id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        arguments: prepared_arguments,
    }
}

async fn prepare_tool_call(
    current_context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_call: &AgentToolCall,
    config: &AgentLoopConfig,
    cancel: &CancellationToken,
) -> ToolPreparation {
    let Some(tool) = current_context.tools.iter().find(|tool| tool.name() == tool_call.name) else {
        return ToolPreparation::Immediate {
            result: error_tool_result(format!("Tool {} not found", tool_call.name)),
            is_error: true,
        };
    };

    let prepared_tool_call = prepare_tool_call_arguments(tool.as_ref(), tool_call);
    match validate_tool_arguments(tool.as_ref(), &prepared_tool_call) {
        Ok(validated_args) => {
            if let Some(before_tool_call) = &config.before_tool_call {
                let before = before_tool_call(
                    BeforeToolCallContext {
                        assistant_message: assistant_message.clone(),
                        tool_call: tool_call.clone(),
                        args: validated_args.clone(),
                        context: current_context.clone(),
                    },
                    cancel.clone(),
                )
                .await;
                if cancel.is_cancelled() {
                    return ToolPreparation::Immediate {
                        result: error_tool_result("Operation aborted"),
                        is_error: true,
                    };
                }
                if let Some(before) = before {
                    if before.block {
                        let mut result = error_tool_result(
                            before
                                .reason
                                .unwrap_or_else(|| "Tool execution was blocked".into()),
                        );
                        result.terminate = before.terminate;
                        return ToolPreparation::Immediate {
                            result,
                            is_error: true,
                        };
                    }
                }
            }
            if cancel.is_cancelled() {
                return ToolPreparation::Immediate {
                    result: error_tool_result("Operation aborted"),
                    is_error: true,
                };
            }
            ToolPreparation::Prepared(PreparedToolCall {
                tool_call: tool_call.clone(),
                tool: tool.clone(),
                args: validated_args,
            })
        }
        Err(error) => ToolPreparation::Immediate {
            result: error_tool_result(error),
            is_error: true,
        },
    }
}

async fn execute_prepared_tool_call(
    prepared: PreparedToolCall,
    cancel: &CancellationToken,
    emit: AgentEventSink,
) -> ExecutedPreparedToolCall {
    let accepting = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let on_update = {
        let emit = emit.clone();
        let tool_call = prepared.tool_call.clone();
        let accepting = accepting.clone();
        Box::new(move |partial_result: AgentToolResult| {
            if !accepting.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let emit = emit.clone();
            let tool_call = tool_call.clone();
            tokio::spawn(async move {
                emit(AgentEvent::ToolExecutionUpdate {
                    tool_call_id: tool_call.id,
                    tool_name: tool_call.name,
                    args: tool_call.arguments,
                    partial_result,
                })
                .await;
            });
        }) as Box<dyn Fn(AgentToolResult) + Send + Sync>
    };

    let result = prepared
        .tool
        .execute(
            &prepared.tool_call.id,
            prepared.args.clone(),
            cancel,
            Some(on_update),
        )
        .await;
    accepting.store(false, std::sync::atomic::Ordering::Relaxed);

    let is_error = result.is_error;
    ExecutedPreparedToolCall {
        tool_call: prepared.tool_call,
        args: prepared.args,
        result,
        is_error,
    }
}

async fn finalize_executed_tool_call(
    current_context: &AgentContext,
    assistant_message: &AssistantMessage,
    executed: ExecutedPreparedToolCall,
    config: &AgentLoopConfig,
    cancel: &CancellationToken,
) -> FinalizedToolCallOutcome {
    let mut result = executed.result;
    let mut is_error = executed.is_error;

    if let Some(after_tool_call) = &config.after_tool_call {
        match after_tool_call(
            AfterToolCallContext {
                assistant_message: assistant_message.clone(),
                tool_call: executed.tool_call.clone(),
                args: executed.args.clone(),
                result: result.clone(),
                is_error,
                context: current_context.clone(),
            },
            cancel.clone(),
        )
        .await
        {
            Some(after) => apply_after_tool_call(&mut result, &mut is_error, after),
            None => {}
        }
    }

    FinalizedToolCallOutcome {
        tool_call: executed.tool_call,
        result,
        is_error,
    }
}

fn apply_after_tool_call(result: &mut AgentToolResult, is_error: &mut bool, after: AfterToolCallResult) {
    if let Some(content) = after.content {
        result.content = content;
    }
    if let Some(details) = after.details {
        result.details = details;
    }
    if let Some(value) = after.is_error {
        *is_error = value;
    }
    if let Some(usage) = after.usage {
        result.usage = Some(usage);
    }
    if let Some(terminate) = after.terminate {
        result.terminate = terminate;
    }
}

fn error_tool_result(message: impl Into<String>) -> AgentToolResult {
    AgentToolResult {
        content: vec![ToolResultContent::text(message)],
        details: Value::Null,
        usage: None,
        added_tool_names: None,
        terminate: false,
        is_error: true,
    }
}

async fn emit_tool_execution_end(finalized: &FinalizedToolCallOutcome, emit: AgentEventSink) {
    emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: finalized.tool_call.id.clone(),
        tool_name: finalized.tool_call.name.clone(),
        result: finalized.result.clone(),
        is_error: finalized.is_error,
    })
    .await;
}

fn create_tool_result_message(finalized: &FinalizedToolCallOutcome) -> ToolResultMessage {
    ToolResultMessage {
        role: "toolResult".into(),
        tool_call_id: finalized.tool_call.id.clone(),
        tool_name: finalized.tool_call.name.clone(),
        content: finalized.result.content.clone(),
        details: Some(finalized.result.details.clone()),
        usage: finalized.result.usage.clone(),
        added_tool_names: finalized.result.added_tool_names.clone(),
        is_error: finalized.is_error,
        timestamp: super::types::now_ms(),
    }
}

async fn emit_tool_result_message(tool_result_message: &ToolResultMessage, emit: AgentEventSink) {
    emit(AgentEvent::MessageStart {
        message: AgentMessage::ToolResult(tool_result_message.clone()),
    })
    .await;
    emit(AgentEvent::MessageEnd {
        message: AgentMessage::ToolResult(tool_result_message.clone()),
    })
    .await;
}
