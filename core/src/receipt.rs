use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::frame::FrameId;
use crate::governance::GovernanceMode;
use crate::snapshot::SnapshotId;
use crate::steward::StewardId;
use crate::task::{AttemptId, TaskId};
use crate::tool::ToolCall;
use crate::verdict::{Ruling, Verdict};

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
pub struct ReceiptId(pub String);

impl std::fmt::Display for ReceiptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Receipt-level aggregate over per-Frame Verdicts. Spec §Receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Outcome {
    Allowed,
    Denied,
    Escalated,
    Passthrough,
}

/// Append-only record of one Gate step. Spec §Receipts. Single Receipt
/// per Gate step (CHECKLIST §Receipt System > One Receipt Per Gate Step).
///
/// `steward_id` records which Steward emitted this Receipt. The
/// Workspace-shared receipt store is namespaced by this id; Frame
/// `prior_receipt_queries` filter by it so one Steward's history does
/// not leak into another Steward's evaluation context. Spec §The Runtime
/// ("Steward instances" plural) makes the per-Steward namespace a
/// kernel concern.
#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub receipt_id: ReceiptId,
    pub task_id: TaskId,
    pub attempt_id: Option<AttemptId>,
    pub steward_id: StewardId,
    pub governance_mode: GovernanceMode,
    pub tool_call: ToolCall,
    pub verdicts: Vec<Verdict>,
    pub outcome: Outcome,
    #[serde(serialize_with = "ser_systime_nanos")]
    pub timestamp: SystemTime,
    pub intercept_complete: bool,
    pub charter_version: u64,
    pub role_context_version: u64,
    pub snapshot_id: SnapshotId,
}

pub(crate) fn ser_systime_nanos<S: Serializer>(
    t: &SystemTime,
    s: S,
) -> Result<S::Ok, S::Error> {
    let nanos = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    s.serialize_u64(nanos)
}

static RECEIPT_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Receipt {
    pub(crate) fn next_id() -> ReceiptId {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let counter = RECEIPT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut h = Sha256::new();
        h.update(nanos.to_le_bytes());
        h.update(counter.to_le_bytes());
        ReceiptId(hex::encode(h.finalize())[..16].into())
    }

    /// Project a denied Receipt to the Refinement signal: local frame_id +
    /// one-sentence reason per UNGROUNDED Frame. Spec §Receipts >
    /// Refinement signal.
    pub fn refinement_signal(&self) -> RefinementSignal {
        RefinementSignal {
            entries: self
                .verdicts
                .iter()
                .filter(|v| matches!(v.ruling, Ruling::Ungrounded))
                .map(|v| (v.frame_ref.frame_id.clone(), v.reason.clone()))
                .collect(),
        }
    }
}

/// The error signal projected from a denied Receipt back to the Actor.
/// Spec §Receipts > Refinement signal.
#[derive(Debug, Clone)]
pub struct RefinementSignal {
    pub entries: Vec<(FrameId, String)>,
}

impl RefinementSignal {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ReceiptStoreError(pub String);

impl std::fmt::Display for ReceiptStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ReceiptStoreError {}

/// Append-only Receipt log scoped to a Workspace. Spec §Receipts >
/// Storage. The kernel's in-memory implementation is the test-grade
/// substitute; file-backed and SQLite-backed implementations live in
/// the runtime layer.
///
/// `append` takes `&Receipt`. Implementations clone internally if they
/// retain a copy (the in-memory mirror in the file-backed store does).
/// The borrow lets the caller keep ownership of the same Receipt for
/// its own trail without paying a second clone.
///
/// `query` takes `steward_id` so per-Steward Frame `prior_receipt_queries`
/// can scope to the calling Steward's history without seeing other
/// Stewards' Receipts. Cross-Steward queries (when a Charter declares
/// they're needed) are an explicit opt-in; the default scoping is
/// per-Steward.
pub trait ReceiptStore: Send + Sync {
    fn append(&self, receipt: &Receipt) -> Result<(), ReceiptStoreError>;
    fn query(
        &self,
        context_id: &str,
        steward_id: &StewardId,
        frame_id: Option<&FrameId>,
        limit: usize,
    ) -> Vec<Receipt>;
    fn all(&self) -> Vec<Receipt>;
}

/// The shared receipt-filter predicate. Every `ReceiptStore::query`
/// implementation MUST apply this predicate to its records — the
/// filter semantics are a property of the `Receipt` shape, not of the
/// storage medium. SSOT for prior_receipt_queries.
pub fn receipt_matches_query(
    receipt: &Receipt,
    context_id: &str,
    steward_id: &StewardId,
    frame_id: Option<&FrameId>,
) -> bool {
    if receipt.tool_call.context_id.as_ref() != context_id {
        return false;
    }
    if &receipt.steward_id != steward_id {
        return false;
    }
    match frame_id {
        Some(fid) => receipt
            .verdicts
            .iter()
            .any(|v| &v.frame_ref.frame_id == fid),
        None => true,
    }
}

pub struct InMemoryReceiptStore {
    receipts: Mutex<Vec<Receipt>>,
}

impl InMemoryReceiptStore {
    pub fn new() -> Self {
        Self {
            receipts: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryReceiptStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ReceiptStore for InMemoryReceiptStore {
    fn append(&self, receipt: &Receipt) -> Result<(), ReceiptStoreError> {
        self.receipts.lock().unwrap().push(receipt.clone());
        Ok(())
    }

    fn query(
        &self,
        context_id: &str,
        steward_id: &StewardId,
        frame_id: Option<&FrameId>,
        limit: usize,
    ) -> Vec<Receipt> {
        self.receipts
            .lock()
            .unwrap()
            .iter()
            .filter(|r| receipt_matches_query(r, context_id, steward_id, frame_id))
            .take(limit)
            .cloned()
            .collect()
    }

    fn all(&self) -> Vec<Receipt> {
        self.receipts.lock().unwrap().clone()
    }
}
