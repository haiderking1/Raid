use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::backend::agent::{AgentTool, AgentToolResult, ToolResultContent};
use crate::backend::tools::env::ToolEnvironment;
use crate::backend::tools::file_mutation_queue::with_file_mutation_queue;

pub struct WriteTool {
    env: Arc<ToolEnvironment>,
}

impl WriteTool {
    pub fn new(env: Arc<ToolEnvironment>) -> Self {
        Self { env }
    }
}

#[async_trait]
impl AgentTool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories."
    }

    fn parameters_schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write (relative or absolute)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            })
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        cancel: &CancellationToken,
        _on_update: Option<Box<dyn Fn(AgentToolResult) + Send + Sync>>,
    ) -> AgentToolResult {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let absolute_path = self.env.resolve_path(path);

        with_file_mutation_queue(self.env.as_ref(), &absolute_path, || async {
            if cancel.is_cancelled() {
                return error_result("Operation aborted");
            }
            if let Some(parent) = absolute_path.parent() {
                if let Err(error) = tokio::fs::create_dir_all(parent).await {
                    return error_result(error.to_string());
                }
            }
            if let Err(error) = tokio::fs::write(&absolute_path, &content).await {
                return error_result(error.to_string());
            }
            if cancel.is_cancelled() {
                return error_result("Operation aborted");
            }
            success_result(format!(
                "Successfully wrote {} bytes to {path}",
                content.chars().count()
            ))
        })
        .await
    }
}

fn success_result(text: impl Into<String>) -> AgentToolResult {
    AgentToolResult {
        content: vec![ToolResultContent::text(text)],
        details: Value::Null,
        usage: None,
        added_tool_names: None,
        terminate: false,
        is_error: false,
    }
}

fn error_result(text: impl Into<String>) -> AgentToolResult {
    AgentToolResult {
        content: vec![ToolResultContent::text(text)],
        details: Value::Null,
        usage: None,
        added_tool_names: None,
        terminate: false,
        is_error: true,
    }
}
