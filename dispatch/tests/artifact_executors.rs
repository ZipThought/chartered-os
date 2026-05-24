//! Artifact ToolExecutor tests against real on-disk state in tempdirs.
//!
//! These exercise the substrate-blind kernel `ToolExecutor`s wired through
//! an `ArtifactStore` that contains both filesystem Backends — proving
//! that one Tool ABI dispatches polymorphically across `kind=text`
//! (workspace files) and `kind=record-store` (`.chartered/<artifact_id>.jsonl`).

use std::sync::Arc;

use chartered_core::{
    ArtifactStore, ListArtifacts, ModifyArtifact, ReadArtifact, ToolExecutor, ToolId, ToolParams,
};
use chartered_core::ArtifactId as CoreArtifactId;
use chartered_dispatch::{DeploymentPaths, FilesystemRecordStore, FilesystemTextBackend};

fn json(v: serde_json::Value) -> ToolParams {
    ToolParams(v)
}

fn range(start: usize, end: usize) -> serde_json::Value {
    serde_json::json!({
        "start": start,
        "end": end,
        "start_line": 1,
        "end_line": 1
    })
}

async fn make_store(workspace: &std::path::Path) -> Arc<ArtifactStore> {
    // Tests put both workspace and chartered_dir under the same tempdir
    // for simplicity. In production they may be independent paths;
    // DeploymentPaths is the single source of truth either way.
    let chartered = workspace.join(".chartered");
    std::fs::create_dir_all(&chartered).unwrap();
    let paths = DeploymentPaths::canonicalize(workspace, &chartered).unwrap();
    let text = Arc::new(FilesystemTextBackend::new(&paths));
    let records = Arc::new(
        FilesystemRecordStore::new(&paths, CoreArtifactId::new("records"))
            .await
            .unwrap(),
    );
    Arc::new(
        ArtifactStore::new()
            .with_backend(records)
            .with_backend(text),
    )
}

#[tokio::test]
async fn read_modify_and_list_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let artifact_path = dir.path().join("docs").join("deal.md");
    std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
    std::fs::write(&artifact_path, "vendor liability cap").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "side note").unwrap();

    let store = make_store(dir.path()).await;
    let read = ReadArtifact::new(ToolId::new("read_artifact"), store.clone());
    let modify = ModifyArtifact::new(ToolId::new("modify_artifact"), store.clone());
    let list = ListArtifacts::new(ToolId::new("list_artifacts"), store.clone());

    let selection = read
        .execute(&json(serde_json::json!({
            "kind": "text",
            "artifact_id": "docs/deal.md",
            "range": range(7, 16)
        })))
        .await
        .expect("read selection");
    assert_eq!(selection["content"], "liability");

    modify
        .execute(&json(serde_json::json!({
            "kind": "text",
            "artifact_id": "docs/deal.md",
            "range": range(17, 20),
            "replacement": "carve-out",
            "summary": "Tighten vendor liability language"
        })))
        .await
        .expect("modify artifact");
    assert_eq!(
        std::fs::read_to_string(&artifact_path).unwrap(),
        "vendor liability carve-out"
    );

    let appended = modify
        .execute(&json(serde_json::json!({
            "kind": "record-store",
            "artifact_id": "records",
            "edit": {
                "append": {
                    "artifact_id": "docs/deal.md",
                    "range": range(0, 6),
                    "concern": "Confidentiality",
                    "severity": "high",
                    "detail": "Disclosure risk"
                }
            },
            "_task_id": "task-1",
            "_steward_id": "reviewer",
            "_snapshot_id": "snapshot-1",
            "_receipt_id": "receipt-1"
        })))
        .await
        .expect("modify_artifact append record");
    assert_eq!(appended["record_id"], "record-receipt-1");

    let record_line =
        std::fs::read_to_string(dir.path().join(".chartered").join("records.jsonl")).unwrap();
    let stored: serde_json::Value = serde_json::from_str(record_line.trim()).unwrap();
    assert_eq!(stored["artifact_id"], "docs/deal.md");
    assert_eq!(stored["task_id"], "task-1");
    assert_eq!(stored["steward_id"], "reviewer");
    assert_eq!(stored["receipt_id"], "receipt-1");

    let listed = list.execute(&json(serde_json::json!({}))).await.unwrap();
    let ids: Vec<_> = listed["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["artifact_id"].as_str().unwrap())
        .collect();
    // Now includes the kind=record-store artifact alongside kind=text files.
    assert!(ids.contains(&"docs/deal.md"));
    assert!(ids.contains(&"notes.txt"));
    assert!(ids.contains(&"records"));

    // Read the record store through the same Tool ABI — proving polymorphism.
    let records_view = read
        .execute(&json(serde_json::json!({
            "kind": "record-store",
            "artifact_id": "records",
            "selector": { "filter": { "severity": "high" } }
        })))
        .await
        .expect("read record store");
    let arr = records_view["records"]
        .as_array()
        .expect("records array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["concern"], "Confidentiality");
    assert_eq!(arr[0]["receipt_id"], "receipt-1");
}

#[tokio::test]
async fn read_rejects_artifact_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path()).await;
    let read = ReadArtifact::new(ToolId::new("read_artifact"), store);
    let r = read
        .execute(&json(serde_json::json!({
            "kind": "text",
            "artifact_id": "../escape.md"
        })))
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
async fn read_rejects_unregistered_kind_with_specific_error() {
    // The store maps kinds to Backends; a kind no one registered must
    // surface a specific "kind not registered" error before any Backend
    // runs — no fall-through, no heuristic.
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path()).await;
    let read = ReadArtifact::new(ToolId::new("read_artifact"), store);
    let r = read
        .execute(&json(serde_json::json!({
            "kind": "nonexistent-kind",
            "artifact_id": "anything"
        })))
        .await;
    let Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(
        msg.contains("nonexistent-kind") && msg.contains("not registered"),
        "expected kind-not-registered error, got: {msg}"
    );
}

#[tokio::test]
async fn read_rejects_missing_kind_field() {
    // Tool calls without `kind` are malformed — surface a specific
    // missing-field error rather than defaulting to text.
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path()).await;
    let read = ReadArtifact::new(ToolId::new("read_artifact"), store);
    let r = read
        .execute(&json(serde_json::json!({
            "artifact_id": "deal.md"
        })))
        .await;
    let Err(msg) = r else {
        panic!("expected Err, got {r:?}");
    };
    assert!(
        msg.contains("kind"),
        "expected missing-kind-field error, got: {msg}"
    );
}
