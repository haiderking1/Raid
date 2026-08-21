use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::backend::agent::{AgentTool, AgentToolResult, ImageContent, ToolResultContent};
use crate::backend::tools::env::ToolEnvironment;
use crate::backend::tools::image::{detect_supported_image_mime_type, encode_base64};
use crate::backend::tools::truncate::{
    format_size, truncate_head, truncation_to_json, TruncationOptions, utf8_byte_length,
    DEFAULT_MAX_BYTES,
};

pub struct ReadTool {
    env: Arc<ToolEnvironment>,
}

impl ReadTool {
    pub fn new(env: Arc<ToolEnvironment>) -> Self {
        Self { env }
    }
}

fn parse_line_number(value: Option<&Value>) -> Option<usize> {
    let number = value?.as_f64()?;
    if !number.is_finite() || number < 1.0 {
        return None;
    }
    Some(number.floor() as usize)
}

#[async_trait]
impl AgentTool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete."
    }

    fn parameters_schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read (relative or absolute)"
                    },
                    "offset": {
                        "type": "number",
                        "description": "Line number to start reading from (1-indexed)"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of lines to read"
                    }
                },
                "required": ["path"],
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
        if cancel.is_cancelled() {
            return error_result("Operation aborted");
        }
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let offset = parse_line_number(args.get("offset"));
        let limit = parse_line_number(args.get("limit"));

        let absolute_path = match self.env.resolve_read_path(path) {
            Ok(path) => path,
            Err(error) => return error_result(error.to_string()),
        };
        let bytes = match tokio::fs::read(&absolute_path).await {
            Ok(bytes) => bytes,
            Err(error) => return error_result(error.to_string()),
        };
        if cancel.is_cancelled() {
            return error_result("Operation aborted");
        }

        if let Some(mime_type) = detect_supported_image_mime_type(&bytes) {
            if mime_type == "image/bmp" {
                return success_result(
                    "Read image file [image/bmp]\n[Image omitted: configure an imageProcessor to convert BMP images.]",
                    Value::Null,
                );
            }
            return AgentToolResult {
                content: vec![
                    ToolResultContent::text(format!("Read image file [{mime_type}]")),
                    ToolResultContent::Image(ImageContent::new(
                        encode_base64(&bytes),
                        mime_type,
                    )),
                ],
                details: Value::Null,
                usage: None,
                added_tool_names: None,
                terminate: false,
                is_error: false,
            };
        }

        let text_content = String::from_utf8_lossy(&bytes).into_owned();
        let all_lines: Vec<&str> = text_content.split('\n').collect();
        let total_file_lines = all_lines.len();
        let start_line = offset.map(|value| value.saturating_sub(1)).unwrap_or(0);
        let start_line_display = start_line + 1;
        if start_line >= all_lines.len() {
            return error_result(format!(
                "Offset {} is beyond end of file ({total_file_lines} lines total)",
                offset.unwrap_or(0)
            ));
        }

        let (selected_content, user_limited_lines) = if let Some(limit) = limit {
            let end_line = (start_line + limit).min(all_lines.len());
            (
                all_lines[start_line..end_line].join("\n"),
                Some(end_line - start_line),
            )
        } else {
            (all_lines[start_line..].join("\n"), None)
        };

        let truncation = truncate_head(&selected_content, TruncationOptions::default());
        let mut output_text;
        let details = if truncation.first_line_exceeds_limit {
            let first_line_size = format_size(
                all_lines
                    .get(start_line)
                    .map(|line| utf8_byte_length(line))
                    .unwrap_or(0),
            );
            output_text = format!(
                "[Line {start_line_display} is {first_line_size}, exceeds {} limit. Use bash: sed -n '{start_line_display}p' {path} | head -c {DEFAULT_MAX_BYTES}]",
                format_size(DEFAULT_MAX_BYTES)
            );
            json!({ "truncation": truncation_to_json(&truncation) })
        } else if truncation.truncated {
            let end_line_display = start_line_display + truncation.output_lines.saturating_sub(1);
            let next_offset = end_line_display + 1;
            output_text = truncation.content.clone();
            if truncation.truncated_by == Some(super::truncate::TruncatedBy::Lines) {
                output_text.push_str(&format!(
                    "\n\n[Showing lines {start_line_display}-{end_line_display} of {total_file_lines}. Use offset={next_offset} to continue.]"
                ));
            } else {
                output_text.push_str(&format!(
                    "\n\n[Showing lines {start_line_display}-{end_line_display} of {total_file_lines} ({} limit). Use offset={next_offset} to continue.]",
                    format_size(DEFAULT_MAX_BYTES)
                ));
            }
            json!({ "truncation": truncation_to_json(&truncation) })
        } else if let Some(user_limited_lines) = user_limited_lines {
            if start_line + user_limited_lines < all_lines.len() {
                let remaining = all_lines.len() - (start_line + user_limited_lines);
                let next_offset = start_line + user_limited_lines + 1;
                output_text = format!(
                    "{}\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]",
                    truncation.content
                );
            } else {
                output_text = truncation.content;
            }
            Value::Null
        } else {
            output_text = truncation.content;
            Value::Null
        };

        success_result(output_text, details)
    }
}

fn success_result(text: impl Into<String>, details: Value) -> AgentToolResult {
    AgentToolResult {
        content: vec![ToolResultContent::text(text)],
        details,
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
