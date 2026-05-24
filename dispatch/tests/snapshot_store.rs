//! Integration tests for the Snapshot manifest store. These exercise
//! the disk-touching surface (`persist`, `list_persisted_ids`, `prune`)
//! with folder-isolated tempdirs per run. They live under `tests/`
//! rather than in `#[cfg(test)] mod tests` because they are not unit
//! tests by `AGENTS.md §Verification` — unit tests are literally
//! stateless.

use std::collections::HashSet;
use std::sync::Arc;

use chartered_core::{Charter, RoleContext, Snapshot, SnapshotId};
use chartered_dispatch::snapshot_store::{
    list_persisted_ids, manifest_path, persist, prune, SnapshotManifest,
};

fn mk_snapshot(charter_hash: &str, role_hash: &str) -> Arc<Snapshot> {
    let charter = Charter {
        frames: Vec::new(),
        permitted_tools: Vec::new(),
        charter_scopes: Vec::new(),
        behavioral_spec: String::new(),
        charter_version: 1,
        charter_content_hash: charter_hash.to_string(),
    };
    let role_context = RoleContext {
        scopes: Vec::new(),
        role_context_version: 1,
        role_context_content_hash: role_hash.to_string(),
    };
    Snapshot::new(charter, role_context, Vec::new())
}

#[tokio::test]
async fn persist_and_list_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = mk_snapshot("c-hash-1", "r-hash-1");

    persist(&snapshot, dir.path()).await.unwrap();

    let path = manifest_path(dir.path(), &snapshot.id);
    assert!(path.exists(), "manifest file written");

    let ids = list_persisted_ids(dir.path()).await.unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], snapshot.id);

    let bytes = tokio::fs::read(&path).await.unwrap();
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(manifest.id, snapshot.id);
    assert_eq!(manifest.charter_content_hash, "c-hash-1");
    assert_eq!(manifest.role_context_content_hash, "r-hash-1");
}

#[tokio::test]
async fn prune_deletes_unreferenced_keeps_retained() {
    let dir = tempfile::tempdir().unwrap();
    let keep = mk_snapshot("c-keep", "r-keep");
    let drop = mk_snapshot("c-drop", "r-drop");
    persist(&keep, dir.path()).await.unwrap();
    persist(&drop, dir.path()).await.unwrap();

    let retain: HashSet<SnapshotId> = [keep.id.clone()].into_iter().collect();
    let deleted = prune(dir.path(), &retain).await.unwrap();
    assert_eq!(deleted, 1);

    let remaining = list_persisted_ids(dir.path()).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0], keep.id);
}

#[tokio::test]
async fn list_on_missing_dir_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("snapshots");
    let ids = list_persisted_ids(&nonexistent).await.unwrap();
    assert!(ids.is_empty());
}
