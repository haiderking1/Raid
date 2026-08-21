use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::backend::agent::{AgentTool, AgentToolResult, ToolResultContent};
use crate::backend::tools::env::ToolEnvironment;
use crate::backend::tools::shell_output::{
    append_status, execute_shell_with_capture, format_truncation_footer, validate_timeout,
    ShellCaptureProgress,
};
use crate::backend::tools::truncate::truncation_to_json;

const BASH_UPDATE_THROTTLE_MS: u64 = 100;

pub struct BashTool {
    env: Arc<ToolEnvironment>,
}

impl BashTool {
    pub fn new(env: Arc<ToolEnvironment>) -> Self {
        Self { env }
    }
}

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds."
    }

    fn parameters_schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Bash command to execute"
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Timeout in seconds (optional, no default timeout)"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            })
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        cancel: &CancellationToken,
        on_update: Option<Box<dyn Fn(AgentToolResult) + Send + Sync>>,
    ) -> AgentToolResult {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let timeout_seconds = match args.get("timeout").and_then(Value::as_f64) {
            Some(value) => match validate_timeout(Some(value)) {
                Ok(parsed) => parsed,
                Err(error) => return error_result(error, Value::Null),
            },
            None => None,
        };

        let on_update = on_update.map(Arc::new);
        if let Some(callback) = &on_update {
            callback(empty_progress_result());
        }

        let last_update = Arc::new(std::sync::Mutex::new(
            Instant::now() - Duration::from_millis(BASH_UPDATE_THROTTLE_MS),
        ));
        let update_dirty = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let latest_progress = Arc::new(std::sync::Mutex::new(None::<ShellCaptureProgress>));

        let on_progress = on_update.as_ref().map(|callback| {
            let callback = Arc::clone(callback);
            let last_update = last_update.clone();
            let update_dirty = update_dirty.clone();
            let latest_progress = latest_progress.clone();
            Box::new(move |progress: ShellCaptureProgress| {
                *latest_progress.lock().expect("latest progress") = Some(progress.clone());
                update_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                let mut last = last_update.lock().expect("last update");
                if last.elapsed() < Duration::from_millis(BASH_UPDATE_THROTTLE_MS) {
                    return;
                }
                if !update_dirty.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                *last = Instant::now();
                callback(progress_to_result(&progress, false));
            }) as Box<dyn FnMut(ShellCaptureProgress) + Send>
        });

        let capture = match execute_shell_with_capture(
            &self.env,
            &command,
            timeout_seconds,
            cancel,
            on_progress,
        )
        .await
        {
            Ok(capture) => capture,
            Err(error) => return error_result(error.to_string(), Value::Null),
        };

        if let Some(callback) = &on_update {
            if update_dirty.load(std::sync::atomic::Ordering::Relaxed) {
                let progress = latest_progress
                    .lock()
                    .expect("latest progress")
                    .clone()
                    .unwrap_or_else(|| ShellCaptureProgress {
                        output: capture.output.clone(),
                        truncation: capture.truncation.clone(),
                        full_output_path: capture.full_output_path.clone(),
                        last_line_bytes: capture.last_line_bytes,
                    });
                callback(progress_to_result(&progress, false));
            }
        }

        let mut output_text = capture.output.clone();
        let details = if capture.truncation.truncated {
            json!({
                "truncation": truncation_to_json(&capture.truncation),
                "fullOutputPath": capture.full_output_path,
            })
        } else {
            Value::Null
        };

        if capture.truncation.truncated {
            output_text = format_truncation_footer(&capture);
        }

        if capture.cancelled || cancel.is_cancelled() {
            return error_result(append_status(&output_text, "Command aborted"), details);
        }
        if let Some(error) = &capture.execution_error {
            if error.is_timeout() {
                let seconds = timeout_seconds.unwrap_or(0);
                return error_result(
                    append_status(&output_text, &format!("Command timed out after {seconds} seconds")),
                    details,
                );
            }
            return error_result(append_status(&output_text, &error.to_string()), details);
        }
        if let Some(code) = capture.exit_code {
            if code != 0 {
                return error_result(
                    append_status(&output_text, &format!("Command exited with code {code}")),
                    details,
                );
            }
        }

        success_result(
            if output_text.is_empty() {
                "(no output)".into()
            } else {
                output_text
            },
            details,
        )
    }
}

fn progress_to_result(progress: &ShellCaptureProgress, is_error: bool) -> AgentToolResult {
    AgentToolResult {
        content: vec![ToolResultContent::text(progress.output.clone())],
        details: if progress.truncation.truncated {
            json!({
                "truncation": truncation_to_json(&progress.truncation),
                "fullOutputPath": progress.full_output_path,
            })
        } else {
            Value::Null
        },
        usage: None,
        added_tool_names: None,
        terminate: false,
        is_error,
    }
}

fn empty_progress_result() -> AgentToolResult {
    AgentToolResult {
        content: vec![],
        details: Value::Null,
        usage: None,
        added_tool_names: None,
        terminate: false,
        is_error: false,
    }
}

fn success_result(text: String, details: Value) -> AgentToolResult {
    AgentToolResult {
        content: vec![ToolResultContent::text(text)],
        details,
        usage: None,
        added_tool_names: None,
        terminate: false,
        is_error: false,
    }
}

fn error_result(text: String, details: Value) -> AgentToolResult {
    AgentToolResult {
        content: vec![ToolResultContent::text(text)],
        details,
        usage: None,
        added_tool_names: None,
        terminate: false,
        is_error: true,
    }
}
