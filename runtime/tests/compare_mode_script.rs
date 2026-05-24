//! Happy-path test for `scripts/compare-mode.sh`. The script fans one
//! corpus across four governance configurations (two ungoverned
//! strawmen — naked, same_context_judge — and two governed — separated_judge,
//! separated_grounded). The fake-only branch here invokes the script
//! with `--governed-only` so it can run without a real LLM endpoint;
//! the full four-way is exercised by the local-only e2e variant in
//! `agent_llm_e2e.rs`.

mod common;

use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

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

fn write_steward_six_halts(dep: &TestDeployment) {
    // Six halts: each config runs the 2-scenario corpus once. The
    // fake actor queue is shared across runs within one Agent — the
    // harness reuses one Agent per config — so two halts per config
    // is enough, but we provision six to make the test resilient to
    // any inner-loop drain we didn't anticipate.
    let halts = std::iter::repeat_n("\"{\\\"halt\\\": true}\"", 6)
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        r#"
[actor]
backend = "fake"
fake_responses = [{halts}]

[evaluator]
backend = "fake"
"#
    );
    dep.write("steward.toml", &toml);
}

fn script_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/compare-mode.sh")
}

#[test]
fn compare_mode_script_governed_only_emits_two_configs_plus_summary() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one("modify_artifact"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_steward_six_halts(&dep);
    dep.write(
        "tools/modify_artifact.toml",
        "id = \"modify_artifact\"\nexecutor = \"native_artifact_modify\"\n",
    );

    let corpus_tmp = TempDir::new().expect("corpus tempdir");
    std::fs::write(
        corpus_tmp.path().join("corpus.jsonl"),
        r#"{"id":"a","brief":"halt","expected_outcome":"quiet","technique":"halt","failure_class":"restraint_warranted"}
{"id":"b","brief":"halt","expected_outcome":"quiet","technique":"halt","failure_class":"restraint_warranted"}
"#,
    )
    .unwrap();

    let out_tmp = TempDir::new().expect("out tempdir");
    let script = script_path();
    assert!(
        script.exists() && script.is_file(),
        "expected script at {}",
        script.display()
    );

    let status = Command::new("bash")
        .arg(&script)
        .arg("--governed-only")
        .arg(&dep.chartered_dir)
        .arg(corpus_tmp.path())
        .arg(out_tmp.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()
        .expect("script runs");
    assert!(status.success(), "script exited nonzero: {status}");

    for label in ["separated_judge", "separated_grounded"] {
        let report_path = out_tmp.path().join(format!("{label}.json"));
        let raw = std::fs::read_to_string(&report_path)
            .unwrap_or_else(|_| panic!("missing report {}", report_path.display()));
        let v: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{label}.json not JSON: {e}"));
        let scenarios = v["scenarios"].as_array().expect("scenarios array");
        assert_eq!(scenarios.len(), 2, "{label}: expected 2 scenarios");
        assert_eq!(
            v["totals"]["total"].as_u64(),
            Some(2),
            "{label}: totals.total"
        );
    }

    let summary_raw = std::fs::read_to_string(out_tmp.path().join("summary.json"))
        .expect("summary.json present");
    let summary: Value = serde_json::from_str(&summary_raw).expect("summary.json is JSON");
    for label in ["separated_judge", "separated_grounded"] {
        let cell = &summary["by_config"][label];
        assert_eq!(
            cell["total"].as_u64(),
            Some(2),
            "summary: {label}.total"
        );
    }
}
