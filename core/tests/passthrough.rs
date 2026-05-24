//! Passthrough mode tests.
//!
//! CHECKLIST §Integration > Functional Equivalence in Passthrough.
//! Spec §The Runtime > Enforcement levels: passthrough records every
//! proposal as Receipt, never denies based on Frame Verdicts, and
//! dispatches the effect. Capability check still applies — the Tool
//! registry is the only path regardless of mode.

use std::sync::Arc;

mod common;
use common::{
    make_frame, make_llm_actor as llm_actor, make_steward_passthrough, sole_steward, MessageLog,
    SendMessageTool,
};
use chartered_core::{
    Charter, Frame, LoopOutcome, LoopRunner, Outcome, RoleContext, Snapshot, ToolExecutor, ToolId,
    ToolParams, ToolRegistry, Workspace, WorkspaceId,
};

/// Frame whose Evaluator would deny in full mode (no fake responses
/// enqueued → empty trace → Ungrounded). In passthrough the Gate
/// short-circuits before invoking the Evaluator at all.
fn ungrounded_frame() -> Frame {
    make_frame("always_ungrounded", &["send_message"], &[])
}

fn snap_with_deny_frame(content_hash: &str) -> Arc<Snapshot> {
    let charter = Charter {
        frames: vec![ungrounded_frame()],
        permitted_tools: vec![ToolId::new("send_message")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: content_hash.into(),
        behavioral_spec: String::new(),
    };
    Snapshot::new(charter, RoleContext::empty(), Vec::new())
}

fn send_proposal() -> serde_json::Value {
    serde_json::json!({
        "tool": "send_message",
        "params": { "channel": "x", "content": "y" }
    })
}

/// In passthrough, a Frame that would deny in full mode does not block
/// the dispatch. The Tool effect happens.
#[tokio::test]
async fn passthrough_dispatches_what_full_mode_would_deny() {
    let log = MessageLog::new();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(SendMessageTool::new(log.clone())));
    let snap = snap_with_deny_frame("pt-1");
    let steward = make_steward_passthrough("sut", snap, registry);
    let ws = Arc::new(
        Workspace::single(WorkspaceId::new("ws-pt"), steward).expect("workspace validates"),
    );
    let steward = sole_steward(&ws);
    let runner = LoopRunner::new(ws, steward);

    let mut actor = llm_actor(
        "test",
        vec![send_proposal(), serde_json::json!({"halt": true})],
    );

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Halted { trail, .. } = result else {
        panic!("expected Halted");
    };
    // Passthrough receipt + synthetic Halt receipt (operator-visible
    // termination signal).
    assert_eq!(trail.len(), 2);
    assert_eq!(trail[0].outcome, Outcome::Passthrough);
    assert!(
        trail[0].verdicts.is_empty(),
        "passthrough skips Frame eval"
    );
    assert_eq!(trail[1].tool_call.tool.0, "<halt>");
    assert_eq!(log.snapshot().len(), 1);
}

/// Capability check still applies in passthrough.
#[tokio::test]
async fn passthrough_still_enforces_capability_check() {
    let log = MessageLog::new();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(SendMessageTool::new(log.clone())));
    let snap = snap_with_deny_frame("pt-2");
    let steward = make_steward_passthrough("sut", snap, registry);
    let ws = Arc::new(
        Workspace::single(WorkspaceId::new("ws-pt-cap"), steward).expect("workspace validates"),
    );
    let steward = sole_steward(&ws);
    let runner = LoopRunner::new(ws, steward).with_budget(0);

    let mut actor = llm_actor(
        "test",
        vec![serde_json::json!({"tool": "not_permitted", "params": {}})],
    );

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Escalated { trail, .. } = result else {
        panic!("expected Escalated (capability denial under budget=0)");
    };
    // Denied capability check + BudgetExhausted controller event.
    // The Denied Receipt retains its outcome (no after-the-fact mutation);
    // the controller's give-up is its own Receipt.
    assert_eq!(trail.len(), 2);
    assert_eq!(trail[0].outcome, Outcome::Denied);
    assert_eq!(trail[1].outcome, Outcome::Escalated);
    assert_eq!(trail[1].tool_call.tool.0, "<budget_exhausted>");
    assert!(log.is_empty());
}

/// Functional equivalence under passthrough: the trail of effects matches
/// a baseline that goes directly to the Tool executor.
#[tokio::test]
async fn passthrough_functional_equivalence_to_baseline() {
    let log_pt = MessageLog::new();
    let log_baseline = MessageLog::new();

    {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SendMessageTool::new(log_pt.clone())));
        let snap = snap_with_deny_frame("eq-pt");
        let steward = make_steward_passthrough("sut", snap, registry);
        let ws = Arc::new(
            Workspace::single(WorkspaceId::new("ws-eq"), steward).expect("workspace validates"),
        );
        let steward = sole_steward(&ws);
        let runner = LoopRunner::new(ws, steward);
        let mut actor = llm_actor(
            "test",
            vec![
                send_proposal(),
                send_proposal(),
                send_proposal(),
                serde_json::json!({"halt": true}),
            ],
        );
        runner.run(&mut actor).await;
    }

    {
        let executor = SendMessageTool::new(log_baseline.clone());
        for _ in 0..3 {
            let p = send_proposal();
            let params = ToolParams(p["params"].clone());
            let _ = executor.execute(&params).await;
        }
    }

    let pt_msgs = log_pt.snapshot();
    let bl_msgs = log_baseline.snapshot();
    assert_eq!(pt_msgs.len(), bl_msgs.len());
    for (a, b) in pt_msgs.iter().zip(bl_msgs.iter()) {
        assert_eq!(a.channel, b.channel);
        assert_eq!(a.content, b.content);
    }
}
