//! Filesystem ToolExecutor tests against real on-disk state in tempdirs.

use chartered_core::{ToolExecutor, ToolId, ToolParams};
use chartered_dispatch::{NativeFsRead, NativeFsWrite};

fn json(v: serde_json::Value) -> ToolParams {
    ToolParams(v)
}

#[tokio::test]
async fn write_then_read_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let writer = NativeFsWrite::new(ToolId::new("write_file"), dir.path()).unwrap();
    let reader = NativeFsRead::new(ToolId::new("read_file"), dir.path()).unwrap();

    let w = writer
        .execute(&json(serde_json::json!({
            "path": "notes/spec.md",
            "content": "# spec\n\nbody\n"
        })))
        .await;
    let Ok(v) = w else {
        panic!("write failed: {w:?}");
    };
    assert_eq!(v["wrote"].as_str(), Some("notes/spec.md"));

    let r = reader
        .execute(&json(serde_json::json!({ "path": "notes/spec.md" })))
        .await;
    let Ok(v) = r else {
        panic!("read failed: {r:?}");
    };
    assert_eq!(v["content"].as_str(), Some("# spec\n\nbody\n"));
}

#[tokio::test]
async fn write_rejects_path_traversal_via_dotdot() {
    let dir = tempfile::tempdir().unwrap();
    let writer = NativeFsWrite::new(ToolId::new("write_file"), dir.path()).unwrap();
    let r = writer
        .execute(&json(serde_json::json!({
            "path": "../escape.txt",
            "content": "leaked"
        })))
        .await;
    let Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(msg.contains("escapes workspace root"), "msg: {msg}");
}

#[tokio::test]
async fn read_rejects_absolute_path_outside_root() {
    let dir = tempfile::tempdir().unwrap();
    let reader = NativeFsRead::new(ToolId::new("read_file"), dir.path()).unwrap();
    let r = reader
        .execute(&json(serde_json::json!({ "path": "/etc/passwd" })))
        .await;
    let Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(
        msg.contains("escapes workspace root") || msg.contains("canonicalize"),
        "msg: {msg}"
    );
}

#[tokio::test]
async fn read_returns_err_for_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let reader = NativeFsRead::new(ToolId::new("read_file"), dir.path()).unwrap();
    let r = reader
        .execute(&json(serde_json::json!({ "path": "does/not/exist.md" })))
        .await;
    let Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(msg.contains("canonicalize") || msg.contains("read failed"), "msg: {msg}");
}

#[tokio::test]
async fn write_rejects_missing_required_field() {
    let dir = tempfile::tempdir().unwrap();
    let writer = NativeFsWrite::new(ToolId::new("write_file"), dir.path()).unwrap();
    let r = writer
        .execute(&json(serde_json::json!({ "path": "x.md" })))
        .await;
    let Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(msg.contains("content"), "msg: {msg}");
}

#[tokio::test]
async fn write_creates_intermediate_directories_within_root() {
    let dir = tempfile::tempdir().unwrap();
    let writer = NativeFsWrite::new(ToolId::new("write_file"), dir.path()).unwrap();
    let r = writer
        .execute(&json(serde_json::json!({
            "path": "deep/nested/dir/file.txt",
            "content": "ok"
        })))
        .await;
    assert!(r.is_ok());
    let read = std::fs::read_to_string(dir.path().join("deep/nested/dir/file.txt")).unwrap();
    assert_eq!(read, "ok");
}
