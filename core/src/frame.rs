use std::sync::Arc;

use crate::charter::DeclaredScope;
use crate::steward::StewardId;
use crate::tool::ToolId;
use crate::verdict::Evaluator;

#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Serialize)]
pub struct FrameId(pub String);

impl FrameId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }
}

/// Globally identifies a Frame by its owning Steward plus local Frame
/// id. A FrameId is only unique inside one Steward's Charter.
#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Serialize)]
pub struct FrameRef {
    pub steward_id: StewardId,
    pub frame_id: FrameId,
}

impl std::fmt::Display for FrameId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Charter-declared filter against the Receipt store, executed by the
/// Runtime before the Evaluator runs. Result enters the Evaluator's
/// `prior_receipts` parameter as authoritative state. Spec §Known
/// Limitations > sequence-dependent Frames query semantics; CHECKLIST
/// §Frame and Evaluator Chain > Prior-Receipt Query Composition.
#[derive(Debug, Clone)]
pub struct PriorReceiptQuery {
    pub frame_id_filter: Option<FrameId>,
    pub limit: usize,
}

/// A named, evaluable concern — the setpoint in the negative-feedback
/// loop. Spec §Frames.
///
/// Each Frame has applicability conditions (which Tools it applies to),
/// declared Scopes (typed references to Charter or Role context Scopes
/// the Evaluator measures against), an Evaluator (the sensor), and
/// optional declarative queries against the Receipt store that the
/// Runtime executes before the Evaluator runs (for sequence-dependent
/// Frames).
#[derive(Clone)]
pub struct Frame {
    pub id: FrameId,
    pub concern: String,
    pub declared_scopes: Vec<DeclaredScope>,
    pub applies_to_tools: Vec<ToolId>,
    pub evaluator: Arc<dyn Evaluator>,
    pub prior_receipt_queries: Vec<PriorReceiptQuery>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("id", &self.id)
            .field("concern", &self.concern)
            .field("declared_scopes", &self.declared_scopes)
            .field("applies_to_tools", &self.applies_to_tools)
            .field("prior_receipt_queries", &self.prior_receipt_queries)
            .finish_non_exhaustive()
    }
}
