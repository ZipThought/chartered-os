//! Steward: the chartered-resident agent. Spec §Vocabulary defines a
//! Steward as "a chartered-resident agent: built for the Runtime,
//! Charter-bound, behavioral-spec-shaped. The framework's deliverable."
//!
//! The Steward is the binding scope for {Charter, Snapshot, Tool
//! registry, governance mode}. Multiple Stewards may coexist within one
//! Workspace (spec §The Runtime: "Steward instances" plural). Each
//! Steward has its own Charter (spec §Vocabulary: Tester "operates as
//! a Steward under its own Charter"), so per-Steward fields are kernel
//! concerns, not deployment-side configuration.

use std::sync::Arc;

use crate::governance::GovernanceMode;
use crate::prompt::assemble_actor_system_prompt;
use crate::snapshot::Snapshot;
use crate::tool::ToolRegistry;

/// Identifies a Steward within a Workspace. Two Stewards in the same
/// Workspace MUST have distinct ids; the Workspace constructor enforces
/// uniqueness.
#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StewardId(pub String);

impl StewardId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for StewardId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A chartered-resident agent. Owns the per-Steward state that the spec
/// defines as "under its own Charter": the Snapshot derived from this
/// Steward's Charter and Role context (sourced from the Snapshot), the
/// ToolRegistry whose `permitted_tools` match this Steward's Charter,
/// and the per-Steward `GovernanceMode`.
///
/// `system_prompt(&self)` returns the Runtime-assembled Actor system
/// prompt per §Cognition Layer block diagram: behavioral spec from the
/// Charter, plus Charter Scopes and Role context Scopes when grounding
/// is on.
pub struct Steward {
    pub id: StewardId,
    pub snapshot: Arc<Snapshot>,
    pub tool_registry: Arc<ToolRegistry>,
    pub governance_mode: GovernanceMode,
}

impl Steward {
    pub fn new(
        id: StewardId,
        snapshot: Arc<Snapshot>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            id,
            snapshot,
            tool_registry,
            governance_mode: GovernanceMode::FULL,
        }
    }

    pub fn with_governance_mode(mut self, mode: GovernanceMode) -> Self {
        self.governance_mode = mode;
        self
    }

    /// Compose the Actor's system prompt from this Steward's Charter
    /// components. Prompt content is stable across calls (Charter +
    /// RoleContext + governance_mode are immutable for this Steward),
    /// so the result is suitable for prefix-cache storage.
    pub fn system_prompt(&self) -> String {
        assemble_actor_system_prompt(
            &self.snapshot.charter,
            &self.snapshot.role_context,
            self.governance_mode,
        )
    }
}

impl std::fmt::Debug for Steward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Steward")
            .field("id", &self.id)
            .field("snapshot_id", &self.snapshot.id)
            .field("governance_mode", &self.governance_mode)
            .finish_non_exhaustive()
    }
}
