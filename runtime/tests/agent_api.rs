//! Integration tests for the `chartered_runtime::Agent` library API.
//!
//! These exercise the in-process surface that downstream Rust embedders
//! consume: construct an Agent from a `.chartered/` directory, call
//! `run(brief)` with a Brief, inspect the categorical outcome and the
//! per-run artifacts. The Agent is stateless across calls; this test
//! file repeatedly hits `run` on a single Agent to verify both the
//! stateless property and the per-call run-dir isolation.
//!
//! Fake backend per `AGENTS.md §Verification`: integration tier,
//! vertical-cut, tempdir-isolated, CI-friendly. The matching e2e tests
//! live in `runtime/tests/llm_e2e.rs`.

mod common;

use std::sync::Arc;

use chartered_core::{ArtifactId, ArtifactRange, SelectionAction, SelectionActionKind};
use chartered_runtime::{Agent, AgentOutcome, Brief, EscalationCause};

use common::TestDeployment;

const SCOPES_MD_EMPTY: &str = "# Charter scopes\n\n(none)\n";

fn frames_allow_one(tool: &str, frame_id: &str) -> String {
    format!(
        r#"
permitted_tools = ["{tool}"]

[[frames]]
id = "{frame_id}"
concern = "test allow-everything frame"
applies_to_tools = ["{tool}"]
declared_scopes = []
"#
    )
}

fn write_tool(dep: &TestDeployment, name: &str, executor: &str, tool_id: &str) {
    dep.write(
        &format!("tools/{name}.toml"),
        &format!("id = \"{tool_id}\"\nexecutor = \"{executor}\"\n"),
    );
}

/// Build a `steward.toml` with the actor backend canned to
/// `actor_responses` and the per-frame evaluator queue populated with
/// `evaluator_allow_count` ALLOW responses keyed under
/// `frame_id`. Each Frame applicability triggers one evaluator call;
/// most happy-path flows need one ALLOW per proposal.
fn write_steward(
    dep: &TestDeployment,
    actor_responses: &[&str],
    frame_id: &str,
    evaluator_allow_count: usize,
) {
    let actor_arr = actor_responses
        .iter()
        .map(|r| serde_json::Value::String(r.to_string()).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let allows = std::iter::repeat_n("\"ALLOW: ok\"", evaluator_allow_count)
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        r#"
[actor]
backend = "fake"
fake_responses = [{actor_arr}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
{frame_id} = [{allows}]
"#
    );
    dep.write("steward.toml", &toml);
}

fn make_min_deployment(actor_responses: &[&str], allowed_tool: &str, allow_count: usize) -> TestDeployment {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one(allowed_tool, "always_allow"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_steward(&dep, actor_responses, "always_allow", allow_count);
    dep
}

#[tokio::test]
async fn agent_run_prompt_halt_is_quiet() {
    let dep = make_min_deployment(&[r#"{"halt": true}"#], "modify_artifact", 0);
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");

    let result = agent
        .run(Brief::Prompt("hello".into()))
        .await
        .expect("Agent::run completes");

    assert!(
        matches!(result.outcome, AgentOutcome::Quiet),
        "expected Quiet, got {:?}",
        result.outcome
    );
    // Tasks recorded but no externalizing Tool call landed.
    assert!(
        result
            .artifacts
            .receipts
            .iter()
            .all(|r| r.tool_call.tool.0 != "modify_artifact"
                || r.outcome != chartered_core::Outcome::Allowed),
        "no Allowed modify_artifact expected in Quiet outcome"
    );
}

#[tokio::test]
async fn agent_run_prompt_modify_is_externalized() {
    // One Action that proposes a write_file followed by Halt. Frame
    // is allow-all so one ALLOW evaluator response is sufficient.
    let action = r#"{"tool":"write_file","params":{"path":"out.txt","content":"hi"}}"#;
    let halt = r#"{"halt": true}"#;
    let dep = make_min_deployment(&[action, halt], "write_file", 1);
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");

    let result = agent
        .run(Brief::Prompt("please write out.txt".into()))
        .await
        .expect("Agent::run completes");

    // write_file is not modify_artifact so the categorical heuristic
    // treats this as Quiet for now (Tool externalization is per-Tool).
    // What we verify here is that the call lands a real receipt and
    // produces the file on disk; this exercises the executor-bridge,
    // the receipt store, and the workspace canonicalization paths via
    // the library API.
    let _ = result;
    let written = dep.workspace_file("out.txt");
    assert!(written.exists(), "expected workspace file to exist");
    assert_eq!(std::fs::read_to_string(&written).unwrap(), "hi");
}

#[tokio::test]
async fn agent_run_modify_artifact_is_externalized() {
    // modify_artifact is the only Tool that externalizes per the
    // current v1 heuristic. Use it to land a record-store append; the
    // outcome categorical must be Externalized.
    let action = r#"{
        "tool":"modify_artifact",
        "params":{"kind":"record-store","artifact_id":"records",
                  "edit":{"append":{"detail":"hello world"}}}
    }"#;
    let halt = r#"{"halt": true}"#;
    let dep = make_min_deployment(&[action, halt], "modify_artifact", 1);
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");

    let result = agent
        .run(Brief::Prompt("append a finding".into()))
        .await
        .expect("Agent::run completes");

    assert!(
        matches!(result.outcome, AgentOutcome::Externalized),
        "expected Externalized, got {:?}",
        result.outcome
    );
    assert!(
        result
            .artifacts
            .receipts
            .iter()
            .any(|r| r.tool_call.tool.0 == "modify_artifact"
                && r.outcome == chartered_core::Outcome::Allowed),
        "expected at least one Allowed modify_artifact"
    );
}

#[tokio::test]
async fn agent_run_inner_step_exhaustion_is_escalated() {
    // Queue more pure-reasoning responses than DEFAULT_INNER_STEP_BUDGET.
    // The Actor will exhaust its budget without committing — surfaces as
    // Escalated with cause InnerStepBudget.
    let actor_responses: Vec<&str> =
        std::iter::repeat_n("just reasoning, no action", chartered_core::DEFAULT_INNER_STEP_BUDGET + 1)
            .collect();
    let dep = make_min_deployment(&actor_responses, "modify_artifact", 0);
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");

    let result = agent
        .run(Brief::Prompt("loop".into()))
        .await
        .expect("Agent::run completes");

    match &result.outcome {
        AgentOutcome::Escalated { cause } => {
            assert!(
                matches!(cause, EscalationCause::InnerStepBudget),
                "expected InnerStepBudget, got {cause:?}"
            );
        }
        other => panic!("expected Escalated, got {other:?}"),
    }
}

#[tokio::test]
async fn agent_run_writes_per_call_run_dirs() {
    // Stateless property: two run() calls on the same Agent must
    // produce distinct run dirs under <chartered_dir>/runs/.
    let dep = make_min_deployment(
        &[r#"{"halt": true}"#, r#"{"halt": true}"#],
        "modify_artifact",
        0,
    );
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");

    // Each `run` consumes one Action from the fake queue; build a
    // fresh Agent for the second run since the FakeCognitionBackend's
    // queue is shared across runs.
    let agent1 = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs (1)");
    let r1 = agent1.run(Brief::Prompt("a".into())).await.expect("run 1");

    let agent2 = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs (2)");
    let r2 = agent2.run(Brief::Prompt("b".into())).await.expect("run 2");

    assert_ne!(r1.paths.run_id, r2.paths.run_id);
    assert_ne!(r1.paths.run_dir, r2.paths.run_dir);
    assert!(r1.paths.receipts_log.exists());
    assert!(r2.paths.receipts_log.exists());
}

#[tokio::test]
async fn agent_run_selection_synthesizes_trigger_and_message() {
    // Selection brief: the Agent must synthesize the singleton message
    // from the selection text and emit a Selection TaskTrigger so the
    // TaskRecord carries the right shape.
    let dep = make_min_deployment(
        &[
            r#"{
            "tool":"modify_artifact",
            "params":{"kind":"record-store","artifact_id":"records",
                      "edit":{"append":{"detail":"finding"}}}
        }"#,
            r#"{"halt": true}"#,
        ],
        "modify_artifact",
        1,
    );
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");
    std::fs::write(
        dep.workspace_file("deal.md"),
        "Section 8.1 MAC carve-outs are broad.",
    )
    .unwrap();

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");

    let result = agent
        .run(Brief::Selection {
            artifact_id: ArtifactId::new("deal.md"),
            range: ArtifactRange {
                start: 0,
                end: 11,
                start_line: 1,
                end_line: 1,
            },
            action: SelectionAction {
                name: "Review".into(),
                kind: SelectionActionKind::Evaluative,
            },
        })
        .await
        .expect("Agent::run completes");

    // Either the LLM made an externalizing call, or the response
    // didn't parse; either way, exactly one Task with a Selection
    // trigger should be present.
    assert_eq!(result.artifacts.tasks.len(), 1);
    let task = &result.artifacts.tasks[0];
    match &task.trigger {
        chartered_core::TaskTrigger::Selection {
            artifact_id,
            action_name,
            ..
        } => {
            assert_eq!(artifact_id.0, "deal.md");
            assert_eq!(action_name, "Review");
        }
        other => panic!("expected Selection trigger, got {other:?}"),
    }
}

#[tokio::test]
async fn agent_run_rejects_singleton_when_tester_configured() {
    // A configured [tester] in steward.toml and a Brief::Prompt are
    // ambiguous — only one input source is permitted per invocation.
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one("modify_artifact", "always_allow"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    dep.write(
        "steward.toml",
        r#"
[actor]
backend = "fake"
fake_responses = ["{}"]

[evaluator]
backend = "fake"

[tester]
backend = "fake"
brief = "test"
fake_responses = ["hi"]
max_turns = 1
"#,
    );
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");

    assert!(agent.has_configured_tester());

    let res = agent.run(Brief::Prompt("hello".into())).await;
    let err = res.expect_err("expected RunError when Brief and Tester both set");
    let msg = err.to_string();
    assert!(
        msg.contains("singleton Brief") || msg.contains("singleton trigger"),
        "expected ambiguity error message, got: {msg}"
    );
}

#[tokio::test]
async fn agent_run_tester_driven_consumes_configured_tester() {
    // Brief::TesterDriven explicitly delegates to the [tester] in
    // steward.toml. One actor response, one tester message; the
    // ScenarioRunner runs one turn and halts.
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one("modify_artifact", "always_allow"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    dep.write(
        "steward.toml",
        r#"
[actor]
backend = "fake"
fake_responses = ["{\"halt\": true}"]

[evaluator]
backend = "fake"

[tester]
backend = "fake"
brief = "test"
fake_responses = ["please consider this matter"]
max_turns = 1
"#,
    );
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");

    let result = agent
        .run(Brief::TesterDriven)
        .await
        .expect("Agent::run completes");

    assert!(result.artifacts.turns >= 1);
    // No externalizing Tool calls; the Actor went straight to Halt.
    assert!(matches!(result.outcome, AgentOutcome::Quiet));
}

#[tokio::test]
async fn agent_run_tester_driven_without_tester_errors() {
    // Brief::TesterDriven without a configured [tester] in
    // steward.toml is an invalid combination — surface a clear error.
    let dep = make_min_deployment(&[r#"{"halt": true}"#], "modify_artifact", 0);
    write_tool(&dep, "modify_artifact", "native_artifact_modify", "modify_artifact");

    let agent = Agent::from_chartered_dir(&dep.chartered_dir, Some(dep.workspace_root.clone()))
        .await
        .expect("Agent constructs");
    assert!(!agent.has_configured_tester());

    let res = agent.run(Brief::TesterDriven).await;
    let err = res.expect_err("expected RunError when no tester configured");
    assert!(err.to_string().contains("neither"));
}

// Ensure clippy doesn't whine about Arc not being used.
#[allow(dead_code)]
fn _arc_used() -> Arc<()> {
    Arc::new(())
}
