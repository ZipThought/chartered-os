//! The single execution path for the chartered-runtime binary.
//!
//! ONE entry. ONE loader. ONE orchestrator. The fake/real swap lives
//! at the `CognitionBackend` (per role, configured in `steward.toml`)
//! and at the `ToolExecutor` registry (per tool, configured in
//! `tools/*.toml`). Nothing else differs between a test deployment
//! and a production deployment.
//!
//! Per invocation, the binary writes:
//!   `<chartered_dir>/runs/<run_id>/receipts.jsonl`
//!   `<chartered_dir>/runs/<run_id>/cognition.jsonl`
//! Both are JSON Lines so operators can grep them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chartered_core::{
    Actor, ActorFactory, ArtifactId, ArtifactRange, CognitionBackend, Evaluator,
    FakeCognitionBackend, FrameDef, Judge, JudgeReport, LlmActor, LlmEvaluator, LlmJudge,
    LlmTester, Receipt, ReceiptStore, ScenarioRunner, SelectionAction, SelectionActionKind,
    Snapshot, TaskTrigger, Tester, TesterError, ToolExecutor, ToolId, ToolRegistry, Trigger,
    Workspace, WorkspaceId,
};
use chartered_dispatch::ExecutorRegistry;
use serde::Serialize;

use crate::config::{self, BackendKind, TesterConfig};
use crate::openai_backend::OpenAiBackendFactory;
use crate::persistence::{self, AppendOnlyFileReceiptStore, JsonlSink, LoggingBackend};

#[derive(Debug, Default)]
pub struct Options {
    pub chartered_dir: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
    pub user_message: Option<String>,
    pub selection_trigger: Option<SelectionTriggerOptions>,
    pub refinement_budget: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SelectionTriggerOptions {
    pub artifact_id: ArtifactId,
    pub range: ArtifactRange,
    pub action: SelectionAction,
}

#[derive(Debug, Clone)]
pub struct RunError(pub String);

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RunError {}

impl From<config::ConfigError> for RunError {
    fn from(e: config::ConfigError) -> Self {
        RunError(e.0)
    }
}

#[derive(Debug, Serialize)]
pub struct RunOutput {
    pub workspace_id: String,
    pub run_id: String,
    pub run_dir: String,
    pub receipts_log: String,
    pub cognition_log: String,
    pub tasks: Vec<chartered_core::TaskRecord>,
    pub attempts: Vec<chartered_core::AttemptRecord>,
    pub receipts: Vec<Receipt>,
    pub judge: JudgeReport,
    pub turns: usize,
    pub terminated_by_budget: bool,
    /// Set when the Tester failed to produce a turn message; the
    /// scenario terminated early. Distinct from `terminated_by_budget`
    /// which means the run completed up to its turn limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tester_failure: Option<String>,
}

pub async fn run(opts: Options) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir().map_err(|e| RunError(format!("cwd: {e}")))?;
    let chartered_dir = match opts.chartered_dir.clone() {
        Some(d) => d,
        None => config::find_chartered_dir(&cwd)
            .ok_or_else(|| RunError("no .chartered/ directory found by walk-up search".into()))?,
    };
    let cfg = config::load(&chartered_dir)?;

    let workspace_root = match opts.workspace_root.clone() {
        Some(p) => p,
        None => cfg
            .chartered_dir
            .parent()
            .ok_or_else(|| RunError("chartered_dir has no parent".into()))?
            .to_path_buf(),
    };

    let run_id = persistence::make_run_id();
    let run_dir = persistence::run_dir(&cfg.chartered_dir, &run_id);
    std::fs::create_dir_all(&run_dir)
        .map_err(|e| RunError(format!("creating run dir {}: {e}", run_dir.display())))?;
    let receipts_path = run_dir.join("receipts.jsonl");
    let cognition_path = run_dir.join("cognition.jsonl");

    let receipt_store: Arc<dyn ReceiptStore> = Arc::new(
        AppendOnlyFileReceiptStore::create(&receipts_path)
            .map_err(|e| RunError(format!("opening {}: {e}", receipts_path.display())))?,
    );
    let cognition_log: Arc<JsonlSink> = Arc::new(
        JsonlSink::create(&cognition_path)
            .map_err(|e| RunError(format!("opening {}: {e}", cognition_path.display())))?,
    );

    let singleton_message = opts.singleton_message(&workspace_root)?;
    let tester_choice = tester_or_message(&cfg.steward.tester, singleton_message)?;
    let max_turns = match &cfg.steward.tester {
        Some(t) => t.max_turns,
        None => 1,
    };

    // Build the OpenAI factory once and reuse the shared reqwest::Client
    // across all OpenAI-backed roles. Constructed lazily — fake-only
    // deployments never read env or build an HTTP client.
    let openai_factory = build_openai_factory_if_needed(&cfg)?;
    let backends = BackendCtx {
        log: cognition_log.clone(),
        openai: openai_factory.as_ref(),
    };

    let actor_backend = build_backend(
        BackendRole::Actor,
        cfg.steward.actor.backend,
        FakeCorpus::Sequence(&cfg.steward.actor.fake_responses),
        cfg.steward.actor.model.as_deref(),
        &backends,
    )?;
    let evaluator_backends_arc = Arc::new(build_evaluator_backends_per_frame(&cfg, &backends)?);
    let mut tester: Box<dyn Tester> = match tester_choice {
        TesterChoice::Configured(t) => Box::new(build_llm_tester(t, &backends)?),
        TesterChoice::Singleton(msg) => Box::new(SingletonTester::new(msg)),
    };
    let judge: Option<LlmJudge> = match &cfg.steward.judge {
        Some(j) => {
            let backend = build_backend(
                BackendRole::Judge,
                j.backend,
                FakeCorpus::Single(j.fake_response.as_deref()),
                j.model.as_deref(),
                &backends,
            )?;
            Some(LlmJudge::new("judge", backend, &j.criteria))
        }
        None => None,
    };

    // workspace_root was already canonicalized at lines 85-95 (or
    // supplied explicitly via --workspace-root). Reuse it as the
    // workspace identifier — substituting a "workspace" literal would
    // mask a defect in path resolution and corrupt the context_id used
    // for prior_receipt_queries.
    let workspace_id_str = workspace_root.display().to_string();

    let tools_for_registry = cfg.tools.clone();
    let governance_mode: chartered_core::GovernanceMode = cfg.runtime.governance.into();

    let evaluator_factory = {
        let backends = evaluator_backends_arc.clone();
        move |fd: &FrameDef| -> Arc<dyn Evaluator> {
            let backend = backends
                .get(&fd.id.0)
                .cloned()
                .expect("evaluator backend constructed for every frame");
            Arc::new(LlmEvaluator::new(
                BackendRole::Evaluator {
                    frame_id: fd.id.0.clone(),
                }
                .id(),
                backend,
                fd.id.clone(),
                fd.concern.clone(),
            ))
        }
    };

    // Build the central deployment-path config BEFORE consuming cfg
    // (which `build_charter` moves). Single source of truth, injected
    // into the executor registry and from there into every Backend.
    let chartered_dir_for_paths = cfg.chartered_dir.clone();
    let (charter, role_context) = cfg.build_charter(evaluator_factory);
    let snapshot = Snapshot::new(charter, role_context);

    let deployment_paths = chartered_dispatch::DeploymentPaths::canonicalize(
        &workspace_root,
        &chartered_dir_for_paths,
    )
    .map_err(|e| {
        RunError(format!(
            "canonicalize deployment paths (workspace_root={}, chartered_dir={}): {e}",
            workspace_root.display(),
            chartered_dir_for_paths.display(),
        ))
    })?;
    let executor_registry = ExecutorRegistry::new(deployment_paths);
    let mut registry = ToolRegistry::new();
    for tr in &tools_for_registry {
        let exec: Arc<dyn ToolExecutor> = executor_registry
            .build(&tr.executor, &ToolId::new(&tr.id))
            .map_err(|e| RunError(format!("building executor for tool `{}`: {e}", tr.id)))?;
        registry.register(exec);
    }

    // Single-Steward deployment for v1: one Steward (id="sut") owns the
    // Charter, Snapshot, and Tool registry. Multi-Steward deployments
    // (Block D) split this into a stewards/<id>.toml directory.
    let sut_steward = chartered_core::Steward::new(
        chartered_core::StewardId::new("sut"),
        snapshot,
        Arc::new(registry),
    )
    .with_governance_mode(governance_mode);
    let workspace = Arc::new(
        Workspace::single_with_store(
            WorkspaceId::new(&workspace_id_str),
            sut_steward,
            receipt_store,
        )
        .map_err(|e| RunError(format!("workspace validation failed: {e}")))?,
    );
    let sut_steward = workspace.sole_steward().clone();

    // Runtime-assembled Actor system prompt per spec §Cognition Layer:
    // base behavior + Charter behavioral spec + Charter Scopes (when
    // grounding) + Role context Scopes (when grounding). The deployment
    // never hand-writes the system prompt; it picks the Charter and
    // governance mode, and the kernel composes.
    let actor_system_prompt = sut_steward.system_prompt();

    let mut factory = LlmActorFactory {
        backend: actor_backend,
        system_prompt: actor_system_prompt,
        context_id: workspace_id_str.clone().into(),
        turn: 0,
    };

    let mut runner = ScenarioRunner::new(workspace.clone(), sut_steward, max_turns);
    if let Some(b) = opts.refinement_budget {
        runner = runner.with_refinement_budget(b);
    }
    if let Some(trigger) = opts.task_trigger() {
        runner = runner.with_task_trigger(trigger);
    }
    let result = runner
        .run(
            tester.as_mut(),
            &mut factory,
            judge.as_ref().map(|j| j as &dyn Judge),
        )
        .await;

    let output = RunOutput {
        workspace_id: workspace_id_str,
        run_id,
        run_dir: run_dir.display().to_string(),
        receipts_log: receipts_path.display().to_string(),
        cognition_log: cognition_path.display().to_string(),
        tasks: result.tasks,
        attempts: result.attempts,
        receipts: result.trail,
        judge: result.judge,
        turns: result.turns,
        terminated_by_budget: result.terminated_by_budget,
        tester_failure: result.tester_failure.map(|e| e.to_string()),
    };
    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

enum TesterChoice<'a> {
    Configured(&'a TesterConfig),
    Singleton(String),
}

impl Options {
    fn singleton_message(
        &self,
        workspace_root: &std::path::Path,
    ) -> Result<Option<String>, RunError> {
        match (self.user_message.as_ref(), self.selection_trigger.as_ref()) {
            (Some(_), Some(_)) => Err(RunError(
                "--user-message and --selection-* are both set; use one trigger".into(),
            )),
            (Some(m), None) => Ok(Some(m.clone())),
            (None, Some(selection)) => selection_message(workspace_root, selection).map(Some),
            (None, None) => Ok(None),
        }
    }

    fn task_trigger(&self) -> Option<TaskTrigger> {
        self.selection_trigger.as_ref().map(|selection| TaskTrigger::Selection {
            artifact_id: selection.artifact_id.clone(),
            range: selection.range,
            action_name: selection.action.name.clone(),
            action_kind: selection.action.kind,
        })
    }
}

fn tester_or_message<'a>(
    configured: &'a Option<TesterConfig>,
    singleton_message: Option<String>,
) -> Result<TesterChoice<'a>, RunError> {
    match (configured, singleton_message) {
        (Some(t), None) => Ok(TesterChoice::Configured(t)),
        (None, Some(m)) => Ok(TesterChoice::Singleton(m)),
        (Some(_), Some(_)) => Err(RunError(
            "[tester] in steward.toml and a singleton trigger are both set; use one".into(),
        )),
        (None, None) => Err(RunError(
            "neither [tester] in steward.toml nor a singleton trigger is set".into(),
        )),
    }
}

fn selection_message(
    workspace_root: &std::path::Path,
    selection: &SelectionTriggerOptions,
) -> Result<String, RunError> {
    let trigger = Trigger::Selection {
        artifact_id: selection.artifact_id.clone(),
        range: selection.range,
        action: selection.action.clone(),
    };
    let selected_text =
        read_selection_text(workspace_root, &selection.artifact_id, selection.range)?;
    let action_kind = selection.action.kind.as_wire_str();
    let required_tool = match selection.action.kind {
        SelectionActionKind::Generative => "modify_artifact",
        SelectionActionKind::Evaluative => "record_finding",
    };
    let response_template = match selection.action.kind {
        SelectionActionKind::Generative => {
            r#"{"tool":"modify_artifact","params":{"kind":"text","artifact_id":"ARTIFACT_ID","range":{"start":START,"end":END,"start_line":START_LINE,"end_line":END_LINE},"replacement":"REPLACEMENT_TEXT","summary":"PROFESSIONAL_SUMMARY"}}"#
        }
        SelectionActionKind::Evaluative => {
            r#"{"tool":"record_finding","params":{"artifact_id":"ARTIFACT_ID","range":{"start":START,"end":END,"start_line":START_LINE,"end_line":END_LINE},"concern":"CONCERN","severity":"medium","detail":"DETAIL"}}"#
        }
    };
    Ok(format!(
        "Selection trigger:\ntrigger_json: {}\naction_name: {}\naction_kind: {}\nartifact_id: {}\nrange: start={}, end={}, start_line={}, end_line={}\nselected_text:\n{}\n\nReturn one raw JSON object and no other text. Propose exactly one `{}` Tool call for this action. Generative actions modify artifacts only through `modify_artifact` with kind=\"text\", artifact_id, range, replacement, and summary. Evaluative actions record findings only through `record_finding` with artifact_id, range, concern, severity, and detail (the Tool resolves the findings store internally).\nJSON shape:\n{}",
        serde_json::to_string(&trigger)
            .map_err(|e| RunError(format!("serialize selection trigger: {e}")))?,
        selection.action.name,
        action_kind,
        selection.artifact_id,
        selection.range.start,
        selection.range.end,
        selection.range.start_line,
        selection.range.end_line,
        selected_text,
        required_tool,
        response_template,
    ))
}

fn read_selection_text(
    workspace_root: &std::path::Path,
    artifact_id: &ArtifactId,
    range: ArtifactRange,
) -> Result<String, RunError> {
    let root = workspace_root.canonicalize().map_err(|e| {
        RunError(format!(
            "canonicalize workspace_root {}: {e}",
            workspace_root.display()
        ))
    })?;
    let candidate = root.join(&artifact_id.0);
    let artifact_path = candidate
        .canonicalize()
        .map_err(|e| RunError(format!("canonicalize artifact `{artifact_id}`: {e}")))?;
    if !artifact_path.starts_with(&root) {
        return Err(RunError(format!(
            "artifact `{artifact_id}` escapes workspace root {}",
            root.display()
        )));
    }
    let content = std::fs::read_to_string(&artifact_path)
        .map_err(|e| RunError(format!("read artifact `{artifact_id}`: {e}")))?;
    if range.start > range.end || range.end > content.len() {
        return Err(RunError(format!(
            "range {}..{} exceeds artifact `{artifact_id}` length {}",
            range.start,
            range.end,
            content.len()
        )));
    }
    if !content.is_char_boundary(range.start) || !content.is_char_boundary(range.end) {
        return Err(RunError(format!(
            "range {}..{} does not align with UTF-8 boundaries for `{artifact_id}`",
            range.start, range.end
        )));
    }
    Ok(content[range.start..range.end].to_string())
}

/// Backend role label. Used as the backend id (operator-visible in the
/// cognition log under `backend_id`) and as the error-context prefix.
#[derive(Debug, Clone)]
enum BackendRole {
    Actor,
    Evaluator { frame_id: String },
    Tester,
    Judge,
}

impl BackendRole {
    fn id(&self) -> String {
        match self {
            BackendRole::Actor => "actor".into(),
            BackendRole::Evaluator { frame_id } => format!("eval-{frame_id}"),
            BackendRole::Tester => "tester".into(),
            BackendRole::Judge => "judge".into(),
        }
    }

    fn label(&self) -> String {
        match self {
            BackendRole::Actor => "actor".into(),
            BackendRole::Evaluator { frame_id } => format!("evaluator for `{frame_id}`"),
            BackendRole::Tester => "tester".into(),
            BackendRole::Judge => "judge".into(),
        }
    }
}

/// Per-role fake-response queue contents. Maps the various TOML shapes
/// (sequence for actor/tester, per-Frame map for evaluator, single for
/// judge) into one input to `build_backend`.
enum FakeCorpus<'a> {
    /// Empty queue is rejected as a configuration error.
    Sequence(&'a [String]),
    /// `None` is rejected as a configuration error.
    Single(Option<&'a str>),
    /// Empty queue is permitted (Frames may have no per-Frame responses
    /// when only some Frames are exercised).
    Optional(&'a [String]),
}

/// Per-invocation context passed to every backend builder: the shared
/// cognition log (Arc-cloned cheaply per builder call) and the shared
/// OpenAI factory when at least one role uses `backend = "openai"`.
struct BackendCtx<'a> {
    log: Arc<JsonlSink>,
    openai: Option<&'a OpenAiBackendFactory>,
}

/// Single backend-construction helper: matches on `BackendKind`,
/// builds the inner backend, wraps it in `LoggingBackend`. The four
/// roles call this; new backends (Anthropic, vLLM, …) are one arm
/// added here.
fn build_backend(
    role: BackendRole,
    kind: BackendKind,
    fakes: FakeCorpus<'_>,
    model: Option<&str>,
    ctx: &BackendCtx<'_>,
) -> Result<Arc<dyn CognitionBackend>, RunError> {
    let id = role.id();
    let inner: Arc<dyn CognitionBackend> = match kind {
        BackendKind::Fake => {
            let backend = Arc::new(FakeCognitionBackend::new(id.clone()));
            match fakes {
                FakeCorpus::Sequence(rs) => {
                    if rs.is_empty() {
                        return Err(RunError(format!(
                            "{} backend = \"fake\" but no fake_responses configured",
                            role.label()
                        )));
                    }
                    for r in rs {
                        backend.enqueue(r);
                    }
                }
                FakeCorpus::Single(opt) => {
                    let r = opt.ok_or_else(|| {
                        RunError(format!(
                            "{} backend = \"fake\" but fake_response not set",
                            role.label()
                        ))
                    })?;
                    backend.enqueue(r);
                }
                FakeCorpus::Optional(rs) => {
                    for r in rs {
                        backend.enqueue(r);
                    }
                }
            };
            backend
        }
        BackendKind::OpenAi => {
            let factory = ctx.openai.ok_or_else(|| {
                RunError(format!(
                    "{} backend = \"openai\" but OpenAI factory not initialized — \
                     LLM_BASE_URL must be set in env",
                    role.label()
                ))
            })?;
            Arc::new(
                factory
                    .build(id, model.map(str::to_string))
                    .map_err(|e| RunError(format!("{} openai backend: {e}", role.label())))?,
            )
        }
    };
    Ok(Arc::new(LoggingBackend::new(inner, Arc::clone(&ctx.log))))
}

/// Build the shared OpenAI factory only when at least one role uses
/// `backend = "openai"` — pure fake-mode deployments stay env-free.
fn build_openai_factory_if_needed(
    cfg: &config::DeploymentConfig,
) -> Result<Option<OpenAiBackendFactory>, RunError> {
    let needs_openai = cfg.steward.actor.backend == BackendKind::OpenAi
        || cfg.steward.evaluator.backend == BackendKind::OpenAi
        || cfg
            .steward
            .tester
            .as_ref()
            .is_some_and(|t| t.backend == BackendKind::OpenAi)
        || cfg
            .steward
            .judge
            .as_ref()
            .is_some_and(|j| j.backend == BackendKind::OpenAi);
    if !needs_openai {
        return Ok(None);
    }
    OpenAiBackendFactory::from_env()
        .map(Some)
        .map_err(|e| RunError(format!("OpenAI factory: {e}")))
}

fn build_evaluator_backends_per_frame(
    cfg: &config::DeploymentConfig,
    ctx: &BackendCtx<'_>,
) -> Result<BTreeMap<String, Arc<dyn CognitionBackend>>, RunError> {
    let mut map: BTreeMap<String, Arc<dyn CognitionBackend>> = BTreeMap::new();
    let evaluator_cfg = &cfg.steward.evaluator;
    for f in &cfg.charter_def.frames {
        let responses: Vec<String> = evaluator_cfg
            .fake_responses
            .get(&f.id.0)
            .cloned()
            .unwrap_or_default();
        let backend = build_backend(
            BackendRole::Evaluator {
                frame_id: f.id.0.clone(),
            },
            evaluator_cfg.backend,
            FakeCorpus::Optional(&responses),
            evaluator_cfg.model.as_deref(),
            ctx,
        )?;
        map.insert(f.id.0.clone(), backend);
    }
    Ok(map)
}

fn build_llm_tester(cfg: &TesterConfig, ctx: &BackendCtx<'_>) -> Result<LlmTester, RunError> {
    let backend = build_backend(
        BackendRole::Tester,
        cfg.backend,
        FakeCorpus::Sequence(&cfg.fake_responses),
        cfg.model.as_deref(),
        ctx,
    )?;
    Ok(LlmTester::new("tester", backend, &cfg.brief))
}

/// Tester adapter that yields a single user message and then empties.
/// Lets `--user-message` drive the same `ScenarioRunner` that a
/// configured Tester drives.
struct SingletonTester {
    msg: Option<String>,
}

impl SingletonTester {
    fn new(msg: String) -> Self {
        Self { msg: Some(msg) }
    }
}

#[async_trait]
impl Tester for SingletonTester {
    fn id(&self) -> &str {
        "singleton-tester"
    }
    async fn next_message(&mut self, _: &[Receipt]) -> Result<String, TesterError> {
        Ok(self.msg.take().unwrap_or_default())
    }
}

/// Per-turn ActorFactory. The actor backend is shared across turns —
/// its queue advances as the loop calls `Actor::step`. A fresh
/// `LlmActor` per turn keeps per-turn conversation history bounded.
struct LlmActorFactory {
    backend: Arc<dyn CognitionBackend>,
    system_prompt: String,
    context_id: Arc<str>,
    turn: usize,
}

impl ActorFactory for LlmActorFactory {
    fn for_turn(&mut self, msg: &str, _prior: &[Receipt]) -> Box<dyn Actor> {
        self.turn += 1;
        Box::new(
            LlmActor::new(
                format!("actor-turn-{}", self.turn),
                self.backend.clone(),
                self.system_prompt.clone(),
                self.context_id.clone(),
            )
            .with_initial_user_message(msg),
        )
    }
}
