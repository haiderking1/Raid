use serde_json::Value;

use super::super::json::parse_tool_call_arguments;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallPart {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
}

pub fn complete_tool_call(id: &str, name: &str, arguments: &str, fallback_index: usize) -> Option<ToolCallPart> {
    if id.is_empty() && name.is_empty() && arguments.trim().is_empty() {
        return None;
    }
    Some(ToolCallPart {
        tool_call_id: if id.is_empty() {
            format!("incomplete-call-{fallback_index}")
        } else {
            id.to_string()
        },
        tool_name: if name.is_empty() {
            "unknown".into()
        } else {
            name.to_string()
        },
        input: parse_tool_call_arguments(arguments),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_empty_tool_calls() {
        assert!(complete_tool_call("", "", "   ", 0).is_none());
    }

    #[test]
    fn uses_fallback_id_and_name() {
        let call = complete_tool_call("", "", r#"{"x":1}"#, 2).expect("call");
        assert_eq!(call.tool_call_id, "incomplete-call-2");
        assert_eq!(call.tool_name, "unknown");
    }
}
