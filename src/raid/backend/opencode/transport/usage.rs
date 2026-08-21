use serde_json::Value;

use super::super::json::read_finite_number;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenUsage {
    pub input_tokens: Option<f64>,
    pub output_tokens: Option<f64>,
    pub reasoning_tokens: Option<f64>,
    pub cache_read_tokens: Option<f64>,
    pub cache_write_tokens: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Other,
}

pub fn token_usage_from_openai(value: &Value) -> Option<TokenUsage> {
    let record = value.as_object()?;
    let input = record
        .get("input_tokens")
        .and_then(read_finite_number)
        .or_else(|| record.get("prompt_tokens").and_then(read_finite_number));
    let output = record
        .get("output_tokens")
        .and_then(read_finite_number)
        .or_else(|| record.get("completion_tokens").and_then(read_finite_number));
    let input_details = record
        .get("input_tokens_details")
        .filter(|value| value.is_object())
        .or_else(|| record.get("prompt_tokens_details").filter(|value| value.is_object()));
    let output_details = record
        .get("output_tokens_details")
        .filter(|value| value.is_object())
        .or_else(|| record.get("completion_tokens_details").filter(|value| value.is_object()));
    let reasoning = output_details
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(read_finite_number);
    let cache_read = input_details.and_then(|details| {
        details
            .get("cached_tokens")
            .and_then(read_finite_number)
            .or_else(|| details.get("cache_read_tokens").and_then(read_finite_number))
    });
    let cache_write = input_details
        .and_then(|details| details.get("cache_write_tokens"))
        .and_then(read_finite_number);
    if input.is_none()
        && output.is_none()
        && reasoning.is_none()
        && cache_read.is_none()
        && cache_write.is_none()
    {
        return None;
    }
    Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    })
}

pub fn merge_usage(current: Option<TokenUsage>, next: Option<TokenUsage>) -> Option<TokenUsage> {
    match (current, next) {
        (None, next) => next,
        (current, None) => current,
        (Some(current), Some(next)) => Some(TokenUsage {
            input_tokens: next.input_tokens.or(current.input_tokens),
            output_tokens: next.output_tokens.or(current.output_tokens),
            reasoning_tokens: next.reasoning_tokens.or(current.reasoning_tokens),
            cache_read_tokens: next.cache_read_tokens.or(current.cache_read_tokens),
            cache_write_tokens: next.cache_write_tokens.or(current.cache_write_tokens),
        }),
    }
}

pub fn chat_finish_reason(value: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match value {
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some("tool_calls") | Some("function_call") => FinishReason::ToolCalls,
        Some("stop") => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        }
        _ => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Other
            }
        }
    }
}

pub fn responses_finish_reason(
    status: Option<&str>,
    incomplete_reason: Option<&str>,
    has_tool_calls: bool,
) -> FinishReason {
    if status == Some("incomplete") {
        return match incomplete_reason {
            Some("max_output_tokens") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => {
                if has_tool_calls {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Other
                }
            }
        };
    }
    if status == Some("failed") {
        return FinishReason::Error;
    }
    if has_tool_calls {
        FinishReason::ToolCalls
    } else {
        FinishReason::Stop
    }
}

pub fn anthropic_finish_reason(value: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match value {
        Some("max_tokens") => FinishReason::Length,
        Some("refusal") => FinishReason::ContentFilter,
        Some("tool_use") => FinishReason::ToolCalls,
        Some("end_turn") | Some("stop_sequence") => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        }
        _ => {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Other
            }
        }
    }
}

pub fn anthropic_usage(value: &Value) -> Option<TokenUsage> {
    let record = value.as_object()?;
    let input = record.get("input_tokens").and_then(read_finite_number);
    let output = record.get("output_tokens").and_then(read_finite_number);
    let cache_read = record
        .get("cache_read_input_tokens")
        .and_then(read_finite_number);
    let cache_write = record
        .get("cache_creation_input_tokens")
        .and_then(read_finite_number);
    if input.is_none() && output.is_none() && cache_read.is_none() && cache_write.is_none() {
        return None;
    }
    let total_input = if input.is_none() && cache_read.is_none() && cache_write.is_none() {
        None
    } else {
        Some(input.unwrap_or(0.0) + cache_read.unwrap_or(0.0) + cache_write.unwrap_or(0.0))
    };
    Some(TokenUsage {
        input_tokens: total_input,
        output_tokens: output,
        reasoning_tokens: None,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    })
}
