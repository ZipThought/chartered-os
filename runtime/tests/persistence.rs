//! Integration tests for the runtime's durable streams.
//!
//! Exercise `AppendOnlyFileReceiptStore` and `LoggingBackend` against
//! real disk, with one `tempfile::tempdir()` per test. They live under
//! `tests/` rather than in `#[cfg(test)] mod tests` because they are
//! not unit tests by `AGENTS.md §Verification` — unit tests are
//! literally stateless.

use std::sync::Arc;

use chartered_core::{
    AttemptId, CognitionBackend, CognitionRequest, FakeCognitionBackend, GovernanceMode, Message,
    Outcome, Receipt, ReceiptId, ReceiptStore, SnapshotId, StewardId, TaskId, ToolCall, ToolId,
    ToolParams,
};
use chartered_dispatch::JsonlSink;
use chartered_runtime::persistence::{AppendOnlyFileReceiptStore, LoggingBackend};

fn mk_receipt(context_id: &str) -> Receipt {
    Receipt {
        receipt_id: ReceiptId("test-id".into()),
        task_id: TaskId::new("task-test"),
        attempt_id: Some(AttemptId::new("attempt-test")),
        steward_id: StewardId::new("test-steward"),
        governance_mode: GovernanceMode::FULL,
        tool_call: ToolCall {
            tool: ToolId::new("noop"),
            params: ToolParams(serde_json::json!({})),
            context_id: Arc::from(context_id),
            source_id: Arc::from("steward"),
        },
        verdicts: vec![],
        outcome: Outcome::Allowed,
        timestamp: std::time::SystemTime::now(),
        intercept_complete: true,
        charter_version: 1,
        role_context_version: 1,
        snapshot_id: SnapshotId("snap-test".into()),
    }
}

#[tokio::test]
async fn receipt_store_writes_jsonl_and_caches_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("receipts.jsonl");
    let store = AppendOnlyFileReceiptStore::create(&path).await.unwrap();

    let receipt = mk_receipt("ctx-1");
    store.append(&receipt).await.unwrap();

    // In-memory mirror reflects the write immediately.
    assert_eq!(store.all().len(), 1);

    // The file on disk is one valid JSON line.
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["tool_call"]["context_id"].as_str(), Some("ctx-1"));
}

#[tokio::test]
async fn cognition_log_records_request_and_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cognition.jsonl");
    let log: Arc<JsonlSink> = Arc::new(JsonlSink::create(&path).await.unwrap());

    let inner = Arc::new(FakeCognitionBackend::new("eval-test"));
    inner.enqueue("ALLOW: ok");
    let wrapped = LoggingBackend::new(inner, log.clone());

    let req = CognitionRequest {
        messages: vec![Message::user("hello")],
        max_output_tokens: Some(64),
    };
    let resp = wrapped.complete(&req).await.unwrap();
    assert_eq!(resp.content, "ALLOW: ok");

    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["backend_id"].as_str(), Some("eval-test"));
    assert_eq!(v["response"]["content"].as_str(), Some("ALLOW: ok"));
    assert!(
        v["request"]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("hello")
    );
}

#[tokio::test]
async fn cognition_log_records_backend_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cognition.jsonl");
    let log: Arc<JsonlSink> = Arc::new(JsonlSink::create(&path).await.unwrap());

    // Empty queue → error on first call.
    let inner = Arc::new(FakeCognitionBackend::new("evaluator"));
    let wrapped = LoggingBackend::new(inner, log.clone());

    let req = CognitionRequest {
        messages: vec![Message::user("x")],
        max_output_tokens: None,
    };
    let result = wrapped.complete(&req).await;
    assert!(result.is_err());

    let text = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(v["response"].is_null());
    assert!(v["error"].as_str().unwrap().contains("queue empty"));
}
