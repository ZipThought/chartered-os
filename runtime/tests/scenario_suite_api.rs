//! Happy-path integration test for the verification harness.
//!
//! Materializes a fake-backend deployment, writes a small corpus, runs
//! the harness in-process, and asserts on the aggregated report
//! shape. The harness itself reuses one Agent across all scenarios —
//! this test exercises the per-call stateless property.

mod common;

use std::path::PathBuf;

use chartered_runtime::scenario_suite;

use common::TestDeployment;

const SCOPES_MD_EMPTY: &str = "# Charter scopes\n\n(none)\n";

fn frames_allow_one(tool: &str) -> String {
    format!(
        r#"
permitted_tools = ["{tool}"]

[[frames]]
id = "always_allow"
concern = "allow-everything frame"
applies_to_tools = ["{tool}"]
declared_scopes = []
"#
    )
}

fn write_steward_three_halts(dep: &TestDeployment) {
    dep.write(
        "steward.toml",
        r#"
[actor]
backend = "fake"
fake_responses = [
  "{\"halt\": true}",
  "{\"halt\": true}",
  "{\"halt\": true}"
]

[evaluator]
backend = "fake"
"#,
    );
}

fn write_corpus(corpus_dir: &PathBuf) {
    std::fs::create_dir_all(corpus_dir).unwrap();
    std::fs::write(
        corpus_dir.join("corpus.jsonl"),
        r#"{"id":"s1","brief":"halt please","expected_outcome":"quiet","technique":"halt_now","failure_class":"restraint_warranted"}
{"id":"s2","brief":"halt now","expected_outcome":"quiet","technique":"halt_now","failure_class":"restraint_warranted"}
{"id":"s3","brief":"this should pass; halt","expected_outcome":"externalized","technique":"halt_now","failure_class":"restraint_warranted"}
"#,
    )
    .unwrap();
}

#[tokio::test]
async fn scenario_suite_runs_corpus_and_aggregates_correctly() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one("modify_artifact"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_steward_three_halts(&dep);
    dep.write(
        "tools/modify_artifact.toml",
        "id = \"modify_artifact\"\nexecutor = \"native_artifact_modify\"\n",
    );

    let corpus_dir = dep.workspace_root.join("scenarios");
    write_corpus(&corpus_dir);

    let report = scenario_suite::run_suite(
        &dep.chartered_dir,
        Some(dep.workspace_root.clone()),
        &corpus_dir,
    )
    .await
    .expect("suite runs");

    // Three scenarios, three run dirs.
    assert_eq!(report.scenarios.len(), 3);
    let run_ids: std::collections::BTreeSet<_> = report
        .scenarios
        .iter()
        .map(|s| s.run_id.clone())
        .collect();
    assert_eq!(run_ids.len(), 3, "each scenario should produce its own run_id");

    // s1/s2 expect quiet; the actor halts immediately so both pass.
    // s3 expects externalized but the actor halts → fails.
    let pass_by_id: std::collections::BTreeMap<_, _> = report
        .scenarios
        .iter()
        .map(|s| (s.id.clone(), s.passed))
        .collect();
    assert_eq!(pass_by_id.get("s1"), Some(&true));
    assert_eq!(pass_by_id.get("s2"), Some(&true));
    assert_eq!(pass_by_id.get("s3"), Some(&false));

    // Aggregations.
    assert_eq!(report.totals.total, 3);
    assert_eq!(report.totals.passed, 2);
    assert_eq!(report.totals.failed, 1);
    let by_tech = report.by_technique.get("halt_now").expect("technique row");
    assert_eq!(by_tech.total, 3);
    assert_eq!(by_tech.passed, 2);
    let by_class = report
        .by_failure_class
        .get("restraint_warranted")
        .expect("failure-class row");
    assert_eq!(by_class.total, 3);
    let by_cell = report
        .by_cell
        .get("halt_now|restraint_warranted")
        .expect("cell row");
    assert_eq!(by_cell.total, 3);
}

#[tokio::test]
async fn scenario_suite_records_per_scenario_paths_for_audit() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one("modify_artifact"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_steward_three_halts(&dep);
    dep.write(
        "tools/modify_artifact.toml",
        "id = \"modify_artifact\"\nexecutor = \"native_artifact_modify\"\n",
    );

    let corpus_dir = dep.workspace_root.join("scenarios");
    std::fs::create_dir_all(&corpus_dir).unwrap();
    std::fs::write(
        corpus_dir.join("corpus.jsonl"),
        r#"{"id":"only","brief":"halt","expected_outcome":"quiet","technique":"halt_now","failure_class":"restraint_warranted"}"#,
    )
    .unwrap();

    let report = scenario_suite::run_suite(
        &dep.chartered_dir,
        Some(dep.workspace_root.clone()),
        &corpus_dir,
    )
    .await
    .expect("suite runs");

    assert_eq!(report.scenarios.len(), 1);
    let row = &report.scenarios[0];
    assert!(row.receipts_path.exists(), "receipts.jsonl should exist");
    assert!(row.cognition_path.exists(), "cognition.jsonl should exist");
}
