use std::sync::Arc;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::charter::{Charter, RoleContext};
use crate::skill::{Skill, skills_content_hash};

#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotId(pub String);

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Frozen Charter, Role context, and Skills at Task creation. Spec
/// §Vocabulary > Snapshot, §Snapshot Lifecycle, §Skills.
///
/// Identity is content-addressed over all three: identical Charter +
/// Role-context + Skills content yields the identical SnapshotId. The
/// freeze timestamp lives in `frozen_at`, never in the identity hash.
pub struct Snapshot {
    pub id: SnapshotId,
    pub charter: Charter,
    pub role_context: RoleContext,
    pub skills: Vec<Skill>,
    pub frozen_at: SystemTime,
}

impl Snapshot {
    /// Construct a Snapshot from Charter, Role context, and Skills.
    /// The Snapshot ID is content-addressed over all three; pass
    /// `Vec::new()` for skills when none are bound.
    pub fn new(
        charter: Charter,
        role_context: RoleContext,
        skills: Vec<Skill>,
    ) -> Arc<Self> {
        let mut hasher = Sha256::new();
        hasher.update(charter.charter_content_hash.as_bytes());
        hasher.update(b":");
        hasher.update(role_context.role_context_content_hash.as_bytes());
        hasher.update(b":");
        hasher.update(skills_content_hash(&skills).as_bytes());
        let id = SnapshotId(hex::encode(hasher.finalize()));
        Arc::new(Self {
            id,
            charter,
            role_context,
            skills,
            frozen_at: SystemTime::now(),
        })
    }
}
