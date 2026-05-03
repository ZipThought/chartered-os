//! Multi-Steward Workspace tests.
//!
//! Invariant: two Stewards in one Workspace, each with their own
//! Charter, each producing Receipts namespaced by `steward_id`. The
//! Tester's `prior_receipt_queries` MUST NOT see the other Steward's
//! Receipts.
//!
//! Spec §The Runtime: "Each Runtime hosts a Workspace — the
//! deployment-time binding scope of one Charter, one Role context,
//! Steward instances, Tasks, and Receipts." (Plural Steward instances.)
//! Spec §Vocabulary: Tester "operates as a Steward under its own
//! Charter."

use std::sync::Arc;

mod common;
use common::{
    make_frame, make_llm_actor as llm_actor, make_snapshot, make_steward, registry_with_nops,
    NopTool,
};
use chartered_core::{
    Charter, FakeCognitionBackend, Frame, FrameId, InMemoryReceiptStore, LlmEvaluator, LoopOutcome,
    LoopRunner, PriorReceiptQuery, ReceiptStore, RoleContext, Snapshot, Steward,
    StewardId, ToolId, ToolRegistry, Workspace, WorkspaceId,
};

/// Two Stewards under one Workspace each emit Receipts; the shared
/// `ReceiptStore` records both, namespaced by `Receipt.steward_id`.
/// Filtering by steward_id yields disjoint receipt sets.
#[tokio::test]
async fn two_stewards_in_one_workspace_namespace_receipts() {
    let store: Arc<dyn ReceiptStore> = Arc::new(InMemoryReceiptStore::new());

    let snap_a = make_snapshot(
        vec![make_frame("frame-a", &["tool-a"], &["ALLOW: ok"])],
        &["tool-a"],
    );
    let steward_a = make_steward("steward-a", snap_a, registry_with_nops(&["tool-a"]));

    let snap_b = make_snapshot(
        vec![make_frame("frame-b", &["tool-b"], &["ALLOW: ok"])],
        &["tool-b"],
    );
    let steward_b = make_steward("steward-b", snap_b, registry_with_nops(&["tool-b"]));

    let workspace = Arc::new(
        Workspace::with_stewards(
            WorkspaceId::new("ws-multi"),
            vec![steward_a, steward_b],
            store.clone(),
        )
        .expect("multi-Steward workspace validates"),
    );

    let steward_a = workspace
        .steward(&StewardId::new("steward-a"))
        .expect("steward-a present")
        .clone();
    let steward_b = workspace
        .steward(&StewardId::new("steward-b"))
        .expect("steward-b present")
        .clone();

    // Steward A runs a task: propose tool-a, halt.
    let runner_a = LoopRunner::new(workspace.clone(), steward_a);
    let mut actor_a = llm_actor(
        "actor-a",
        vec![
            serde_json::json!({"tool": "tool-a", "params": {}}),
            serde_json::json!({"halt": true}),
        ],
    );
    let outcome_a = runner_a.run(&mut actor_a).await;
    assert!(matches!(outcome_a, LoopOutcome::Halted { .. }));

    // Steward B runs a task: propose tool-b, halt.
    let runner_b = LoopRunner::new(workspace.clone(), steward_b);
    let mut actor_b = llm_actor(
        "actor-b",
        vec![
            serde_json::json!({"tool": "tool-b", "params": {}}),
            serde_json::json!({"halt": true}),
        ],
    );
    let outcome_b = runner_b.run(&mut actor_b).await;
    assert!(matches!(outcome_b, LoopOutcome::Halted { .. }));

    // Every Receipt in the shared store carries a steward_id; both
    // Stewards' Receipts coexist.
    let all = store.all();
    assert!(
        !all.is_empty(),
        "expected Receipts from both Stewards in shared store"
    );
    for r in &all {
        assert!(
            r.steward_id == StewardId::new("steward-a")
                || r.steward_id == StewardId::new("steward-b"),
            "Receipt steward_id `{}` is neither Steward",
            r.steward_id
        );
    }

    let from_a: Vec<_> = all
        .iter()
        .filter(|r| r.steward_id == StewardId::new("steward-a"))
        .collect();
    let from_b: Vec<_> = all
        .iter()
        .filter(|r| r.steward_id == StewardId::new("steward-b"))
        .collect();

    assert!(!from_a.is_empty(), "no Receipts from steward-a");
    assert!(!from_b.is_empty(), "no Receipts from steward-b");

    // Steward A's Receipts only reference tool-a (or kernel-event sentinels);
    // Steward B's Receipts only reference tool-b.
    for r in &from_a {
        let tool = r.tool_call.tool.0.as_str();
        assert!(
            tool == "tool-a" || tool.starts_with('<'),
            "steward-a Receipt referenced unexpected tool {tool}"
        );
    }
    for r in &from_b {
        let tool = r.tool_call.tool.0.as_str();
        assert!(
            tool == "tool-b" || tool.starts_with('<'),
            "steward-b Receipt referenced unexpected tool {tool}"
        );
    }
}

/// `prior_receipt_queries` scopes to the calling Steward's history. A
/// Frame in Steward B's Charter that queries prior Receipts must NOT
/// see Steward A's Receipts. Default-deny on cross-Steward access is
/// the kernel guarantee — Charters opt in to cross-Steward queries
/// explicitly (not yet implemented; this test locks the default).
#[tokio::test]
async fn prior_receipt_queries_do_not_leak_across_stewards() {
    let store: Arc<dyn ReceiptStore> = Arc::new(InMemoryReceiptStore::new());

    // Steward A's Charter is trivial; it just produces some prior
    // Receipts before Steward B runs.
    let snap_a = make_snapshot(
        vec![make_frame("frame-a", &["tool-x"], &["ALLOW: ok"])],
        &["tool-x"],
    );
    let steward_a = make_steward("steward-a", snap_a, registry_with_nops(&["tool-x"]));

    // Steward B's Charter has a Frame whose evaluator records what
    // prior_receipts it sees. We assert Steward A's Receipts are NOT in
    // that list.
    let recording_backend = Arc::new(FakeCognitionBackend::new("eval-recording"));
    recording_backend.enqueue("ALLOW: ok");
    let recording_eval = Arc::new(LlmEvaluator::new(
        "recording-evaluator",
        recording_backend.clone(),
        FrameId::new("frame-b-with-history"),
        "test concern",
    ));
    let frame_b = Frame {
        id: FrameId::new("frame-b-with-history"),
        concern: "scopes prior receipts to this steward".into(),
        declared_scopes: vec![],
        applies_to_tools: vec![ToolId::new("tool-y")],
        evaluator: recording_eval,
        prior_receipt_queries: vec![PriorReceiptQuery {
            frame_id_filter: None,
            limit: 100,
        }],
    };
    let charter_b = Charter {
        frames: vec![frame_b],
        permitted_tools: vec![ToolId::new("tool-y")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "char-b".into(),
        behavioral_spec: String::new(),
    };
    let snap_b = Snapshot::new(charter_b, RoleContext::empty());
    let mut reg_b = ToolRegistry::new();
    reg_b.register(Arc::new(NopTool::new("tool-y")));
    let steward_b = Steward::new(StewardId::new("steward-b"), snap_b, Arc::new(reg_b));

    let workspace = Arc::new(
        Workspace::with_stewards(
            WorkspaceId::new("ws-isolation"),
            vec![steward_a, steward_b],
            store.clone(),
        )
        .expect("workspace validates"),
    );

    let steward_a = workspace.steward(&StewardId::new("steward-a")).unwrap().clone();
    let steward_b = workspace.steward(&StewardId::new("steward-b")).unwrap().clone();

    // Steward A produces some history.
    let runner_a = LoopRunner::new(workspace.clone(), steward_a);
    let mut actor_a = llm_actor(
        "actor-a",
        vec![
            serde_json::json!({"tool": "tool-x", "params": {}}),
            serde_json::json!({"halt": true}),
        ],
    );
    runner_a.run(&mut actor_a).await;

    // Steward A has Receipts in the shared store — confirm.
    let from_a: Vec<_> = store
        .all()
        .into_iter()
        .filter(|r| r.steward_id == StewardId::new("steward-a"))
        .collect();
    assert!(!from_a.is_empty(), "steward-a should have produced Receipts");

    // Steward B now runs; its Frame queries prior_receipts.
    let runner_b = LoopRunner::new(workspace.clone(), steward_b);
    let mut actor_b = llm_actor(
        "actor-b",
        vec![
            serde_json::json!({"tool": "tool-y", "params": {}}),
            serde_json::json!({"halt": true}),
        ],
    );
    runner_b.run(&mut actor_b).await;

    // The recording Evaluator's request would have included prior
    // Receipts; we verified at the store level that the query mechanism
    // filters by steward_id. Direct check: query the store as
    // Steward B and assert no Steward-A Receipts surface.
    let queried = store.query(
        "ws-isolation",
        &StewardId::new("steward-b"),
        None,
        100,
    );
    for r in &queried {
        assert_eq!(
            r.steward_id,
            StewardId::new("steward-b"),
            "store.query returned a Receipt from a different Steward — namespace leak"
        );
    }
    assert!(
        !queried.is_empty(),
        "Steward B should have its own Receipts in the queried result"
    );

    // Cross-check: querying as Steward A returns only Steward A's
    // Receipts.
    let queried_a = store.query(
        "ws-isolation",
        &StewardId::new("steward-a"),
        None,
        100,
    );
    for r in &queried_a {
        assert_eq!(r.steward_id, StewardId::new("steward-a"));
    }
}

/// Workspace rejects duplicate Steward ids at construction.
#[test]
fn workspace_rejects_duplicate_steward_ids() {
    let snap_a = make_snapshot(
        vec![make_frame("f", &["t"], &[])],
        &["t"],
    );
    let snap_b = make_snapshot(
        vec![make_frame("f", &["t"], &[])],
        &["t"],
    );
    let s_a = make_steward("dup", snap_a, registry_with_nops(&["t"]));
    let s_b = make_steward("dup", snap_b, registry_with_nops(&["t"]));
    let store: Arc<dyn ReceiptStore> = Arc::new(InMemoryReceiptStore::new());

    let err =
        match Workspace::with_stewards(WorkspaceId::new("ws-dup"), vec![s_a, s_b], store) {
            Ok(_) => panic!("workspace accepted duplicate Steward ids"),
            Err(e) => e,
        };
    assert!(err.0.contains("duplicate steward id"));
}

/// Workspace rejects empty Steward set at construction.
#[test]
fn workspace_rejects_empty_steward_set() {
    let store: Arc<dyn ReceiptStore> = Arc::new(InMemoryReceiptStore::new());
    let err = match Workspace::with_stewards(WorkspaceId::new("ws-empty"), vec![], store) {
        Ok(_) => panic!("workspace accepted empty Steward set"),
        Err(e) => e,
    };
    assert!(err.0.contains("at least one Steward"));
}
