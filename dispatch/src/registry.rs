//! Map deployment-config `executor` strings (e.g., `"native_fs_read"`)
//! to concrete `ToolExecutor` instances keyed by the deployment's
//! `tool_id`.
//!
//! Open/Closed via registry: adding a new executor is one
//! `register(name, builder)` call, never an edit to a match ladder.
//! Native built-ins are pre-registered by `with_native_defaults`.
//!
//! **Single config entry point.** `DeploymentPaths` is built once by
//! the runtime and threaded into the registry. Every Backend and every
//! native executor receives `&DeploymentPaths` rather than reaching
//! for individual paths — new directory dimensions extend the struct
//! without rippling through constructor signatures.
//!
//! **Artifact-substrate executors** (`artifact_read`, `artifact_modify`,
//! `artifact_list`, `record_finding`) share a single `ArtifactStore`
//! constructed at `with_native_defaults` time, populated with the
//! deployment's filesystem-backed Backends (`FilesystemTextBackend`
//! for `kind=text`, `FilesystemFindingsBackend` for `kind=findings-store`).
//! All four executors dispatch through that store, which routes by kind
//! to the owning Backend. The Tool surface is fixed; substrate
//! variation lives in Backends.

use std::collections::HashMap;
use std::sync::Arc;

use chartered_core::{
    ArtifactStore, ListArtifacts, ModifyArtifact, ReadArtifact, RecordFinding, ToolExecutor, ToolId,
};

use crate::paths::DeploymentPaths;
use crate::{
    FilesystemFindingsBackend, FilesystemTextBackend, NativeExec, NativeFsRead, NativeFsWrite,
};

#[derive(Debug, Clone)]
pub enum ExecutorBuildError {
    UnknownExecutor(String),
    BuilderFailed { name: String, reason: String },
}

impl std::fmt::Display for ExecutorBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorBuildError::UnknownExecutor(n) => {
                write!(f, "unknown executor `{n}`")
            }
            ExecutorBuildError::BuilderFailed { name, reason } => {
                write!(f, "builder for `{name}` failed: {reason}")
            }
        }
    }
}

impl std::error::Error for ExecutorBuildError {}

/// Builder closure: receives the deployment's `&DeploymentPaths`, the
/// per-deployment tool id, and the registry's shared `ArtifactStore`.
/// Builders that don't need the store ignore it; artifact-substrate
/// builders use it to construct kernel-resident `ToolExecutor`s wired
/// to the deployment's Backends.
pub type ExecutorBuilder = Arc<
    dyn Fn(&DeploymentPaths, &ToolId, &Arc<ArtifactStore>) -> Result<Arc<dyn ToolExecutor>, String>
        + Send
        + Sync,
>;

/// Maps executor names (from `tools/*.toml` `executor = "..."`) to
/// builders. New executors register their builder rather than modifying
/// a central match. Holds the deployment's central path config.
pub struct ExecutorRegistry {
    paths: DeploymentPaths,
    artifact_store: Arc<ArtifactStore>,
    builders: HashMap<&'static str, ExecutorBuilder>,
}

impl ExecutorRegistry {
    /// Construct a registry pre-populated with the native built-ins
    /// (`native_fs_read`, `native_fs_write`, `native_exec`, and the
    /// artifact-substrate executors backed by filesystem Backends).
    /// Takes ownership of the central `DeploymentPaths`.
    pub fn new(paths: DeploymentPaths) -> Self {
        Self::empty(paths).with_native_defaults()
    }

    /// Construct a registry without the native built-ins. Tests that
    /// want to verify `unknown executor` errors against an empty
    /// registry use this. The artifact substrate is empty (no Backends);
    /// callers that intend to build artifact executors must register
    /// Backends and re-bind the store before building.
    pub fn empty(paths: DeploymentPaths) -> Self {
        Self {
            paths,
            artifact_store: Arc::new(ArtifactStore::new()),
            builders: HashMap::new(),
        }
    }

    /// Pre-populate the artifact substrate with the two reference
    /// filesystem Backends (`kind=text` against
    /// `paths.workspace_root()`, `kind=findings-store` against
    /// `paths.chartered_dir()`) and register the corresponding executor
    /// builders.
    pub fn with_native_defaults(mut self) -> Self {
        let text_backend = Arc::new(FilesystemTextBackend::new(&self.paths));
        let findings_backend = Arc::new(FilesystemFindingsBackend::new(&self.paths));
        // ArtifactStore registers Backends keyed by ArtifactKindId; one
        // Backend per kind is enforced at registration. Tool calls carry
        // `kind` as a first-class param and dispatch lands deterministically
        // — no fall-through, no ownership heuristic.
        let store = Arc::new(
            ArtifactStore::new()
                .with_backend(findings_backend)
                .with_backend(text_backend),
        );
        self.artifact_store = store;

        self.register("native_fs_read", |paths, id, _store| {
            NativeFsRead::new(id.clone(), paths.workspace_root().to_path_buf())
                .map(|e| Arc::new(e) as Arc<dyn ToolExecutor>)
                .map_err(|e| e.to_string())
        });
        self.register("native_fs_write", |paths, id, _store| {
            NativeFsWrite::new(id.clone(), paths.workspace_root().to_path_buf())
                .map(|e| Arc::new(e) as Arc<dyn ToolExecutor>)
                .map_err(|e| e.to_string())
        });
        self.register("native_exec", |paths, id, _store| {
            Ok(Arc::new(NativeExec::new(
                id.clone(),
                paths.workspace_root().to_path_buf(),
            )))
        });

        // Artifact-substrate executors share the registry's ArtifactStore.
        // The names retain the `native_artifact_*` prefix for backward
        // compatibility with existing tools/*.toml files; semantically,
        // these are the kernel's substrate-blind executors wired to the
        // filesystem Backends.
        self.register("native_artifact_read", |_paths, id, store| {
            Ok(Arc::new(ReadArtifact::new(id.clone(), store.clone())))
        });
        self.register("native_artifact_modify", |_paths, id, store| {
            Ok(Arc::new(ModifyArtifact::new(id.clone(), store.clone())))
        });
        self.register("native_artifact_list", |_paths, id, store| {
            Ok(Arc::new(ListArtifacts::new(id.clone(), store.clone())))
        });
        self.register("native_artifact_record_finding", |_paths, id, store| {
            Ok(Arc::new(RecordFinding::new(id.clone(), store.clone())))
        });

        self
    }

    /// Register a builder for one executor name. Later registrations
    /// shadow earlier ones — useful for tests that want to substitute
    /// a stub for a native executor.
    pub fn register<F>(&mut self, name: &'static str, builder: F) -> &mut Self
    where
        F: Fn(
                &DeploymentPaths,
                &ToolId,
                &Arc<ArtifactStore>,
            ) -> Result<Arc<dyn ToolExecutor>, String>
            + Send
            + Sync
            + 'static,
    {
        self.builders.insert(name, Arc::new(builder));
        self
    }

    pub fn build(
        &self,
        executor_name: &str,
        tool_id: &ToolId,
    ) -> Result<Arc<dyn ToolExecutor>, ExecutorBuildError> {
        let builder = self
            .builders
            .get(executor_name)
            .ok_or_else(|| ExecutorBuildError::UnknownExecutor(executor_name.to_string()))?;
        builder(&self.paths, tool_id, &self.artifact_store).map_err(|reason| {
            ExecutorBuildError::BuilderFailed {
                name: executor_name.to_string(),
                reason,
            }
        })
    }

    /// Access the registry's central `DeploymentPaths`. Callers that
    /// need to read deployment paths outside the Tool dispatch path
    /// use this handle (single source of truth, no parallel copies).
    pub fn paths(&self) -> &DeploymentPaths {
        &self.paths
    }

    /// Access the registry's shared `ArtifactStore`. Callers that operate
    /// on artifacts outside the Tool dispatch path (e.g., the dashboard
    /// host enumerating workspace artifacts) use this handle.
    pub fn artifact_store(&self) -> Arc<ArtifactStore> {
        self.artifact_store.clone()
    }
}
