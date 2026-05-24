//! Happy-path test for the `agent_embed` example program. The example
//! demonstrates the in-process consumer pattern: construct an Agent
//! from a `.chartered/`, call `run(Brief::Prompt(...))`, print the
//! categorical outcome. This test materializes a fake-backend
//! deployment in a tempdir, invokes the example as a subprocess, and
//! asserts on its stdout.

mod common;

use std::process::Command;

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

fn make_halt_only_deployment() -> TestDeployment {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one("modify_artifact"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    dep.write(
        "steward.toml",
        r#"
[actor]
backend = "fake"
fake_responses = ["{\"halt\": true}"]

[evaluator]
backend = "fake"
"#,
    );
    dep.write(
        "tools/modify_artifact.toml",
        "id = \"modify_artifact\"\nexecutor = \"native_artifact_modify\"\n",
    );
    dep
}

#[test]
fn agent_embed_example_runs_against_fake_deployment_and_reports_quiet() {
    let dep = make_halt_only_deployment();

    // Build the example up front so the subsequent invocation is just a
    // process spawn (no compile time inside the assertion path).
    let manifest = env!("CARGO_MANIFEST_DIR");
    let build = Command::new("cargo")
        .args(["build", "--quiet", "--example", "agent_embed"])
        .current_dir(manifest)
        .output()
        .expect("cargo build invocable");
    assert!(
        build.status.success(),
        "example build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let example_bin = std::path::PathBuf::from(manifest)
        .join("target/debug/examples/agent_embed");
    assert!(
        example_bin.exists(),
        "expected example binary at {}",
        example_bin.display()
    );

    let out = Command::new(&example_bin)
        .arg(&dep.chartered_dir)
        .arg("hello, please halt")
        .output()
        .expect("example invocable");

    assert!(
        out.status.success(),
        "example exited nonzero ({})\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("outcome: quiet"),
        "expected `outcome: quiet` in stdout, got:\n{stdout}"
    );
    assert!(
        stdout.contains("run_id="),
        "expected `run_id=` line in stdout, got:\n{stdout}"
    );
}
