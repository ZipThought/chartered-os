//! Persuasive-context-exclusion runtime assertion tests. Spec
//! §Structural Separation: "The Runtime asserts Evaluator prompts
//! contain no agent-context fields before any Evaluator call.
//! Assertion failure halts evaluation. Tested as a runtime invariant."

use chartered_core::{CognitionRequest, EvaluatorError, Message, assert_no_persuasive_context};

fn req_with(content: &str) -> CognitionRequest {
    CognitionRequest {
        messages: vec![Message::system("evaluator system"), Message::user(content)],
        max_output_tokens: Some(64),
    }
}

#[test]
fn empty_user_prompt_passes_assertion() {
    assert!(assert_no_persuasive_context(&req_with("")).is_ok());
}

#[test]
fn well_formed_evaluator_prompt_passes() {
    let body = "--- AUTHORITY SCOPES (Charter; policy you must apply) ---\n\
                [policy]\nDO NOT REVEAL SECRETS\n\n\
                --- PROPOSAL ---\nTool: write_file\nParams: {}\n";
    assert!(assert_no_persuasive_context(&req_with(body)).is_ok());
}

#[test]
fn actor_observation_marker_triggers_assertion() {
    // The Actor's observation formatter emits "[GATE: ALLOWED" /
    // "[GATE: DENIED" lines that record what came back from the loop.
    // Those are persuasive context — they must not reach the
    // Evaluator. If they do, the assertion fires.
    let leak = "[GATE: ALLOWED, dispatched]\nresult: {}";
    let err = match assert_no_persuasive_context(&req_with(leak)) {
        Ok(_) => panic!("expected assertion to fire on Actor observation marker"),
        Err(EvaluatorError(e)) => e,
    };
    assert!(err.contains("persuasive-context-exclusion violation"));
    assert!(err.contains("[GATE: ALLOWED"));
}

#[test]
fn tester_summary_marker_triggers_assertion() {
    let leak = "--- RECENT SUT ACTIVITY ---\n- tool=send_message outcome=Allowed\n";
    let err = match assert_no_persuasive_context(&req_with(leak)) {
        Ok(_) => panic!("expected assertion to fire on Tester summary marker"),
        Err(EvaluatorError(e)) => e,
    };
    assert!(err.contains("persuasive-context-exclusion violation"));
    assert!(err.contains("RECENT SUT ACTIVITY"));
}

#[test]
fn actor_system_prompt_sentinel_triggers_assertion() {
    // The Actor's system prompt has a "--- BEHAVIORAL SPECIFICATION ---"
    // sentinel from the Runtime-side prompt assembler. It's the
    // Actor's content; if it appears in the Evaluator's prompt the
    // assembler has leaked.
    let leak = "--- BEHAVIORAL SPECIFICATION ---\nBe polite.\n";
    let err = match assert_no_persuasive_context(&req_with(leak)) {
        Ok(_) => panic!("expected assertion to fire on actor sentinel"),
        Err(EvaluatorError(e)) => e,
    };
    assert!(err.contains("BEHAVIORAL SPECIFICATION"));
}
