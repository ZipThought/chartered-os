use std::sync::Arc;

use chartered_core::{
    ArtifactId, Finding, InMemoryArtifactStore, LegacyArtifact, ListArtifacts, LoopOutcome,
    LoopRunner, ModifyArtifact, Outcome, ReadArtifact, Receipt, ReceiptStore, ReceiptStoreError,
    RecordFinding, StewardId, ToolExecutor, ToolId, ToolParams, ToolRegistry, Workspace,
    WorkspaceId,
};

mod common;
use common::{make_frame, make_llm_actor, make_snapshot, make_steward};

#[tokio::test]
async fn artifact_tools_read_modify_list_and_record_findings() {
    let store = Arc::new(InMemoryArtifactStore::new([LegacyArtifact {
        id: ArtifactId::new("deal.md"),
        content: "vendor liability cap".into(),
    }]));
    let artifact_store = store.artifact_store();

    let read = ReadArtifact::new(ToolId::new("read_artifact"), artifact_store.clone());
    let modify = ModifyArtifact::new(ToolId::new("modify_artifact"), artifact_store.clone());
    let list = ListArtifacts::new(ToolId::new("list_artifacts"), artifact_store.clone());
    let record = RecordFinding::new(ToolId::new("record_finding"), artifact_store.clone());

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
    assert!(ids.contains(&"findings"), "list missing findings: {ids:?}");

    record
        .execute(&ToolParams(serde_json::json!({
            "artifact_id": "deal.md",
            "range": { "start": 0, "end": 6, "start_line": 1, "end_line": 1 },
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

    let findings: Vec<Finding> = store.findings();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].artifact_id, ArtifactId::new("deal.md"));
    assert_eq!(findings[0].author_steward_id, StewardId::new("reviewer"));
    assert_eq!(findings[0].admitting_receipt_id.0, "receipt-1");
}

struct FailingReceiptStore;

impl ReceiptStore for FailingReceiptStore {
    fn append(&self, _receipt: &Receipt) -> Result<(), ReceiptStoreError> {
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
    let artifact_store = Arc::new(InMemoryArtifactStore::new([LegacyArtifact {
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
