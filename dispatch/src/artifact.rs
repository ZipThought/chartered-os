//! ArtifactBackend implementations bridging to the local filesystem.
//!
//! Two reference Backends:
//!
//! - `FilesystemTextBackend` (`kind=text`) — workspace-relative `.md`/`.txt`
//!   files; reads return content (optionally sliced by Selector range);
//!   modifies splice content at a byte range under a containment-checked
//!   path.
//! - `FilesystemRecordStore` (`kind=record-store`) — backed by
//!   `<chartered_dir>/<artifact_id>.jsonl`; reads return the filtered
//!   list (optionally narrowed by a Selector `filter`); modifies append
//!   one Record with kernel-injected provenance.
//!
//! Both Backends are constructed with the canonical workspace root and
//! reject any path that resolves outside it (after canonicalization, so
//! symlinks cannot escape).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chartered_core::{
    Artifact, ArtifactBackend, ArtifactId, ArtifactKindId, Edit, Projection, Record, Selector,
    apply_text_edit, kind_record_store, kind_text, parse_record_append, parse_record_filter,
    parse_text_edit, parse_text_range_from_selector, slice_range,
};

use crate::fs::{ensure_within_root, resolve_existing, resolve_for_write};
use crate::paths::DeploymentPaths;

// ============================================================
// kind=text — workspace text files
// ============================================================

pub struct FilesystemTextBackend {
    kind: ArtifactKindId,
    workspace_root: PathBuf,
}

impl FilesystemTextBackend {
    /// Built from the central `DeploymentPaths`. Reads
    /// `paths.workspace_root()`.
    pub fn new(paths: &DeploymentPaths) -> Self {
        Self {
            kind: kind_text(),
            workspace_root: paths.workspace_root().to_path_buf(),
        }
    }
}

#[async_trait]
impl ArtifactBackend for FilesystemTextBackend {
    fn kind(&self) -> &ArtifactKindId {
        &self.kind
    }

    fn list(&self) -> Vec<Artifact> {
        let mut paths = Vec::new();
        if collect_artifacts(&self.workspace_root, &self.workspace_root, &mut paths).is_err() {
            return Vec::new();
        }
        paths.sort();
        let kind = self.kind.clone();
        paths
            .into_iter()
            .map(|id| Artifact {
                id: ArtifactId::new(id),
                kind: kind.clone(),
            })
            .collect()
    }

    async fn read(
        &self,
        artifact_id: &ArtifactId,
        selector: &Selector,
    ) -> Result<Projection, String> {
        let resolved = resolve_existing(&self.workspace_root, &artifact_id.0)?;
        let full_content = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| format!("read failed for {}: {e}", resolved.display()))?;
        let range = parse_text_range_from_selector(selector)?;
        let body = match range {
            Some(r) => slice_range(&full_content, r)?.to_string(),
            None => full_content,
        };
        Ok(Projection(serde_json::json!({
            "artifact_id": artifact_id,
            "content": body,
            "range": range,
        })))
    }

    async fn modify(&self, artifact_id: &ArtifactId, edit: &Edit) -> Result<Projection, String> {
        let (range, replacement) = parse_text_edit(edit)?;
        let resolved = resolve_for_write(&self.workspace_root, &artifact_id.0)?;
        let content = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| format!("read failed for {}: {e}", resolved.display()))?;
        let next = apply_text_edit(&content, range, &replacement)?;
        tokio::fs::write(&resolved, next)
            .await
            .map_err(|e| format!("write failed for {}: {e}", resolved.display()))?;
        Ok(Projection(serde_json::json!({
            "artifact_id": artifact_id,
            "range": range,
            "applied_text": replacement,
        })))
    }
}

// ============================================================
// kind=record-store — `.chartered/<artifact_id>.jsonl`
// ============================================================

/// `kind=record-store` Backend backed by a JSONL file under
/// `<chartered_dir>/<artifact_id>.jsonl`. Exposes one artifact whose id
/// the constructor takes; `read` returns the filtered list of records
/// (each line one JSON object), `modify` appends one Record (durably
/// synced before returning). The kernel injects runtime provenance into
/// the Edit; the Steward-supplied content fields are persisted at the
/// top level alongside provenance for uniform filtering.
pub struct FilesystemRecordStore {
    kind: ArtifactKindId,
    artifact_id: ArtifactId,
    file_path: PathBuf,
    sink: crate::persistence::JsonlSink,
}

impl FilesystemRecordStore {
    /// Built from the central `DeploymentPaths`. The store's file lives
    /// at `paths.chartered_dir()/<artifact_id>.jsonl` so deployments
    /// that point `--workspace-root` and `--chartered-dir` at
    /// independent paths still produce a single record stream per
    /// artifact id. Each ArtifactId gets its own file; multiple
    /// record-store Backends can coexist in one deployment.
    pub async fn new(
        paths: &DeploymentPaths,
        artifact_id: ArtifactId,
    ) -> std::io::Result<Self> {
        let file_path = paths.chartered_dir().join(format!("{artifact_id}.jsonl"));
        let sink = crate::persistence::JsonlSink::create(&file_path).await?;
        Ok(Self {
            kind: kind_record_store(),
            artifact_id,
            file_path,
            sink,
        })
    }
}

#[async_trait]
impl ArtifactBackend for FilesystemRecordStore {
    fn kind(&self) -> &ArtifactKindId {
        &self.kind
    }

    fn list(&self) -> Vec<Artifact> {
        vec![Artifact {
            id: self.artifact_id.clone(),
            kind: self.kind.clone(),
        }]
    }

    async fn read(
        &self,
        artifact_id: &ArtifactId,
        selector: &Selector,
    ) -> Result<Projection, String> {
        if artifact_id != &self.artifact_id {
            return Err(format!(
                "record-store has no artifact `{artifact_id}` (only `{}` exists)",
                self.artifact_id
            ));
        }
        let filter = parse_record_filter(selector)?;
        let raw = match tokio::fs::read_to_string(&self.file_path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(format!(
                    "read failed for {}: {e}",
                    self.file_path.display()
                ));
            }
        };
        let mut records: Vec<serde_json::Value> = Vec::new();
        for (lineno, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                format!(
                    "parse failed for {} line {}: {e}",
                    self.file_path.display(),
                    lineno + 1
                )
            })?;
            if filter.matches_value(&v) {
                records.push(v);
            }
        }
        Ok(Projection(serde_json::json!({
            "records": records,
        })))
    }

    async fn modify(&self, artifact_id: &ArtifactId, edit: &Edit) -> Result<Projection, String> {
        if artifact_id != &self.artifact_id {
            return Err(format!(
                "record-store has no artifact `{artifact_id}` (only `{}` exists)",
                self.artifact_id
            ));
        }
        let append = parse_record_append(edit)?;
        // Record IDs are derived from the admitting receipt — durable
        // and unique across runs without an in-process counter.
        let record = Record {
            id: format!("record-{}", append.receipt_id),
            task_id: append.task_id,
            steward_id: append.steward_id,
            snapshot_id: append.snapshot_id,
            receipt_id: append.receipt_id,
            content: append.content,
        };
        let record_id = record.id.clone();
        self.sink
            .append(&record)
            .await
            .map_err(|e| format!("append record {record_id} to {}: {e}", self.file_path.display()))?;
        Ok(Projection(serde_json::json!({ "record_id": record_id })))
    }
}

fn collect_artifacts(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("read_dir entry {}: {e}", dir.display()))?;
        let path = entry.path();
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("canonicalize {}: {e}", path.display()))?;
        ensure_within_root(&canonical, root)?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name == ".chartered")
        {
            continue;
        }
        if canonical.is_dir() {
            collect_artifacts(root, &canonical, out)?;
        } else if is_plain_text_artifact(&canonical) {
            let relative = canonical
                .strip_prefix(root)
                .map_err(|e| format!("strip root from {}: {e}", canonical.display()))?;
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

fn is_plain_text_artifact(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("markdown") | Some("txt")
    )
}
