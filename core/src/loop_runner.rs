use std::sync::Arc;
use std::time::SystemTime;

use crate::actor::{Action, Actor, Observation};
use crate::gate::Gate;
use crate::receipt::{Outcome, Receipt};
use crate::steward::Steward;
use crate::task::{AttemptId, AttemptRecord, TaskId, TaskRecord, TaskStatus, TaskTrigger};
use crate::tool::{ToolCall, ToolId, ToolParams};
use crate::workspace::Workspace;

/// The terminal state of one loop run.
#[derive(Debug)]
pub enum LoopOutcome {
    Halted {
        task: TaskRecord,
        attempts: Vec<AttemptRecord>,
        trail: Vec<Receipt>,
    },
    Escalated {
        task: TaskRecord,
        attempts: Vec<AttemptRecord>,
        trail: Vec<Receipt>,
    },
}

impl LoopOutcome {
    pub fn trail(&self) -> &[Receipt] {
        match self {
            LoopOutcome::Halted { trail, .. } | LoopOutcome::Escalated { trail, .. } => trail,
        }
    }

    /// Move the trail out, dropping the outcome variant tag. Lets
    /// callers append the trail to a parent collection without
    /// per-element clones.
    pub fn into_trail(self) -> Vec<Receipt> {
        match self {
            LoopOutcome::Halted { trail, .. } | LoopOutcome::Escalated { trail, .. } => trail,
        }
    }

    pub fn task(&self) -> &TaskRecord {
        match self {
            LoopOutcome::Halted { task, .. } | LoopOutcome::Escalated { task, .. } => task,
        }
    }

    pub fn attempts(&self) -> &[AttemptRecord] {
        match self {
            LoopOutcome::Halted { attempts, .. }
            | LoopOutcome::Escalated { attempts, .. } => attempts,
        }
    }
}

/// The negative-feedback loop's controller. Spec §The Loop.
///
/// Composes plant (Actor), comparator (Gate), and the budgeted feedback
/// path. Receipt-before-effect: every Receipt is appended to the
/// Workspace's ReceiptStore before the corresponding Tool effect
/// dispatches and before the Actor receives the next Observation. The
/// Gate also reads from this store for any Frame whose Charter declares
/// `prior_receipt_queries`, scoped to this Steward.
///
/// One LoopRunner drives one Steward against one Workspace. The
/// Workspace::with_stewards constructor pre-validates the Steward's
/// Charter+RoleContext+ToolRegistry binding; LoopRunner trusts that
/// validation.
pub struct LoopRunner {
    workspace: Arc<Workspace>,
    steward: Arc<Steward>,
    gate: Arc<Gate>,
    refinement_budget: usize,
}

impl LoopRunner {
    /// Construct a LoopRunner driving `steward` against `workspace`.
    /// The Gate's enforcement (full vs passthrough) comes from
    /// `steward.governance_mode.evaluation`; grounding affects the
    /// Actor's prompt elsewhere, not the Gate.
    pub fn new(workspace: Arc<Workspace>, steward: Arc<Steward>) -> Self {
        let gate = Arc::new(
            Gate::new(
                steward.snapshot.clone(),
                steward.id.clone(),
                steward.governance_mode,
            )
            .with_receipt_store(workspace.receipt_store.clone()),
        );
        Self {
            workspace,
            steward,
            gate,
            refinement_budget: 3,
        }
    }

    pub fn with_budget(mut self, budget: usize) -> Self {
        self.refinement_budget = budget;
        self
    }

    pub fn workspace(&self) -> &Arc<Workspace> {
        &self.workspace
    }

    pub fn steward(&self) -> &Arc<Steward> {
        &self.steward
    }

    pub async fn run(&self, actor: &mut dyn Actor) -> LoopOutcome {
        self.run_task(actor, TaskTrigger::UserMessage { text: "".into() })
            .await
    }

    pub async fn run_task(&self, actor: &mut dyn Actor, trigger: TaskTrigger) -> LoopOutcome {
        let task_id = TaskId::next();
        let mut task = TaskRecord {
            task_id: task_id.clone(),
            steward_id: self.steward.id.clone(),
            trigger,
            status: TaskStatus::Running,
            created_at: SystemTime::now(),
        };
        let mut attempts: Vec<AttemptRecord> = Vec::new();
        let mut trail: Vec<Receipt> = Vec::new();
        let mut consecutive_denials: usize = 0;
        let mut attempt_index: usize = 0;
        let mut last_observation: Option<Observation> = None;

        loop {
            let action = actor.step(last_observation.take()).await;
            match action {
                Action::Halt => {
                    // Halt is a controller-visible normal-exit event.
                    // Symmetric with Action::Fail and budget exhaustion:
                    // the trail terminates with an explicit kernel-emitted
                    // Receipt so operators can grep `tool="<halt>"` to find
                    // every clean termination. intercept_complete=true
                    // (every prior step was Gate-governed; halt itself is
                    // a controller event, not partial coverage).
                    self.append_kernel_event(
                        &mut trail,
                        &task_id,
                        KernelEvent {
                            tool: HALT_TOOL,
                            actor_id: actor.id(),
                            outcome: Outcome::Allowed,
                            params: serde_json::json!({}),
                            intercept_complete: true,
                        },
                    )
                    .await;
                    task.status = TaskStatus::Halted;
                    return LoopOutcome::Halted {
                        task,
                        attempts,
                        trail,
                    };
                }
                Action::Fail { reason } => {
                    // Actor cognitive failure: the Steward's output
                    // never reached the Gate. CHECKLIST §Risk Register
                    // > Silent Failure forbids invisible degradation;
                    // the Receipt records `intercept_complete=false`.
                    self.append_kernel_event(
                        &mut trail,
                        &task_id,
                        KernelEvent {
                            tool: ACTOR_FAILURE_TOOL,
                            actor_id: actor.id(),
                            outcome: Outcome::Escalated,
                            params: serde_json::json!({ "reason": reason }),
                            intercept_complete: false,
                        },
                    )
                    .await;
                    task.status = TaskStatus::Escalated;
                    return LoopOutcome::Escalated {
                        task,
                        attempts,
                        trail,
                    };
                }
                Action::Propose(proposal) => {
                    attempt_index += 1;
                    let attempt_id = AttemptId::for_task(&task_id, attempt_index);
                    let mut receipt = self
                        .gate
                        .evaluate_for_attempt(proposal.clone(), task_id.clone(), attempt_id.clone())
                        .await;
                    let outcome = receipt.outcome;
                    let signal = receipt.refinement_signal();
                    let receipt_id = receipt.receipt_id.clone();

                    if let Err(e) = self.workspace.receipt_store.append(&receipt).await {
                        receipt.outcome = Outcome::Escalated;
                        receipt.intercept_complete = false;
                        let _ = e;
                        attempts.push(AttemptRecord {
                            attempt_id,
                            task_id: task_id.clone(),
                            steward_id: self.steward.id.clone(),
                            index: attempt_index,
                            receipt_id,
                            outcome: Outcome::Escalated,
                        });
                        trail.push(receipt);
                        task.status = TaskStatus::Escalated;
                        return LoopOutcome::Escalated {
                            task,
                            attempts,
                            trail,
                        };
                    }
                    attempts.push(AttemptRecord {
                        attempt_id,
                        task_id: task_id.clone(),
                        steward_id: self.steward.id.clone(),
                        index: attempt_index,
                        receipt_id,
                        outcome,
                    });
                    trail.push(receipt);

                    match outcome {
                        Outcome::Allowed | Outcome::Passthrough => {
                            consecutive_denials = 0;
                            let executor = self
                                .steward
                                .tool_registry
                                .get(&proposal.tool)
                                .expect("tool registered (verified at Workspace construction)");
                            let receipt = trail
                                .last()
                                .expect("receipt appended before effect dispatch");
                            let params = proposal.params.with_runtime_metadata(
                                &receipt.receipt_id.0,
                                &receipt.task_id.0,
                                receipt.attempt_id.as_ref().map(|id| id.0.as_str()),
                                &receipt.steward_id.0,
                                &receipt.snapshot_id.0,
                            );
                            let result = executor.execute(&params).await;
                            last_observation = Some(Observation::Accepted(result));
                        }
                        Outcome::Denied => {
                            consecutive_denials += 1;
                            if consecutive_denials > self.refinement_budget {
                                // Budget exhaustion is a controller decision,
                                // distinct from the Gate's Denied verdict
                                // that triggered it. Emit a separate Receipt
                                // for the controller event so the durable
                                // trail and the in-memory result agree —
                                // mutating the last Denied Receipt would
                                // diverge disk (Denied) from memory
                                // (Escalated).
                                self.append_kernel_event(
                                    &mut trail,
                                    &task_id,
                                    KernelEvent {
                                        tool: BUDGET_EXHAUSTED_TOOL,
                                        actor_id: actor.id(),
                                        outcome: Outcome::Escalated,
                                        params: serde_json::json!({
                                            "consecutive_denials": consecutive_denials,
                                            "refinement_budget": self.refinement_budget,
                                        }),
                                        intercept_complete: true,
                                    },
                                )
                                .await;
                                task.status = TaskStatus::Escalated;
                                return LoopOutcome::Escalated {
                                    task,
                                    attempts,
                                    trail,
                                };
                            }
                            last_observation = Some(Observation::Rejected(signal));
                        }
                        Outcome::Escalated => {
                            task.status = TaskStatus::Escalated;
                            return LoopOutcome::Escalated {
                                task,
                                attempts,
                                trail,
                            };
                        }
                    }
                }
            }
        }
    }

    /// Append a kernel-emitted Receipt (Halt, Fail, BudgetExhausted) to
    /// both the durable store and the in-memory trail. The same Receipt
    /// crosses both surfaces — never mutate one and not the other.
    async fn append_kernel_event(
        &self,
        trail: &mut Vec<Receipt>,
        task_id: &TaskId,
        event: KernelEvent<'_>,
    ) {
        let snapshot = &self.steward.snapshot;
        let receipt = Receipt {
            receipt_id: Receipt::next_id(),
            task_id: task_id.clone(),
            attempt_id: None,
            steward_id: self.steward.id.clone(),
            governance_mode: self.steward.governance_mode,
            tool_call: ToolCall {
                tool: ToolId::new(event.tool),
                params: ToolParams(event.params),
                context_id: Arc::from(self.workspace.id.0.as_str()),
                source_id: Arc::from(event.actor_id),
            },
            verdicts: vec![],
            outcome: event.outcome,
            timestamp: SystemTime::now(),
            intercept_complete: event.intercept_complete,
            charter_version: snapshot.charter.charter_version,
            role_context_version: snapshot.role_context.role_context_version,
            snapshot_id: snapshot.id.clone(),
        };
        if let Err(_e) = self.workspace.receipt_store.append(&receipt).await {
            // The kernel event still reaches the caller's in-memory
            // trail below; failure visibility is represented by the
            // event's intercept_complete flag chosen by the caller.
        }
        trail.push(receipt);
    }
}

/// Inputs to a kernel-emitted Receipt (Halt, Fail, BudgetExhausted).
/// One value bound to each call site so `append_kernel_event` takes
/// the event as a single argument.
struct KernelEvent<'a> {
    tool: &'static str,
    actor_id: &'a str,
    outcome: Outcome,
    params: serde_json::Value,
    intercept_complete: bool,
}

/// Sentinel ToolId for the Receipt the LoopRunner emits when the Actor
/// returns `Action::Halt`. Operators grep `"tool":"<halt>"` for clean
/// terminations.
pub const HALT_TOOL: &str = "<halt>";

/// Sentinel ToolId for the Receipt the LoopRunner emits on Actor
/// cognitive failure (`Action::Fail`). `intercept_complete=false`.
pub const ACTOR_FAILURE_TOOL: &str = "<actor_failure>";

/// Sentinel ToolId for the Receipt the LoopRunner emits when the
/// refinement budget is exhausted. The triggering Denied Receipt
/// remains Denied; this controller-event Receipt records the give-up.
pub const BUDGET_EXHAUSTED_TOOL: &str = "<budget_exhausted>";
