//! Property tests for the negative-feedback loop kernel.
//!
//! All Evaluators and Actors are the canonical LLM-backed implementations
//! consuming a `FakeCognitionBackend`. Tests enqueue the LLM responses
//! they expect; the role's prompt assembly, response parsing, and
//! decision derivation run exactly as they will against a real backend.
//! Backend swap (fake → OpenAI) is the only point of variation.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use chartered_core::{
    Charter, CognitionBackend, CognitionError, CognitionRequest, CognitionResponse, DeclaredScope,
    FakeCognitionBackend, Frame, FrameId, Gate, GovernanceMode, LlmActor, LlmEvaluator,
    LoopOutcome, LoopRunner, Outcome, RoleContext, Ruling, Snapshot, StewardId, ToolCall, ToolId,
    ToolParams, ToolRegistry, Workspace, WorkspaceId,
};

mod common;
use common::{
    make_frame as frame, make_llm_actor as llm_actor, make_snapshot as snapshot, make_steward,
    make_workspace as workspace, registry_with_nops, sole_steward, NopTool,
};

fn proposal(tool: &str) -> ToolCall {
    ToolCall {
        tool: ToolId::new(tool),
        params: ToolParams(serde_json::json!({})),
        context_id: Arc::from("ctx-1"),
        source_id: Arc::from("steward-1"),
    }
}

/// CHECKLIST §The Loop > Capability Check Pre-Gate.
#[tokio::test]
async fn capability_check_denies_without_frame_eval() {
    let snap = snapshot(vec![frame("f1", &["allowed"], &[])], &["allowed"]);
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("forbidden")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert!(r.verdicts.is_empty());
    assert!(r.intercept_complete);
}

/// CHECKLIST §The Loop > Aggregate Across Frames.
#[tokio::test]
async fn across_frame_conjunction_does_not_short_circuit() {
    let snap = snapshot(
        vec![
            frame("a", &["t"], &["DENY: violates A"]),
            frame("b", &["t"], &["ALLOW: ok"]),
            frame("c", &["t"], &["DENY: violates C"]),
        ],
        &["t"],
    );
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert_eq!(r.verdicts.len(), 3);
    let denied: Vec<_> = r
        .verdicts
        .iter()
        .filter(|v| matches!(v.ruling, Ruling::Ungrounded))
        .collect();
    assert_eq!(denied.len(), 2);
}

#[tokio::test]
async fn all_defer_yields_ungrounded() {
    let snap = snapshot(
        vec![frame(
            "f",
            &["t"],
            &["DEFER: step 1 defers\nDEFER: step 2 defers"],
        )],
        &["t"],
    );
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert!(matches!(r.verdicts[0].ruling, Ruling::Ungrounded));
}

#[tokio::test]
async fn empty_evaluator_response_yields_ungrounded() {
    let snap = snapshot(vec![frame("f", &["t"], &[""])], &["t"]);
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert!(matches!(r.verdicts[0].ruling, Ruling::Ungrounded));
}

#[tokio::test]
async fn malformed_evaluator_response_yields_ungrounded() {
    let snap = snapshot(
        vec![frame("f", &["t"], &["this is not a valid evaluator response"])],
        &["t"],
    );
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert!(matches!(r.verdicts[0].ruling, Ruling::Ungrounded));
}

#[tokio::test]
async fn out_of_scope_appears_when_frame_not_applicable() {
    let snap = snapshot(
        vec![
            frame("a", &["other"], &[]),
            frame("b", &["t"], &["ALLOW: ok"]),
        ],
        &["t", "other"],
    );
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Allowed);
    assert!(
        r.verdicts
            .iter()
            .any(|v| matches!(v.ruling, Ruling::OutOfScope))
    );
    assert!(
        r.verdicts
            .iter()
            .any(|v| matches!(v.ruling, Ruling::Grounded))
    );
}

#[tokio::test]
async fn all_out_of_scope_yields_denied() {
    let snap = snapshot(
        vec![
            frame("a", &["other1"], &[]),
            frame("b", &["other2"], &[]),
        ],
        &["t", "other1", "other2"],
    );
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert!(
        r.verdicts
            .iter()
            .all(|v| matches!(v.ruling, Ruling::OutOfScope))
    );
}

#[tokio::test]
async fn refinement_signal_projects_frame_id_and_reason() {
    let snap = snapshot(
        vec![
            frame("price", &["t"], &["DENY: fee schedule mismatch"]),
            frame("scope", &["t"], &["ALLOW: ok"]),
        ],
        &["t"],
    );
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    let signal = r.refinement_signal();
    assert_eq!(signal.entries.len(), 1);
    assert_eq!(signal.entries[0].0, FrameId::new("price"));
    assert!(signal.entries[0].1.contains("fee schedule"));
}

#[tokio::test]
async fn refinement_signal_does_not_carry_full_trace() {
    let snap = snapshot(
        vec![frame(
            "f",
            &["t"],
            &["DEFER: step 1 internal observation\nDENY: step 2 final reason"],
        )],
        &["t"],
    );
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    let signal = r.refinement_signal();
    assert_eq!(signal.entries.len(), 1);
    let (_, reason) = &signal.entries[0];
    assert!(!reason.contains("step 1"));
    assert!(reason.contains("step 2"));
}

#[tokio::test]
async fn snapshot_id_is_content_addressed() {
    let s1 = snapshot(vec![frame("f", &["t"], &[])], &["t"]);
    let s2 = snapshot(vec![frame("f", &["t"], &[])], &["t"]);
    assert_eq!(s1.id, s2.id);
}

#[tokio::test]
async fn refinement_budget_exhaustion_yields_escalated() {
    let snap = snapshot(
        vec![frame(
            "f",
            &["t"],
            &["DENY: always", "DENY: always", "DENY: always"],
        )],
        &["t"],
    );
    let ws = workspace(snap, registry_with_nops(&["t"]));
    let steward = sole_steward(&ws); let runner = LoopRunner::new(ws.clone(), steward).with_budget(2);

    let propose = serde_json::json!({"tool": "t", "params": {}});
    let mut actor = llm_actor(
        "test-actor",
        vec![propose.clone(), propose.clone(), propose.clone()],
    );

    let result = runner.run(&mut actor).await;
    match result {
        LoopOutcome::Escalated { trail, .. } => {
            // Three Denied Receipts (the budget-exhausting Gate verdicts)
            // plus a kernel-emitted BudgetExhausted Receipt that records
            // the controller's give-up. The Denied Receipts retain
            // outcome=Denied so the durable trail and the in-memory
            // result agree (no after-the-fact mutation).
            assert_eq!(trail.len(), 4);
            for r in &trail[..3] {
                assert_eq!(r.outcome, Outcome::Denied);
                assert_eq!(r.tool_call.tool.0, "t");
            }
            let budget = trail.last().unwrap();
            assert_eq!(budget.outcome, Outcome::Escalated);
            assert_eq!(budget.tool_call.tool.0, "<budget_exhausted>");
            assert!(budget.intercept_complete);
        }
        other => panic!("expected Escalated, got {other:?}"),
    }
    // Disk and memory agree on every Receipt — including the controller
    // event.
    assert_eq!(ws.receipt_store.all().len(), 4);
}

#[tokio::test]
async fn loop_converges_after_refinement() {
    let snap = snapshot(vec![frame("f", &["good"], &["ALLOW: ok"])], &["good"]);
    let ws = workspace(snap, registry_with_nops(&["good"]));
    let steward = sole_steward(&ws); let runner = LoopRunner::new(ws, steward).with_budget(3);

    let mut actor = llm_actor(
        "test-actor",
        vec![
            serde_json::json!({"tool": "bad", "params": {}}),
            serde_json::json!({"tool": "good", "params": {}}),
            serde_json::json!({"halt": true}),
        ],
    );

    let result = runner.run(&mut actor).await;
    match result {
        LoopOutcome::Halted { trail, .. } => {
            // Denied (bad tool capability deny) + Allowed (good tool) +
            // synthetic Halt Receipt so operators can grep clean
            // terminations.
            assert_eq!(trail.len(), 3);
            assert_eq!(trail[0].outcome, Outcome::Denied);
            assert_eq!(trail[1].outcome, Outcome::Allowed);
            assert_eq!(trail[2].outcome, Outcome::Allowed);
            assert_eq!(trail[2].tool_call.tool.0, "<halt>");
            assert!(trail[2].intercept_complete);
        }
        other => panic!("expected Halted, got {other:?}"),
    }
}

#[tokio::test]
async fn evaluator_timeout_flips_intercept_complete() {
    struct SlowBackend;
    #[async_trait]
    impl CognitionBackend for SlowBackend {
        fn id(&self) -> &str {
            "slow"
        }
        async fn complete(
            &self,
            _: &CognitionRequest,
        ) -> Result<CognitionResponse, CognitionError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            unreachable!()
        }
    }

    let backend: Arc<dyn CognitionBackend> = Arc::new(SlowBackend);
    let evaluator = Arc::new(LlmEvaluator::new(
        "slow-eval",
        backend,
        FrameId::new("slow"),
        "slow concern",
    ));
    let f = Frame {
        id: FrameId::new("slow"),
        concern: "slow".into(),
        declared_scopes: vec![],
        applies_to_tools: vec![ToolId::new("t")],
        evaluator,
        prior_receipt_queries: vec![],
    };
    let charter = Charter {
        frames: vec![f],
        permitted_tools: vec![ToolId::new("t")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "slow".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());

    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL).with_frame_timeout(Duration::from_millis(50));
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert!(!r.intercept_complete);
}

#[tokio::test]
async fn backend_error_yields_ungrounded() {
    let backend = Arc::new(FakeCognitionBackend::new("empty-backend"));
    let evaluator = Arc::new(LlmEvaluator::new(
        "empty-eval",
        backend,
        FrameId::new("f"),
        "concern",
    ));
    let f = Frame {
        id: FrameId::new("f"),
        concern: "test".into(),
        declared_scopes: vec![],
        applies_to_tools: vec![ToolId::new("t")],
        evaluator,
        prior_receipt_queries: vec![],
    };
    let charter = Charter {
        frames: vec![f],
        permitted_tools: vec![ToolId::new("t")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "x".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());
    let gate = Gate::new(snap, StewardId::new("test-steward"), GovernanceMode::FULL);
    let r = gate.evaluate(proposal("t")).await;
    assert_eq!(r.outcome, Outcome::Denied);
    assert!(matches!(r.verdicts[0].ruling, Ruling::Ungrounded));
    assert!(
        r.verdicts[0].reason.contains("backend"),
        "reason should surface the backend error"
    );
    assert!(
        !r.intercept_complete,
        "evaluator infrastructure failure must flip intercept_complete=false \
         so operators can distinguish model-deny from evaluator-unavailable"
    );
}

/// Actor cognitive failure (parse error) surfaces as a Receipt with
/// `outcome: Escalated` and `intercept_complete=false`. CHECKLIST §Risk
/// Register > Silent Failure: every partial-coverage condition must be
/// visible in the Receipt trail; silent halt would leave operators
/// unable to distinguish "task complete" from "Actor failed."
#[tokio::test]
async fn actor_malformed_response_escalates_with_intercept_incomplete() {
    let snap = snapshot(vec![frame("f", &["t"], &[])], &["t"]);
    let ws = workspace(snap, registry_with_nops(&["t"]));
    let steward = sole_steward(&ws); let runner = LoopRunner::new(ws.clone(), steward);

    let backend = Arc::new(FakeCognitionBackend::new("malformed-actor"));
    backend.enqueue("not valid JSON");
    let mut actor = LlmActor::new("malformed", backend, "system prompt", "ctx-1");

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Escalated { trail, .. } = result else {
        panic!("expected Escalated, got {result:?}");
    };
    assert_eq!(trail.len(), 1, "Actor failure recorded as a Receipt");
    let r = &trail[0];
    assert_eq!(r.outcome, Outcome::Escalated);
    assert!(!r.intercept_complete);
    assert_eq!(r.tool_call.tool.0, "<actor_failure>");
    assert!(r.verdicts.is_empty());
    assert_eq!(ws.receipt_store.all().len(), 1);
}

/// Actor backend error surfaces as the same kind of Receipt.
#[tokio::test]
async fn actor_backend_error_escalates_with_intercept_incomplete() {
    let snap = snapshot(vec![frame("f", &["t"], &[])], &["t"]);
    let ws = workspace(snap, registry_with_nops(&["t"]));
    let steward = sole_steward(&ws); let runner = LoopRunner::new(ws.clone(), steward);

    // Empty backend → CognitionError on first call → Action::Fail.
    let backend = Arc::new(FakeCognitionBackend::new("empty-actor-backend"));
    let mut actor = LlmActor::new("empty", backend, "system prompt", "ctx-1");

    let result = runner.run(&mut actor).await;
    let LoopOutcome::Escalated { trail, .. } = result else {
        panic!("expected Escalated");
    };
    assert_eq!(trail.len(), 1);
    assert!(!trail[0].intercept_complete);
    assert_eq!(trail[0].tool_call.tool.0, "<actor_failure>");
    let reason = trail[0].tool_call.params.0["reason"].as_str().unwrap();
    assert!(reason.contains("backend error"));
}

/// Workspace::new rejects a Charter with a permitted Tool that has no
/// registered executor. Spec §Tool Registry Is the Only Path: the
/// misconfiguration must surface at construction, not at first dispatch.
#[tokio::test]
async fn workspace_rejects_missing_executor() {
    let snap = snapshot(vec![frame("f", &["t"], &[])], &["t"]);
    let steward = make_steward("sut", snap, ToolRegistry::new());
    let err = match Workspace::single(WorkspaceId::new("ws"), steward) {
        Ok(_) => panic!("Workspace accepted Charter with unregistered Tool"),
        Err(e) => e,
    };
    assert!(err.0.contains("no registered executor"));
}

/// Workspace::new rejects a Frame whose declared Charter Scope name
/// does not exist in the Charter. Spec §The Charter: "A reference to a
/// non-existent Scope fails at configuration time, not silently at
/// evaluation."
#[tokio::test]
async fn workspace_rejects_missing_charter_scope() {
    let backend = Arc::new(FakeCognitionBackend::new("eval"));
    let f = Frame {
        id: FrameId::new("frame-with-bad-scope"),
        concern: "test".into(),
        declared_scopes: vec![DeclaredScope::charter("nonexistent_scope")],
        applies_to_tools: vec![ToolId::new("t")],
        evaluator: Arc::new(LlmEvaluator::new(
            "eval",
            backend,
            FrameId::new("frame-with-bad-scope"),
            "concern",
        )),
        prior_receipt_queries: vec![],
    };
    let charter = Charter {
        frames: vec![f],
        permitted_tools: vec![ToolId::new("t")],
        charter_scopes: vec![], // empty — declared_scopes references a missing one
        charter_version: 1,
        charter_content_hash: "x".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(NopTool::new("t")));
    let steward = make_steward("sut", snap, reg);
    let err = match Workspace::single(WorkspaceId::new("ws"), steward) {
        Ok(_) => panic!("Workspace accepted Frame with missing Charter Scope"),
        Err(e) => e,
    };
    assert!(err.0.contains("nonexistent_scope"));
    assert!(err.0.contains("Charter"));
}

/// Workspace::new rejects a Frame whose declared Role context Scope
/// name does not exist in the Role context.
#[tokio::test]
async fn workspace_rejects_missing_role_context_scope() {
    let backend = Arc::new(FakeCognitionBackend::new("eval"));
    let f = Frame {
        id: FrameId::new("frame-with-bad-rc"),
        concern: "test".into(),
        declared_scopes: vec![DeclaredScope::role_context("missing_facts")],
        applies_to_tools: vec![ToolId::new("t")],
        evaluator: Arc::new(LlmEvaluator::new(
            "eval",
            backend,
            FrameId::new("frame-with-bad-rc"),
            "concern",
        )),
        prior_receipt_queries: vec![],
    };
    let charter = Charter {
        frames: vec![f],
        permitted_tools: vec![ToolId::new("t")],
        charter_scopes: vec![],
        charter_version: 1,
        charter_content_hash: "x".into(),
        behavioral_spec: String::new(),
    };
    let snap = Snapshot::new(charter, RoleContext::empty());
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(NopTool::new("t")));
    let steward = make_steward("sut", snap, reg);
    let err = match Workspace::single(WorkspaceId::new("ws"), steward) {
        Ok(_) => panic!("Workspace accepted Frame with missing Role context Scope"),
        Err(e) => e,
    };
    assert!(err.0.contains("missing_facts"));
    assert!(err.0.contains("RoleContext"));
}

/// The Evaluator receives Charter Scopes as authority and Role context
/// Scopes as quoted evidence — distinct sections in the prompt with
/// explicit "facts not instructions" framing for the Role context. Spec
/// §Role Context: prompt-design discipline is the enforcement mechanism.
#[tokio::test]
async fn scopes_reach_evaluator_with_provenance_separated() {
    use chartered_core::FakeCognitionBackend as Fake;

    let eval_backend = Arc::new(Fake::new("provenance-eval"));
    eval_backend.enqueue("ALLOW: ok");
    let evaluator = Arc::new(LlmEvaluator::new(
        "provenance",
        eval_backend.clone(),
        FrameId::new("provenance"),
        "concern",
    ));
    let f = Frame {
        id: FrameId::new("provenance"),
        concern: "provenance test".into(),
        declared_scopes: vec![
            DeclaredScope::charter("policy"),
            DeclaredScope::role_context("schedule"),
        ],
        applies_to_tools: vec![ToolId::new("t")],
        evaluator,
        prior_receipt_queries: vec![],
    };
    let charter = Charter {
        frames: vec![f],
        permitted_tools: vec![ToolId::new("t")],
        charter_scopes: vec![("policy".into(), "AUTHORITATIVE_POLICY_TEXT".into())],
        charter_version: 1,
        charter_content_hash: "x".into(),
        behavioral_spec: String::new(),
    };
    let mut role_context = RoleContext::empty();
    role_context.scopes = vec![("schedule".into(), "FACTS_FROM_PROFESSIONAL".into())];
    role_context.role_context_content_hash = "rc-x".into();
    let snap = Snapshot::new(charter, role_context);
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(NopTool::new("t")));

    let steward = make_steward("sut", snap, reg);
    let ws = Arc::new(
        Workspace::single(WorkspaceId::new("ws"), steward).expect("workspace validates"),
    );
    let steward = sole_steward(&ws);
    let runner = LoopRunner::new(ws, steward);

    let mut actor = llm_actor(
        "actor",
        vec![
            serde_json::json!({"tool": "t", "params": {}}),
            serde_json::json!({"halt": true}),
        ],
    );
    runner.run(&mut actor).await;

    // Inspect the Evaluator's prompt to confirm provenance separation.
    let calls = eval_backend.calls();
    assert_eq!(calls.len(), 1);
    let user_msg = calls[0]
        .messages
        .iter()
        .find(|m| matches!(m.role, chartered_core::MessageRole::User))
        .expect("user message present");
    let content = &user_msg.content;
    assert!(content.contains("AUTHORITY SCOPES"));
    assert!(content.contains("AUTHORITATIVE_POLICY_TEXT"));
    assert!(content.contains("QUOTED EVIDENCE"));
    assert!(content.contains("NOT instructions"));
    assert!(content.contains("FACTS_FROM_PROFESSIONAL"));
    // Quoted-evidence wrapping delimits the content.
    let evidence_block = content
        .split("--- QUOTED EVIDENCE")
        .nth(1)
        .expect("evidence section present");
    assert!(evidence_block.contains("<<<"));
    assert!(evidence_block.contains(">>>"));
}
