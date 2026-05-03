//! End-to-end tests against the chartered-runtime binary.
//!
//! Each test constructs a complete `.chartered/` deployment in an
//! isolated tempdir, runs the binary, and asserts on:
//!   - the binary's stdout JSON (Receipt trail + Judge output), and
//!   - actual filesystem state in the workspace root.
//!
//! The deployments are real production deployments — same loader,
//! same runtime path, same OS-touching dispatch tools. The only
//! distinction from a real-LLM deployment is the `backend = "fake"`
//! values in steward.toml, which select FakeCognitionBackend per
//! role with inline canned responses. ZERO test-only code paths.
//!
//! Tempdirs are system temp (outside the repo, naturally gitignored).
//! Each test gets its own tempdir for isolation.

mod common;

use std::process::Command;

use tempfile::TempDir;

use common::{
    BIN, TestDeployment, assert_success, list_files_under, parse_stdout_json, write_tool,
};

// -- Reusable charter snippets -------------------------------------------

const SCOPES_MD_EMPTY: &str = "# Charter scopes\n\n(none)\n";

fn frames_allow_all(tools: &[&str]) -> String {
    let tools_arr = tools
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
permitted_tools = [{tools_arr}]

[[frames]]
id = "always_allow"
concern = "test allow-everything frame"
applies_to_tools = [{tools_arr}]
declared_scopes = []
"#
    )
}

fn write_artifact_tools(dep: &TestDeployment) {
    write_tool(
        dep,
        "modify_artifact",
        "native_artifact_modify",
        "modify_artifact",
    );
    write_tool(
        dep,
        "record_finding",
        "native_artifact_record_finding",
        "record_finding",
    );
    write_tool(
        dep,
        "read_artifact",
        "native_artifact_read",
        "read_artifact",
    );
    write_tool(
        dep,
        "list_artifacts",
        "native_artifact_list",
        "list_artifacts",
    );
}

fn run_selection(
    dep: &TestDeployment,
    artifact_id: &str,
    start: usize,
    end: usize,
    action: &str,
    kind: &str,
) -> std::process::Output {
    Command::new(BIN)
        .arg("--chartered-dir")
        .arg(&dep.chartered_dir)
        .arg("--workspace-root")
        .arg(&dep.workspace_root)
        .arg("--selection-artifact")
        .arg(artifact_id)
        .arg("--selection-start")
        .arg(start.to_string())
        .arg("--selection-end")
        .arg(end.to_string())
        .arg("--selection-start-line")
        .arg("1")
        .arg("--selection-end-line")
        .arg("1")
        .arg("--selection-action")
        .arg(action)
        .arg("--selection-kind")
        .arg(kind)
        .output()
        .expect("binary executes")
}

// =========================================================================
// E2E TESTS
// =========================================================================

#[test]
fn e2e_selection_refine_modifies_artifact_after_allowed_receipt() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["modify_artifact"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_artifact_tools(&dep);
    std::fs::write(dep.workspace_file("deal.md"), "vendor liability cap").unwrap();

    let proposal = serde_json::json!({
        "tool": "modify_artifact",
        "params": {
            "kind": "text",
            "artifact_id": "deal.md",
            "range": { "start": 17, "end": 20, "start_line": 1, "end_line": 1 },
            "replacement": "carve-out",
            "summary": "Tightens vendor liability carve-out language"
        }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["modify_artifact.toml", "record_finding.toml", "read_artifact.toml", "list_artifacts.toml"]

[actor]
backend = "fake"
fake_responses = [{proposal}, {halt}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        proposal = serde_json::Value::String(proposal.to_string()),
        halt = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = run_selection(&dep, "deal.md", 0, 20, "Refine", "generative");
    assert_success(&out);
    let v = parse_stdout_json(&out);
    let receipts = v["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Allowed"));
    assert_eq!(
        receipts[0]["tool_call"]["tool"].as_str(),
        Some("modify_artifact")
    );
    assert_eq!(receipts[1]["tool_call"]["tool"].as_str(), Some("<halt>"));
    assert_eq!(
        std::fs::read_to_string(dep.workspace_file("deal.md")).unwrap(),
        "vendor liability carve-out"
    );
}

#[test]
fn e2e_selection_reject_refine_allow_changes_only_allowed_text() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["modify_artifact"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_artifact_tools(&dep);
    std::fs::write(dep.workspace_file("reply.md"), "confirm deal exists").unwrap();

    let unsafe_reply = serde_json::json!({
        "tool": "modify_artifact",
        "params": {
            "kind": "text",
            "artifact_id": "reply.md",
            "range": { "start": 0, "end": 19, "start_line": 1, "end_line": 1 },
            "replacement": "Yes, the deal exists.",
            "summary": "Confirm deal existence"
        }
    });
    let safe_reply = serde_json::json!({
        "tool": "modify_artifact",
        "params": {
            "kind": "text",
            "artifact_id": "reply.md",
            "range": { "start": 0, "end": 19, "start_line": 1, "end_line": 1 },
            "replacement": "Acknowledged. We cannot comment on any matter.",
            "summary": "Acknowledges receipt without confirming a deal"
        }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["modify_artifact.toml", "record_finding.toml", "read_artifact.toml", "list_artifacts.toml"]

[actor]
backend = "fake"
fake_responses = [{unsafe_reply}, {safe_reply}, {halt}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["DENY: disclosure risk during exclusivity", "ALLOW: ok"]
"#,
        unsafe_reply = serde_json::Value::String(unsafe_reply.to_string()),
        safe_reply = serde_json::Value::String(safe_reply.to_string()),
        halt = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = run_selection(&dep, "reply.md", 0, 19, "Draft Response", "generative");
    assert_success(&out);
    let v = parse_stdout_json(&out);
    let receipts = v["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 3);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Denied"));
    assert_eq!(receipts[1]["outcome"].as_str(), Some("Allowed"));
    assert_eq!(receipts[2]["tool_call"]["tool"].as_str(), Some("<halt>"));
    let tasks = v["tasks"].as_array().unwrap();
    let attempts = v["attempts"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(attempts.len(), 2);
    let task_id = tasks[0]["task_id"].as_str().unwrap();
    assert!(attempts.iter().all(|a| a["task_id"].as_str() == Some(task_id)));
    assert!(receipts.iter().all(|r| r["task_id"].as_str() == Some(task_id)));
    assert_eq!(
        std::fs::read_to_string(dep.workspace_file("reply.md")).unwrap(),
        "Acknowledged. We cannot comment on any matter."
    );
}

#[test]
fn e2e_selection_review_records_finding_without_mutating_artifact() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["record_finding"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_artifact_tools(&dep);
    let original = "shared cache without tenant isolation";
    std::fs::write(dep.workspace_file("architecture.md"), original).unwrap();

    let proposal = serde_json::json!({
        "tool": "record_finding",
        "params": {
            "artifact_id": "architecture.md",
            "range": { "start": 0, "end": 37, "start_line": 1, "end_line": 1 },
            "concern": "Tenant isolation",
            "severity": "high",
            "detail": "Shared cache lacks tenant keying"
        }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["modify_artifact.toml", "record_finding.toml", "read_artifact.toml", "list_artifacts.toml"]

[actor]
backend = "fake"
fake_responses = [{proposal}, {halt}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        proposal = serde_json::Value::String(proposal.to_string()),
        halt = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = run_selection(
        &dep,
        "architecture.md",
        0,
        original.len(),
        "Review",
        "evaluative",
    );
    assert_success(&out);
    let v = parse_stdout_json(&out);
    let receipts = v["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Allowed"));
    assert_eq!(
        receipts[0]["tool_call"]["tool"].as_str(),
        Some("record_finding")
    );
    assert_eq!(receipts[1]["tool_call"]["tool"].as_str(), Some("<halt>"));
    assert_eq!(
        std::fs::read_to_string(dep.workspace_file("architecture.md")).unwrap(),
        original
    );
    let findings =
        std::fs::read_to_string(dep.workspace_file(".chartered/findings.jsonl")).unwrap();
    let finding: serde_json::Value = serde_json::from_str(findings.trim()).unwrap();
    assert_eq!(finding["task_id"], receipts[0]["task_id"]);
    assert_eq!(finding["author_steward_id"], "sut");
    assert!(finding.get("frame_id").is_none());
    assert_eq!(finding["artifact_id"], "architecture.md");
    assert_eq!(finding["admitting_receipt_id"], receipts[0]["receipt_id"]);
}

/// Happy path: Actor proposes write_file → Frame ALLOW → file written →
/// Actor halts. Trail has one Receipt with Outcome::Allowed; the file
/// exists in the workspace with the right content.
#[test]
fn e2e_write_file_then_halt() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "out/notes.md", "content": "# notes\n\nfirst entry\n" }
    });
    let halt = serde_json::json!({"halt": true});

    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [
  {actor_propose},
  {actor_halt}
]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        actor_propose = serde_json::Value::String(propose.to_string()),
        actor_halt = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("write notes.md");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    // write_file Allowed + synthetic Halt Receipt.
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Allowed"));
    assert_eq!(receipts[1]["tool_call"]["tool"].as_str(), Some("<halt>"));

    // Reconciliation: the file actually exists with the right content.
    let written = std::fs::read_to_string(dep.workspace_file("out/notes.md")).unwrap();
    assert_eq!(written, "# notes\n\nfirst entry\n");
}

/// Capability denial: Actor proposes a tool not in permitted_tools.
/// Receipt has outcome Denied, no Verdicts; budget=0 escalates.
/// No file is created in the workspace.
#[test]
fn e2e_capability_denial_no_dispatch() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "send_email",   // NOT in permitted_tools
        "params": { "to": "x", "body": "y" }
    });
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{propose_str}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = []
"#,
        propose_str = serde_json::Value::String(propose.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = Command::new(BIN)
        .arg("--chartered-dir")
        .arg(&dep.chartered_dir)
        .arg("--workspace-root")
        .arg(&dep.workspace_root)
        .arg("--user-message")
        .arg("propose forbidden tool")
        .arg("--refinement-budget")
        .arg("0")
        .output()
        .expect("binary executes");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    // Capability deny + BudgetExhausted controller event (budget=0).
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Denied"));
    let verdicts = receipts[0]["verdicts"].as_array().unwrap();
    assert!(verdicts.is_empty(), "capability denial has no Verdicts");
    assert_eq!(receipts[1]["outcome"].as_str(), Some("Escalated"));
    assert_eq!(
        receipts[1]["tool_call"]["tool"].as_str(),
        Some("<budget_exhausted>")
    );
    assert_eq!(receipts[0]["task_id"], receipts[1]["task_id"]);
    assert!(receipts[1]["attempt_id"].is_null());

    // No file should exist anywhere in the workspace beyond .chartered/.
    let workspace_files = list_files_under(&dep.workspace_root);
    let leaked: Vec<_> = workspace_files
        .iter()
        .filter(|p| !p.starts_with(&dep.chartered_dir) && !p.starts_with(&dep.charter_dir))
        .collect();
    assert!(leaked.is_empty(), "unexpected files: {leaked:?}");
}

/// Refinement convergence: Frame DENIES first proposal; Actor refines;
/// Frame ALLOWS the refined proposal; loop halts. Trail has Denied then
/// Allowed; only the second proposal's file exists.
#[test]
fn e2e_frame_denial_then_refinement_converges() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose1 = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "draft.md", "content": "v0 draft" }
    });
    let propose2 = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "final.md", "content": "v1 final" }
    });
    let halt = serde_json::json!({"halt": true});

    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p1}, {p2}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["DENY: draft path rejected", "ALLOW: ok"]
"#,
        p1 = serde_json::Value::String(propose1.to_string()),
        p2 = serde_json::Value::String(propose2.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("write a spec");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    // Denied (refined) + Allowed (final) + Halt.
    assert_eq!(receipts.len(), 3);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Denied"));
    assert_eq!(receipts[1]["outcome"].as_str(), Some("Allowed"));
    assert_eq!(receipts[2]["tool_call"]["tool"].as_str(), Some("<halt>"));

    assert!(!dep.workspace_file("draft.md").exists());
    assert_eq!(
        std::fs::read_to_string(dep.workspace_file("final.md")).unwrap(),
        "v1 final"
    );
}

/// Budget exhaustion: Frame keeps DENYing; Actor keeps proposing.
/// LoopRunner escalates after refinement_budget+1 denials. Final
/// Receipt's outcome is Escalated; no file written.
#[test]
fn e2e_budget_exhaustion_escalates() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "bad.md", "content": "x" }
    });
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}, {p}, {p}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["DENY: no", "DENY: no", "DENY: no"]
"#,
        p = serde_json::Value::String(propose.to_string()),
    );
    dep.write("steward.toml", &steward);

    // refinement_budget=2 → escalate after 3rd consecutive denial.
    let out = Command::new(BIN)
        .arg("--chartered-dir")
        .arg(&dep.chartered_dir)
        .arg("--workspace-root")
        .arg(&dep.workspace_root)
        .arg("--user-message")
        .arg("try forever")
        .arg("--refinement-budget")
        .arg("2")
        .output()
        .expect("binary executes");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    // budget=2: the LoopRunner accepts 2 refinements; the third Denied
    // triggers controller exhaustion. Trail = 3 Denied (Gate verdicts) +
    // 1 BudgetExhausted (controller event) — disk and memory agree on
    // every Receipt; no after-the-fact mutation.
    assert_eq!(
        receipts.len(),
        4,
        "expected 4 Receipts (3 Denied + BudgetExhausted), got {}",
        receipts.len()
    );
    for r in &receipts[..3] {
        assert_eq!(r["outcome"].as_str(), Some("Denied"));
    }
    let last = &receipts[receipts.len() - 1];
    assert_eq!(last["outcome"].as_str(), Some("Escalated"));
    assert_eq!(
        last["tool_call"]["tool"].as_str(),
        Some("<budget_exhausted>")
    );
    for r in receipts {
        assert_eq!(r["task_id"], last["task_id"]);
    }
    assert!(last["attempt_id"].is_null());
    assert!(!dep.workspace_file("bad.md").exists());
}

/// Actor parse failure: Actor's fake response is not valid JSON →
/// Action::Fail → Receipt with intercept_complete=false, outcome
/// Escalated, tool_call.tool = "<actor_failure>". Spec §Risk Register
/// > Silent Failure: cognitive failure surfaces with operator visibility.
#[test]
fn e2e_actor_malformed_response_escalates_with_intercept_incomplete() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let steward = r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = ["this response is not JSON at all"]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = []
"#;
    dep.write("steward.toml", steward);

    let out = dep.run_with_user_message("trigger parse failure");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 1);
    let r = &receipts[0];
    assert_eq!(r["outcome"].as_str(), Some("Escalated"));
    assert_eq!(r["intercept_complete"].as_bool(), Some(false));
    assert_eq!(r["tool_call"]["tool"].as_str(), Some("<actor_failure>"));
}

/// Path traversal: Actor proposes write_file with `../escape.txt`. The
/// Frame says ALLOW (no path inspection in this Frame), so dispatch
/// runs; NativeFsWrite rejects the path because it canonicalizes
/// outside the workspace root. Observation::Accepted carries
/// ToolResult::Err. Actor halts. Trail shows Allowed (Gate said yes)
/// but no file is created outside the workspace root.
#[test]
fn e2e_path_traversal_denied_by_dispatch_not_silently_allowed() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "../escape.txt", "content": "leak" }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: frame doesn't inspect path"]
"#,
        p = serde_json::Value::String(propose.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("try escape");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    // write_file Allowed (Gate said yes) + Halt.
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Allowed"));
    assert_eq!(receipts[1]["tool_call"]["tool"].as_str(), Some("<halt>"));

    // Reconciliation: no escape file created above workspace root.
    let parent = dep.workspace_root.parent().unwrap();
    assert!(
        !parent.join("escape.txt").exists(),
        "path traversal must not create files outside workspace root"
    );
}

/// Across-Frame conjunction: two Frames apply; one DENIES, one ALLOWS.
/// Receipt has both Verdicts; Outcome is Denied; Refinement signal
/// names only the violating Frame.
#[test]
fn e2e_across_frame_conjunction_records_both_verdicts() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    let frames = r#"
permitted_tools = ["write_file"]

[[frames]]
id = "frame_a"
concern = "frame a"
applies_to_tools = ["write_file"]
declared_scopes = []

[[frames]]
id = "frame_b"
concern = "frame b"
applies_to_tools = ["write_file"]
declared_scopes = []
"#;
    dep.write_charter(frames, SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "x.md", "content": "x" }
    });
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
frame_a = ["DENY: a says no"]
frame_b = ["ALLOW: b says yes"]
"#,
        p = serde_json::Value::String(propose.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = Command::new(BIN)
        .arg("--chartered-dir")
        .arg(&dep.chartered_dir)
        .arg("--workspace-root")
        .arg(&dep.workspace_root)
        .arg("--user-message")
        .arg("try")
        .arg("--refinement-budget")
        .arg("0")
        .output()
        .expect("binary executes");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    let r = &receipts[0];
    let verdicts = r["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), 2, "both Frames must contribute Verdicts");
    let outcome = r["outcome"].as_str().unwrap();
    assert!(outcome == "Denied" || outcome == "Escalated");
    assert!(!dep.workspace_file("x.md").exists());
}

/// Multi-turn with [tester] in steward.toml: Tester emits 2 messages,
/// Actor responds per turn. The same code path that runs production
/// (with --user-message) runs here (with [tester]); only the Tester
/// configuration differs.
#[test]
fn e2e_tester_in_steward_drives_multi_turn() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose1 = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "turn1.md", "content": "t1" }
    });
    let propose2 = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "turn2.md", "content": "t2" }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p1}, {h}, {p2}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok", "ALLOW: ok"]

[tester]
backend = "fake"
brief = "send two messages"
max_turns = 2
fake_responses = ["write turn1", "write turn2"]
"#,
        p1 = serde_json::Value::String(propose1.to_string()),
        p2 = serde_json::Value::String(propose2.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    // No --user-message; Tester drives.
    let out = dep.run_no_user_message();
    assert_success(&out);
    let v = parse_stdout_json(&out);

    assert_eq!(v["turns"].as_u64(), Some(2));
    let receipts = v["receipts"].as_array().unwrap();
    // 2 turns × (1 Allowed write_file + 1 Halt) = 4.
    assert_eq!(receipts.len(), 4);
    assert!(dep.workspace_file("turn1.md").exists());
    assert!(dep.workspace_file("turn2.md").exists());
}

/// Workspace validation: Frame declares a Charter Scope that doesn't
/// exist. Binary exits nonzero with a structured error before any
/// loop step runs.
#[test]
fn e2e_workspace_validation_fails_on_missing_scope() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    let bad_frames = r#"
permitted_tools = ["write_file"]

[[frames]]
id = "needs_missing_scope"
concern = "references nonexistent scope"
applies_to_tools = ["write_file"]
declared_scopes = [{ name = "missing", kind = "Charter" }]
"#;
    dep.write_charter(bad_frames, SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");
    // Provide non-empty fake_responses so the binary reaches workspace
    // validation (which is what this test exercises) before failing.
    dep.write(
        "steward.toml",
        r#"
system_prompt = "x"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = ["{\"halt\": true}"]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
needs_missing_scope = []
"#,
    );

    let out = dep.run_with_user_message("x");
    assert!(!out.status.success(), "binary should reject missing scope");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("missing") || stderr.contains("Charter"),
        "stderr did not name the error: {stderr}"
    );
}

/// Unknown executor in tools/*.toml. Binary exits nonzero before the
/// loop runs.
#[test]
fn e2e_unknown_executor_fails_at_startup() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    dep.write(
        "tools/write_file.toml",
        "id = \"write_file\"\nexecutor = \"nonexistent_executor_kind\"\n",
    );
    // Provide a non-empty actor backend so the binary reaches executor
    // construction (the path this test exercises) before failing.
    dep.write(
        "steward.toml",
        r#"
system_prompt = "x"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = ["{\"halt\": true}"]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = []
"#,
    );

    let out = dep.run_with_user_message("x");
    assert!(
        !out.status.success(),
        "binary should reject unknown executor"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nonexistent_executor_kind") || stderr.contains("unknown executor"),
        "stderr did not name the unknown executor: {stderr}"
    );
}

/// Read-after-write roundtrip with native_fs_read + native_fs_write:
/// the Steward writes a file in turn 1, reads it back in turn 2,
/// and the Receipt reflects what the file contained.
#[test]
fn e2e_read_after_write_roundtrip_via_dispatch() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(
        &frames_allow_all(&["write_file", "read_file"]),
        SCOPES_MD_EMPTY,
    );
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");
    write_tool(&dep, "read_file", "native_fs_read", "read_file");

    let write = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "data.txt", "content": "secret-token-42" }
    });
    let read = serde_json::json!({
        "tool": "read_file",
        "params": { "path": "data.txt" }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml", "read_file.toml"]

[actor]
backend = "fake"
fake_responses = [{w}, {r}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok", "ALLOW: ok"]
"#,
        w = serde_json::Value::String(write.to_string()),
        r = serde_json::Value::String(read.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("write then read");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    // write Allowed + read Allowed + Halt.
    assert_eq!(receipts.len(), 3);
    for r in &receipts[..3] {
        assert_eq!(r["outcome"].as_str(), Some("Allowed"));
    }
    assert_eq!(receipts[2]["tool_call"]["tool"].as_str(), Some("<halt>"));
    let on_disk = std::fs::read_to_string(dep.workspace_file("data.txt")).unwrap();
    assert_eq!(on_disk, "secret-token-42");
}

/// exec_command with native_exec: the Steward runs `echo` and the
/// loop completes against a real subprocess.
#[test]
#[cfg(unix)]
fn e2e_exec_command_via_native_exec() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["exec_command"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "exec_command", "native_exec", "exec_command");

    let exec = serde_json::json!({
        "tool": "exec_command",
        "params": { "cmd": "echo", "args": ["hello-from-real-subprocess"] }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["exec_command.toml"]

[actor]
backend = "fake"
fake_responses = [{e}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        e = serde_json::Value::String(exec.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("run echo");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts = v["receipts"].as_array().unwrap();
    // exec_command Allowed + Halt.
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["outcome"].as_str(), Some("Allowed"));
    assert_eq!(receipts[1]["tool_call"]["tool"].as_str(), Some("<halt>"));
}

/// Binary exits nonzero with a structured error when no .chartered/
/// can be found anywhere by walk-up.
#[test]
fn e2e_walk_up_failure_when_no_chartered_dir() {
    let tmp = TempDir::new().unwrap();
    // Override HOME so the home-dir fallback also misses.
    let out = Command::new(BIN)
        .env("HOME", tmp.path())
        .current_dir(tmp.path())
        .arg("--user-message")
        .arg("x")
        .output()
        .expect("binary executes");
    assert!(!out.status.success(), "expected nonzero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(".chartered/") || stderr.contains("not found"),
        "stderr did not name the missing dir: {stderr}"
    );
}

/// Binary exits nonzero when neither --user-message nor [tester] are
/// provided.
#[test]
fn e2e_neither_user_message_nor_tester_fails() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");
    dep.write(
        "steward.toml",
        r#"
system_prompt = "x"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = []

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = []
"#,
    );
    let out = dep.run_no_user_message();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("user-message") || stderr.contains("tester"),
        "stderr did not name the missing input: {stderr}"
    );
}

// -- Persistence-layer tests --------------------------------------------

/// Each binary invocation persists a receipts.jsonl that contains
/// every Receipt as one JSON line, matching the stdout JSON. The file
/// lives under <chartered_dir>/runs/<run_id>/.
#[test]
fn e2e_receipts_jsonl_persisted_and_grepable() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "out.txt", "content": "x" }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        p = serde_json::Value::String(propose.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("write");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let receipts_log = v["receipts_log"].as_str().unwrap();
    let path = std::path::PathBuf::from(receipts_log);
    assert!(path.exists(), "receipts.jsonl missing: {receipts_log}");

    // One Receipt per line, valid JSON; line count matches stdout count.
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let stdout_count = v["receipts"].as_array().unwrap().len();
    assert_eq!(lines.len(), stdout_count);
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line).expect("valid jsonl");
        assert!(parsed["receipt_id"].is_string());
        assert!(parsed["outcome"].is_string());
    }

    // grep-able: lines containing "Allowed" — both the write_file Receipt
    // and the synthetic Halt Receipt. The Halt Receipt is itself
    // grep-able as `"tool":"<halt>"` so operators can locate clean
    // terminations without filtering by outcome.
    let allowed_count = lines
        .iter()
        .filter(|l| l.contains("\"outcome\":\"Allowed\""))
        .count();
    assert_eq!(allowed_count, 2);
    let halt_count = lines
        .iter()
        .filter(|l| l.contains("\"tool\":\"<halt>\""))
        .count();
    assert_eq!(halt_count, 1);
}

/// Each binary invocation persists a cognition.jsonl that records
/// every LLM call (request + response or error) keyed by backend_id,
/// so operators can grep prompts and responses by role.
#[test]
fn e2e_cognition_jsonl_records_every_llm_call() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "x.txt", "content": "y" }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        p = serde_json::Value::String(propose.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("write file");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let cognition_log = v["cognition_log"].as_str().unwrap();
    let path = std::path::PathBuf::from(cognition_log);
    assert!(path.exists(), "cognition.jsonl missing");

    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    // Expected calls: 2 actor (propose + halt) + 1 evaluator (always_allow on the propose).
    assert_eq!(
        lines.len(),
        3,
        "expected 3 LLM call entries; got {}",
        lines.len()
    );

    let entries: Vec<serde_json::Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).expect("valid jsonl"))
        .collect();

    // Every entry has backend_id, request, response (Ok) or error (Err).
    for e in &entries {
        assert!(e["backend_id"].is_string());
        assert!(e["request"].is_object());
        assert!(e["started_ns"].is_number());
        assert!(e["finished_ns"].is_number());
        assert!(
            e["response"].is_object() || e["error"].is_string(),
            "entry must have response or error"
        );
    }

    // Operator can grep for the actor and the evaluator separately.
    let actor_calls: Vec<_> = entries
        .iter()
        .filter(|e| e["backend_id"].as_str() == Some("actor"))
        .collect();
    let eval_calls: Vec<_> = entries
        .iter()
        .filter(|e| e["backend_id"].as_str() == Some("eval-always_allow"))
        .collect();
    assert_eq!(actor_calls.len(), 2);
    assert_eq!(eval_calls.len(), 1);

    // Prompts and responses are present in the log content.
    let actor_first = &actor_calls[0];
    let user_msg = actor_first["request"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user")
        .expect("user message in actor request");
    assert!(
        user_msg["content"].as_str().unwrap().contains("write file"),
        "actor's first user message should carry the --user-message"
    );
    assert!(
        actor_first["response"]["text"]
            .as_str()
            .unwrap()
            .contains("write_file")
    );
    assert_eq!(
        eval_calls[0]["response"]["text"].as_str(),
        Some("ALLOW: ok")
    );
}

/// Two binary invocations against the same deployment land in two
/// distinct run directories. Each run's logs are isolated from the
/// other.
#[test]
fn e2e_per_run_isolation_in_runs_subdir() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "a.txt", "content": "1" }
    });
    let halt = serde_json::json!({"halt": true});
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        p = serde_json::Value::String(propose.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out_a = dep.run_with_user_message("first");
    assert_success(&out_a);
    let v_a = parse_stdout_json(&out_a);

    // Re-prime the actor backend for the second invocation by rewriting
    // steward.toml with a fresh queue (the file is the only state
    // between invocations; the actor's queued responses live there).
    let propose2 = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "b.txt", "content": "2" }
    });
    let steward2 = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}, {h}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["ALLOW: ok"]
"#,
        p = serde_json::Value::String(propose2.to_string()),
        h = serde_json::Value::String(halt.to_string()),
    );
    dep.write("steward.toml", &steward2);

    // Sleep 1ms so timestamps differ; run_id is timestamp-prefixed.
    std::thread::sleep(std::time::Duration::from_millis(2));
    let out_b = dep.run_with_user_message("second");
    assert_success(&out_b);
    let v_b = parse_stdout_json(&out_b);

    let run_a = v_a["run_id"].as_str().unwrap();
    let run_b = v_b["run_id"].as_str().unwrap();
    assert_ne!(run_a, run_b, "each invocation gets its own run_id");

    let dir_a = std::path::PathBuf::from(v_a["run_dir"].as_str().unwrap());
    let dir_b = std::path::PathBuf::from(v_b["run_dir"].as_str().unwrap());
    assert_ne!(dir_a, dir_b);

    // Each run's receipts.jsonl is isolated.
    let text_a = std::fs::read_to_string(dir_a.join("receipts.jsonl")).unwrap();
    let text_b = std::fs::read_to_string(dir_b.join("receipts.jsonl")).unwrap();
    assert!(text_a.contains("\"a.txt\""));
    assert!(!text_a.contains("\"b.txt\""));
    assert!(text_b.contains("\"b.txt\""));
    assert!(!text_b.contains("\"a.txt\""));

    // Both files exist on disk.
    assert!(dep.workspace_file("a.txt").exists());
    assert!(dep.workspace_file("b.txt").exists());

    // Both run dirs nested under <chartered_dir>/runs/.
    let runs_root = dep.chartered_dir.join("runs");
    let run_subdirs: Vec<_> = std::fs::read_dir(&runs_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    assert!(run_subdirs.len() >= 2);
}

/// Cognition log records backend errors (e.g., empty fake queue) so
/// operators can debug failed LLM calls from the trail. Pairs with
/// the actor-failure E2E that asserts the Receipt visibility.
#[test]
fn e2e_cognition_log_captures_backend_error() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_all(&["write_file"]), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_tool(&dep, "write_file", "native_fs_write", "write_file");

    // Actor backend has only ONE response, but the loop will need
    // a second call after the first proposal is rejected. The second
    // call exhausts the fake queue → CognitionError → Action::Fail.
    let propose = serde_json::json!({
        "tool": "write_file",
        "params": { "path": "x.txt", "content": "x" }
    });
    let steward = format!(
        r#"
system_prompt = "test"
tool_registry = ["write_file.toml"]

[actor]
backend = "fake"
fake_responses = [{p}]

[evaluator]
backend = "fake"
[evaluator.fake_responses]
always_allow = ["DENY: nope"]
"#,
        p = serde_json::Value::String(propose.to_string()),
    );
    dep.write("steward.toml", &steward);

    let out = dep.run_with_user_message("trigger backend error");
    assert_success(&out);
    let v = parse_stdout_json(&out);

    let cognition_log = v["cognition_log"].as_str().unwrap();
    let text = std::fs::read_to_string(cognition_log).unwrap();
    // At least one entry should carry a non-null `error` field.
    let has_error = text
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .any(|e| e["error"].is_string());
    assert!(
        has_error,
        "expected at least one error entry in cognition.jsonl"
    );
}
