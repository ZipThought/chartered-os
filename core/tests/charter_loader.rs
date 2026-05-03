//! Charter+RoleContext loader integration tests against the
//! `examples/charters/*/` artifacts. Verifies the loader produces a
//! Charter that Workspace::new accepts (declared Scopes resolve, tool
//! registry has all permitted tools).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use chartered_core::{
    build_charter, build_role_context, load_charter_def, load_role_context_def, Evaluator,
    FrameDef, LlmEvaluator, ScopeKind, Snapshot, Steward, StewardId, ToolExecutor, ToolId,
    ToolParams, ToolRegistry, ToolResult, Workspace, WorkspaceId,
};
use chartered_core::FakeCognitionBackend;

fn examples_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
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

fn registry_for(tools: &[ToolId]) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    for t in tools {
        r.register(Arc::new(StubTool { id: t.clone() }));
    }
    r
}

fn evaluator_factory() -> impl FnMut(&FrameDef) -> Arc<dyn Evaluator> {
    |fd: &FrameDef| {
        let backend = Arc::new(FakeCognitionBackend::new(format!("eval-{}", fd.id)));
        Arc::new(LlmEvaluator::new(
            format!("eval-{}", fd.id),
            backend,
            fd.id.clone(),
            fd.concern.clone(),
        )) as Arc<dyn Evaluator>
    }
}

#[test]
fn load_coding_agent_charter() {
    let dir = examples_dir().join("examples/charters/coding-agent");
    let def = load_charter_def(&dir).expect("load coding-agent");
    assert_eq!(def.frames.len(), 4);
    assert_eq!(
        def.permitted_tools.len(),
        3,
        "permitted_tools = read_file, write_file, exec_command"
    );

    // All declared scopes are Charter-kind for coding-agent.
    for f in &def.frames {
        for ds in &f.declared_scopes {
            assert_eq!(ds.kind, ScopeKind::Charter, "frame {}", f.id);
        }
    }

    // scopes.md produced 4 named sections.
    let scope_names: Vec<_> = def.charter_scopes.iter().map(|(n, _)| n.as_str()).collect();
    assert!(scope_names.contains(&"file_system_access"), "names: {scope_names:?}");
    assert!(scope_names.contains(&"shell_commands"));
    assert!(scope_names.contains(&"network_access"));
    assert!(scope_names.contains(&"git_operations"));

    // Every Frame's declared scope resolves into Charter Scopes.
    for f in &def.frames {
        for ds in &f.declared_scopes {
            assert!(
                def.charter_scopes.iter().any(|(n, _)| n == &ds.name),
                "frame {} references missing scope `{}`",
                f.id,
                ds.name
            );
        }
    }

    // Build Charter and Workspace; validation should pass.
    let permitted = def.permitted_tools.clone();
    let charter = build_charter(def, 1, evaluator_factory());
    let snap = Snapshot::new(charter, chartered_core::RoleContext::empty());
    let registry = registry_for(&permitted);
    let steward = Steward::new(StewardId::new("sut"), snap, Arc::new(registry));
    let _ws = Workspace::single(WorkspaceId::new("ws-coding"), steward)
        .expect("workspace validates against loaded coding-agent Charter");
}

#[test]
fn load_customer_service_charter_with_role_context() {
    let dir = examples_dir().join("examples/charters/customer-service");
    let def = load_charter_def(&dir).expect("load customer-service");
    assert_eq!(def.frames.len(), 3);
    assert_eq!(def.permitted_tools.len(), 1);

    // All declared scopes are RoleContext-kind for customer-service.
    for f in &def.frames {
        for ds in &f.declared_scopes {
            assert_eq!(ds.kind, ScopeKind::RoleContext, "frame {}", f.id);
        }
    }

    // Load Role context from the shipped template.
    let rc_path = dir.join("role_context_template.md");
    let rc_def = load_role_context_def(&rc_path).expect("load role context template");
    let rc_names: Vec<_> = rc_def.scopes.iter().map(|(n, _)| n.as_str()).collect();
    assert!(rc_names.contains(&"product_pricing_fees"), "names: {rc_names:?}");
    assert!(rc_names.contains(&"returns_warranty_policy"));
    assert!(rc_names.contains(&"service_scope_limitations"));

    // Build everything; validation should pass because RoleContext
    // supplies the names the Frames declare.
    let permitted = def.permitted_tools.clone();
    let charter = build_charter(def, 2, evaluator_factory());
    let role_context = build_role_context(rc_def, 5);
    let snap = Snapshot::new(charter, role_context);
    let registry = registry_for(&permitted);
    let steward = Steward::new(StewardId::new("sut"), snap, Arc::new(registry));
    let _ws = Workspace::single(WorkspaceId::new("ws-cs"), steward)
        .expect("workspace validates against customer-service Charter + role context");
}

#[test]
fn workspace_validation_fails_when_role_context_omitted() {
    // Loading customer-service with empty RoleContext should fail
    // validation: declared RoleContext scopes don't resolve.
    let dir = examples_dir().join("examples/charters/customer-service");
    let def = load_charter_def(&dir).unwrap();
    let permitted = def.permitted_tools.clone();
    let charter = build_charter(def, 1, evaluator_factory());
    let snap = Snapshot::new(charter, chartered_core::RoleContext::empty());
    let registry = registry_for(&permitted);
    let steward = Steward::new(StewardId::new("sut"), snap, Arc::new(registry));
    let err = match Workspace::single(WorkspaceId::new("ws"), steward) {
        Ok(_) => panic!("must reject missing RoleContext scope"),
        Err(e) => e,
    };
    assert!(
        err.0.contains("RoleContext"),
        "error did not mention RoleContext: {}",
        err.0
    );
}

