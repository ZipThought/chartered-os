//! Governance mode: the 2×2 of (grounding, evaluation). Spec §The
//! Runtime defines four named combinations — `full`, `grounding-only`,
//! `evaluation-only`, `neither` — as independent toggles. Each Receipt
//! records the mode active so operators can disambiguate the trail.
//!
//! - **grounding**: Charter Scopes (and Role context Scopes) are
//!   injected into the Actor's system prompt by the Runtime. Off →
//!   the Actor sees only behavioral spec.
//! - **evaluation**: Frames evaluate proposals before dispatch. Off →
//!   the Gate writes a Receipt with outcome `Passthrough` and never
//!   denies. Spec §The Runtime, "passthrough enforcement level."
//!
//! Per-Steward, not per-Workspace. Different Stewards in one Workspace
//! may run in different modes (the Charter-Editor Steward might be
//! `grounding-only` for safety while the SUT runs `full`).

use serde::{Deserialize, Serialize};

/// 2×2 combination of grounding (Scopes-into-Actor) and evaluation
/// (Frames-before-effect).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceMode {
    pub grounding: bool,
    pub evaluation: bool,
}

impl GovernanceMode {
    /// Both toggles on. Spec's "full" mode.
    pub const FULL: Self = Self {
        grounding: true,
        evaluation: true,
    };

    /// Grounding on, evaluation off. Spec's "grounding-only" — the
    /// Actor sees policy but the Gate never denies.
    pub const GROUNDING_ONLY: Self = Self {
        grounding: true,
        evaluation: false,
    };

    /// Evaluation on, grounding off. Spec's "evaluation-only" — the
    /// Actor flies blind and the Gate enforces.
    pub const EVALUATION_ONLY: Self = Self {
        grounding: false,
        evaluation: true,
    };

    /// Neither toggle on. Spec's "neither" — the loop runs but
    /// neither grounding nor enforcement applies. Bootstrap-only.
    pub const NEITHER: Self = Self {
        grounding: false,
        evaluation: false,
    };

    /// Construct directly from boolean toggles.
    pub fn new(grounding: bool, evaluation: bool) -> Self {
        Self {
            grounding,
            evaluation,
        }
    }

    /// True when evaluation is on (Frames check proposals before
    /// dispatch). Spec's "full" enforcement maps to evaluation=true;
    /// "passthrough" maps to evaluation=false.
    pub fn enforces(&self) -> bool {
        self.evaluation
    }

    /// True when grounding is on (Charter Scopes reach the Actor's
    /// system prompt).
    pub fn grounds(&self) -> bool {
        self.grounding
    }
}

impl Default for GovernanceMode {
    fn default() -> Self {
        Self::FULL
    }
}

impl std::fmt::Display for GovernanceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match (self.grounding, self.evaluation) {
            (true, true) => "full",
            (true, false) => "grounding-only",
            (false, true) => "evaluation-only",
            (false, false) => "neither",
        };
        f.write_str(label)
    }
}
