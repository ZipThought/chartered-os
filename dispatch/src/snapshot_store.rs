//! Snapshot persistence — one immutable file per Snapshot, keyed by ID.
//!
//! Spec §Snapshot Lifecycle: "Snapshots are content-addressed and
//! persist via the unified primitive. Stable references: the Snapshot
//! ID embedded in every Receipt resolves to a persisted Snapshot
//! record. Append-only: new Snapshots are added; existing Snapshots
//! are not mutated. Pruning: old versions are trimmed when no longer
//! referenced."
//!
//! Layout: `<chartered_dir>/snapshots/<snapshot_id>.json` per
//! Snapshot. Single-shot writes (not appends) — each file is the whole
//! manifest for one Snapshot, content-immutable for the Snapshot's
//! lifetime. Deletion is the prune operation.
//!
//! `SnapshotManifest` carries the durable identity (id, frozen-at,
//! Charter and Role-context versions and content hashes). Full
//! Snapshot reconstruction from a manifest requires the matching
//! Charter and Role-context source materials and is left to higher
//! layers; this module owns the manifest write/read/list/prune
//! lifecycle.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chartered_core::{Snapshot, SnapshotId, skills_content_hash};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

/// Durable identity record for one Snapshot. Content-addressed by `id`;
/// the file path is derived from `id` so lookup is O(1) without a
/// directory scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub id: SnapshotId,
    pub frozen_at_unix_nanos: u128,
    pub charter_version: u64,
    pub charter_content_hash: String,
    pub role_context_version: u64,
    pub role_context_content_hash: String,
    /// Aggregate content hash over the Skills bound to this Snapshot
    /// (sha256 over sorted `[id, content_hash]` pairs). Empty Skills
    /// hash to the sha256 of the empty input so a Skill-less Snapshot
    /// still has a defined value.
    pub skills_content_hash: String,
}

impl SnapshotManifest {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        let frozen_at_unix_nanos = snapshot
            .frozen_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Self {
            id: snapshot.id.clone(),
            frozen_at_unix_nanos,
            charter_version: snapshot.charter.charter_version,
            charter_content_hash: snapshot.charter.charter_content_hash.clone(),
            role_context_version: snapshot.role_context.role_context_version,
            role_context_content_hash: snapshot.role_context.role_context_content_hash.clone(),
            skills_content_hash: skills_content_hash(&snapshot.skills),
        }
    }
}

/// Resolve the on-disk path for a Snapshot ID under `dir`.
pub fn manifest_path(dir: &Path, id: &SnapshotId) -> PathBuf {
    dir.join(format!("{}.json", id.0))
}

/// Persist a Snapshot manifest as `<dir>/<id>.json`. Idempotent — the
/// content-addressed ID means a re-persist of an unchanged Snapshot
/// overwrites with identical bytes. Write+fsync discipline matches
/// `JsonlSink` (durable before return; serialization failure
/// surfaces).
pub async fn persist(snapshot: &Snapshot, dir: &Path) -> std::io::Result<()> {
    let manifest = SnapshotManifest::from_snapshot(snapshot);
    let bytes = serde_json::to_vec(&manifest).map_err(|e| {
        eprintln!(
            "snapshot_store::persist({}): serialize failure: {e}",
            snapshot.id
        );
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    })?;
    tokio::fs::create_dir_all(dir).await?;
    let path = manifest_path(dir, &snapshot.id);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .await?;
    file.write_all(&bytes).await?;
    file.sync_data().await?;
    Ok(())
}

/// Enumerate persisted Snapshot IDs under `dir`. Reads no manifest
/// content — only filenames. Missing `dir` returns an empty list (no
/// snapshots persisted yet is not an error).
pub async fn list_persisted_ids(dir: &Path) -> std::io::Result<Vec<SnapshotId>> {
    let mut ids = Vec::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
        Err(e) => return Err(e),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            ids.push(SnapshotId(stem.to_string()));
        }
    }
    Ok(ids)
}

/// Delete persisted Snapshot manifests under `dir` whose IDs are not in
/// `retain`. Returns the count of deleted manifests.
///
/// Caller composes `retain` from the live reference set — in-flight
/// Tasks' pinned Snapshots and Receipts within the audit window. This
/// function does not enforce any retention policy; it executes the
/// caller's decision.
pub async fn prune(dir: &Path, retain: &HashSet<SnapshotId>) -> std::io::Result<usize> {
    let ids = list_persisted_ids(dir).await?;
    let mut deleted = 0;
    for id in ids {
        if retain.contains(&id) {
            continue;
        }
        let path = manifest_path(dir, &id);
        tokio::fs::remove_file(&path).await?;
        deleted += 1;
    }
    Ok(deleted)
}

