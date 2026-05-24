//! CharteredOS kernel.
//!
//! The negative feedback loop: setpoint = Frame, plant = Actor,
//! sensor = Evaluator, comparator = Gate, error signal = Refinement
//! signal. See `../docs/SPECIFICATION.md`.
//!
//! Backend swap (fake / OpenAI / Anthropic / vLLM / SGLang) lives at
//! the CognitionBackend trait — one role implementation per role
//! (LlmActor, LlmEvaluator, LlmTester, LlmJudge), one swap point.

mod actor;
mod artifact;
mod charter;
mod charter_loader;
mod cognition;
mod frame;
mod gate;
mod governance;
mod loop_runner;
mod prompt;
mod receipt;
mod scenario;
mod skill;
mod snapshot;
mod steward;
mod task;
mod tool;
mod verdict;
mod workspace;

pub use actor::{Action, Actor, DEFAULT_INNER_STEP_BUDGET, LlmActor, Observation};
pub use artifact::{
    Artifact, ArtifactBackend, ArtifactId, ArtifactKindId, ArtifactRange, ArtifactStore,
    BackendConflict, Edit, InMemoryArtifactStore, InMemoryRecordStore, InMemoryTextBackend,
    ListArtifacts, ModifyArtifact, Projection, ReadArtifact, Record, RecordAppend, RecordFilter,
    Selector, TextArtifactSeed, apply_text_edit, kind_record_store, kind_text,
    parse_artifact_range, parse_record_append, parse_record_filter, parse_text_edit,
    parse_text_range_from_selector, slice_range, validate_range,
};
pub use charter::{Charter, DeclaredScope, RoleContext, Scope, ScopeKind};
pub use charter_loader::{
    CharterDef, CharterLoadError, FrameDef, RoleContextDef, build_charter, build_role_context,
    parse_charter_def, parse_role_context_def,
};
pub use cognition::{
    ActionHint, CognitionBackend, CognitionError, CognitionRequest, CognitionResponse,
    DecisionLine, FakeCognitionBackend, Message, MessageRole,
};
pub use frame::{Frame, FrameId, FrameRef, PriorReceiptQuery};
pub use gate::Gate;
pub use governance::GovernanceMode;
pub use loop_runner::{LoopOutcome, LoopRunner};
pub use prompt::assemble_actor_system_prompt;
pub use receipt::{
    InMemoryReceiptStore, Outcome, Receipt, ReceiptId, ReceiptStore, ReceiptStoreError,
    RefinementSignal, receipt_matches_query,
};
pub use scenario::{
    ActorFactory, Judge, JudgeError, JudgeOutput, JudgeReport, LlmJudge, LlmTester, ScenarioResult,
    ScenarioRunner, SelectionAction, SelectionActionKind, Tester, TesterError, Trigger,
};
pub use skill::{Skill, skills_content_hash};
pub use snapshot::{Snapshot, SnapshotId};
pub use steward::{Steward, StewardId};
pub use task::{AttemptId, AttemptRecord, TaskId, TaskRecord, TaskStatus, TaskTrigger};
pub use tool::{ToolCall, ToolExecutor, ToolId, ToolParams, ToolRegistry, ToolResult};
pub use verdict::{
    Decision, Evaluator, EvaluatorEntry, EvaluatorError, LlmEvaluator, Ruling, Verdict,
    assert_no_persuasive_context,
};
pub use workspace::{Workspace, WorkspaceId, WorkspaceValidationError};
