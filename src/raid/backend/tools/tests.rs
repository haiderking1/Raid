use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::backend::agent::AgentTool;
use crate::backend::tools::truncate::DEFAULT_MAX_BYTES;
use crate::backend::tools::{default_tools, BashTool, ReadTool, ToolEnvironment, WriteTool};

fn temp_workspace() -> (std::path::PathBuf, impl Drop) {
    let path = std::env::temp_dir().join(format!(
        "raid-tools-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&path).expect("create temp workspace");
    (path.clone(), TempDirCleanup(path))
}

struct TempDirCleanup(std::path::PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(result: &crate::backend::agent::AgentToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|part| part.as_text())
        .unwrap_or_default()
}

#[tokio::test]
async fn write_creates_file_and_read_returns_content() {
    let (path, _cleanup) = temp_workspace();
    let env = Arc::new(ToolEnvironment::with_cwd(&path));
    let write = WriteTool::new(env.clone());
    let read = ReadTool::new(env);
    let cancel = CancellationToken::new();

    let write_result = write
        .execute(
            "call-1",
            serde_json::json!({ "path": "notes.txt", "content": "hello world" }),
            &cancel,
            None,
        )
        .await;
    assert!(!write_result.is_error);
    assert!(text(&write_result).contains("Successfully wrote"));

    let read_result = read
        .execute(
            "call-2",
            serde_json::json!({ "path": "notes.txt" }),
            &cancel,
            None,
        )
        .await;
    assert!(!read_result.is_error);
    assert_eq!(text(&read_result), "hello world");
}

#[tokio::test]
async fn bash_echoes_output_and_reports_non_zero_exit() {
    let (path, _cleanup) = temp_workspace();
    let env = Arc::new(ToolEnvironment::with_cwd(path));
    let bash = BashTool::new(env);
    let cancel = CancellationToken::new();

    let ok = bash
        .execute(
            "call-1",
            serde_json::json!({ "command": "printf 'hi'" }),
            &cancel,
            None,
        )
        .await;
    assert!(!ok.is_error);
    assert_eq!(text(&ok), "hi");

    let failed = bash
        .execute(
            "call-2",
            serde_json::json!({ "command": "exit 7" }),
            &cancel,
            None,
        )
        .await;
    assert!(failed.is_error);
    assert!(text(&failed).contains("exited with code 7"));
}

#[tokio::test]
async fn read_rejects_offset_past_eof() {
    let (path, _cleanup) = temp_workspace();
    std::fs::write(path.join("one.txt"), "only\n").expect("write");
    let env = Arc::new(ToolEnvironment::with_cwd(path));
    let read = ReadTool::new(env);
    let cancel = CancellationToken::new();

    let result = read
        .execute(
            "call-1",
            serde_json::json!({ "path": "one.txt", "offset": 5 }),
            &cancel,
            None,
        )
        .await;
    assert!(result.is_error);
    assert!(text(&result).contains("beyond end of file"));
}

#[tokio::test]
async fn read_reports_first_line_byte_limit_with_utf8() {
    let (path, _cleanup) = temp_workspace();
    let line = "é".repeat(DEFAULT_MAX_BYTES);
    std::fs::write(path.join("wide.txt"), line).expect("write");
    let env = Arc::new(ToolEnvironment::with_cwd(path));
    let read = ReadTool::new(env);
    let cancel = CancellationToken::new();

    let result = read
        .execute(
            "call-1",
            serde_json::json!({ "path": "wide.txt" }),
            &cancel,
            None,
        )
        .await;
    assert!(!result.is_error);
    assert!(text(&result).contains("exceeds"));
}
#[test]
fn default_tools_registers_bash_read_write() {
    let env = Arc::new(ToolEnvironment::new());
    let tools = default_tools(env);
    let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
    assert_eq!(names, vec!["bash", "read", "write"]);
}

#[tokio::test]
async fn bash_emits_progress_updates() {
    let (path, _cleanup) = temp_workspace();
    let env = Arc::new(ToolEnvironment::with_cwd(path));
    let bash = BashTool::new(env);
    let cancel = CancellationToken::new();
    let updates = Arc::new(std::sync::Mutex::new(Vec::new()));
    let updates_for_callback = updates.clone();
    let callback = Box::new(move |result: crate::backend::agent::AgentToolResult| {
        updates_for_callback
            .lock()
            .expect("updates")
            .push(result.content.len());
    });

    let result = bash
        .execute(
            "call-1",
            serde_json::json!({ "command": "printf 'streaming'" }),
            &cancel,
            Some(callback),
        )
        .await;
    assert!(!result.is_error);
    let captured = updates.lock().expect("updates");
    assert!(captured.len() >= 2);
    assert_eq!(captured[0], 0);
}

#[cfg(unix)]
#[tokio::test]
async fn bash_cancellation_kills_the_entire_process_group() {
    let (path, _cleanup) = temp_workspace();
    let child_pid_path = path.join("child.pid");
    let env = Arc::new(ToolEnvironment::with_cwd(path));
    let bash = Arc::new(BashTool::new(env));
    let cancel = CancellationToken::new();
    let command = format!(
        "python3 -c 'import subprocess; child = subprocess.Popen([\"sh\", \"-c\", \"trap \\\"\\\" TERM INT; while :; do sleep 1; done\"]); open(\"{}\", \"w\").write(str(child.pid)); child.wait()'",
        child_pid_path.display()
    );

    let task = {
        let bash = bash.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            bash.execute(
                "call-cancel",
                serde_json::json!({ "command": command }),
                &cancel,
                None,
            )
            .await
        })
    };

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !child_pid_path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child pid file");
    let child_pid = std::fs::read_to_string(&child_pid_path)
        .expect("read child pid")
        .trim()
        .parse::<u32>()
        .expect("child pid");

    cancel.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("bash cancellation should not hang")
        .expect("bash task");

    assert!(result.is_error);
    assert!(text(&result).contains("Command aborted"));
    assert!(wait_for_process_to_exit(child_pid).await);
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_a_bash_task_kills_the_entire_process_group() {
    let (path, _cleanup) = temp_workspace();
    let child_pid_path = path.join("dropped-child.pid");
    let env = Arc::new(ToolEnvironment::with_cwd(path));
    let bash = Arc::new(BashTool::new(env));
    let command = format!(
        "python3 -c 'import subprocess; child = subprocess.Popen([\"sh\", \"-c\", \"while :; do sleep 1; done\"]); open(\"{}\", \"w\").write(str(child.pid)); child.wait()'",
        child_pid_path.display()
    );
    let task = tokio::spawn(async move {
        bash.execute(
            "call-drop",
            serde_json::json!({ "command": command }),
            &CancellationToken::new(),
            None,
        )
        .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !child_pid_path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child pid file");
    let child_pid = std::fs::read_to_string(&child_pid_path)
        .expect("read child pid")
        .trim()
        .parse::<u32>()
        .expect("child pid");

    task.abort();
    let _ = task.await;

    assert!(wait_for_process_to_exit(child_pid).await);
}

#[cfg(unix)]
async fn wait_for_process_to_exit(pid: u32) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !process_is_gone(pid) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

#[cfg(unix)]
fn process_is_gone(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return true;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.chars().next())
        .is_some_and(|state| state == 'Z')
}
