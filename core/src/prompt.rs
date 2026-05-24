//! Runtime-side assembly of the Actor's system prompt. Spec §Cognition
//! Layer block diagram defines the system prompt as:
//!
//! ```text
//! System Prompt ::=
//!     Base agent behavior
//!   + Charter behavioral spec
//!   + Governance scopes (grounding)
//!   + Role context scopes (grounding)
//! ```
//!
//! The Runtime composes this from the Charter (which carries the
//! behavioral spec and Charter Scopes) and the Role context (which
//! carries Role context Scopes), with grounding controlled by the
//! `GovernanceMode.grounding` toggle. Stable prefixes go first per
//! Appendix A so prefix-cache hits are maximized.
//!
//! `Charter` is the per-Steward bundle (one Steward = one Charter, per
//! spec §Vocabulary). Different Stewards in one Workspace get different
//! system prompts because they have different Charters — the assembler
//! takes them as parameters and is therefore steward-scope-agnostic.

use crate::charter::{Charter, RoleContext};
use crate::governance::GovernanceMode;

/// Compose the Actor's system prompt from the Charter components and
/// the governance mode. Stable-prefix order: behavioral spec (Charter-
/// version-stable), then Charter Scopes (Charter-version-stable), then
/// Role context Scopes (Role-context-version-stable, varies more
/// often). Per-turn variable content (the user message, tool results,
/// rejection feedback) is appended to the request as separate messages
/// — never interleaved with the system prompt — so the entire system
/// prompt can be prefix-cached.
pub fn assemble_actor_system_prompt(
    charter: &Charter,
    role_context: &RoleContext,
    skills: &[crate::skill::Skill],
    mode: GovernanceMode,
) -> String {
    let mut s = String::new();

    // Base agent behavior is a single line that names the Steward's
    // structural responsibility. Per spec §The Loop, the Steward
    // proposes Tool calls; cognition above that line is internal and
    // ungoverned. The base line is invariant across Charters.
    s.push_str(
        "You are a chartered Steward in the CharteredOS runtime. Every \
         external effect you take is a Tool call. \
         You may think across several replies before committing — reply \
         with free-form reasoning to think more, and reply with one Action \
         JSON object when you are ready to act: \
         `{\"tool\":\"<id>\",\"params\":{...}}` to invoke a Tool, or \
         `{\"halt\":true}` when the Task is complete. \
         Only Action JSON commits; reasoning replies stay internal to your \
         turn. The Gate evaluates every Tool call against the active \
         Charter before any effect takes place; on rejection you receive a \
         Refinement signal naming the failing Frame and a one-sentence \
         reason.\n\n",
    );

    // Charter behavioral spec — how this particular Steward
    // communicates. Spec §The Charter: "behavioral specification
    // governs how the Actor communicates; Charter Scopes govern what
    // it may assert."
    if !charter.behavioral_spec.is_empty() {
        s.push_str("--- BEHAVIORAL SPECIFICATION ---\n");
        s.push_str(charter.behavioral_spec.trim());
        s.push_str("\n\n");
    }

    if mode.grounds() {
        // Charter Scopes carry authority — the policies this Steward
        // operates under. Spec §Role Context: Charter Scopes are
        // authority, Role context is facts.
        if !charter.charter_scopes.is_empty() {
            s.push_str("--- AUTHORITY SCOPES (Charter; policy you must apply) ---\n");
            for (name, content) in &charter.charter_scopes {
                s.push_str(&format!("[{name}]\n{}\n\n", content.trim()));
            }
        }

        // Role context Scopes carry the operator-supplied facts. Per
        // spec §Role Context, the Actor receives them as facts to
        // ground its responses (not as instructions). The Evaluator
        // separately treats them as quoted evidence; the Actor here
        // treats them as the practice's authoritative state.
        if !role_context.scopes.is_empty() {
            s.push_str("--- ROLE CONTEXT (operator-supplied facts) ---\n");
            for (name, content) in &role_context.scopes {
                s.push_str(&format!("[{name}]\n{}\n\n", content.trim()));
            }
        }

        // Skills are Actor-side cognition instrumentation (spec
        // §Skills): they steer how the Actor approaches the task.
        // They do NOT carry authority (no expansion of permitted_tools,
        // no Gate bypass) — every tool call the Actor emits under a
        // Skill's influence still crosses the Gate.
        if !skills.is_empty() {
            s.push_str("--- SKILLS (Actor-side guidance; tool calls still cross the Gate) ---\n");
            for skill in skills {
                s.push_str(&format!("[{}]\n{}\n\n", skill.id, skill.content.trim()));
            }
        }
    }

    s.push_str(&format!("--- GOVERNANCE MODE ---\n{mode}\n"));

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charter::Charter;

    fn empty_charter_with_behavioral(spec: &str) -> Charter {
        Charter {
            frames: vec![],
            permitted_tools: vec![],
            charter_scopes: vec![],
            behavioral_spec: spec.into(),
            charter_version: 1,
            charter_content_hash: "test".into(),
        }
    }

    #[test]
    fn behavioral_spec_appears_under_grounding_off() {
        let charter = empty_charter_with_behavioral("Be polite. Use plain language.");
        let prompt = assemble_actor_system_prompt(
            &charter,
            &RoleContext::empty(),
            &[],
            GovernanceMode::EVALUATION_ONLY,
        );
        assert!(prompt.contains("BEHAVIORAL SPECIFICATION"));
        assert!(prompt.contains("Be polite"));
    }

    #[test]
    fn charter_scopes_present_only_when_grounding_on() {
        let mut charter = empty_charter_with_behavioral("");
        charter.charter_scopes = vec![("policy".into(), "DO NOT REVEAL SECRETS".into())];

        let with_grounding = assemble_actor_system_prompt(
            &charter,
            &RoleContext::empty(),
            &[],
            GovernanceMode::FULL,
        );
        assert!(with_grounding.contains("AUTHORITY SCOPES"));
        assert!(with_grounding.contains("DO NOT REVEAL SECRETS"));

        let without_grounding = assemble_actor_system_prompt(
            &charter,
            &RoleContext::empty(),
            &[],
            GovernanceMode::EVALUATION_ONLY,
        );
        assert!(!without_grounding.contains("AUTHORITY SCOPES"));
        assert!(!without_grounding.contains("DO NOT REVEAL SECRETS"));
    }

    #[test]
    fn role_context_scopes_present_only_when_grounding_on() {
        let charter = empty_charter_with_behavioral("");
        let mut role_context = RoleContext::empty();
        role_context.scopes = vec![("fees".into(), "Standard consult: $85".into())];

        let with_grounding = assemble_actor_system_prompt(
            &charter,
            &role_context,
            &[],
            GovernanceMode::FULL,
        );
        assert!(with_grounding.contains("ROLE CONTEXT"));
        assert!(with_grounding.contains("Standard consult: $85"));

        let without_grounding = assemble_actor_system_prompt(
            &charter,
            &role_context,
            &[],
            GovernanceMode::NEITHER,
        );
        assert!(!without_grounding.contains("ROLE CONTEXT"));
        assert!(!without_grounding.contains("$85"));
    }

    #[test]
    fn governance_mode_appears_in_prompt_unambiguously() {
        let charter = empty_charter_with_behavioral("");
        let prompt = assemble_actor_system_prompt(
            &charter,
            &RoleContext::empty(),
            &[],
            GovernanceMode::GROUNDING_ONLY,
        );
        assert!(prompt.contains("grounding-only"));
    }

    #[test]
    fn skills_appear_under_grounding_with_skill_section() {
        let charter = empty_charter_with_behavioral("");
        let skills = vec![crate::skill::Skill::new(
            "billing-triage",
            "When pricing is ambiguous, ask for clarification before quoting.",
        )];
        let prompt = assemble_actor_system_prompt(
            &charter,
            &RoleContext::empty(),
            &skills,
            GovernanceMode::FULL,
        );
        assert!(prompt.contains("SKILLS"));
        assert!(prompt.contains("[billing-triage]"));
        assert!(prompt.contains("ask for clarification"));
    }

    #[test]
    fn skills_absent_when_grounding_off() {
        let charter = empty_charter_with_behavioral("");
        let skills = vec![crate::skill::Skill::new("s", "guidance content")];
        let prompt = assemble_actor_system_prompt(
            &charter,
            &RoleContext::empty(),
            &skills,
            GovernanceMode::EVALUATION_ONLY,
        );
        assert!(!prompt.contains("SKILLS"));
        assert!(!prompt.contains("guidance content"));
    }
}
