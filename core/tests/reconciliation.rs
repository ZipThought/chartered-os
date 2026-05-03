//! Reconciliation tests: Receipt outcome ↔ observed effect.
//!
//! CHECKLIST §Receipt System > Reconciliation. In fake mode the effect
//! surface is the in-memory harness Tools and the LLM responses live in
//! a FakeCognitionBackend queue.

use std::sync::Arc;

mod common;
use common::{
    make_frame as frame, make_llm_actor as llm_actor, make_steward, sole_steward, InMemoryFs,
    MessageLog, SendMessageTool, WriteSpecTool,
};
use chartered_core::{
    Charter, LoopOutcome, LoopRunner, Outcome, RoleContext, Snapshot, ToolId, ToolRegistry,
    Workspace, WorkspaceId,
};

/// ALLOWED proposal → effect happens.
#[tokio::test]
async fn allowed_proposal_dispatches_send_message() {
    let log = MessageLog::new();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(SendMessageTool::new(log.clone())));

    let charter = Charter {
        frames: vec![frame(
            "always_grounded",
            &["send_message"],
            &["ALLOW: ok"],
        )],
        permitted_tools: vec![ToolId::new("send_message")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "recon-allow".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());
    let steward = make_steward("sut", snap, registry);
    let ws = Arc::new(
        Workspace::single(WorkspaceId::new("ws-allow"), steward).expect("workspace validates"),
    );
    let steward = sole_steward(&ws);
    let runner = LoopRunner::new(ws, steward);

    let mut actor = llm_actor(
        "test",
        vec![
            serde_json::json!({
                "tool": "send_message",
                "params": { "channel": "main", "content": "hello" }
            }),
            serde_json::json!({"halt": true}),
        ],
    );

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Halted { trail, .. } = result else {
        panic!("expected Halted");
    };
    // Allowed send_message + synthetic Halt Receipt.
    assert_eq!(trail.len(), 2);
    assert_eq!(trail[0].outcome, Outcome::Allowed);
    assert_eq!(trail[1].tool_call.tool.0, "<halt>");

    let messages = log.snapshot();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].channel, "main");
    assert_eq!(messages[0].content, "hello");
}

/// DENIED proposal → effect does not happen.
#[tokio::test]
async fn denied_proposal_does_not_dispatch() {
    let log = MessageLog::new();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(SendMessageTool::new(log.clone())));

    let charter = Charter {
        frames: vec![frame(
            "always_ungrounded",
            &["send_message"],
            &["DENY: no"],
        )],
        permitted_tools: vec![ToolId::new("send_message")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "recon-deny".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());
    let steward = make_steward("sut", snap, registry);
    let ws = Arc::new(
        Workspace::single(WorkspaceId::new("ws-deny"), steward).expect("workspace validates"),
    );
    let steward = sole_steward(&ws);
    let runner = LoopRunner::new(ws, steward).with_budget(0);

    let mut actor = llm_actor(
        "test",
        vec![serde_json::json!({
            "tool": "send_message",
            "params": { "channel": "main", "content": "leaked" }
        })],
    );

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Escalated { trail, .. } = result else {
        panic!("expected Escalated");
    };
    // Denied (Frame deny under budget=0) + BudgetExhausted controller event.
    assert_eq!(trail.len(), 2);
    assert_eq!(trail[0].outcome, Outcome::Denied);
    assert_eq!(trail[1].outcome, Outcome::Escalated);
    assert_eq!(trail[1].tool_call.tool.0, "<budget_exhausted>");

    assert!(log.is_empty(), "denied proposal must not dispatch effect");
}

/// Capability check denial → no Verdicts and no effect.
#[tokio::test]
async fn capability_denial_does_not_dispatch_or_produce_verdicts() {
    let log = MessageLog::new();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(SendMessageTool::new(log.clone())));

    let charter = Charter {
        frames: vec![frame(
            "always_grounded",
            &["send_message"],
            &[], // no eval calls — capability denial fires first
        )],
        permitted_tools: vec![ToolId::new("send_message")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "recon-cap".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());
    let steward = make_steward("sut", snap, registry);
    let ws = Arc::new(
        Workspace::single(WorkspaceId::new("ws-cap"), steward).expect("workspace validates"),
    );
    let steward = sole_steward(&ws);
    let runner = LoopRunner::new(ws, steward).with_budget(0);

    let mut actor = llm_actor(
        "test",
        vec![serde_json::json!({
            "tool": "exec_command",
            "params": { "cmd": "rm", "args": ["-rf", "/"] }
        })],
    );

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Escalated { trail, .. } = result else {
        panic!("expected Escalated");
    };
    // Capability deny + BudgetExhausted controller event.
    assert_eq!(trail.len(), 2);
    assert_eq!(trail[0].outcome, Outcome::Denied);
    assert!(trail[0].verdicts.is_empty(), "no Verdicts on capability denial");
    assert_eq!(trail[1].outcome, Outcome::Escalated);
    assert_eq!(trail[1].tool_call.tool.0, "<budget_exhausted>");

    assert!(log.is_empty(), "no Tool dispatched on capability denial");
}

/// Mixed sequence: denied, refined, accepted. Reconciliation across all
/// three. The Frame's path-rule lives in the LLM response; the Evaluator
/// is the same LlmEvaluator code path as a real backend would use.
#[tokio::test]
async fn refinement_sequence_dispatches_only_accepted_proposal() {
    let fs = InMemoryFs::new();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(WriteSpecTool::new(fs.clone())));

    // The "path" Frame: first call denies (Actor proposed draft path),
    // second call allows (Actor refined to non-draft).
    let charter = Charter {
        frames: vec![frame(
            "path",
            &["write_spec"],
            &[
                "DENY: path contains 'draft'; promote to final before writing",
                "ALLOW: path acceptable",
            ],
        )],
        permitted_tools: vec![ToolId::new("write_spec")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "recon-refine".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());
    let steward = make_steward("sut", snap, registry);
    let ws = Arc::new(
        Workspace::single(WorkspaceId::new("ws-refine"), steward).expect("workspace validates"),
    );
    let steward = sole_steward(&ws);
    let runner = LoopRunner::new(ws, steward).with_budget(3);

    let mut actor = llm_actor(
        "test",
        vec![
            serde_json::json!({
                "tool": "write_spec",
                "params": { "path": "draft/spec.md", "content": "v0" }
            }),
            serde_json::json!({
                "tool": "write_spec",
                "params": { "path": "spec.md", "content": "v0" }
            }),
            serde_json::json!({"halt": true}),
        ],
    );

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Halted { trail, .. } = result else {
        panic!("expected Halted");
    };
    // Denied (refined) + Allowed (final write) + synthetic Halt.
    assert_eq!(trail.len(), 3);
    assert_eq!(trail[0].outcome, Outcome::Denied);
    assert_eq!(trail[1].outcome, Outcome::Allowed);
    assert_eq!(trail[2].tool_call.tool.0, "<halt>");

    assert!(!fs.contains("draft/spec.md"), "denied path must not exist");
    assert_eq!(fs.read("spec.md").as_deref(), Some("v0"));
}
