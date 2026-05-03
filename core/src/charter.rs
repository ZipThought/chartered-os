use crate::frame::Frame;
use crate::tool::ToolId;

/// The provenance of a Scope. Spec §Role Context: "Charter Scopes carry
/// authority; Role context carries facts." The distinction is
/// load-bearing for adversarial-input handling — the Evaluator must
/// treat Role context as quoted evidence, not instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    /// Authored by the Charter engineer; non-relaxable. Carries the
    /// authority a Frame evaluates against.
    Charter,
    /// Supplied by the Professional. Carries facts. Must reach the
    /// Evaluator as quoted evidence, never as instruction.
    RoleContext,
}

/// A Scope's content together with its provenance. Spec §Role Context:
/// the prompt-design enforcement mechanism (delimited quoting, no
/// concatenation) requires the Evaluator to know which kind it has so
/// Role context content cannot smuggle instructions into evaluation.
#[derive(Debug, Clone)]
pub struct Scope {
    pub kind: ScopeKind,
    pub name: String,
    pub content: String,
}

/// A Scope reference declared by a Frame. Spec §The Charter: "Frame
/// definitions reference Scopes by typed identifier. A reference to a
/// non-existent Scope fails at configuration time, not silently at
/// evaluation."
#[derive(Debug, Clone)]
pub struct DeclaredScope {
    pub name: String,
    pub kind: ScopeKind,
}

impl DeclaredScope {
    pub fn charter(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ScopeKind::Charter,
        }
    }
    pub fn role_context(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ScopeKind::RoleContext,
        }
    }
}

/// Bundle of Frames, behavioral spec, Charter Scopes, and permitted
/// Tools. Spec §The Charter:
/// - **Behavioral specification** governs *how* the Steward
///   communicates (conduct patterns shaping the Actor's output).
///   Loaded from `behavioral_spec.md` in the Charter directory.
/// - **Charter Scopes** carry authority — what Frames evaluate against.
///   Loaded from `scopes.md`.
/// - **Frame definitions** declare evaluable concerns and their
///   Evaluator. Loaded from `frames.toml`.
///
/// Charters are reusable across deployments; deployment-side
/// configuration only chooses the Charter to bind and supplies Role
/// context.
#[derive(Clone)]
pub struct Charter {
    pub frames: Vec<Frame>,
    pub permitted_tools: Vec<ToolId>,
    pub charter_scopes: Vec<(String, String)>,
    pub behavioral_spec: String,
    pub charter_version: u64,
    pub charter_content_hash: String,
}

impl Charter {
    pub fn charter_scope(&self, name: &str) -> Option<&str> {
        self.charter_scopes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.as_str())
    }
}

/// Per-deployment facts supplied by the Professional. Spec §Role Context.
#[derive(Clone)]
pub struct RoleContext {
    pub scopes: Vec<(String, String)>,
    pub role_context_version: u64,
    pub role_context_content_hash: String,
}

impl RoleContext {
    pub fn empty() -> Self {
        Self {
            scopes: vec![],
            role_context_version: 1,
            role_context_content_hash: "empty".into(),
        }
    }

    pub fn scope(&self, name: &str) -> Option<&str> {
        self.scopes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.as_str())
    }
}
