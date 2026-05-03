//! NativeExec tests against real subprocess spawns.
//!
//! Tests use widely-available POSIX commands (`echo`, `false`,
//! `nonexistent_command_x`). Skipped on platforms without them.

use chartered_core::{ToolExecutor, ToolId, ToolParams, ToolResult};
use chartered_dispatch::NativeExec;

fn json(v: serde_json::Value) -> ToolParams {
    ToolParams(v)
}

#[tokio::test]
#[cfg(unix)]
async fn echo_captures_stdout_and_zero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let exec = NativeExec::new(ToolId::new("exec_command"), dir.path());
    let r = exec
        .execute(&json(serde_json::json!({
            "cmd": "echo",
            "args": ["hello", "world"]
        })))
        .await;
    let ToolResult::Ok(v) = r else {
        panic!("expected Ok, got {r:?}");
    };
    assert_eq!(v["exit_code"].as_i64(), Some(0));
    let stdout = v["stdout"].as_str().unwrap();
    assert!(stdout.contains("hello world"), "stdout: {stdout}");
}

#[tokio::test]
#[cfg(unix)]
async fn nonzero_exit_is_reported_in_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let exec = NativeExec::new(ToolId::new("exec_command"), dir.path());
    // `false` exits 1 with no output.
    let r = exec
        .execute(&json(serde_json::json!({
            "cmd": "false",
            "args": []
        })))
        .await;
    let ToolResult::Ok(v) = r else {
        panic!("expected Ok (the spawn succeeded), got {r:?}");
    };
    assert_eq!(v["exit_code"].as_i64(), Some(1));
}

#[tokio::test]
async fn missing_cmd_field_yields_err() {
    let dir = tempfile::tempdir().unwrap();
    let exec = NativeExec::new(ToolId::new("exec_command"), dir.path());
    let r = exec.execute(&json(serde_json::json!({}))).await;
    let ToolResult::Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(msg.contains("cmd"), "msg: {msg}");
}

#[tokio::test]
async fn unknown_command_yields_err() {
    let dir = tempfile::tempdir().unwrap();
    let exec = NativeExec::new(ToolId::new("exec_command"), dir.path());
    let r = exec
        .execute(&json(serde_json::json!({
            "cmd": "this_command_should_not_exist_anywhere_xyzzy_42",
            "args": []
        })))
        .await;
    let ToolResult::Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(msg.contains("spawn") || msg.contains("failed"), "msg: {msg}");
}
