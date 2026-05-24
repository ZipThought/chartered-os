//! Skills: Actor-side cognition instrumentation.
//!
//! Spec §Skills: "Actor-side cognition instrumentation following the
//! SKILL.md convention. The Actor consults Skills during cognition; any
//! tool call produced under a Skill's guidance crosses the Gate. Skills
//! do not constitute a new Tool category and do not bypass the
//! Charter's `permitted_tools`."
//!
//! A Skill is operator-curated text (typically markdown). It enters the
//! Actor's system prompt as a labeled section. It carries no authority
//! to expand the permitted-tools set or to bypass Frame evaluation —
//! every tool call the Actor emits under a Skill's influence still
//! crosses the Gate.

use sha2::{Digest, Sha256};

/// One Skill — a named, content-hashed piece of operator-curated text.
/// Identity is `id`; `content_hash` detects drift between source file
/// and loaded content. Skills are unversioned by design — drift surfaces
/// as a changed content_hash, which propagates into a new Snapshot ID
/// (see *Snapshot Lifecycle*).
#[derive(Debug, Clone)]
pub struct Skill {
    pub id: String,
    pub content: String,
    pub content_hash: String,
}

impl Skill {
    /// Construct a Skill from raw content. `content_hash` is computed
    /// from the content; callers MUST NOT pre-compute it (single source
    /// of truth for the hash function).
    pub fn new(id: impl Into<String>, content: impl Into<String>) -> Self {
        let id = id.into();
        let content = content.into();
        let content_hash = hash_content(&content);
        Self {
            id,
            content,
            content_hash,
        }
    }
}

/// Single source of truth for the Skill content-hash function. Shared
/// across construction (`Skill::new`) and aggregate hashing
/// (`skills_content_hash`).
fn hash_content(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    hex::encode(h.finalize())
}

/// Combined content hash over a set of Skills, used in Snapshot
/// identity composition and in the persisted SnapshotManifest. Order-
/// stable: callers MUST sort by Skill id before hashing so the same
/// set of Skills yields the same hash regardless of load order.
pub fn skills_content_hash(skills: &[Skill]) -> String {
    let mut sorted: Vec<&Skill> = skills.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut h = Sha256::new();
    for s in sorted {
        h.update(s.id.as_bytes());
        h.update(b":");
        h.update(s.content_hash.as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_content_addressed() {
        let a = Skill::new("s1", "hello");
        let b = Skill::new("s1", "hello");
        assert_eq!(a.content_hash, b.content_hash);
        let c = Skill::new("s1", "hello!");
        assert_ne!(a.content_hash, c.content_hash);
    }

    #[test]
    fn aggregate_hash_is_order_independent() {
        let s1 = Skill::new("billing", "rules");
        let s2 = Skill::new("triage", "steps");
        let h_ab = skills_content_hash(&[s1.clone(), s2.clone()]);
        let h_ba = skills_content_hash(&[s2, s1]);
        assert_eq!(h_ab, h_ba);
    }

    #[test]
    fn aggregate_hash_changes_with_content() {
        let h1 = skills_content_hash(&[Skill::new("s", "v1")]);
        let h2 = skills_content_hash(&[Skill::new("s", "v2")]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn aggregate_hash_of_empty_is_stable() {
        let h = skills_content_hash(&[]);
        assert_eq!(h.len(), 64); // sha256 hex
    }
}
