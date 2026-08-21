use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::env::ToolEnvironment;
use super::truncate::{
    truncate_tail, TruncatedBy, TruncationOptions, TruncationResult, utf8_byte_length,
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};

const MAX_TIMEOUT_SECONDS: f64 = 2_147_483_647.0 / 1000.0;
const MAX_TAIL_BUFFER_BYTES: usize = DEFAULT_MAX_BYTES * 2;

#[derive(Debug, Clone)]
pub struct ShellCaptureProgress {
    pub output: String,
    pub truncation: TruncationResult,
    pub full_output_path: Option<PathBuf>,
    pub last_line_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct ShellCaptureResult {
    pub output: String,
    pub truncation: TruncationResult,
    pub full_output_path: Option<PathBuf>,
    pub last_line_bytes: usize,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub execution_error: Option<ShellExecutionError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellExecutionError {
    Timeout { seconds: u64 },
    Failed { message: String },
}

impl ShellExecutionError {
    pub fn to_string(&self) -> String {
        match self {
            Self::Timeout { seconds } => format!("Command timed out after {seconds} seconds"),
            Self::Failed { message } => message.clone(),
        }
    }

    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }
}

pub fn validate_timeout(timeout_seconds: Option<f64>) -> Result<Option<u64>, String> {
    let Some(timeout_seconds) = timeout_seconds else {
        return Ok(None);
    };
    if !timeout_seconds.is_finite() || timeout_seconds <= 0.0 {
        return Err("Invalid timeout: must be a finite number of seconds".into());
    }
    if timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(format!(
            "Invalid timeout: maximum is {} seconds",
            MAX_TIMEOUT_SECONDS as u64
        ));
    }
    Ok(Some(timeout_seconds.round() as u64))
}

pub fn sanitize_binary_output(input: &str) -> String {
    input
        .chars()
        .filter(|ch| {
            let code = *ch as u32;
            (code == 0x09 || code == 0x0a || code == 0x0d)
                || (code > 0x1f && !(0xfff9..=0xfffb).contains(&code))
        })
        .collect()
}

pub async fn execute_shell_with_capture(
    env: &ToolEnvironment,
    command: &str,
    timeout_seconds: Option<u64>,
    cancel: &CancellationToken,
    on_progress: Option<Box<dyn FnMut(ShellCaptureProgress) + Send>>,
) -> Result<ShellCaptureResult, ShellExecutionError> {
    if !env.cwd().exists() {
        return Err(ShellExecutionError::Failed {
            message: "Working directory does not exist. Please restart the session.".into(),
        });
    }

    let accumulator = Arc::new(tokio::sync::Mutex::new(OutputAccumulator::new()));
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut child = Command::new(shell);
    child
        .arg("-c")
        .arg(command)
        .current_dir(env.cwd())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = child
        .spawn()
        .map_err(|error| ShellExecutionError::Failed {
            message: error.to_string(),
        })?;

    let stdout = child.stdout.take().ok_or_else(|| ShellExecutionError::Failed {
        message: "Failed to capture shell stdout".into(),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| ShellExecutionError::Failed {
        message: "Failed to capture shell stderr".into(),
    })?;

    let on_progress = Arc::new(tokio::sync::Mutex::new(on_progress));

    let stdout_task = spawn_reader(stdout, accumulator.clone(), cancel.clone(), on_progress.clone());
    let stderr_task = spawn_reader(stderr, accumulator.clone(), cancel.clone(), on_progress);

    let wait_result = if let Some(seconds) = timeout_seconds {
        match timeout(Duration::from_secs(seconds), wait_for_child(&mut child, cancel.clone())).await
        {
            Ok(result) => result,
            Err(_) => {
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                let mut guard = accumulator.lock().await;
                guard.finalize_capture();
                let progress = guard.progress();
                drop(guard);
                return Ok(progress.into_result(
                    None,
                    false,
                    Some(ShellExecutionError::Timeout { seconds }),
                ));
            }
        }
    } else {
        wait_for_child(&mut child, cancel.clone()).await
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let mut guard = accumulator.lock().await;
    guard.finalize_capture();
    let progress = guard.progress();
    drop(guard);

    if let Ok(()) = &wait_result {
        if cancel.is_cancelled() {
            let _ = child.kill().await;
            return Ok(progress.into_result(None, true, None));
        }
        match child.try_wait() {
            Ok(Some(status)) => Ok(progress.into_result(status.code(), false, None)),
            Ok(None) => Ok(progress.into_result(None, false, None)),
            Err(error) => Ok(progress.into_result(
                None,
                false,
                Some(ShellExecutionError::Failed {
                    message: error.to_string(),
                }),
            )),
        }
    } else if cancel.is_cancelled() {
        let _ = child.kill().await;
        Ok(progress.into_result(None, true, None))
    } else {
        Ok(progress.into_result(
            None,
            false,
            Some(ShellExecutionError::Failed {
                message: wait_result.unwrap_err().to_string(),
            }),
        ))
    }
}

async fn wait_for_child(child: &mut Child, cancel: CancellationToken) -> Result<(), std::io::Error> {
    tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        result = child.wait() => result.map(|_| ()),
    }
}

fn spawn_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: R,
    accumulator: Arc<tokio::sync::Mutex<OutputAccumulator>>,
    cancel: CancellationToken,
    on_progress: Arc<tokio::sync::Mutex<Option<Box<dyn FnMut(ShellCaptureProgress) + Send>>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut buffer = [0u8; 4096];
        loop {
            if cancel.is_cancelled() {
                break;
            }
            match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(count) => {
                    let chunk = String::from_utf8_lossy(&buffer[..count]);
                    let progress = {
                        let mut guard = accumulator.lock().await;
                        guard.push_chunk(&chunk);
                        guard.progress()
                    };
                    if let Some(callback) = on_progress.lock().await.as_mut() {
                        callback(progress);
                    }
                }
                Err(_) => break,
            }
        }
    })
}

impl ShellCaptureProgress {
    fn into_result(
        self,
        exit_code: Option<i32>,
        cancelled: bool,
        execution_error: Option<ShellExecutionError>,
    ) -> ShellCaptureResult {
        ShellCaptureResult {
            output: self.output,
            truncation: self.truncation,
            full_output_path: self.full_output_path,
            last_line_bytes: self.last_line_bytes,
            exit_code,
            cancelled,
            execution_error,
        }
    }
}

struct OutputAccumulator {
    tail_output: String,
    total_bytes: usize,
    completed_lines: usize,
    has_open_line: bool,
    current_line_bytes: usize,
    full_output_path: Option<PathBuf>,
    full_output_requested: bool,
    accepting_output: bool,
}

impl OutputAccumulator {
    fn new() -> Self {
        Self {
            tail_output: String::new(),
            total_bytes: 0,
            completed_lines: 0,
            has_open_line: false,
            current_line_bytes: 0,
            full_output_path: None,
            full_output_requested: false,
            accepting_output: true,
        }
    }

    fn push_chunk(&mut self, chunk: &str) {
        if !self.accepting_output {
            return;
        }
        let text = sanitize_binary_output(chunk).replace('\r', "");
        let text_bytes = utf8_byte_length(&text);
        self.total_bytes += text_bytes;
        let newline_count = text.matches('\n').count();
        self.completed_lines += newline_count;
        if let Some(last_newline) = text.rfind('\n') {
            let trailing = &text[last_newline + 1..];
            self.current_line_bytes = utf8_byte_length(trailing);
            self.has_open_line = !trailing.is_empty();
        } else if !text.is_empty() {
            self.current_line_bytes += text_bytes;
            self.has_open_line = true;
        }

        self.tail_output.push_str(&text);
        let total_lines = self.completed_lines + usize::from(self.has_open_line);
        if (self.total_bytes > DEFAULT_MAX_BYTES || total_lines > DEFAULT_MAX_LINES)
            && !self.full_output_requested
        {
            self.ensure_full_output_file();
        } else if self.full_output_requested {
            self.append_full_output(&text);
        }
        self.tail_output = trim_to_last_utf8_bytes(&self.tail_output, MAX_TAIL_BUFFER_BYTES);
    }

    fn finalize_capture(&mut self) {
        self.accepting_output = false;
        let progress = self.progress();
        if progress.truncation.truncated && !self.full_output_requested {
            self.ensure_full_output_file();
        }
    }

    fn progress(&self) -> ShellCaptureProgress {
        let tail_truncation = truncate_tail(&self.tail_output, TruncationOptions::default());
        let total_lines = self.completed_lines + usize::from(self.has_open_line);
        let truncated = total_lines > DEFAULT_MAX_LINES || self.total_bytes > DEFAULT_MAX_BYTES;
        let truncated_by = if truncated {
            tail_truncation.truncated_by.or_else(|| {
                if self.total_bytes > DEFAULT_MAX_BYTES {
                    Some(TruncatedBy::Bytes)
                } else {
                    Some(TruncatedBy::Lines)
                }
            })
        } else {
            None
        };
        let truncation = TruncationResult {
            content: tail_truncation.content.clone(),
            truncated,
            truncated_by,
            total_lines,
            total_bytes: self.total_bytes,
            output_lines: tail_truncation.output_lines,
            output_bytes: tail_truncation.output_bytes,
            last_line_partial: tail_truncation.last_line_partial,
            first_line_exceeds_limit: tail_truncation.first_line_exceeds_limit,
            max_lines: DEFAULT_MAX_LINES,
            max_bytes: DEFAULT_MAX_BYTES,
        };
        ShellCaptureProgress {
            output: if truncated {
                truncation.content.clone()
            } else {
                self.tail_output.clone()
            },
            truncation,
            full_output_path: self.full_output_path.clone(),
            last_line_bytes: self.current_line_bytes,
        }
    }

    fn ensure_full_output_file(&mut self) {
        if self.full_output_requested {
            return;
        }
        self.full_output_requested = true;
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("raid-bash-{}-{}.log", std::process::id(), id));
        let _ = std::fs::write(&path, &self.tail_output);
        self.full_output_path = Some(path);
    }

    fn append_full_output(&self, text: &str) {
        if let Some(path) = &self.full_output_path {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
                let _ = file.write_all(text.as_bytes());
            }
        }
    }
}

fn trim_to_last_utf8_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    while start < text.len() && (text.as_bytes()[start] & 0xc0) == 0x80 {
        start += 1;
    }
    text[start..].to_string()
}

pub fn append_status(output: &str, status: &str) -> String {
    if output.is_empty() {
        status.to_string()
    } else {
        format!("{output}\n\n{status}")
    }
}

pub fn format_truncation_footer(capture: &ShellCaptureResult) -> String {
    use super::truncate::format_size;

    let truncation = &capture.truncation;
    if !truncation.truncated {
        return capture.output.clone();
    }
    let full_output_path = capture
        .full_output_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let start_line = truncation
        .total_lines
        .saturating_sub(truncation.output_lines)
        .saturating_add(1);
    let end_line = truncation.total_lines;
    let mut output_text = capture.output.clone();
    if truncation.last_line_partial {
        let last_line_size = format_size(capture.last_line_bytes);
        output_text.push_str(&format!(
            "\n\n[Showing last {} of line {end_line} (line is {last_line_size}). Full output: {full_output_path}]",
            format_size(truncation.output_bytes)
        ));
    } else if truncation.truncated_by == Some(TruncatedBy::Lines) {
        output_text.push_str(&format!(
            "\n\n[Showing lines {start_line}-{end_line} of {}. Full output: {full_output_path}]",
            truncation.total_lines
        ));
    } else {
        output_text.push_str(&format!(
            "\n\n[Showing lines {start_line}-{end_line} of {} ({} limit). Full output: {full_output_path}]",
            truncation.total_lines,
            format_size(DEFAULT_MAX_BYTES)
        ));
    }
    output_text
}
