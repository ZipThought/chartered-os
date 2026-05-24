//! Charter+RoleContext loader integration tests against the
//! `examples/charters/*/` artifacts. Verifies the loader produces a
//! Charter that Workspace::new accepts (declared Scopes resolve, tool
//! registry has all permitted tools).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use chartered_core::{
    build_charter, build_role_context, Evaluator, FakeCognitionBackend, FrameDef, LlmEvaluator,
    ScopeKind, Snapshot, Steward, StewardId, ToolExecutor, ToolId, ToolParams, ToolRegistry,
    ToolResult, Workspace, WorkspaceId,
};
use chartered_runtime::charter_loader::{load_charter_def, load_role_context_def, load_skills};

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
    let scope_names: Vec<&str> = def
        .charter_scopes
        .iter()
        .map(|(n, _): &(String, String)| n.as_str())
        .collect();
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
    let snap = Snapshot::new(charter, chartered_core::RoleContext::empty(), Vec::new());
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
    let rc_names: Vec<&str> = rc_def
        .scopes
        .iter()
        .map(|(n, _): &(String, String)| n.as_str())
        .collect();
    assert!(rc_names.contains(&"product_pricing_fees"), "names: {rc_names:?}");
    assert!(rc_names.contains(&"returns_warranty_policy"));
    assert!(rc_names.contains(&"service_scope_limitations"));

    // Build everything; validation should pass because RoleContext
    // supplies the names the Frames declare.
    let permitted = def.permitted_tools.clone();
    let charter = build_charter(def, 2, evaluator_factory());
    let role_context = build_role_context(rc_def, 5);
    let snap = Snapshot::new(charter, role_context, Vec::new());
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
    let snap = Snapshot::new(charter, chartered_core::RoleContext::empty(), Vec::new());
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

#[test]
fn load_skills_returns_empty_when_directory_absent() {
    // Skills are optional: a Charter without `skills/` loads as
    // empty Vec<Skill>, not as an error.
    let dir = tempfile::tempdir().unwrap();
    let skills = load_skills(dir.path()).expect("load_skills tolerates missing dir");
    assert!(skills.is_empty());
}

#[test]
fn load_skills_reads_markdown_files_sorted_by_id() {
    // <charter_dir>/skills/<id>.md → one Skill each. Loader sorts by id
    // so the resulting `skills_content_hash` is filesystem-iteration
    // order-independent.
    let dir = tempfile::tempdir().unwrap();
    let skills_dir = dir.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::write(skills_dir.join("triage.md"), "triage guidance body").unwrap();
    std::fs::write(skills_dir.join("billing.md"), "billing guidance body").unwrap();
    // A non-.md file should be ignored — only `*.md` becomes a Skill.
    std::fs::write(skills_dir.join("notes.txt"), "should be ignored").unwrap();

    let skills = load_skills(dir.path()).expect("load_skills");
    assert_eq!(skills.len(), 2, "two .md files → two Skills");
    assert_eq!(skills[0].id, "billing", "sorted by id");
    assert_eq!(skills[1].id, "triage");
    assert_eq!(skills[0].content.trim(), "billing guidance body");
    assert_ne!(skills[0].content_hash, skills[1].content_hash);
}

#[test]
fn skills_content_propagates_into_snapshot_id() {
    use chartered_core::{skills_content_hash, Charter, RoleContext, Skill};
    let mk_charter = || Charter {
        frames: vec![],
        permitted_tools: vec![],
        charter_scopes: vec![],
        behavioral_spec: String::new(),
        charter_version: 1,
        charter_content_hash: "c".into(),
    };
    let mk_rc = || RoleContext {
        scopes: vec![],
        role_context_version: 1,
        role_context_content_hash: "r".into(),
    };

    // Same Charter + Role context, different Skills → different
    // Snapshot IDs (Skills are part of the content-addressed identity).
    let snap_no_skills = Snapshot::new(mk_charter(), mk_rc(), Vec::new());
    let snap_with_skill = Snapshot::new(
        mk_charter(),
        mk_rc(),
        vec![Skill::new("s", "skill body")],
    );
    assert_ne!(snap_no_skills.id, snap_with_skill.id);

    // Aggregate hash of empty Skills differs from aggregate hash of any
    // non-empty Skill set (sanity for the composition rule).
    assert_ne!(
        skills_content_hash(&[]),
        skills_content_hash(&[Skill::new("s", "skill body")])
    );
}


#[test]
fn load_synthetic_data_charter() {
    let dir = examples_dir().join("examples/charters/synthetic-data");
    let def = load_charter_def(&dir).expect("load synthetic-data");
    assert_eq!(def.permitted_tools.len(), 1);
    assert_eq!(def.permitted_tools[0].0, "modify_artifact");
    assert_eq!(def.frames.len(), 5);
    let ids: Vec<&str> = def.frames.iter().map(|f| f.id.0.as_str()).collect();
    for required in [
        "no_real_world_likeness",
        "scenario_novelty",
        "technique_coverage",
        "failure_class_discipline",
        "claimed_label_explicit",
    ] {
        assert!(
            ids.contains(&required),
            "Frame `{required}` missing; found: {ids:?}"
        );
    }
}

#[test]
fn load_gold_labeler_charter() {
    let dir = examples_dir().join("examples/charters/gold-labeler");
    let def = load_charter_def(&dir).expect("load gold-labeler");
    assert_eq!(def.permitted_tools.len(), 1);
    assert_eq!(def.permitted_tools[0].0, "modify_artifact");
    assert_eq!(def.frames.len(), 3);
    let ids: Vec<&str> = def.frames.iter().map(|f| f.id.0.as_str()).collect();
    for required in [
        "judgment_traceable_to_scope",
        "label_uses_charter_frames",
        "blinding_from_generator_claim",
    ] {
        assert!(ids.contains(&required), "Frame `{required}` missing; found: {ids:?}");
    }
}

#[test]
fn load_same_context_baseline_charter() {
    let dir = examples_dir().join("examples/charters/same-context-baseline");
    let def = load_charter_def(&dir).expect("load same-context-baseline");
    // The strawman has no Frames — every proposal is OUT_OF_SCOPE; the
    // harness pairs it with passthrough mode.
    assert!(def.frames.is_empty());
    // Several permitted tools so the strawman can stand in for any
    // production Charter the harness contrasts it against.
    assert!(def.permitted_tools.len() >= 4);
}
