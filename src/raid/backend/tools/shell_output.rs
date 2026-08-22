use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;

use super::env::ToolEnvironment;
use super::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncatedBy, TruncationOptions, TruncationResult,
    truncate_tail, utf8_byte_length,
};

const MAX_TIMEOUT_SECONDS: f64 = 2_147_483_647.0 / 1000.0;
const MAX_TAIL_BUFFER_BYTES: usize = DEFAULT_MAX_BYTES * 2;
const EXIT_STDIO_IDLE_GRACE: Duration = Duration::from_millis(100);

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
                || (code > 0x1f
                    && !(0x7f..=0x9f).contains(&code)
                    && !(0xfff9..=0xfffb).contains(&code))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
enum EscapeState {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    String,
    StringEscape,
}

#[derive(Debug, Default)]
struct TerminalSequenceStripper {
    state: EscapeState,
}

impl TerminalSequenceStripper {
    fn push(&mut self, input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        for character in input.chars() {
            match self.state {
                EscapeState::Ground => match character {
                    '\u{001b}' => self.state = EscapeState::Escape,
                    '\u{009b}' => self.state = EscapeState::Csi,
                    '\u{0090}' | '\u{0098}' | '\u{009d}' | '\u{009e}' | '\u{009f}' => {
                        self.state = EscapeState::String;
                    }
                    '\u{0080}'..='\u{009f}' | '\u{007f}' => {}
                    _ => output.push(character),
                },
                EscapeState::Escape => match character {
                    '[' => self.state = EscapeState::Csi,
                    ']' | 'P' | 'X' | '^' | '_' => self.state = EscapeState::String,
                    '\u{0020}'..='\u{002f}' => self.state = EscapeState::EscapeIntermediate,
                    '\u{001b}' => {}
                    _ => self.state = EscapeState::Ground,
                },
                EscapeState::EscapeIntermediate => match character {
                    '\u{0020}'..='\u{002f}' => {}
                    '\u{0030}'..='\u{007e}' => self.state = EscapeState::Ground,
                    '\u{001b}' => self.state = EscapeState::Escape,
                    _ => self.state = EscapeState::Ground,
                },
                EscapeState::Csi => match character {
                    '\u{0040}'..='\u{007e}' => self.state = EscapeState::Ground,
                    '\u{001b}' => self.state = EscapeState::Escape,
                    _ => {}
                },
                EscapeState::String => match character {
                    '\u{0007}' | '\u{009c}' => self.state = EscapeState::Ground,
                    '\u{001b}' => self.state = EscapeState::StringEscape,
                    _ => {}
                },
                EscapeState::StringEscape => match character {
                    '\\' => self.state = EscapeState::Ground,
                    '\u{001b}' => {}
                    _ => self.state = EscapeState::String,
                },
            }
        }
        output
    }
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

    #[cfg(unix)]
    child.process_group(0);

    let mut child = child.spawn().map_err(|error| ShellExecutionError::Failed {
        message: error.to_string(),
    })?;
    let mut process_group = ProcessGroupGuard::new(&child);

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ShellExecutionError::Failed {
            message: "Failed to capture shell stdout".into(),
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ShellExecutionError::Failed {
            message: "Failed to capture shell stderr".into(),
        })?;

    let on_progress = Arc::new(tokio::sync::Mutex::new(on_progress));
    let reader_stop = CancellationToken::new();
    let output_activity = Arc::new(AtomicU64::new(0));

    let stdout_task = spawn_reader(
        stdout,
        accumulator.clone(),
        reader_stop.clone(),
        output_activity.clone(),
        on_progress.clone(),
    );
    let stderr_task = spawn_reader(
        stderr,
        accumulator.clone(),
        reader_stop.clone(),
        output_activity.clone(),
        on_progress,
    );

    let wait_result = wait_for_child(&mut child, cancel, timeout_seconds).await;
    let (exit_code, cancelled, execution_error, drain_readers) = match wait_result {
        ChildWaitResult::Exited(Ok(status)) => (status.code(), false, None, true),
        ChildWaitResult::Exited(Err(error)) => {
            terminate_process_tree(&mut child, &mut process_group).await;
            (
                None,
                false,
                Some(ShellExecutionError::Failed {
                    message: error.to_string(),
                }),
                false,
            )
        }
        ChildWaitResult::Cancelled => {
            terminate_process_tree(&mut child, &mut process_group).await;
            (None, true, None, false)
        }
        ChildWaitResult::TimedOut { seconds } => {
            terminate_process_tree(&mut child, &mut process_group).await;
            (
                None,
                false,
                Some(ShellExecutionError::Timeout { seconds }),
                false,
            )
        }
    };

    finish_reader_tasks(
        stdout_task,
        stderr_task,
        reader_stop,
        output_activity,
        drain_readers,
    )
    .await;
    process_group.disarm();

    let mut guard = accumulator.lock().await;
    guard.finalize_capture();
    let progress = guard.progress();
    drop(guard);

    Ok(progress.into_result(exit_code, cancelled, execution_error))
}

enum ChildWaitResult {
    Exited(Result<std::process::ExitStatus, std::io::Error>),
    Cancelled,
    TimedOut { seconds: u64 },
}

async fn wait_for_child(
    child: &mut Child,
    cancel: &CancellationToken,
    timeout_seconds: Option<u64>,
) -> ChildWaitResult {
    if let Some(seconds) = timeout_seconds {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => ChildWaitResult::Cancelled,
            _ = sleep(Duration::from_secs(seconds)) => ChildWaitResult::TimedOut { seconds },
            result = child.wait() => ChildWaitResult::Exited(result),
        }
    } else {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => ChildWaitResult::Cancelled,
            result = child.wait() => ChildWaitResult::Exited(result),
        }
    }
}

#[cfg(unix)]
async fn terminate_process_tree(child: &mut Child, process_group: &mut ProcessGroupGuard) {
    let group_killed = process_group.kill();
    if !group_killed {
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

#[cfg(windows)]
async fn terminate_process_tree(child: &mut Child, _process_group: &mut ProcessGroupGuard) {
    if let Some(pid) = child.id() {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status()
            .await;
    }
    let _ = child.kill().await;
}

#[cfg(unix)]
struct ProcessGroupGuard {
    process_group: Option<Pid>,
}

#[cfg(unix)]
impl ProcessGroupGuard {
    fn new(child: &Child) -> Self {
        Self {
            process_group: child
                .id()
                .and_then(|pid| i32::try_from(pid).ok())
                .map(Pid::from_raw),
        }
    }

    fn kill(&mut self) -> bool {
        self.process_group
            .take()
            .is_some_and(|process_group| killpg(process_group, Signal::SIGKILL).is_ok())
    }

    fn disarm(&mut self) {
        self.process_group = None;
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

#[cfg(windows)]
struct ProcessGroupGuard;

#[cfg(windows)]
impl ProcessGroupGuard {
    fn new(_child: &Child) -> Self {
        Self
    }

    fn disarm(&mut self) {}
}

async fn finish_reader_tasks(
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    reader_stop: CancellationToken,
    output_activity: Arc<AtomicU64>,
    drain: bool,
) {
    if !drain {
        reader_stop.cancel();
        let _ = tokio::join!(stdout_task, stderr_task);
        return;
    }

    let readers = async move {
        let _ = tokio::join!(stdout_task, stderr_task);
    };
    tokio::pin!(readers);
    loop {
        let activity = output_activity.load(Ordering::Relaxed);
        tokio::select! {
            _ = &mut readers => break,
            _ = sleep(EXIT_STDIO_IDLE_GRACE) => {
                if output_activity.load(Ordering::Relaxed) == activity {
                    reader_stop.cancel();
                    readers.await;
                    break;
                }
            }
        }
    }
}

fn spawn_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: R,
    accumulator: Arc<tokio::sync::Mutex<OutputAccumulator>>,
    stop: CancellationToken,
    output_activity: Arc<AtomicU64>,
    on_progress: Arc<tokio::sync::Mutex<Option<Box<dyn FnMut(ShellCaptureProgress) + Send>>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut buffer = [0u8; 4096];
        loop {
            let read = tokio::select! {
                _ = stop.cancelled() => break,
                read = reader.read(&mut buffer) => read,
            };
            match read {
                Ok(0) => break,
                Ok(count) => {
                    output_activity.fetch_add(1, Ordering::Relaxed);
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
    sequence_stripper: TerminalSequenceStripper,
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
            sequence_stripper: TerminalSequenceStripper::default(),
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
        let text = self.sequence_stripper.push(chunk);
        let text = sanitize_binary_output(&text).replace('\r', "");
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
        let path =
            std::env::temp_dir().join(format!("raid-bash-{}-{}.log", std::process::id(), id));
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

#[cfg(test)]
mod tests {
    use super::OutputAccumulator;

    #[test]
    fn strips_color_sequences_split_across_chunks() {
        let mut output = OutputAccumulator::new();
        output.push_chunk("before \u{001b}[1;");
        output.push_chunk("34mblue\u{001b}[0m after");

        assert_eq!(output.progress().output, "before blue after");
    }

    #[test]
    fn strips_terminal_title_sequences() {
        let mut output = OutputAccumulator::new();
        output.push_chunk("\u{001b}]0;secret title\u{0007}visible");

        assert_eq!(output.progress().output, "visible");
    }
}
