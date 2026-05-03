use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::artifact::{ArtifactId, ArtifactRange};
use crate::receipt::{Outcome, ReceiptId};
use crate::scenario::SelectionActionKind;
use crate::steward::StewardId;

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }

    pub fn next() -> Self {
        Self(next_id("task"))
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
pub struct AttemptId(pub String);

impl AttemptId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }

    pub fn for_task(task_id: &TaskId, index: usize) -> Self {
        Self(format!("{}-attempt-{}", task_id.0, index))
    }
}

impl std::fmt::Display for AttemptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskRecord {
    pub task_id: TaskId,
    pub steward_id: StewardId,
    pub trigger: TaskTrigger,
    pub status: TaskStatus,
    #[serde(serialize_with = "crate::receipt::ser_systime_nanos")]
    pub created_at: SystemTime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskTrigger {
    UserMessage {
        text: String,
    },
    Selection {
        artifact_id: ArtifactId,
        range: ArtifactRange,
        action_name: String,
        action_kind: SelectionActionKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TaskStatus {
    Running,
    Allowed,
    Denied,
    Escalated,
    Halted,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttemptRecord {
    pub attempt_id: AttemptId,
    pub task_id: TaskId,
    pub steward_id: StewardId,
    pub index: usize,
    pub receipt_id: ReceiptId,
    pub outcome: Outcome,
}

static TASK_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    h.update(nanos.to_le_bytes());
    h.update(counter.to_le_bytes());
    format!("{prefix}-{}", &hex::encode(h.finalize())[..16])
}
