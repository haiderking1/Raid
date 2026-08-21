use jsonschema::Validator;
use serde_json::Value;

use super::types::{AgentTool, AgentToolCall, ToolDefinition};

pub fn validate_tool_arguments(tool: &dyn AgentTool, tool_call: &AgentToolCall) -> Result<Value, String> {
    validate_tool_arguments_schema(tool.parameters_schema(), tool_call)
}

pub fn validate_tool_arguments_definition(
    tool: &ToolDefinition,
    tool_call: &AgentToolCall,
) -> Result<Value, String> {
    validate_tool_arguments_schema(&tool.parameters, tool_call)
}

pub fn validate_tool_arguments_schema(schema: &Value, tool_call: &AgentToolCall) -> Result<Value, String> {
    let args = tool_call.arguments.clone();
    let validator = Validator::new(schema).map_err(|error| error.to_string())?;
    if let Err(error) = validator.validate(&args) {
        return Err(format!(
            "Validation failed for tool \"{}\":\n  - {}: {}\n\nReceived arguments:\n{}",
            tool_call.name,
            error.instance_path,
            error,
            serde_json::to_string_pretty(&tool_call.arguments).unwrap_or_else(|_| "{}".into())
        ));
    }
    Ok(args)
}
