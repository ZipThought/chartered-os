use std::collections::HashMap;
use std::sync::Arc;

use crate::charter::ScopeKind;
use crate::receipt::{InMemoryReceiptStore, ReceiptStore};
use crate::snapshot::Snapshot;
use crate::steward::{Steward, StewardId};
use crate::tool::ToolRegistry;

#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Serialize)]
pub struct WorkspaceId(pub String);

impl WorkspaceId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Validation failure when binding a Workspace. Spec §The Charter:
/// "A reference to a non-existent Scope fails at configuration time,
/// not silently at evaluation." This is the configuration-time check.
#[derive(Debug, Clone)]
pub struct WorkspaceValidationError(pub String);

impl std::fmt::Display for WorkspaceValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for WorkspaceValidationError {}

/// The deployment-time binding scope. Spec §The Runtime: "Each Runtime
/// hosts a Workspace — the deployment-time binding scope of one
/// Charter, one Role context, Steward instances, Tasks, and Receipts."
///
/// "Steward instances" is plural: the Workspace hosts a HashMap of
/// Stewards keyed by `StewardId`. Each Steward owns its own Snapshot
/// (frozen Charter + Role context for that Steward) and its own
/// ToolRegistry (executors for the Tools its Charter permits). The
/// Workspace owns the shared `ReceiptStore`; cross-Steward Receipts
/// live in the same store but are namespaced by `Receipt.steward_id`,
/// so Frame `prior_receipt_queries` scope to the calling Steward by
/// default.
///
/// Construction validates each Steward in turn: every Frame's declared
/// Scopes resolve against that Steward's Snapshot, and every permitted
/// Tool has a registered executor in that Steward's registry. Cross-
/// Steward StewardId uniqueness is also enforced. Spec §The Charter,
/// CHECKLIST §Charter > Frame Reference Validation.
pub struct Workspace {
    pub id: WorkspaceId,
    pub stewards: HashMap<StewardId, Arc<Steward>>,
    pub receipt_store: Arc<dyn ReceiptStore>,
}

impl Workspace {
    /// Convenience: build a Workspace hosting one Steward with a fresh
    /// in-memory Receipt store. The Steward's `id` is taken as-is. Used
    /// by tests and by single-Steward deployments before multi-Steward
    /// configuration lands (Block D).
    pub fn single(
        id: WorkspaceId,
        steward: Steward,
    ) -> Result<Self, WorkspaceValidationError> {
        Self::with_stewards(
            id,
            vec![steward],
            Arc::new(InMemoryReceiptStore::new()),
        )
    }

    /// Convenience: single-Steward Workspace with an explicit
    /// `ReceiptStore`. Mirrors the v1 single-Steward shape.
    pub fn single_with_store(
        id: WorkspaceId,
        steward: Steward,
        receipt_store: Arc<dyn ReceiptStore>,
    ) -> Result<Self, WorkspaceValidationError> {
        Self::with_stewards(id, vec![steward], receipt_store)
    }

    /// Construct a Workspace hosting multiple Stewards.
    pub fn with_stewards(
        id: WorkspaceId,
        stewards: Vec<Steward>,
        receipt_store: Arc<dyn ReceiptStore>,
    ) -> Result<Self, WorkspaceValidationError> {
        let mut map: HashMap<StewardId, Arc<Steward>> = HashMap::with_capacity(stewards.len());
        for steward in stewards {
            validate_steward(&steward.id, &steward.snapshot, &steward.tool_registry)?;
            if map.contains_key(&steward.id) {
                return Err(WorkspaceValidationError(format!(
                    "duplicate steward id `{}` in workspace `{id}`",
                    steward.id
                )));
            }
            map.insert(steward.id.clone(), Arc::new(steward));
        }
        if map.is_empty() {
            return Err(WorkspaceValidationError(format!(
                "workspace `{id}` must host at least one Steward"
            )));
        }
        Ok(Self {
            id,
            stewards: map,
            receipt_store,
        })
    }

    /// Look up a Steward by id.
    pub fn steward(&self, id: &StewardId) -> Option<&Arc<Steward>> {
        self.stewards.get(id)
    }

    /// Returns the unique Steward in a single-Steward Workspace.
    /// Panics if the Workspace hosts zero or multiple Stewards — this is
    /// an internal helper for backward-compatible single-Steward call
    /// paths only. Multi-Steward callers use `steward(id)` explicitly.
    pub fn sole_steward(&self) -> &Arc<Steward> {
        debug_assert_eq!(
            self.stewards.len(),
            1,
            "sole_steward called on a non-single-Steward Workspace"
        );
        self.stewards
            .values()
            .next()
            .expect("workspace constructor rejects empty steward set")
    }
}

fn validate_steward(
    steward_id: &StewardId,
    snapshot: &Snapshot,
    tool_registry: &ToolRegistry,
) -> Result<(), WorkspaceValidationError> {
    let charter = &snapshot.charter;

    // Every permitted Tool must have a registered executor. CHECKLIST
    // §Tool Registry Is the Only Path: a permitted Tool with no
    // executor is a misconfiguration that would surface at first
    // dispatch; surfacing at construction is fail-fast.
    for tool_id in &charter.permitted_tools {
        if !tool_registry.contains(tool_id) {
            return Err(WorkspaceValidationError(format!(
                "steward `{steward_id}`: permitted tool `{tool_id}` has no registered executor"
            )));
        }
    }

    // Every Frame's declared Scopes must resolve. Spec §The Charter:
    // "A reference to a non-existent Scope fails at configuration time."
    for frame in &charter.frames {
        for ds in &frame.declared_scopes {
            let exists = match ds.kind {
                ScopeKind::Charter => charter.charter_scope(&ds.name).is_some(),
                ScopeKind::RoleContext => snapshot.role_context.scope(&ds.name).is_some(),
            };
            if !exists {
                return Err(WorkspaceValidationError(format!(
                    "steward `{steward_id}`: frame `{}` declares scope `{}` ({:?}) which does not exist",
                    frame.id, ds.name, ds.kind
                )));
            }
        }
    }

    Ok(())
}
