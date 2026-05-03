use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::time::timeout;

use crate::charter::{Scope, ScopeKind};
use crate::frame::{Frame, FrameRef};
use crate::governance::GovernanceMode;
use crate::receipt::{InMemoryReceiptStore, Outcome, Receipt, ReceiptStore};
use crate::snapshot::Snapshot;
use crate::steward::StewardId;
use crate::task::{AttemptId, TaskId};
use crate::tool::ToolCall;
use crate::verdict::{Decision, EvaluatorEntry, Ruling, Verdict};

const DEFAULT_PER_FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// The comparator in the negative-feedback loop. Spec §The Loop,
/// §Conjunction.
///
/// Capability check → parallel Frame evaluation (no across-Frame
/// short-circuit, per-Frame timeout, per-Frame prior-Receipt query
/// pre-fetch, typed Scope resolution) → aggregation → Receipt.
/// Constructor injection: the Gate holds the Snapshot, the
/// `steward_id` of the Steward this Gate runs for, and the
/// `governance_mode` (which determines whether Frames evaluate at all
/// — `evaluation=false` produces a Passthrough Receipt instead). All
/// evaluations under one Gate execute against that Snapshot — mid-loop
/// policy switch is structurally impossible.
pub struct Gate {
    snapshot: Arc<Snapshot>,
    steward_id: StewardId,
    governance_mode: GovernanceMode,
    receipt_store: Arc<dyn ReceiptStore>,
    per_frame_timeout: Duration,
}

impl Gate {
    pub fn new(
        snapshot: Arc<Snapshot>,
        steward_id: StewardId,
        governance_mode: GovernanceMode,
    ) -> Self {
        Self {
            snapshot,
            steward_id,
            governance_mode,
            receipt_store: Arc::new(InMemoryReceiptStore::new()),
            per_frame_timeout: DEFAULT_PER_FRAME_TIMEOUT,
        }
    }

    pub fn with_receipt_store(mut self, store: Arc<dyn ReceiptStore>) -> Self {
        self.receipt_store = store;
        self
    }

    pub fn with_frame_timeout(mut self, t: Duration) -> Self {
        self.per_frame_timeout = t;
        self
    }

    pub fn snapshot(&self) -> &Arc<Snapshot> {
        &self.snapshot
    }

    pub fn steward_id(&self) -> &StewardId {
        &self.steward_id
    }

    pub fn governance_mode(&self) -> GovernanceMode {
        self.governance_mode
    }

    /// Evaluate a proposal. Returns the Receipt; the Refinement signal
    /// is derived by `Receipt::refinement_signal`.
    pub async fn evaluate(&self, proposal: ToolCall) -> Receipt {
        let task_id = TaskId::next();
        let attempt_id = AttemptId::for_task(&task_id, 1);
        self.evaluate_for_attempt(proposal, task_id, attempt_id).await
    }

    pub async fn evaluate_for_attempt(
        &self,
        proposal: ToolCall,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Receipt {
        let charter = &self.snapshot.charter;

        if !charter.permitted_tools.contains(&proposal.tool) {
            return Receipt {
                receipt_id: Receipt::next_id(),
                task_id,
                attempt_id: Some(attempt_id),
                steward_id: self.steward_id.clone(),
                governance_mode: self.governance_mode,
                tool_call: proposal,
                verdicts: vec![],
                outcome: Outcome::Denied,
                timestamp: SystemTime::now(),
                intercept_complete: true,
                charter_version: charter.charter_version,
                role_context_version: self.snapshot.role_context.role_context_version,
                snapshot_id: self.snapshot.id.clone(),
            };
        }

        if !self.governance_mode.enforces() {
            return Receipt {
                receipt_id: Receipt::next_id(),
                task_id,
                attempt_id: Some(attempt_id),
                steward_id: self.steward_id.clone(),
                governance_mode: self.governance_mode,
                tool_call: proposal,
                verdicts: vec![],
                outcome: Outcome::Passthrough,
                timestamp: SystemTime::now(),
                intercept_complete: true,
                charter_version: charter.charter_version,
                role_context_version: self.snapshot.role_context.role_context_version,
                snapshot_id: self.snapshot.id.clone(),
            };
        }

        let proposal_arc = Arc::new(proposal.clone());
        let snapshot = self.snapshot.clone();
        let per_frame_timeout = self.per_frame_timeout;
        let receipt_store = self.receipt_store.clone();
        let steward_id = self.steward_id.clone();

        let mut handles = Vec::with_capacity(charter.frames.len());
        for frame in charter.frames.iter() {
            let frame = frame.clone();
            let proposal = Arc::clone(&proposal_arc);
            let snapshot = snapshot.clone();
            let receipt_store = receipt_store.clone();
            let steward_id = steward_id.clone();
            handles.push(tokio::spawn(async move {
                evaluate_frame(
                    frame,
                    proposal,
                    snapshot,
                    receipt_store,
                    steward_id,
                    per_frame_timeout,
                )
                .await
            }));
        }

        let mut verdicts = Vec::with_capacity(handles.len());
        let mut intercept_complete = true;
        for h in handles {
            match h.await {
                Ok((verdict, ok)) => {
                    if !ok {
                        intercept_complete = false;
                    }
                    verdicts.push(verdict);
                }
                Err(_join_err) => {
                    intercept_complete = false;
                }
            }
        }

        let outcome = aggregate(&verdicts);

        Receipt {
            receipt_id: Receipt::next_id(),
            task_id,
            attempt_id: Some(attempt_id),
            steward_id: self.steward_id.clone(),
            governance_mode: self.governance_mode,
            tool_call: proposal,
            verdicts,
            outcome,
            timestamp: SystemTime::now(),
            intercept_complete,
            charter_version: charter.charter_version,
            role_context_version: self.snapshot.role_context.role_context_version,
            snapshot_id: self.snapshot.id.clone(),
        }
    }
}

async fn evaluate_frame(
    frame: Frame,
    proposal: Arc<ToolCall>,
    snapshot: Arc<Snapshot>,
    receipt_store: Arc<dyn ReceiptStore>,
    steward_id: StewardId,
    per_frame_timeout: Duration,
) -> (Verdict, bool) {
    if !frame.applies_to_tools.contains(&proposal.tool) {
        return (
            Verdict {
                frame_ref: FrameRef {
                    steward_id,
                    frame_id: frame.id.clone(),
                },
                ruling: Ruling::OutOfScope,
                reason: "frame does not apply to this tool".into(),
                trace: vec![],
            },
            true,
        );
    }

    // Resolve declared Scopes with provenance. Workspace::new
    // pre-validates that every Frame.declared_scopes reference exists
    // in the Snapshot, and Snapshot is held by Arc — neither Charter
    // nor RoleContext can mutate under us. The .expect therefore names
    // a structural invariant, not a runtime hazard.
    let resolved_scopes: Vec<Scope> = frame
        .declared_scopes
        .iter()
        .map(|ds| {
            let content = match ds.kind {
                ScopeKind::Charter => snapshot.charter.charter_scope(&ds.name),
                ScopeKind::RoleContext => snapshot.role_context.scope(&ds.name),
            }
            .expect("Workspace::new validated declared_scopes references");
            Scope {
                kind: ds.kind,
                name: ds.name.clone(),
                content: content.to_string(),
            }
        })
        .collect();

    let mut prior_receipts: Vec<Receipt> = Vec::new();
    for q in frame.prior_receipt_queries.iter() {
        let mut r = receipt_store.query(
            &proposal.context_id,
            &steward_id,
            q.frame_id_filter.as_ref(),
            q.limit,
        );
        prior_receipts.append(&mut r);
    }

    let trace_fut = frame
        .evaluator
        .evaluate(&proposal, &resolved_scopes, &prior_receipts);
    let trace = match timeout(per_frame_timeout, trace_fut).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            return (
                Verdict {
                    frame_ref: FrameRef {
                        steward_id: steward_id.clone(),
                        frame_id: frame.id.clone(),
                    },
                    ruling: Ruling::Ungrounded,
                    reason: format!("evaluator infrastructure failure: {e}"),
                    trace: vec![],
                },
                false,
            );
        }
        Err(_) => {
            return (
                Verdict {
                    frame_ref: FrameRef {
                        steward_id: steward_id.clone(),
                        frame_id: frame.id.clone(),
                    },
                    ruling: Ruling::Ungrounded,
                    reason: "evaluator timeout".into(),
                    trace: vec![],
                },
                false,
            );
        }
    };

    let ruling = ruling_from_trace(&trace);
    let reason = trace
        .last()
        .map(|e| e.observation.clone())
        .unwrap_or_else(|| "no evaluator output".into());

    (
        Verdict {
            frame_ref: FrameRef {
                steward_id,
                frame_id: frame.id,
            },
            ruling,
            reason,
            trace,
        },
        true,
    )
}

fn ruling_from_trace(trace: &[EvaluatorEntry]) -> Ruling {
    if trace.is_empty() {
        return Ruling::Ungrounded;
    }
    for entry in trace {
        match entry.decision {
            Decision::Allow => return Ruling::Grounded,
            Decision::Deny => return Ruling::Ungrounded,
            Decision::Escalate => return Ruling::Ungrounded,
            Decision::Defer => continue,
        }
    }
    Ruling::Ungrounded
}

fn aggregate(verdicts: &[Verdict]) -> Outcome {
    if verdicts.is_empty() {
        return Outcome::Denied;
    }
    if verdicts
        .iter()
        .any(|v| matches!(v.ruling, Ruling::Ungrounded | Ruling::Uncertain))
    {
        return Outcome::Denied;
    }
    if verdicts.iter().all(|v| matches!(v.ruling, Ruling::OutOfScope)) {
        return Outcome::Denied;
    }
    Outcome::Allowed
}
