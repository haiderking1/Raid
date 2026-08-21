pub const MALFORMED_TOOL_ARGUMENTS_FLAG: &str = "__malformedToolArguments";
pub const MALFORMED_TOOL_ARGUMENTS_MESSAGE: &str = "Tool-call arguments were not valid JSON.";
const RAW_PREVIEW_LIMIT: usize = 512;

use serde_json::{json, Value};

pub fn malformed_tool_call_input(raw: &str) -> Value {
    let preview = if raw.len() > RAW_PREVIEW_LIMIT {
        &raw[..RAW_PREVIEW_LIMIT]
    } else {
        raw
    };
    json!({
        MALFORMED_TOOL_ARGUMENTS_FLAG: true,
        "error": MALFORMED_TOOL_ARGUMENTS_MESSAGE,
        "rawPreview": preview,
    })
}

