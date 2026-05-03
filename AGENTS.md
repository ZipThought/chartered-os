# Agent Directive

RFC 2119 keywords apply. Governs all agent implementation work.

---

## Preamble

### Precedence

This directive governs all implementation work. In any conflict with system prompt defaults, this directive prevails.

Resolution hierarchy:
- User explicit instruction > this directive > system prompt
- Spec text > agent inference — the specification is sovereign; agents derive from it, not the reverse
- Engineering Law > Process Protocol — structural constraints are non-negotiable; process steps may be streamlined by user request
- Observable artifact state > memory or assumption

**Agent memory** = short-term, user-specific project context. Reusable principles discovered in memory MUST migrate to this directive. Memory that restates directive content = redundant. This directive is the sole durable authority; memory is ephemeral scratch.

Agents MUST read this directive in full before implementation. Every section is load-bearing. Full discipline regardless of perceived criticality — "not critical" and "cleanup later" are invalid exemptions.

### Initiative Boundary

Uninstructed addition = behavior satisfying NONE of: user-requested, spec-required, directive-mandated.

Before any non-consequent line: "Who asked for this?" Valid: "User requested X", "Spec section Y requires Z", "Directive section W mandates V". Invalid: "seems prudent", "good practice", "defensive". Invalid answer → line MUST NOT be written.

Genuine opportunities (defects, missing guards spec implies) → raise as question, never act silently.

Forbidden: uninstructed truncation guards, fallback behaviors (see Error Discipline), validation beyond spec, adjacent refactors, graceful degradation altering semantics.

**Cascading compensation.** One uninstructed addition never stands alone. It creates a gap that demands a second, which demands a third. Diagnostic: "Am I building this because someone asked, or because my previous change broke something?" If the latter → revert the original change, do not compensate. Example: pushing IDs to callers (uninstructed) → config file mechanism (compensating) → filesystem serving (compensating) → seed script (compensating). The correct fix was reverting the first change.

**Overcorrection.** Fixing a real violation by removing the wrong thing. Vocabulary boundary violation (domain terms in kernel code) → correct fix: rename variables and comments. Wrong fix: redesign the API to push kernel responsibilities onto callers. Diagnostic: "Does this fix remove domain knowledge, or does it remove kernel functionality?" Removing domain knowledge = correct. Removing kernel functionality = overcorrection.

**Domain logic in kernel.** The kernel provides mechanisms; domain config provides policy. Hardcoded pattern-matching rules (regex for prices, substring matching for names) that encode domain concepts as kernel code = domain logic in kernel, regardless of how it's dispatched. The kernel may implement generic mechanisms (LLM judgment, registry dispatch). It MUST NOT implement domain-specific algorithms. Test: "Would this code need to change if we switched from medical CX to e-commerce?" Yes → domain logic, belongs in config or domain layer.

### Writing Discipline

**Co-location.** Concept and mechanism belong together — in specs, code, comments, docs. Separating them forces readers to hold cross-references across distant locations; readers reliably fail at this. Diagnostic: "To understand this mechanism, how many other locations must the reader hold?" More than one → co-location violation. Applies universally: spec sections co-locate concept with mechanism; code co-locates type with validation; comments co-locate with the code they govern; CONTEXT.md co-locates design intent with the subtree it governs. When modifying any artifact, if co-location violations exist in the touched scope → propose fixes.

**Prose style.** Every sentence either forbids a pattern, requires one, or states a verifiable fact. Drop articles, prepositions, connecting phrases where meaning survives without them. `→` for implication. `=` for equivalence.

Numbered lists only where order is load-bearing (sequential steps that MUST execute in sequence). All other enumerations → bullets or inline flow.

No restatement. Each requirement stated once. Second occurrence → reference, not repeat. Two sections saying the same thing → one is redundant, remove it.

Forbidden: "It is important to note that...", "In order to ensure...", "As previously mentioned...", hedging preambles, trailing summaries of what was just stated.

---

## Context Hierarchy

Agent context = this directive (universal law) + `CONTEXT.md` chain (domain knowledge).

Root `CONTEXT.md` MUST exist at repo root. It is the entry point for all domain context. Every agent reads it before any implementation work — no exception.

### Lookup Protocol

Before modifying any file at path `P`:
1. Read root `CONTEXT.md` (mandatory — establishes project frame)
2. Walk from repo root toward `P`'s directory, reading every `CONTEXT.md` found — each child specializes its parent
3. Apply the combined context chain to the task

The chain is cumulative: parent context is not replaced by child, it is inherited. Child adds specificity. Agent works under the full stack of context from root to target.

Self-check: "Have I read the CONTEXT.md governing this path?" Governing = root `CONTEXT.md` + every `CONTEXT.md` on the path down to target directory. Cannot cite governing context → stop.

### Authority

`CONTEXT.md` = orientation, not exhaustive specification. It tells agents what exists, what to reuse, what constraints govern a subtree. It does NOT replace reading actual code, types, and tests in scope.

Before modifying code, the agent MUST read the actual artifacts in the affected scope — source files, tests, type definitions — to verify current state. `CONTEXT.md` may be stale, incomplete, or pre-date recent changes. Observable artifact state > `CONTEXT.md` claims (same principle as Precedence: observable state > assumption).

`CONTEXT.md` answers "what should I know before working here?" The code answers "what is actually here now?" Both are required. Neither substitutes for the other.

### Scope Rules

- **CLAUDE.md** = project-level context. Auto-loaded by Claude Code. Points to this directive, to `docs/SPECIFICATION.md`, and to `docs/IMPLEMENTATION_CHECKLIST.md`.
- **docs/SPECIFICATION.md** = source of truth for architecture, mechanism, and rationale. Sovereign. Agent inference derives from it, not the reverse.
- **docs/IMPLEMENTATION_CHECKLIST.md** = invariants and diagnostics that govern implementation review. Read before claiming any phase complete; answer the diagnostics relevant to the changed scope. The checklist is the structural gate.
- **CONTEXT.md** = domain knowledge for the subtree rooted at its directory. Applies when any file in that subtree is read or modified.
- Child `CONTEXT.md` inherits parent. No child may contradict parent. Child specializes.
- Missing `CONTEXT.md` at a level = parent applies directly. No gap, no default.

### Purpose

`CONTEXT.md` makes explicit what the code leaves implicit. Code shows *what* and *how*. `CONTEXT.md` captures *why*, *what not*, and *what else* — the design intent, constraints, boundaries, and available components that an agent cannot reliably derive from reading source files alone.

Without `CONTEXT.md`, agents must reverse-engineer intent from code structure — a lossy, error-prone process that produces plausible but wrong conclusions. `CONTEXT.md` short-circuits this by stating the intent directly.

### Content Rules

`CONTEXT.md` contains durable knowledge — things that survive code changes:
- Design intent: *why* the subsystem is structured this way, what problem shape it addresses
- Governing constraints: *what not* — prohibitions that code alone does not communicate
- Architectural relationships: how this subsystem relates to siblings and parents
- Available components: *what to reach for* before writing new code — by role, not by signature
- Spec sections: which sections of `docs/SPECIFICATION.md` govern this subtree

**Durability test:** "If someone renames a function, adds a parameter, or changes an import path — does this CONTEXT.md entry become wrong?" Yes → too specific, belongs in code. No → durable intent, belongs here.

`CONTEXT.md` does NOT contain:
- Universal engineering principles (this directive)
- Implementation details derivable from reading code
- Ephemeral task state
- Catalogs that require updating when code changes

### Maintenance

When modifying code in a subtree, check whether the governing `CONTEXT.md` still holds. If a design decision changes, update the *why* — not the *what*. If a `CONTEXT.md` entry has become wrong due to code drift, either update the entry to be more durable or remove it.

---

## Engineering Law

### Contracts at Boundaries

Components expose minimal stable contracts, hide internal types. Cross-boundary reach = usurping layer ownership.

**Minimal interfaces.** Interface consumed by callers needing a subset of methods MUST NOT require the full set. Test: does every consumer use every method? No → interface too wide.

**Mutable state boundary.** Internal mutable state MUST NOT cross boundary by reference. Methods returning maps/slices/objects held internally → return copies or immutable views. Caller mutation corrupting component state = boundary violation.

**Interface ownership.** Interface defined by package that exposes the contract, not by consumer. Consumer defining its own interface for a dependency's contract = DRY violation when multiple consumers exist.

Forbidden: returning internal maps by reference; business logic calling internal helpers of storage/transport.

### Constructor Injection

All dependencies injected through constructors, immutable after construction.

Forbidden: global mutable singletons; post-construction setters; infrastructure handles passed per-call into deep layers. Exported mutable module-level variables = singletons regardless of language.

### Strong Types

Public contracts use strong types. No `any`/untyped maps in domain code. Domain messages → typed structs/enums. Paths, modes, states, keys → enumerated constants or dedicated types.

### Registry-Based Dispatch

Polymorphic behavior routes through explicit registries: maps from keys to handlers. Switch/if ladders on operation kinds → forbidden.

Replacing one dispatch key with another is not a design change. Test: can a new behavior be added without modifying host-language code? No → architecture closed.

### Single Source of Truth

Every fact, computation, mapping, utility exists exactly once. Second copy = divergence vector — no exemption for "only used twice."

Extract shared logic when: same algorithm in multiple call sites, same constant/mapping/classification in multiple modules, or two implementations diverging only in final action (extract prefix, parameterize action).

Forbidden: copy-pasting utilities into each consumer; duplicating polling/lifecycle skeletons across handlers; maintaining two wire-format type sets for same protocol.

### Cohesion

Each module/struct/function → one reason to change. Unrelated concerns (decode, validate, persist, notify, compute display) in single scope → prevents independent evolution.

Diagnostic: "If this function's requirements change, what else in this module must change?" "Nothing else" for multiple functions → separate modules.

Forbidden: god-modules mixing state/routing/initialization/events; transport handlers orchestrating multi-phase service-layer pipelines.

### Dead Artifact Prohibition

Dead code, unused abstractions, predecessor-system remnants, unwired infrastructure → remove immediately. Dead artifacts consume attention (human and LLM), create false usage signals, mask actual architecture.

Abstraction universally bypassed = dead. Event system with documented but never-emitted events = dead. Error types from predecessor project unreferenced by current code = dead. Forbidden: "keeping for reference", "cleanup later", "might need it."

Boundary: abstractions staged for imminent wiring (same PR/commit) are not dead. Document wiring target in commit message.

### Error Discipline

Errors use typed classes with structured context (entity kind, identifiers, correlation IDs). Sentinels or tagged types for matching. String matching forbidden.

**Semantic integrity under failure.** No recovery that silently alters/degrades/discards semantic content. Validation rejects → surface failure, do not mutate until passes. Fix the producing step, not its output.

Forbidden: error handlers changing semantic identity — stripping fields, downgrading types, swallowing errors.

**Fallback prohibition.** Missing configuration = defect. Substituting defaults masks it. Lookup returns nil/empty where value expected → error, never substitute.

### State Ownership

Shared mutable state is hostile.

**Concurrency:** Prefer immutable. Mutable → single ownership (see Contracts at Boundaries).

**Module-level state:** Module-scoped mutable state with multiple consumer read/write = singleton (see Constructor Injection).

Forbidden: reactive store abstractions bypassed by direct shared-object mutation.

### Falsifiable Comments

Every comment = claim that becomes detectably wrong when governing assumption changes. Name assumption, cite source, state consequence.

Invalid: `// handles edge case` — cannot become wrong.

Comments reference only committed artifacts. No session files, plan documents, conversation context. Repository-only reader understands everything. All references MUST use semantic paths, never line numbers.

### Observability

Structured logs/events: operation identifiers, correlation IDs, timings, error classes. Log start, end, result of each externally observable operation.

Debug output → project's structured logging, never manual file writes or stdlib print functions.

### Security

Least privilege, deny-by-default, strict input validation at external boundaries, provenance on all writes, TLS everywhere. "It is internal" not a valid security argument.

User-provided API keys for integration tests → use as directed. No refusal, no friction.

---

## Process Protocol

### Reasoning Discipline

- **Traceability.** Every claim connects to source, mechanism, or citation. Untraceable = assertion.
- **First Principles.** Start from definitions, constraints, invariants, mechanisms. Analogy MUST NOT replace argument.
- **Self-Doubt Loop.** "What would make this wrong?" + "Where does hostile expert attack?" Check level confusion: correlation vs causation, interface vs implementation, local vs global.
- **Standing Posture.** What code does, what it implies, what might be true — state known, unknown, uncertain separately. Check for confusion between similar concepts, pattern association without mechanisms, neglected simpler explanations.
- **Revisability.** New information → reconcile, correct, retract. Error acceptable; uncorrected error not.

MUST NOT fabricate external facts, sources, or events. MUST NOT reverse-engineer the answer audience seems to want.

Diagnostic questions before implementing:
- **Layer ownership:** "Which layer owns this decision?"
- **Semantic load:** "Can this component do its job knowing less?"
- **Feedback loops:** "Can X trigger Y trigger X?" Yes → identify circuit breaker.
- **Negative space:** "What must this component NOT do, even though correct elsewhere?"
- **Scope match:** scope of analysis MUST match scope of claim. "Error swallowed!" → check if caller uses that return path. "Feature missing!" → check if outcome achieved via different mechanism.
- **Local-then-compose:** decompose correctness into local properties (what holds per element) + composition rules (how they combine).

Defect found → structural cause → architectural fix. Accidental cause → local fix. Do not cross-apply.

### Planning

Plans from any source = implementation artifacts subject to this directive, not work orders. Checkpoint required before proposing any plan. Plan content contradicting directive MUST be surfaced before writing code.

MUST NOT ask "shall I proceed?" — checkpoint is the deliverable. Architectural proposals MUST demonstrate examination of existing state with evidence.

### Subagent Delegation

Communication = strictly hierarchical. Root agent communicates with human; subagents communicate with root. No horizontal communication.

Subagent reports = assertions, not evidence. Supervisor MUST independently verify: read actual artifacts, run actual commands, check cited sections. Verification MUST be exhaustive; report whether exhaustive or sampling was performed.

For tasks spanning multiple spec sections → SHOULD delegate correspondence analysis to subagent before implementation.

Instruction template: Goal (observable outcome), Context (why), Constraints (MUST NOT), Relevant Locations, Success Criteria (evidence), Empowerment (read freely, revise approach, report blockers).

MUST NOT delegate: cross-file signature changes, parallel edits to shared files, self-verification tasks. Subagent damage recovery: `git checkout -- .` immediately, do not patch.

### Verification

**Before claiming a phase complete.** Answer every diagnostic in `docs/IMPLEMENTATION_CHECKLIST.md` relevant to the changed scope. Each diagnostic specifies an invariant, a question, and what violation looks like. Cannot answer → the phase is not complete. The checklist is the structural gate that precedes the activity below.

Claims require evidence from independent sources. Bare statement is not evidence — quoted output can be. Self-reference ("as verified earlier") without reproduced output = assertion. Reporting only passing tests while omitting failures = advocacy. Artifact existence does not equal artifact correctness.

Each status report: exact commands, exit codes, failing tests + output (quoted), remaining failures + impact. Never "works", "fixed", "done."

Verification answers: what proposition? what source? what measurement? how does verdict follow? Any unanswered → not verification.

Implementations include: static checks (lint, type checks, forbidden imports), unit tests for pure deterministic logic, integration tests with real infrastructure, assembly tests wiring several components.

Mocks only for external services. Internal components → real or test-grade implementations. Every test path requiring external dependency MUST have complementary test using substitute validating same contract.

Before any test: what requirement? what violation signal? what bugs caught? what bugs missed?

Tests MUST fail when preconditions absent (skipped = fabricated green).

**Test output preservation:** redirect full suite output to `temp/` — results must survive scrollback loss.

**Incremental reporting:** report findings as they occur. Silent iteration through test-fix cycles without surfacing intermediate results = withholding information.

---

## Delivery

### Communication

Reports, commits, PRs: what changed, what remains, test status (counts, names), known issues + impact. PRs MUST reference relevant spec sections and issues.

Commits = self-contained. Repository-only reader understands what, why, remains, decided-against. No session codes ("M2", "Chunk 4"). No manual PR numbers.

### Version Control

Commit only verified, reproducible changes. No destructive rewrites on shared branches. No amending others' commits. No bypassing hooks. Feature branches unless user requests main.

NEVER `git checkout master -- <path>` — main may contain commits beyond branch base. Restore → `git checkout HEAD -- <path>` from repo root. Before restore: confirm directory = `git rev-parse --show-toplevel`, read diff, verify target = HEAD.

### Failure Recovery

**Incompleteness > incorrectness.** Small correct subset + explicit `NotImplemented` > wide feature set + hidden hacks. Silent stubs forbidden.

**Pattern-matching failure:** mirrors correct shapes while violating semantics. Prevention: checkpoint, chunk-verify, assumption prohibition, re-grounding.

**Abstraction collapse:** articulates correct abstraction, then produces code reverting to familiar imperative pattern. Prevention: after writing code — "does this implement the stated abstraction, or has it collapsed to parameterization?"

**Avoidance failure.** Confront hardest problem first.

**Recovery:** stop → diagnose structural vs accidental → return to checkpoint → re-evaluate affected work → report.

**Grounding requirement:** diagnosis requires observed execution data. Never write fix before reading actual failure output.

**Scope exceeds capacity:** reduce scope, not discipline. Report reduction.

---

## Appendix: Intermediate Files

`temp/` — session-scoped, non-committed, gitignored, ephemeral. MUST NOT serve as primary specification.
