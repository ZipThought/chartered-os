//! Artifact ToolExecutor tests against real on-disk state in tempdirs.
//!
//! These exercise the substrate-blind kernel `ToolExecutor`s wired through
//! an `ArtifactStore` that contains both filesystem Backends — proving
//! that one Tool ABI dispatches polymorphically across `kind=text`
//! (workspace files) and `kind=findings-store` (`.chartered/findings.jsonl`).

use std::sync::Arc;

use chartered_core::{
    ArtifactStore, ListArtifacts, ModifyArtifact, ReadArtifact, RecordFinding, ToolExecutor,
    ToolId, ToolParams,
};
use chartered_dispatch::{DeploymentPaths, FilesystemFindingsBackend, FilesystemTextBackend};

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

fn make_store(workspace: &std::path::Path) -> Arc<ArtifactStore> {
    // Tests put both workspace and chartered_dir under the same tempdir
    // for simplicity. In production they may be independent paths;
    // DeploymentPaths is the single source of truth either way.
    let chartered = workspace.join(".chartered");
    std::fs::create_dir_all(&chartered).unwrap();
    let paths = DeploymentPaths::canonicalize(workspace, &chartered).unwrap();
    let text = Arc::new(FilesystemTextBackend::new(&paths));
    let findings = Arc::new(FilesystemFindingsBackend::new(&paths));
    Arc::new(
        ArtifactStore::new()
            .with_backend(findings)
            .with_backend(text),
    )
}

#[tokio::test]
async fn read_modify_record_and_list_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let artifact_path = dir.path().join("docs").join("deal.md");
    std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
    std::fs::write(&artifact_path, "vendor liability cap").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "side note").unwrap();

    let store = make_store(dir.path());
    let read = ReadArtifact::new(ToolId::new("read_artifact"), store.clone());
    let modify = ModifyArtifact::new(ToolId::new("modify_artifact"), store.clone());
    let record = RecordFinding::new(ToolId::new("record_finding"), store.clone());
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

    let finding = record
        .execute(&json(serde_json::json!({
            "artifact_id": "docs/deal.md",
            "range": range(0, 6),
            "concern": "Confidentiality",
            "severity": "high",
            "detail": "Disclosure risk",
            "_task_id": "task-1",
            "_steward_id": "reviewer",
            "_snapshot_id": "snapshot-1",
            "_receipt_id": "receipt-1"
        })))
        .await
        .expect("record finding");
    assert_eq!(finding["finding_id"], "finding-receipt-1");

    let finding_line =
        std::fs::read_to_string(dir.path().join(".chartered").join("findings.jsonl")).unwrap();
    let stored: serde_json::Value = serde_json::from_str(finding_line.trim()).unwrap();
    assert_eq!(stored["artifact_id"], "docs/deal.md");
    assert_eq!(stored["task_id"], "task-1");
    assert_eq!(stored["author_steward_id"], "reviewer");
    assert_eq!(stored["admitting_receipt_id"], "receipt-1");
    assert!(stored.get("frame_id").is_none());

    let listed = list.execute(&json(serde_json::json!({}))).await.unwrap();
    let ids: Vec<_> = listed["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["artifact_id"].as_str().unwrap())
        .collect();
    // Now includes the kind=findings-store artifact alongside kind=text files.
    assert!(ids.contains(&"docs/deal.md"));
    assert!(ids.contains(&"notes.txt"));
    assert!(ids.contains(&"findings"));

    // Read the findings store through the same Tool ABI — proving polymorphism.
    let findings_view = read
        .execute(&json(serde_json::json!({
            "kind": "findings-store",
            "artifact_id": "findings",
            "selector": { "filter": { "severity": "high" } }
        })))
        .await
        .expect("read findings store");
    let arr = findings_view["findings"]
        .as_array()
        .expect("findings array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["concern"], "Confidentiality");
    assert_eq!(arr[0]["admitting_receipt_id"], "receipt-1");
}

#[tokio::test]
async fn read_rejects_artifact_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
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
    let store = make_store(dir.path());
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
    let store = make_store(dir.path());
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
