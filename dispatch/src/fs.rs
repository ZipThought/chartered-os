//! Native filesystem ToolExecutors. Both implementations are scoped to
//! a workspace root and reject any path that resolves outside it.
//!
//! Containment is the central security invariant: every resolved path
//! must canonicalize within the workspace root. The shared
//! `ensure_within_root` predicate is the SSOT for that check —
//! divergence between read-side and write-side containment would be a
//! sandbox-escape bug.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chartered_core::{ToolExecutor, ToolId, ToolParams, ToolResult};

/// `read_file(path)` — reads a UTF-8 file inside the workspace root.
pub struct NativeFsRead {
    id: ToolId,
    workspace_root: PathBuf,
}

impl NativeFsRead {
    /// `workspace_root` is canonicalized once at construction. Returns
    /// `Err` if the root doesn't exist or cannot be canonicalized —
    /// silent fallback would let containment checks operate on the
    /// non-canonical (symlink-permissive) form.
    pub fn new(id: ToolId, workspace_root: impl Into<PathBuf>) -> std::io::Result<Self> {
        Ok(Self {
            id,
            workspace_root: workspace_root.into().canonicalize()?,
        })
    }
}

#[async_trait]
impl ToolExecutor for NativeFsRead {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        let raw = params.require_str("path")?;
        let resolved = resolve_existing(&self.workspace_root, raw)?;
        let content = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| format!("read failed for {}: {e}", resolved.display()))?;
        Ok(serde_json::json!({
            "path": raw,
            "content": content,
        }))
    }
}

/// `write_file(path, content)` — writes a UTF-8 file inside the
/// workspace root. Creates parent directories if missing (within root).
pub struct NativeFsWrite {
    id: ToolId,
    workspace_root: PathBuf,
}

impl NativeFsWrite {
    /// `workspace_root` is canonicalized once at construction. Returns
    /// `Err` if the root doesn't exist or cannot be canonicalized.
    pub fn new(id: ToolId, workspace_root: impl Into<PathBuf>) -> std::io::Result<Self> {
        Ok(Self {
            id,
            workspace_root: workspace_root.into().canonicalize()?,
        })
    }
}

#[async_trait]
impl ToolExecutor for NativeFsWrite {
    fn id(&self) -> &ToolId {
        &self.id
    }

    async fn execute(&self, params: &ToolParams) -> ToolResult {
        let raw_path = params.require_str("path")?;
        let content = params.require_str("content")?;
        let resolved = resolve_for_write(&self.workspace_root, raw_path)?;
        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("creating parent {}: {e}", parent.display()))?;
        }
        tokio::fs::write(&resolved, content)
            .await
            .map_err(|e| format!("write failed for {}: {e}", resolved.display()))?;
        Ok(serde_json::json!({
            "wrote": raw_path,
            "bytes": content.len(),
        }))
    }
}

/// Build the candidate path: requested-as-given when absolute, joined
/// against `root` when relative. No canonicalization or existence check.
pub(crate) fn candidate_path(root: &Path, requested: &str) -> PathBuf {
    let req = Path::new(requested);
    if req.is_absolute() {
        req.to_path_buf()
    } else {
        root.join(req)
    }
}

/// SSOT containment predicate: returns Ok iff `canonical` lies within
/// `root_canonical`. `root_canonical` MUST already be canonical (callers
/// canonicalize once at construction). Symlink escapes are rejected
/// because canonicalization follows symlinks before the comparison.
pub(crate) fn ensure_within_root(canonical: &Path, root_canonical: &Path) -> Result<(), String> {
    if canonical.starts_with(root_canonical) {
        Ok(())
    } else {
        Err(format!(
            "path {} escapes workspace root {}",
            canonical.display(),
            root_canonical.display()
        ))
    }
}

/// Resolve a path that must already exist and lie within `root`.
pub(crate) fn resolve_existing(root_canonical: &Path, requested: &str) -> Result<PathBuf, String> {
    let candidate = candidate_path(root_canonical, requested);
    let canonical = candidate
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", candidate.display()))?;
    ensure_within_root(&canonical, root_canonical)?;
    Ok(canonical)
}

/// Resolve a path for writing: the file may not exist yet, but its
/// parent directory must resolve within `root` after canonicalization.
pub(crate) fn resolve_for_write(root_canonical: &Path, requested: &str) -> Result<PathBuf, String> {
    let candidate = candidate_path(root_canonical, requested);
    let parent = candidate
        .parent()
        .ok_or_else(|| format!("path {} has no parent", candidate.display()))?;
    // The parent might not exist yet; walk up to the deepest existing
    // ancestor and canonicalize from there.
    let (existing, suffix) = first_existing_ancestor(parent);
    let existing_canonical = existing
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", existing.display()))?;
    ensure_within_root(&existing_canonical, root_canonical)?;
    let mut full = existing_canonical;
    full.push(suffix);
    full.push(candidate.file_name().ok_or("no file name")?);
    Ok(full)
}

fn first_existing_ancestor(p: &Path) -> (PathBuf, PathBuf) {
    let mut current = p.to_path_buf();
    let mut suffix = PathBuf::new();
    loop {
        if current.exists() {
            return (current, suffix);
        }
        let name = match current.file_name() {
            Some(n) => n.to_owned(),
            None => return (current, suffix),
        };
        if !current.pop() {
            return (current, suffix);
        }
        suffix = {
            let mut s = PathBuf::from(name);
            s.push(suffix);
            s
        };
    }
}
