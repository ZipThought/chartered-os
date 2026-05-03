//! Single entry point for the deployment-time directory layout.
//!
//! `DeploymentPaths` is the only place where the runtime's directory
//! conventions live. Backends, the executor registry, and any future
//! OS-touching component take `&DeploymentPaths` rather than reaching
//! for individual `PathBuf`s. New directory dimensions (per-run dir,
//! per-Backend config dir, per-Steward state dir, …) extend this
//! struct rather than rippling through every constructor signature.
//!
//! Canonicalization happens once, here. Downstream users get paths
//! that are already absolute and symlink-resolved.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DeploymentPaths {
    /// User-facing artifact universe (workspace files for `kind=text`).
    /// Containment checks anchor on this canonical root.
    pub workspace_root: PathBuf,
    /// Deployment configuration + audit state (`charter/`, `tools/` /
    /// `backends/`, `runs/<id>/{receipts,cognition}.jsonl`,
    /// `findings.jsonl`, `role_context.md`). Distinct from
    /// `workspace_root`: deployments may point them at independent paths.
    pub chartered_dir: PathBuf,
}

impl DeploymentPaths {
    /// Build paths from raw inputs. Canonicalizes both — fails if either
    /// directory does not exist on disk.
    pub fn canonicalize(
        workspace_root: impl Into<PathBuf>,
        chartered_dir: impl Into<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            workspace_root: workspace_root.into().canonicalize()?,
            chartered_dir: chartered_dir.into().canonicalize()?,
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn chartered_dir(&self) -> &Path {
        &self.chartered_dir
    }
}
