//! Library-facing API: `chartered_runtime::Agent` is the embeddable
//! drop-in surface. Construct once with `from_chartered_dir`; call
//! `run(brief)` per invocation. Each call is atomic: one Brief → one
//! Task → one `RunResult`. The Agent is stateless across calls; disk is
//! the continuity medium per spec §The Runtime. Holding an Agent across
//! many calls is a performance choice (warmed config, pooled HTTP
//! clients, canonicalized paths), never a correctness one.
//!
//! The binary in `main.rs` is one consumer of this surface; the
//! dashboard subprocess-spawning the binary is another; downstream
//! Rust embedders are a third.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chartered_core::{
    ACTOR_FAILURE_TOOL, Actor, ActorFactory, ArtifactId, ArtifactRange, BUDGET_EXHAUSTED_TOOL,
    CognitionBackend, Evaluator, FakeCognitionBackend, FrameDef, HALT_TOOL, Judge, JudgeReport,
    LlmActor, LlmEvaluator, LlmJudge, LlmTester, Outcome, Receipt, ReceiptStore, ScenarioRunner,
    SelectionAction, SelectionActionKind, Snapshot, TaskRecord, TaskTrigger, Tester, TesterError,
    ToolExecutor, ToolId, ToolRegistry, Trigger,
};
use chartered_dispatch::{DeploymentPaths, ExecutorRegistry, JsonlSink};
use serde::Serialize;

use crate::config::{self, BackendKind, DeploymentConfig, TesterConfig};
use crate::gemini_backend::GeminiBackendFactory;
use crate::openai_backend::OpenAiBackendFactory;
use crate::persistence::{self, AppendOnlyFileReceiptStore, LoggingBackend};

/// Per-Brief result of one Agent invocation. Mirrors the binary's
/// stdout JSON shape so the binary can serialize this directly.
#[derive(Debug, Serialize)]
pub struct RunResult {
    pub outcome: AgentOutcome,
    pub artifacts: RunArtifacts,
    pub paths: RunPaths,
}

/// Categorical outcome of one Agent invocation. Restraint (Quiet) is a
/// first-class outcome — the loop ran, considered, and chose not to
/// produce an externally-observable effect. Distinct from Failed
/// (cognition error) and Escalated (bounded budget exhausted).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentOutcome {
    /// At least one `modify_artifact` proposal landed `Allowed`. The
    /// loop produced an externally-observable effect.
    Externalized,
    /// Loop ran to a clean Halt without any `modify_artifact` reaching
    /// `Allowed`. Receipts may still exist for read-only proposals or
    /// for denied externalizing proposals.
    Quiet,
    /// Loop terminated because a bounded budget was exhausted.
    Escalated { cause: EscalationCause },
    /// Structural failure of the Actor's cognition or the Tester
    /// driving it. Surfaced so operators distinguish "task complete"
    /// from "Actor failed."
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EscalationCause {
    /// The Actor's agentic inner loop hit `DEFAULT_INNER_STEP_BUDGET`
    /// without producing an `ActionHint`. Receipt carries
    /// `intercept_complete=false`.
    InnerStepBudget,
    /// The outer refinement budget exhausted: too many denied
    /// proposals in one Task without convergence.
    RefinementBudget,
}

/// The observable artifacts of one Agent invocation: every Task,
/// Attempt, and Receipt the loop emitted, plus the optional Judge
/// report and Tester accounting.
#[derive(Debug, Serialize)]
pub struct RunArtifacts {
    pub tasks: Vec<TaskRecord>,
    pub attempts: Vec<chartered_core::AttemptRecord>,
    pub receipts: Vec<Receipt>,
    pub judge: JudgeReport,
    pub turns: usize,
    pub terminated_by_budget: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tester_failure: Option<String>,
}

/// On-disk paths produced by one Agent invocation. Each call writes a
/// fresh run dir.
#[derive(Debug, Serialize)]
pub struct RunPaths {
    pub workspace_id: String,
    pub run_id: String,
    pub run_dir: PathBuf,
    pub receipts_log: PathBuf,
    pub cognition_log: PathBuf,
}

/// The shape of one input event into an Agent invocation. The
/// simplest case (`Prompt`) maps onto any agentic loop's
/// string-in/string-out interface. `Selection` carries the
/// dashboard's text-selection trigger. `TesterDriven` defers turn
/// generation to a `[tester]` configured in `steward.toml`, for
/// scenario-driven multi-turn runs. Additional variants (Standing for
/// passive subscription events) are added behind the boundary as new
/// substrates land.
#[derive(Debug, Clone)]
pub enum Brief {
    /// Single user message; drives a one-turn Task.
    Prompt(String),
    /// User selected a range in an artifact and chose an action
    /// (Refine | Review). The Agent synthesizes the Task brief from
    /// the selection text and the action's response template.
    Selection {
        artifact_id: ArtifactId,
        range: ArtifactRange,
        action: SelectionAction,
    },
    /// The Agent's configured `[tester]` drives turns; no externally
    /// supplied input. Errors if no Tester is configured.
    TesterDriven,
}

/// Streaming event surface for consumers that want incremental
/// visibility (the dashboard, in particular). Emitted in roughly the
/// order events occur; consumers may correlate by `task_id` /
/// `attempt_id` / `receipt_id` carried inside the payloads.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The Actor produced reasoning text without an ActionHint. Burns
    /// one inner-step-budget slot; not externalizable.
    Reasoning { backend_id: String, text: String },
    /// The Actor committed to a Tool call. Crosses the Gate next.
    Proposed { tool_call: chartered_core::ToolCall },
    /// A per-Frame Evaluator returned a Verdict.
    Evaluated {
        frame_ref: chartered_core::FrameRef,
        verdict: chartered_core::Verdict,
    },
    /// A Receipt landed on disk.
    Receipted { receipt: Receipt },
    /// A Tool call's dispatch produced an externally-observable
    /// effect.
    Externalized { tool: ToolId },
    /// The loop ended. The outcome is the categorical summary.
    Done { outcome: AgentOutcome },
}

/// Errors raised by `Agent::from_chartered_dir`.
#[derive(Debug)]
pub struct AgentBuildError(pub String);

impl std::fmt::Display for AgentBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AgentBuildError {}

/// Errors raised by `Agent::run`. Distinct from `AgentOutcome::Failed`:
/// a `RunError` means the invocation never started a Task (config
/// mismatch, IO failure opening the receipt log); `Failed` means the
/// invocation started but the Actor's cognition could not produce a
/// valid Action.
#[derive(Debug)]
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

impl From<config::ConfigError> for AgentBuildError {
    fn from(e: config::ConfigError) -> Self {
        AgentBuildError(e.0)
    }
}

/// The embeddable governed-runtime handle. Construct once with
/// `from_chartered_dir`; call `run(brief)` per invocation. Stateless
/// across calls — every `run` produces a fresh `run_id` and writes a
/// fresh JSONL pair under `<chartered_dir>/runs/<run_id>/`.
pub struct Agent {
    cfg: DeploymentConfig,
    workspace_root: PathBuf,
    snapshots_dir: PathBuf,
    executor_registry: ExecutorRegistry,
    openai_factory: Option<OpenAiBackendFactory>,
    gemini_factory: Option<GeminiBackendFactory>,
    refinement_budget: Option<usize>,
}

impl Agent {
    /// Build an Agent from a `.chartered/` directory. Parses
    /// configuration, builds backend factories, canonicalizes
    /// deployment paths, opens the executor registry. Performs no LLM
    /// calls and writes no Receipts; both happen on `run`.
    pub async fn from_chartered_dir(
        chartered_dir: impl AsRef<Path>,
        workspace_root: Option<PathBuf>,
    ) -> Result<Self, AgentBuildError> {
        let chartered_dir = chartered_dir.as_ref().to_path_buf();
        let cfg = config::load(&chartered_dir)?;
        let workspace_root = match workspace_root {
            Some(p) => p,
            None => cfg
                .chartered_dir
                .parent()
                .ok_or_else(|| AgentBuildError("chartered_dir has no parent".into()))?
                .to_path_buf(),
        };
        let openai_factory = build_openai_factory_if_needed(&cfg)
            .map_err(|e| AgentBuildError(e.0))?;
        let gemini_factory = build_gemini_factory_if_needed(&cfg)
            .map_err(|e| AgentBuildError(e.0))?;
        let snapshots_dir = cfg.chartered_dir.join("snapshots");
        let deployment_paths =
            DeploymentPaths::canonicalize(&workspace_root, &cfg.chartered_dir).map_err(|e| {
                AgentBuildError(format!(
                    "canonicalize deployment paths (workspace_root={}, chartered_dir={}): {e}",
                    workspace_root.display(),
                    cfg.chartered_dir.display(),
                ))
            })?;
        let executor_registry = ExecutorRegistry::new(deployment_paths)
            .await
            .map_err(|e| AgentBuildError(format!("constructing executor registry: {e}")))?;
        Ok(Self {
            cfg,
            workspace_root,
            snapshots_dir,
            executor_registry,
            openai_factory,
            gemini_factory,
            refinement_budget: None,
        })
    }

    /// Override the default outer refinement budget (kernel default is
    /// 3). Applies to every subsequent `run`.
    pub fn with_refinement_budget(mut self, budget: usize) -> Self {
        self.refinement_budget = Some(budget);
        self
    }

    /// Reference back to the `.chartered/` directory the Agent loaded.
    pub fn chartered_dir(&self) -> &Path {
        &self.cfg.chartered_dir
    }

    /// Reference back to the canonicalized workspace root.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Whether the Agent uses a configured Tester for multi-turn input.
    /// When true, Brief inputs that supply a singleton trigger conflict
    /// with the Tester and `run` will return `RunError`.
    pub fn has_configured_tester(&self) -> bool {
        self.cfg.steward.tester.is_some()
    }

    /// Run one Brief to completion. Atomic: opens a new run dir,
    /// dispatches the loop, writes Receipts, returns the categorical
    /// outcome and the per-run artifacts. Each call is independent —
    /// the Agent holds no per-call state.
    pub async fn run(&self, brief: Brief) -> Result<RunResult, RunError> {
        let singleton_message = brief_singleton_message(&self.workspace_root, &brief)?;
        let task_trigger = brief_task_trigger(&brief);

        // Reject the impossible state up front: a configured Tester
        // already drives turns; a Brief with a singleton message would
        // double-drive. Spec §The Runtime → triggers compose, but never
        // by accident at this boundary.
        let tester_choice = pick_tester(&self.cfg.steward.tester, singleton_message)?;
        let max_turns = match &self.cfg.steward.tester {
            Some(t) => t.max_turns,
            None => 1,
        };

        let run_id = persistence::make_run_id();
        let run_dir = persistence::run_dir(&self.cfg.chartered_dir, &run_id);
        std::fs::create_dir_all(&run_dir)
            .map_err(|e| RunError(format!("creating run dir {}: {e}", run_dir.display())))?;
        let receipts_path = run_dir.join("receipts.jsonl");
        let cognition_path = run_dir.join("cognition.jsonl");

        let receipt_store: Arc<dyn ReceiptStore> = Arc::new(
            AppendOnlyFileReceiptStore::create(&receipts_path)
                .await
                .map_err(|e| RunError(format!("opening {}: {e}", receipts_path.display())))?,
        );
        let cognition_log: Arc<JsonlSink> = Arc::new(
            JsonlSink::create(&cognition_path)
                .await
                .map_err(|e| RunError(format!("opening {}: {e}", cognition_path.display())))?,
        );

        let backends = BackendCtx {
            log: cognition_log.clone(),
            openai: self.openai_factory.as_ref(),
            gemini: self.gemini_factory.as_ref(),
        };

        let actor_backend = build_backend(
            BackendRole::Actor,
            self.cfg.steward.actor.backend,
            FakeCorpus::Sequence(&self.cfg.steward.actor.fake_responses),
            self.cfg.steward.actor.model.as_deref(),
            &backends,
        )?;
        let evaluator_backends_arc =
            Arc::new(build_evaluator_backends_per_frame(&self.cfg, &backends)?);
        let mut tester: Box<dyn Tester> = match tester_choice {
            TesterChoice::Configured(t) => Box::new(build_llm_tester(t, &backends)?),
            TesterChoice::Singleton(msg) => Box::new(SingletonTester::new(msg)),
        };
        let judge: Option<LlmJudge> = match &self.cfg.steward.judge {
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

        // workspace_root was canonicalized at Agent construction; reuse
        // its display form as the workspace identifier. Substituting a
        // literal would mask path-resolution defects and corrupt
        // `context_id` used for prior_receipt_queries.
        let workspace_id_str = self.workspace_root.display().to_string();
        let governance_mode: chartered_core::GovernanceMode = self.cfg.runtime.governance.into();

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

        let (charter, role_context, skills) = self.cfg.build_charter(evaluator_factory);
        let snapshot = Snapshot::new(charter, role_context, skills);

        chartered_dispatch::persist_snapshot(&snapshot, &self.snapshots_dir)
            .await
            .map_err(|e| {
                RunError(format!(
                    "persist Snapshot {} to {}: {e}",
                    snapshot.id,
                    self.snapshots_dir.display()
                ))
            })?;

        let mut registry = ToolRegistry::new();
        for tr in &self.cfg.tools {
            let exec: Arc<dyn ToolExecutor> = self
                .executor_registry
                .build(&tr.executor, &ToolId::new(&tr.id))
                .map_err(|e| RunError(format!("building executor for tool `{}`: {e}", tr.id)))?;
            registry.register(exec);
        }

        let sut_steward = chartered_core::Steward::new(
            chartered_core::StewardId::new("sut"),
            snapshot,
            Arc::new(registry),
        )
        .with_governance_mode(governance_mode);
        let workspace = Arc::new(
            chartered_core::Workspace::single_with_store(
                chartered_core::WorkspaceId::new(&workspace_id_str),
                sut_steward,
                receipt_store,
            )
            .map_err(|e| RunError(format!("workspace validation failed: {e}")))?,
        );
        let sut_steward = workspace.sole_steward().clone();
        let actor_system_prompt = sut_steward.system_prompt();

        let mut factory = LlmActorFactory {
            backend: actor_backend,
            system_prompt: actor_system_prompt,
            context_id: workspace_id_str.clone().into(),
            turn: 0,
        };

        let mut runner = ScenarioRunner::new(workspace.clone(), sut_steward, max_turns);
        if let Some(b) = self.refinement_budget {
            runner = runner.with_refinement_budget(b);
        }
        if let Some(trigger) = task_trigger {
            runner = runner.with_task_trigger(trigger);
        }
        let result = runner
            .run(
                tester.as_mut(),
                &mut factory,
                judge.as_ref().map(|j| j as &dyn Judge),
            )
            .await;

        let outcome = classify_outcome(&result);

        Ok(RunResult {
            outcome,
            artifacts: RunArtifacts {
                tasks: result.tasks,
                attempts: result.attempts,
                receipts: result.trail,
                judge: result.judge,
                turns: result.turns,
                terminated_by_budget: result.terminated_by_budget,
                tester_failure: result.tester_failure.map(|e| e.to_string()),
            },
            paths: RunPaths {
                workspace_id: workspace_id_str,
                run_id,
                run_dir,
                receipts_log: receipts_path,
                cognition_log: cognition_path,
            },
        })
    }
}

/// Categorize the ScenarioRunner output into one of four buckets:
/// Externalized | Quiet | Escalated | Failed.
///
/// The kernel emits sentinel-tool Receipts that pinpoint each
/// terminal state — `BUDGET_EXHAUSTED_TOOL` for refinement budget,
/// `ACTOR_FAILURE_TOOL` for Action::Fail (inner-step budget),
/// `HALT_TOOL` for clean Halt. Classification reads those sentinels;
/// `ScenarioRunner.terminated_by_budget` is just "reached max_turns"
/// and not a useful signal here.
fn classify_outcome(result: &chartered_core::ScenarioResult) -> AgentOutcome {
    if let Some(err) = &result.tester_failure {
        return AgentOutcome::Failed {
            reason: err.to_string(),
        };
    }
    // Refinement-budget exhaustion: the kernel emits a sentinel
    // Receipt with tool=BUDGET_EXHAUSTED_TOOL, outcome=Escalated.
    let refinement_exhausted = result
        .trail
        .iter()
        .any(|r| r.tool_call.tool.0 == BUDGET_EXHAUSTED_TOOL);
    if refinement_exhausted {
        return AgentOutcome::Escalated {
            cause: EscalationCause::RefinementBudget,
        };
    }
    // Inner-step-budget exhaustion: ACTOR_FAILURE_TOOL with reason
    // carrying the kernel's diagnostic phrase.
    let inner_step_exhausted = result.trail.iter().any(|r| {
        r.tool_call.tool.0 == ACTOR_FAILURE_TOOL
            && receipt_reason_contains(r, "inner step budget")
    });
    if inner_step_exhausted {
        return AgentOutcome::Escalated {
            cause: EscalationCause::InnerStepBudget,
        };
    }
    // Any other ACTOR_FAILURE_TOOL is a non-budget cognitive failure
    // (backend error, parse failure). Surface as Failed.
    if let Some(r) = result
        .trail
        .iter()
        .find(|r| r.tool_call.tool.0 == ACTOR_FAILURE_TOOL)
    {
        return AgentOutcome::Failed {
            reason: receipt_reason_string(r).unwrap_or_else(|| "actor cognitive failure".into()),
        };
    }
    let externalized = result
        .trail
        .iter()
        .any(|r| r.outcome == Outcome::Allowed && tool_externalizes(&r.tool_call.tool));
    if externalized {
        AgentOutcome::Externalized
    } else {
        AgentOutcome::Quiet
    }
}

fn receipt_reason_contains(r: &Receipt, needle: &str) -> bool {
    receipt_reason_string(r)
        .map(|s| s.contains(needle))
        .unwrap_or(false)
}

fn receipt_reason_string(r: &Receipt) -> Option<String> {
    r.tool_call
        .params
        .0
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn tool_externalizes(tool: &ToolId) -> bool {
    // A Tool externalizes when its dispatch produces an effect a
    // downstream observer can perceive (file write, message send,
    // record append visible in the dashboard, network call). The
    // read-shaped Tools (`read_artifact`, `list_artifacts`,
    // `query_artifact`, `subscribe_artifact`, `cite_artifact`,
    // `attest_artifact`, `ask_question`) are internal-only by
    // construction (see spec §The Chartered Boundary). The kernel's
    // sentinel Receipts (`<halt>`, `<actor_failure>`,
    // `<budget_exhausted>`) are controller events with no Tool
    // dispatch behind them. Everything else — `modify_artifact`,
    // `write_file`, `exec_command`, ad-hoc deployment Tools — is
    // treated as externalizing here. Deployments that need a finer
    // distinction can wrap or rename their Tools to fall under the
    // internal set or expose a future per-Backend `is_external` bit.
    !matches!(
        tool.0.as_str(),
        "read_artifact"
            | "list_artifacts"
            | "query_artifact"
            | "subscribe_artifact"
            | "cite_artifact"
            | "attest_artifact"
            | "ask_question"
            | HALT_TOOL
            | ACTOR_FAILURE_TOOL
            | BUDGET_EXHAUSTED_TOOL
    )
}

// --- Brief → singleton/trigger conversion ------------------------------

fn brief_singleton_message(
    workspace_root: &Path,
    brief: &Brief,
) -> Result<Option<String>, RunError> {
    match brief {
        Brief::Prompt(text) => Ok(Some(text.clone())),
        Brief::Selection {
            artifact_id,
            range,
            action,
        } => selection_message(workspace_root, artifact_id, *range, action).map(Some),
        Brief::TesterDriven => Ok(None),
    }
}

fn brief_task_trigger(brief: &Brief) -> Option<TaskTrigger> {
    match brief {
        Brief::Prompt(_) => None,
        Brief::Selection {
            artifact_id,
            range,
            action,
        } => Some(TaskTrigger::Selection {
            artifact_id: artifact_id.clone(),
            range: *range,
            action_name: action.name.clone(),
            action_kind: action.kind,
        }),
        Brief::TesterDriven => None,
    }
}

/// Read the artifact slice referenced by a Selection trigger and
/// compose the singleton message the Actor consumes. Mirrors the
/// payload the dashboard's old `/trigger/selection` produced.
fn selection_message(
    workspace_root: &Path,
    artifact_id: &ArtifactId,
    range: ArtifactRange,
    action: &SelectionAction,
) -> Result<String, RunError> {
    let trigger = Trigger::Selection {
        artifact_id: artifact_id.clone(),
        range,
        action: action.clone(),
    };
    let selected_text = read_selection_text(workspace_root, artifact_id, range)?;
    let action_kind = action.kind.as_wire_str();
    let response_template = match action.kind {
        SelectionActionKind::Generative => {
            r#"{"tool":"modify_artifact","params":{"kind":"text","artifact_id":"ARTIFACT_ID","range":{"start":START,"end":END,"start_line":START_LINE,"end_line":END_LINE},"replacement":"REPLACEMENT_TEXT","summary":"PROFESSIONAL_SUMMARY"}}"#
        }
        SelectionActionKind::Evaluative => {
            r#"{"tool":"modify_artifact","params":{"kind":"record-store","artifact_id":"records","edit":{"append":{"artifact_id":"ARTIFACT_ID","range":{"start":START,"end":END,"start_line":START_LINE,"end_line":END_LINE},"concern":"CONCERN","severity":"medium","detail":"DETAIL"}}}}"#
        }
    };
    Ok(format!(
        "Selection trigger:\ntrigger_json: {}\naction_name: {}\naction_kind: {}\nartifact_id: {}\nrange: start={}, end={}, start_line={}, end_line={}\nselected_text:\n{}\n\nReturn one raw JSON object and no other text. Propose exactly one `modify_artifact` Tool call for this action. Generative actions use `kind=\"text\"` with artifact_id, range, replacement, and summary. Evaluative actions use `kind=\"record-store\"` against the `records` artifact, with `edit.append` carrying artifact_id (source), range, concern, severity, and detail.\nJSON shape:\n{}",
        serde_json::to_string(&trigger)
            .map_err(|e| RunError(format!("serialize selection trigger: {e}")))?,
        action.name,
        action_kind,
        artifact_id,
        range.start,
        range.end,
        range.start_line,
        range.end_line,
        selected_text,
        response_template,
    ))
}

fn read_selection_text(
    workspace_root: &Path,
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

// --- Backend construction (per-run; pure factories) --------------------

enum TesterChoice<'a> {
    Configured(&'a TesterConfig),
    Singleton(String),
}

fn pick_tester<'a>(
    configured: &'a Option<TesterConfig>,
    singleton_message: Option<String>,
) -> Result<TesterChoice<'a>, RunError> {
    match (configured, singleton_message) {
        (Some(t), None) => Ok(TesterChoice::Configured(t)),
        (None, Some(m)) => Ok(TesterChoice::Singleton(m)),
        (Some(_), Some(_)) => Err(RunError(
            "[tester] in steward.toml and a singleton Brief are both set; use one".into(),
        )),
        (None, None) => Err(RunError(
            "neither [tester] in steward.toml nor a singleton Brief is set".into(),
        )),
    }
}

/// Backend role label. Used as the backend id in the cognition log
/// (`backend_id` field) and as the error-context prefix.
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
/// into one input to `build_backend`.
enum FakeCorpus<'a> {
    /// Empty queue is rejected as a configuration error.
    Sequence(&'a [String]),
    /// `None` is rejected as a configuration error.
    Single(Option<&'a str>),
    /// Empty queue is permitted (Frames may have no per-Frame responses
    /// when only some Frames are exercised).
    Optional(&'a [String]),
}

/// Per-invocation backend context: the shared cognition log (Arc) and
/// the cached OpenAI/Gemini factories from the Agent.
struct BackendCtx<'a> {
    log: Arc<JsonlSink>,
    openai: Option<&'a OpenAiBackendFactory>,
    gemini: Option<&'a GeminiBackendFactory>,
}

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
            let enqueue_role = |b: &FakeCognitionBackend, r: &str| match role {
                BackendRole::Actor => {
                    let hint = crate::canonicalize::canonicalize_action_hint(r);
                    b.enqueue_with_action(r.to_string(), hint);
                }
                BackendRole::Evaluator { .. } => {
                    let lines = crate::canonicalize::canonicalize_verdict_lines(r);
                    if lines.is_empty() {
                        b.enqueue(r);
                    } else {
                        b.enqueue_verdict_lines(lines);
                    }
                }
                BackendRole::Judge => {
                    if let Some(out) = crate::canonicalize::canonicalize_judge_output(r) {
                        b.enqueue_judge_output(out);
                    } else {
                        b.enqueue(r);
                    }
                }
                BackendRole::Tester => {
                    b.enqueue(r);
                }
            };
            match fakes {
                FakeCorpus::Sequence(rs) => {
                    if rs.is_empty() {
                        return Err(RunError(format!(
                            "{} backend = \"fake\" but no fake_responses configured",
                            role.label()
                        )));
                    }
                    for r in rs {
                        enqueue_role(&backend, r);
                    }
                }
                FakeCorpus::Single(opt) => {
                    let r = opt.ok_or_else(|| {
                        RunError(format!(
                            "{} backend = \"fake\" but fake_response not set",
                            role.label()
                        ))
                    })?;
                    enqueue_role(&backend, r);
                }
                FakeCorpus::Optional(rs) => {
                    for r in rs {
                        enqueue_role(&backend, r);
                    }
                }
            };
            backend
        }
        BackendKind::OpenAi => {
            let factory = ctx.openai.ok_or_else(|| {
                RunError(format!(
                    "{} backend = \"openai\" but OpenAI factory not initialized — \
                     OPEN_AI_BASE_URL must be set in env",
                    role.label()
                ))
            })?;
            Arc::new(
                factory
                    .build(id, model.map(str::to_string))
                    .map_err(|e| RunError(format!("{} openai backend: {e}", role.label())))?,
            )
        }
        BackendKind::Gemini => {
            let factory = ctx.gemini.ok_or_else(|| {
                RunError(format!(
                    "{} backend = \"gemini\" but Gemini factory not initialized — \
                     GEMINI_API_KEY must be set in env",
                    role.label()
                ))
            })?;
            Arc::new(
                factory
                    .build(id, model.map(str::to_string))
                    .map_err(|e| RunError(format!("{} gemini backend: {e}", role.label())))?,
            )
        }
    };
    Ok(Arc::new(LoggingBackend::new(inner, Arc::clone(&ctx.log))))
}

fn build_openai_factory_if_needed(
    cfg: &DeploymentConfig,
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

fn build_gemini_factory_if_needed(
    cfg: &DeploymentConfig,
) -> Result<Option<GeminiBackendFactory>, RunError> {
    let needs_gemini = cfg.steward.actor.backend == BackendKind::Gemini
        || cfg.steward.evaluator.backend == BackendKind::Gemini
        || cfg
            .steward
            .tester
            .as_ref()
            .is_some_and(|t| t.backend == BackendKind::Gemini)
        || cfg
            .steward
            .judge
            .as_ref()
            .is_some_and(|j| j.backend == BackendKind::Gemini);
    if !needs_gemini {
        return Ok(None);
    }
    GeminiBackendFactory::from_env()
        .map(Some)
        .map_err(|e| RunError(format!("Gemini factory: {e}")))
}

fn build_evaluator_backends_per_frame(
    cfg: &DeploymentConfig,
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
/// Lets a Brief drive the same `ScenarioRunner` that a configured
/// multi-turn Tester drives.
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

// --- Unit tests for the type vocabulary --------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_outcome_serializes_with_kind_tag() {
        let q = serde_json::to_value(AgentOutcome::Quiet).unwrap();
        assert_eq!(q, json!({"kind": "quiet"}));
        let e = serde_json::to_value(AgentOutcome::Externalized).unwrap();
        assert_eq!(e, json!({"kind": "externalized"}));
        let esc = serde_json::to_value(AgentOutcome::Escalated {
            cause: EscalationCause::InnerStepBudget,
        })
        .unwrap();
        assert_eq!(
            esc,
            json!({"kind": "escalated", "cause": "inner_step_budget"})
        );
        let f = serde_json::to_value(AgentOutcome::Failed {
            reason: "x".into(),
        })
        .unwrap();
        assert_eq!(f, json!({"kind": "failed", "reason": "x"}));
    }

    #[test]
    fn tool_externalizes_excludes_read_shaped_and_sentinels() {
        for externalizing in [
            "modify_artifact",
            "write_file",
            "exec_command",
            "send_message",
            "post_finding",
        ] {
            assert!(
                tool_externalizes(&ToolId::new(externalizing)),
                "{externalizing} must externalize"
            );
        }
        for internal in [
            "read_artifact",
            "list_artifacts",
            "query_artifact",
            "subscribe_artifact",
            "cite_artifact",
            "attest_artifact",
            "ask_question",
            HALT_TOOL,
            ACTOR_FAILURE_TOOL,
            BUDGET_EXHAUSTED_TOOL,
        ] {
            assert!(
                !tool_externalizes(&ToolId::new(internal)),
                "{internal} must not externalize"
            );
        }
    }

    #[test]
    fn brief_prompt_extracts_singleton_and_no_trigger() {
        let brief = Brief::Prompt("hi".into());
        let workspace = std::env::temp_dir();
        let msg = brief_singleton_message(&workspace, &brief).unwrap();
        assert_eq!(msg.as_deref(), Some("hi"));
        assert!(brief_task_trigger(&brief).is_none());
    }
}
