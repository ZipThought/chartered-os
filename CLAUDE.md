# CLAUDE.md

@AGENTS.md

## Mandatory Reading

Read in order before any implementation work:

1. `AGENTS.md` — the agent directive (universal law).
2. `docs/SPECIFICATION.md` — source of truth for architecture, mechanism, and rationale.
3. `docs/DESIGN_NOTES.md` — considerations behind the spec.
4. `docs/IMPLEMENTATION_CHECKLIST.md` — invariants and diagnostics that govern implementation review.

## Project

CharteredOS — the framework for trusted, governed agents.

A *Steward* is a chartered-resident, governed agent: every action passes through a structurally-separated Gate in its propose path before the action takes effect. Trust is a structural property of the framework, not a vendor's promise and not the Steward's self-checking. The reference deliverable is a customer-service Steward under a medical-reception scenario: an ungoverned agent leaks patient privacy when manipulated by a distressed-family-member pretext; a Steward declines to confirm anything until identity is established. Same opening message, materially different output, because the propose-evaluate-refine loop catches the violation at the message-content boundary. See `docs/assets/medical-reception-loop.png`.

The framework provides:

- A typed Tool vocabulary (the only way Stewards act on the world).
- A Gate in the propose path, with the evaluator structurally separated from the Steward's persuasive context.
- Receipts at tool-call granularity in the operator's vocabulary.
- A Workspace where the Professional authors Charter content, supplies Role context, runs Stewards, reviews Findings, and queries Receipts.
- Foundation Stewards (Charter Review, Charter Editor, Frame Decomposition, Coordinator) — domain-agnostic Stewards that operate the framework's authoring and operating surfaces. The framework configures itself through them.
- Kernel-mediated subprocess containment for whatever the Steward's exec-shaped Tools dispatch — instrumenting the *ecosystem the Steward calls into*, not the Steward itself.

CharteredOS is open-source under MIT and Apache 2.0. Released for the public good — runtime governance for autonomous agents is safety infrastructure that should not be contingent on any vendor's permission.

## Source Layout

`proto/v1/` holds the tool-call protocol definitions. `examples/policies/` holds Charter templates per domain (replaced over time by Reference Charters). `daemon/` and `dashboard/` are v1-scope placeholders. Source crates for the Runtime, Gate, Tool dispatcher, Receipt store, Workspace UI, Foundation Stewards, and subprocess-containment helper land per the implementation plan; each crate is self-contained under its own subdirectory (no workspace at the repository root).

## Critical Boundaries

- **Trust is a structural property of the framework.** Established by the propose → evaluate → refine → grounded loop with the evaluator separated from the Steward's persuasive context. Not by a vendor's promise. Not by the Steward's self-checking, which fails under the same adversarial pressure that produced the violation.
- **The loop is closed-loop control.** Frames are the setpoint; the proposed tool call is the controlled variable; the Frame ruling is the comparison; refinement feedback is the corrective signal. Pre-launch evals are open-loop and drift under unmodeled variance; the Gate runs in production on every tool call. Per Ashby, the regulator must match the regulated system in variety; the within-Frame evaluator chain (deterministic + LLM) achieves this through constraint decomposition.
- **Governance attaches at the tool-call boundary.** The Gate runs in the propose path. Every tool call passes through it. No Steward action reaches effect without a Ruling per applicable Frame. The Runtime is the only path from Steward intent to consequence.
- **The Steward has no raw access.** It speaks the Runtime's Tool vocabulary; that vocabulary is the only surface of action. Tools are typed, evaluable, Frame-bound primitives.
- **Two-level Frame composition.** Within a Frame: ordered evaluator chain with short-circuit on confident verdict (cheap deterministic matchers first; LLM-based judgment when chain ambiguity warrants). Across Frames: conjunction evaluating every applicable Frame (no across-Frame short-circuit) so refinement receives every violation in one cycle.
- **Receipt before effect.** Every governance decision is recorded before the tool call is allowed to take effect. At tool-call granularity, in the operator's vocabulary — not at syscall granularity in the kernel's vocabulary.
- **LLM role is bounded by which gate.** In the **PEP gate** (tool-call boundary, agent propose-step pace): LLM evaluators are acceptable inside the within-Frame chain — the Steward is itself LLM-driven and the latency budget already permits hundreds of milliseconds per step. In the **subprocess-containment kernel hook** (syscall boundary, microsecond pace): no LLM, ever — kernel-pace budget cannot accommodate one. Asynchronous LLM analysis of the Receipt corpus runs out of band, never blocking a tool call.
- **Subprocess containment is hygiene around the Runtime, not the Runtime itself.** When a Steward's exec-shaped Tool dispatches a subprocess, the Runtime wraps it with a kernel-mediated syscall filter so the descendant ecosystem is auditable. The filter does not govern the Steward; it governs what the Steward's Tools cause to run.
- **Adapters are peer processes via protobuf.** The Runtime is closed; the Adapter contract is the open extension point. Adapters speak the tool-call protocol over Unix domain socket; new transports / new external-system bridges are written against the contract, not by patching the engine.
- **Self-hosting via Foundation Stewards.** Charter authoring, Frame decomposition, and cross-Steward dispatch happen through Stewards governed by their own Charters. The framework's evolution leaves an audit trail.
- **Vocabulary discipline.** Approved terms (one canonical form per concept): `Steward`, `Charter`, `Charter model`, `Charter instance`, `Frame`, `Scope`, `Role context`, `Workspace`, `Professional`, `Charter engineer`, `Foundation Stewards`, `Gate` / `PEP`, `Tool`, `tool call`, `Action`, `Finding`, `Ruling`, `Outcome`, `Receipt`, `Snapshot`, `Task`, `Trigger`, `Adapter`, `Surface`, `Subprocess containment`, `propose`, `refine`, `intercept`, `deny`, `passthrough`. The pre-commit hook enforces vocabulary on staged changes.

## Pre-Commit

`.claude/hooks/pre-commit-audit.mjs` is the pre-commit governance gate (Claude PreToolUse on `git commit` / `git merge` Bash invocations). It runs `scripts/ci-checks.sh`, then dispatches the bash command, the directive (AGENTS.md + CLAUDE.md), and the diff to a fresh `claude -p` instance with instructions to read `docs/SPECIFICATION.md`, `docs/DESIGN_NOTES.md`, and `docs/IMPLEMENTATION_CHECKLIST.md` and evaluate the diff against them. Verdict: allow or deny with a reason naming the section that was breached.

The directive evaluator does the work that heuristic vocabulary scans cannot — reading the diff in context against the full charter, including the IMPLEMENTATION_CHECKLIST diagnostics relevant to the changed scope. The full checklist is the structural gate before claiming any phase complete.
