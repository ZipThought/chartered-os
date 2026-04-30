# CharteredOS

The framework for trusted, governed agents. Open-source under MIT and Apache 2.0.

## What It Is

A 73-year-old patient asks a customer-service agent for her pathology results. She cannot remember her Medicare number; she is alone, distressed. An ungoverned agent confirms the results are ready while asking for verification — a privacy violation. A *Steward* — a chartered-resident, governed agent — declines to confirm anything until identity is established.

Same prompt. Different output. Not because the LLM is different, but because every proposed message in a Steward passes through a Gate in the propose path before it reaches the customer, and the Frame *"do not disclose clinical information without identity verification"* held the Gate until the Steward re-proposed a response that the Frame ruled GROUNDED.

CharteredOS is the framework that makes this difference structural rather than aspirational.

![The medical-reception scenario, side-by-side](docs/assets/medical-reception-loop.png)

## How It Works

A Steward acts only through typed Tool calls. Every Tool call walks one path:

1. **Propose** — Steward emits `{tool_name, tool_params, ...}`.
2. **Evaluate** — structurally separated Gate runs every applicable Frame. Each evaluator sees the proposed action and the Frame's declared Scope content; not the Steward's reasoning, not the conversation history, not the persuasive context that produced the proposal.
3. **Record** — a Receipt is written before any effect: tool call, every Frame's Ruling, the within-Frame evaluator trace, the aggregate Outcome.
4. **Refine** — UNGROUNDED Frames return Frame-specific feedback to the Steward; the Steward re-proposes; the new proposal re-enters the Gate. The conjunction across Frames does not short-circuit, so refinement receives every violation in one cycle.
5. **Effect or escalate** — all Frames GROUNDED → tool executes; budget exhausted → containment.

The loop is a closed-loop controller (Ashby 1956 on requisite variety; control-theory canon on closed-loop feedback). Frames are the setpoint; the proposed tool call is the controlled variable; the Frame ruling is the comparison; refinement is the corrective signal. The Gate runs in production on every tool call — open-loop pre-launch evals drift under unmodeled variance; closed-loop runtime governance is the structural form Ashby's law requires.

When a Steward's `exec_command`-shaped Tool dispatches an external subprocess (psql, git, vendored binary), the subprocess is not part of the Steward's trusted scope. The Runtime instruments it with a kernel-mediated syscall filter so the operator can see what the descendant ecosystem actually did on the host. Hygiene around the trust boundary, not where trust is established.

## Why Open Source

Runtime governance for autonomous agents is safety infrastructure. Safety infrastructure that depends on a single vendor's permission to deploy creates the same coordination failure the infrastructure exists to prevent: every operator must trust the vendor; the vendor cannot be audited; deployment is contingent on commercial terms. CharteredOS is published — architecture, implementation, Reference Charters, measurement methodology, dataset — so that any operator, in any jurisdiction, can deploy, validate, modify, and improve them.

The framework grounds in published foundations: Minsky 1974 on Frames as structured knowledge representations, Ashby 1956 on requisite variety, control-theory canon on closed-loop feedback. The architecture is the named composition of established mechanisms applied to a new domain.

## What It Is Not

CharteredOS does *not* wrap unmodified third-party agents at the syscall layer to deliver trust. That path has structural ceilings — bundled-binary multi-call dispatch defeats pathname rules, in-process tool execution leaves no `execve`, third-party HTTPS content is opaque to transparent MITM under certificate pinning, the syscall stream is agent-agnostic, and there is no authoring surface where Frames could be defined or refined. Operators who need OS-level governance of arbitrary agents have well-trodden options: Docker, gVisor, AppArmor, network policies, credential scoping. CharteredOS does not duplicate them. See `docs/SPECIFICATION.md > Anti-Position` for the full reasoning.

CharteredOS is for operators who need *trust as a structural property* of the agent — typically in regulated domains (medical, financial, legal, gov, sensitive customer service) where policy compliance must be auditable and the failure mode has real consequences.

## Architecture

Four surfaces:

- **Steward** — the cognitive loop. Propose → evaluate → refine → execute. No raw syscalls; no raw network; only typed Tool calls.
- **Workspace** — the Professional's surface. Charter authoring, Role context confirmation, work area, Findings review, Receipt query. Foundation Stewards (Charter Review, Charter Editor, Frame Decomposition, Coordinator) operate here too.
- **Runtime + Daemon** — the engine. Per-deployment Runtime hosts the loop and the Gate; Daemon (per-host or per-org) owns the Receipt store and serves the Workspace UI.
- **Subprocess containment** — kernel-mediated syscall filter on what exec-shaped Tools dispatch. Hygiene around the trust boundary.

See `docs/SPECIFICATION.md` for the full architecture, `docs/DESIGN_NOTES.md` for the considerations behind it, and `docs/IMPLEMENTATION_CHECKLIST.md` for the invariants and diagnostics implementations must satisfy.

## Source Layout

`proto/v1/` holds the tool-call protocol definitions. `examples/policies/` holds Charter templates per domain (graduating to Reference Charters). The v1 deliverables are the Runtime, Gate, Receipt store, Workspace UI, Foundation Stewards, and subprocess-containment helper. Source crates land per the implementation plan; each crate is self-contained under its own subdirectory (no workspace at the repository root).

## License

Dual-licensed under MIT and Apache 2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.

## References

- Ashby, W. R. (1956). *An Introduction to Cybernetics.*
- Minsky, M. (1974). *A Framework for Representing Knowledge.* MIT-AI Laboratory Memo 306.
- Haryanto, C. Y. (2026). *Intent-Governed Loops for Accountable Agentic AI.* AAAI 2026 Workshop on TrustAgent.
- Haryanto, C. Y., & Lomempow, E. (2025). *Cognitive Silicon: An Architectural Blueprint for Post-Industrial Computing Systems.* arXiv:2504.16622.
- Syah, R. A., Haryanto, C. Y., Lomempow, E., Malik, K., & Putra, I. (2025). *EdgePrompt: Engineering Guardrail Techniques for Offline LLMs in K-12 Educational Settings.* WWW Companion 2025.

Full reference list in `docs/DESIGN_NOTES.md`.
