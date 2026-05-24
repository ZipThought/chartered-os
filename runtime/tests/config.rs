//! Deployment-config loader tests against the shipped example
//! `examples/deployments/coding-agent-min/.chartered/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use chartered_core::{
    Evaluator, FakeCognitionBackend, FrameDef, GovernanceMode, LlmEvaluator, ScopeKind, Snapshot,
    ToolExecutor, ToolId, ToolParams, ToolRegistry, ToolResult, WorkspaceId,
};
use chartered_runtime::config::{find_chartered_dir, load, BackendKind};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn example_deployment() -> PathBuf {
    repo_root().join("examples/deployments/coding-agent-min")
}

#[test]
fn find_chartered_dir_finds_local_dot_chartered() {
    let dep = example_deployment();
    let found = find_chartered_dir(&dep).expect("walk-up finds .chartered/");
    assert_eq!(
        found.canonicalize().unwrap(),
        dep.join(".chartered").canonicalize().unwrap()
    );
}

#[test]
fn find_chartered_dir_walks_up_from_nested_path() {
    // Start at .chartered/tools/ (real subdirectory below the dep);
    // walk-up should resolve back to the dep's .chartered/.
    let nested = example_deployment().join(".chartered").join("tools");
    let found = find_chartered_dir(&nested).expect("walk-up finds .chartered/");
    assert!(found.ends_with(".chartered"));
    assert_eq!(
        found.canonicalize().unwrap(),
        example_deployment().join(".chartered").canonicalize().unwrap()
    );
}

#[test]
fn load_full_deployment_config() {
    let chartered_dir = example_deployment().join(".chartered");
    let cfg = load(&chartered_dir).expect("load deployment config");

    // Runtime-level: governance mode parses (defaulting to FULL when
    // the field is absent or partial).
    let mode: GovernanceMode = cfg.runtime.governance.into();
    assert_eq!(mode, GovernanceMode::FULL);

    // Steward-level
    assert_eq!(cfg.steward.actor.backend, BackendKind::OpenAi);
    assert_eq!(cfg.steward.actor.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(cfg.steward.evaluator.backend, BackendKind::OpenAi);

    // Charter reference
    assert_eq!(cfg.charter_ref.version, 1);

    assert_eq!(cfg.charter_def.frames.len(), 4);
    assert_eq!(cfg.charter_def.permitted_tools.len(), 3);
    for f in &cfg.charter_def.frames {
        for ds in &f.declared_scopes {
            assert_eq!(ds.kind, ScopeKind::Charter);
        }
    }

    // Role context (placeholder file present, no ## sections)
    let rc_def = cfg.role_context_def.as_ref().expect("role_context.md present");
    assert!(
        rc_def.scopes.is_empty(),
        "placeholder role_context.md has no ## sections"
    );

    // tools/*.toml — three entries, sorted by path
    assert_eq!(cfg.tools.len(), 3);
    let ids: Vec<&str> = cfg.tools.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"read_file"));
    assert!(ids.contains(&"write_file"));
    assert!(ids.contains(&"exec_command"));
    let executors: Vec<&str> = cfg.tools.iter().map(|t| t.executor.as_str()).collect();
    assert!(executors.contains(&"native_fs_read"));
    assert!(executors.contains(&"native_fs_write"));
    assert!(executors.contains(&"native_exec"));
}

struct StubTool {
    id: ToolId,
}
#[async_trait]
impl ToolExecutor for StubTool {
    fn id(&self) -> &ToolId {
        &self.id
    }
    async fn execute(&self, _: &ToolParams) -> ToolResult {
        Ok(serde_json::json!({}))
    }
}

#[test]
fn loaded_charter_builds_into_valid_workspace() {
    // Load the deployment config, materialize the Charter using a
    // FakeCognitionBackend per Frame, register stub Tool executors,
    // and confirm Workspace::new accepts the result. This proves the
    // loader produces shapes the kernel validates.
    let chartered_dir = example_deployment().join(".chartered");
    let cfg = load(&chartered_dir).expect("load");

    let permitted = cfg.charter_def.permitted_tools.clone();
    let factory = |fd: &FrameDef| -> Arc<dyn Evaluator> {
        let backend = Arc::new(FakeCognitionBackend::new(format!("eval-{}", fd.id)));
        Arc::new(LlmEvaluator::new(
            format!("eval-{}", fd.id),
            backend,
            fd.id.clone(),
            fd.concern.clone(),
        ))
    };
    let (charter, role_context, skills) = cfg.build_charter(factory);
    let snap = Snapshot::new(charter, role_context, skills);

    let mut registry = ToolRegistry::new();
    for t in &permitted {
        registry.register(Arc::new(StubTool { id: t.clone() }));
    }
    let steward = chartered_core::Steward::new(
        chartered_core::StewardId::new("sut"),
        snap,
        Arc::new(registry),
    );
    let _ws = chartered_core::Workspace::single(WorkspaceId::new("ws-from-deployment"), steward)
        .expect("workspace validates against loaded deployment");
}
