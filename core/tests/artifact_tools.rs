use std::sync::Arc;

use chartered_core::{
    ArtifactId, InMemoryArtifactStore, ListArtifacts, LoopOutcome, LoopRunner, ModifyArtifact,
    Outcome, ReadArtifact, Receipt, ReceiptStore, ReceiptStoreError, Record, StewardId,
    TextArtifactSeed, ToolExecutor, ToolId, ToolParams, ToolRegistry, Workspace, WorkspaceId,
};

mod common;
use common::{make_frame, make_llm_actor, make_snapshot, make_steward};

#[tokio::test]
async fn artifact_tools_read_modify_list_and_records_via_modify_artifact() {
    let store = Arc::new(InMemoryArtifactStore::new([TextArtifactSeed {
        id: ArtifactId::new("deal.md"),
        content: "vendor liability cap".into(),
    }]));
    let artifact_store = store.artifact_store();

    let read = ReadArtifact::new(ToolId::new("read_artifact"), artifact_store.clone());
    let modify = ModifyArtifact::new(ToolId::new("modify_artifact"), artifact_store.clone());
    let list = ListArtifacts::new(ToolId::new("list_artifacts"), artifact_store.clone());

    let selected = read
        .execute(&ToolParams(serde_json::json!({
            "kind": "text",
            "artifact_id": "deal.md",
            "range": { "start": 7, "end": 16, "start_line": 1, "end_line": 1 }
        })))
        .await
        .expect("read selection");
    assert_eq!(selected["content"], "liability");

    modify
        .execute(&ToolParams(serde_json::json!({
            "kind": "text",
            "artifact_id": "deal.md",
            "range": { "start": 17, "end": 20, "start_line": 1, "end_line": 1 },
            "replacement": "carve-out",
            "summary": "Tighten liability language"
        })))
        .await
        .expect("modify artifact");

    let artifacts = store.artifacts();
    assert_eq!(artifacts[0].content, "vendor liability carve-out");

    let listed = list
        .execute(&ToolParams(serde_json::json!({})))
        .await
        .unwrap();
    let ids: Vec<&str> = listed["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["artifact_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"deal.md"), "list missing deal.md: {ids:?}");
    assert!(ids.contains(&"records"), "list missing records: {ids:?}");

    // Records are appended by calling `modify_artifact` with
    // `kind=record-store`. The runtime injects `_*` provenance keys
    // (receipt_id, task_id, steward_id, snapshot_id) at the top of
    // params; `ModifyArtifact::execute` propagates them into the Edit
    // so the record Backend records provenance without a domain-
    // specific Tool wrapper. Content fields under `edit.append` are
    // opaque to the kernel — the Charter defines whatever shape suits
    // the deployment.
    modify
        .execute(&ToolParams(serde_json::json!({
            "kind": "record-store",
            "artifact_id": "records",
            "edit": {
                "append": {
                    "artifact_id": "deal.md",
                    "range": { "start": 0, "end": 6, "start_line": 1, "end_line": 1 },
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

    let records: Vec<Record> = store.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].steward_id, StewardId::new("reviewer"));
    assert_eq!(records[0].receipt_id.0, "receipt-1");
    assert_eq!(
        records[0].content.get("artifact_id").and_then(|v| v.as_str()),
        Some("deal.md")
    );
    assert_eq!(
        records[0].content.get("severity").and_then(|v| v.as_str()),
        Some("high")
    );
}

struct FailingReceiptStore;

#[async_trait::async_trait]
impl ReceiptStore for FailingReceiptStore {
    async fn append(&self, _receipt: &Receipt) -> Result<(), ReceiptStoreError> {
        Err(ReceiptStoreError("forced append failure".into()))
    }

    fn query(
        &self,
        _context_id: &str,
        _steward_id: &StewardId,
        _frame_id: Option<&chartered_core::FrameId>,
        _limit: usize,
    ) -> Vec<Receipt> {
        Vec::new()
    }

    fn all(&self) -> Vec<Receipt> {
        Vec::new()
    }
}

#[tokio::test]
async fn artifact_modification_stops_when_receipt_append_fails() {
    let artifact_store = Arc::new(InMemoryArtifactStore::new([TextArtifactSeed {
        id: ArtifactId::new("architecture.md"),
        content: "public gateway uses shared cache".into(),
    }]));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ModifyArtifact::new(
        ToolId::new("modify_artifact"),
        artifact_store.artifact_store(),
    )));

    let snapshot = make_snapshot(
        vec![make_frame(
            "architecture_baseline",
            &["modify_artifact"],
            &["ALLOW: proposed edit follows the baseline"],
        )],
        &["modify_artifact"],
    );
    let steward = make_steward("drafter", snapshot, registry);
    let workspace = Arc::new(
        Workspace::single_with_store(
            WorkspaceId::new("ws"),
            steward,
            Arc::new(FailingReceiptStore),
        )
        .expect("workspace validates"),
    );
    let steward = workspace.sole_steward().clone();
    let runner = LoopRunner::new(workspace, steward);
    let mut actor = make_llm_actor(
        "drafter",
        vec![serde_json::json!({
            "tool": "modify_artifact",
            "params": {
                "kind": "text",
                "artifact_id": "architecture.md",
                "range": { "start": 20, "end": 32, "start_line": 1, "end_line": 1 },
                "replacement": "tenant-keyed cache",
                "summary": "Tighten cache isolation"
            }
        })],
    );

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Escalated { trail, .. } = result else {
        panic!("expected Escalated when receipt append fails");
    };
    assert_eq!(trail.len(), 1);
    assert_eq!(trail[0].outcome, Outcome::Escalated);
    assert!(!trail[0].intercept_complete);
    assert_eq!(
        artifact_store.artifacts()[0].content,
        "public gateway uses shared cache"
    );
}
