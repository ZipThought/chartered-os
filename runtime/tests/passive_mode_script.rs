//! Happy-path test for `scripts/passive-mode.sh`. The script watches
//! a directory and fires one Runtime invocation per new file. This
//! test materializes a fake-backend deployment, points the script at
//! a watch dir, drops two files into it after startup, and verifies
//! that two governed invocations ran (two `runs/<run_id>/` dirs landed
//! under `chartered_dir`).

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

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

fn write_steward_many_halts(dep: &TestDeployment) {
    let halts = std::iter::repeat_n("\"{\\\"halt\\\": true}\"", 8)
        .collect::<Vec<_>>()
        .join(", ");
    dep.write(
        "steward.toml",
        &format!(
            r#"
[actor]
backend = "fake"
fake_responses = [{halts}]

[evaluator]
backend = "fake"
"#
        ),
    );
}

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/passive-mode.sh")
}

fn count_run_dirs(chartered_dir: &Path) -> usize {
    let runs = chartered_dir.join("runs");
    if !runs.exists() {
        return 0;
    }
    std::fs::read_dir(&runs)
        .map(|it| it.filter_map(Result::ok).filter(|e| e.path().is_dir()).count())
        .unwrap_or(0)
}

#[test]
fn passive_mode_script_invokes_runtime_per_new_file() {
    let dep = TestDeployment::new();
    dep.write_chartered_toml();
    dep.write_charter_ref(1);
    dep.write_charter(&frames_allow_one("modify_artifact"), SCOPES_MD_EMPTY);
    dep.write_role_context_md();
    write_steward_many_halts(&dep);
    dep.write(
        "tools/modify_artifact.toml",
        "id = \"modify_artifact\"\nexecutor = \"native_artifact_modify\"\n",
    );

    let watch_dir = dep.workspace_root.join("data-room");
    std::fs::create_dir_all(&watch_dir).unwrap();

    let script = script_path();
    assert!(script.exists(), "expected script at {}", script.display());

    // Use idle_seconds=3 so the script exits after a quiet window
    // following the second file arrival. The polling cadence inside
    // the script is 1s.
    let mut child = Command::new("bash")
        .arg(&script)
        .arg(&dep.chartered_dir)
        .arg(&watch_dir)
        .arg("3")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("script spawns");

    // Give the script's initial-snapshot pass time to settle.
    thread::sleep(Duration::from_millis(1500));

    // Drop two files into the watch dir, spaced enough that the 1-Hz
    // polling loop catches each.
    std::fs::write(watch_dir.join("alpha.md"), "alpha contents").unwrap();
    thread::sleep(Duration::from_millis(2200));
    std::fs::write(watch_dir.join("beta.md"), "beta contents").unwrap();

    let status = child.wait().expect("script terminates");
    assert!(status.success(), "script exited nonzero: {status}");

    let runs = count_run_dirs(&dep.chartered_dir);
    assert!(
        runs >= 2,
        "expected at least 2 run dirs after two file arrivals, found {runs}",
    );
}
