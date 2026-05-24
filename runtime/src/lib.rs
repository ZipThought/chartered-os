//! Runtime library: deployment-config loading and the embeddable
//! governed agent surface. Spec §The Runtime, §The Chartered Boundary.
//!
//! Consumers reach for `chartered_runtime::Agent` first — construct
//! once from a `.chartered/` directory, call `run(brief)` per
//! invocation. The Agent is stateless across calls; the binary in
//! `main.rs` is one consumer of this surface among many.
//!
//! ONE loader. ONE Agent. The fake/real swap lives at the
//! `CognitionBackend` (per-role config in `steward.toml`) and at the
//! `ToolExecutor` registry (per-tool config in `tools/*.toml`).
//! Nothing else differs between a test deployment and a production
//! deployment.

pub mod agent;
pub mod canonicalize;
pub mod charter_loader;
pub mod config;
pub mod gemini_backend;
pub mod openai_backend;
pub mod persistence;
pub mod print_charter;
pub mod run;
pub mod scenario_suite;

pub use agent::{
    Agent, AgentBuildError, AgentEvent, AgentOutcome, Brief, EscalationCause, RunArtifacts,
    RunPaths, RunResult,
};
pub use scenario_suite::{
    CellAggregate, ExpectedOutcome, ScenarioEntry, ScenarioReport, SuiteError, SuiteReport,
};
