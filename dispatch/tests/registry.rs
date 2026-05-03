//! ExecutorRegistry tests: deployment-config `executor` strings map to
//! concrete ToolExecutor instances; unknown names yield a structured
//! error.

use chartered_core::ToolId;
use chartered_dispatch::{DeploymentPaths, ExecutorRegistry};

fn make_paths(workspace: &std::path::Path) -> DeploymentPaths {
    let chartered = workspace.join(".chartered");
    std::fs::create_dir_all(&chartered).unwrap();
    DeploymentPaths::canonicalize(workspace, &chartered).unwrap()
}

#[test]
fn registry_builds_known_executors() {
    let dir = tempfile::tempdir().unwrap();
    let reg = ExecutorRegistry::new(make_paths(dir.path()));

    let r = reg
        .build("native_fs_read", &ToolId::new("read_file"))
        .expect("native_fs_read builds");
    assert_eq!(r.id().0, "read_file");

    let w = reg
        .build("native_fs_write", &ToolId::new("write_file"))
        .expect("native_fs_write builds");
    assert_eq!(w.id().0, "write_file");

    let e = reg
        .build("native_exec", &ToolId::new("exec_command"))
        .expect("native_exec builds");
    assert_eq!(e.id().0, "exec_command");

    let ar = reg
        .build("native_artifact_read", &ToolId::new("read_artifact"))
        .expect("native_artifact_read builds");
    assert_eq!(ar.id().0, "read_artifact");

    let am = reg
        .build("native_artifact_modify", &ToolId::new("modify_artifact"))
        .expect("native_artifact_modify builds");
    assert_eq!(am.id().0, "modify_artifact");

    let af = reg
        .build(
            "native_artifact_record_finding",
            &ToolId::new("record_finding"),
        )
        .expect("native_artifact_record_finding builds");
    assert_eq!(af.id().0, "record_finding");

    let al = reg
        .build("native_artifact_list", &ToolId::new("list_artifacts"))
        .expect("native_artifact_list builds");
    assert_eq!(al.id().0, "list_artifacts");
}

#[test]
fn registry_rejects_unknown_executor() {
    let dir = tempfile::tempdir().unwrap();
    let reg = ExecutorRegistry::new(make_paths(dir.path()));
    let err = match reg.build("nonexistent_executor", &ToolId::new("x")) {
        Ok(_) => panic!("expected unknown-executor error"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(msg.contains("nonexistent_executor"), "msg: {msg}");
}
