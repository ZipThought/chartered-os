//! Runtime library: deployment-config loading and the single execution
//! path that drives the loop. Spec §The Runtime.
//!
//! ONE binary entry. ONE loader. ONE orchestrator. The fake/real swap
//! lives at the `CognitionBackend` (per-role config in `steward.toml`)
//! and at the `ToolExecutor` registry (per-tool config in
//! `tools/*.toml`). Nothing else differs between a test deployment
//! and a production deployment.

pub mod canonicalize;
pub mod charter_loader;
pub mod config;
pub mod gemini_backend;
pub mod openai_backend;
pub mod persistence;
pub mod print_charter;
pub mod run;
