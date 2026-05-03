use std::sync::Arc;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::charter::{Charter, RoleContext};

#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Serialize)]
pub struct SnapshotId(pub String);

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Frozen Charter and Role context at Task creation. Spec §Vocabulary >
/// Snapshot, §Receipts.
///
/// Identity is content-addressed: identical content yields identical
/// SnapshotId. The freeze timestamp lives in `frozen_at`, never in
/// the identity hash.
pub struct Snapshot {
    pub id: SnapshotId,
    pub charter: Charter,
    pub role_context: RoleContext,
    pub frozen_at: SystemTime,
}

impl Snapshot {
    pub fn new(charter: Charter, role_context: RoleContext) -> Arc<Self> {
        let mut hasher = Sha256::new();
        hasher.update(charter.charter_content_hash.as_bytes());
        hasher.update(b":");
        hasher.update(role_context.role_context_content_hash.as_bytes());
        let id = SnapshotId(hex::encode(hasher.finalize()));
        Arc::new(Self {
            id,
            charter,
            role_context,
            frozen_at: SystemTime::now(),
        })
    }
}
