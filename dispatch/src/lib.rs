//! Dispatcher crate: OS-touching `ToolExecutor` implementations for the
//! CharteredOS Runtime. Spec §Tools (executor kinds: in-Runtime native,
//! peer process, contained subprocess) — this crate provides the
//! in-Runtime-native ones that touch the operating system.
//!
//! Quarantine: `std::fs`, `std::process`, `tokio::fs`, `tokio::process`
//! are confined to this crate. The kernel (`chartered-core`) stays
//! in-memory. CHECKLIST §Tool Registry Is the Only Path: every effect
//! flows through a registered executor.
//!
//! All executors are scoped to a workspace root and reject any path
//! that resolves outside it (after canonicalization, so symlinks
//! cannot escape). Path traversal denials surface as `ToolResult::Err`.

pub mod artifact;
pub mod exec;
pub mod fs;
pub mod paths;
pub mod registry;

pub use artifact::{FilesystemFindingsBackend, FilesystemTextBackend};
pub use exec::NativeExec;
pub use fs::{NativeFsRead, NativeFsWrite};
pub use paths::DeploymentPaths;
pub use registry::{ExecutorBuildError, ExecutorRegistry};
