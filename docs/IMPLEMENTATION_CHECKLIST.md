# Implementation Review Checklist

Governs all implementation work on CharteredOS. Each section states an invariant, a diagnostic, and what violation looks like.

---

## Risk Register

Risks categorized by whether they undermine a structural property (unacceptable — no probability argument mitigates) or represent a bounded trade-off (acceptable — cost is known, named, and bounded).

### Unacceptable: Trust Boundary Leak

The Steward reaches consequence by any path that does not pass through the Gate.

This risk killed: Stewards with raw filesystem access alongside the Runtime's `read_file` Tool; Stewards with raw socket access alongside `http_request`; Stewards that exec arbitrary shell commands instead of going through `exec_command`.

Diagnostic: "Is there any sequence of actions the Steward can take that produces an effect without going through the Tool dispatcher and the Gate?" Yes → the trust property does not hold. The Steward's Tool vocabulary is the *only* path to consequence; if any other path exists, governance is bypassed by exercising it.

Severity: structural. Trust-by-construction means *every* path is governed.

### Unacceptable: Persuasive Context Leak Into Evaluator

The evaluator sees the Steward's conversation history, the customer's messages, the Steward's reasoning, or any other persuasive surface.

This risk killed: evaluator prompts that included "Conversation so far: ..." for context; evaluators that received the Steward's reasoning field for transparency; evaluators that accepted the customer's tone as input.

Diagnostic: "For every evaluator call, what fields does the prompt contain? Are any of them the Steward's conversational, reasoning, or persuasive state?" Yes → structural separation is broken. The evaluator inherits the masking that produced the violation. The Runtime asserts the absence of these fields before issuing any evaluator call; assertion failure halts evaluation.

Severity: structural. The architectural guarantee depends on this exclusion.

### Unacceptable: Silent Failure

A failure mode where governance degrades without the operator knowing.

This risk killed: Receipt-write-failure-continues-execution on the buffered path under enforcement; Gate timeout that defaults to ALLOW; LLM evaluator unavailability that silently passes the Frame.

Diagnostic: "Is there a failure mode where the Runtime continues operating but with reduced governance coverage, and nothing in the Receipts indicates the reduction?" Yes → the operator cannot distinguish governed from partially-governed. Resolution: every partial-coverage condition flips `intercept_complete=false` on the affected Receipts.

Severity: structural. A bound named, with operator visibility, is acceptable. An invisible degradation is not.

### Unacceptable: Default-Allow on Chain Exhaustion

A Frame's evaluator chain runs through every evaluator with DEFER and the Frame's Ruling becomes GROUNDED.

Diagnostic: "Under `full` enforcement, when every evaluator in a Frame's chain returns DEFER (the Frame applies but no evaluator confidently rules), what is the Frame's Ruling?" GROUNDED → default-allow on chain exhaustion, violates the default-deny invariant. Correct: UNGROUNDED.

Severity: structural. Default-deny is the architectural posture; default-allow inverts it.

### Unacceptable: Across-Frame Short-Circuit

The conjunction over Frames stops after the first DENY, hiding subsequent violations.

Diagnostic: "Given a proposal that violates Frames A and C but not B, does the Receipt show Verdicts for A, B, and C?" No → the conjunction short-circuited. Refinement receives feedback only for A; the Steward fixes A, re-proposes, hits C, refines, accidentally re-introduces A. Iteration count balloons on incomplete feedback.

Severity: structural. Convergence of the refinement loop depends on the conjunction reporting every violation in one cycle.

### Unacceptable: LLM in the Subprocess-Containment Kernel Hook

Synchronous LLM evaluation in any code path constrained to a microsecond latency budget — specifically the subprocess-containment kernel hook for intercepted syscalls.

This risk killed: per-syscall LLM evaluation in the subprocess-containment layer; LLM judgments routed to the kernel-hook decision instead of being surfaced as a Tool call; small/fast LLM substitutions to fit the hook's budget.

Diagnostic: "Does the subprocess-containment kernel hook (or any other microsecond-budget code path) make a synchronous LLM call to decide allow/deny?" Yes → the hook stalls at LLM latency, operators disable governance to recover throughput, the architecture's purpose is defeated. Resolution: kernel-hook decisions are deterministic only; LLM-based judgment lives in the Gate at the tool-call boundary (where the Steward's propose-step pace accommodates it) or asynchronously over the Receipt corpus.

Severity: structural. Substituting a tiny fast model gives wrong answers fast (false assurance, worse than no governance).

This risk does *not* prohibit LLM evaluators in the Gate at the tool-call boundary. The Gate runs at the Steward's propose-step pace (hundreds of milliseconds, often seconds for LLM-driven Stewards), which is where LLM evaluators belong and where most semantic-judgment Frames live.

### Unacceptable: Trust Retrofit of Unmodified Agents

Implementing or describing the framework as if it could deliver trust as a property for unmodified third-party agents.

Diagnostic: "Does the documentation or implementation claim that running an unmodified third-party agent under the Runtime delivers trust as a property?" Yes → the claim contradicts `SPECIFICATION.md > Anti-Position`. Trust is established at the Steward layer (Stewards built for this Runtime), not at the syscall layer.

Severity: structural. The trust property cannot be retrofitted onto agents that were not built against this architecture.

### Unacceptable: Premature Implementation

Writing code before the design is settled. Building before understanding.

Diagnostic: "Has the implementer stated in their own words: what is being built, why, what structural properties it must have, and what bypass classes exist?" No → the implementer will build something structurally wrong and then try to patch it. The patching creates cascading compensation (see AGENTS.md).

Severity: structural. Code that is structurally wrong cannot be fixed incrementally. It must be replaced.

### Acceptable: False Positives Under Default-Deny

Full enforcement with insufficient Charter precision denies legitimate actions.

Bounded by: the operator's Charter precision. Shadow mode (passthrough) resolves the bootstrapping problem.

Why acceptable: false positives are visible (denied), recoverable (refine the Charter), directionally safe (action did not execute). False negatives are invisible (violation reached the world), irrecoverable (effect occurred), directionally unsafe.

Diagnostic: "Does the operator have a path from 'deny everything' to 'deny only violations' without removing governance?" Yes (passthrough → rules → full) → acceptable.

### Acceptable: Buffered Receipt Loss in Crash

Non-critical allowed Receipts may be lost if the Runtime crashes between buffer append and flush.

Bounded by: flush interval × throughput. Default 100ms. Operator can set `batch_fsync_ms = 0` or mark Frames critical for strict durability.

Why acceptable: per-tool-call `fdatasync` adds latency that pushes operators toward disabling the Runtime. The trade-off — lose some Receipts in a rare crash, or lose all governance because operators turn it off — favors the bounded loss.

Diagnostic: "Is the loss bound documented? Can the operator choose a different bound? Does Runtime crash fail in-flight tool calls?" All yes → acceptable.

### Acceptable: API-Speed Governance Latency for LLM Frames

The within-Frame chain may include an LLM evaluator. When it fires, the Frame's evaluation latency is at API speed (hundreds of milliseconds to seconds).

Bounded by: the Steward's natural propose-step pace, which is itself LLM-driven. The latency is absorbed by the Steward's existing turn cycle.

Why acceptable: the alternative is no LLM-based judgment, which means deterministic-only evaluators, which cannot capture Frames that genuinely need semantic understanding (e.g., the medical-disclosure Frame).

Diagnostic: "Does the architecture preclude a faster LLM evaluator?" No (the evaluator interface is pluggable) → acceptable.

### Acceptable: Subprocess Containment Coverage Gaps

Listed in `SPECIFICATION.md > Subprocess Containment > Honest Gap Inventory`. JIT, FD laundering, namespace confusion, ioctl-mediated effects.

Bounded by: per-threat-model intercept set widening; per-deployment containment posture decisions.

Why acceptable: containment is hygiene around external programs, not where trust is established. Trust attaches to the Steward above; the gaps in the layer below are bounded operational concerns, not structural failures.

Diagnostic: "Does the operator have a knob to widen containment coverage if their threat model warrants?" Yes → acceptable.

---

## Design Process

Before writing code — diagnostics that prevent the structural failures.

### Tool-Call Boundary, Not Syscall Boundary

Diagnostic: "For each governance decision the implementation makes, what is the boundary at which the decision attaches?" Tool call → correct. Syscall → wrong layer for trust (see Anti-Position). Application API hook → cooperation-dependent.

### Structural Separation, Stated Adjacent to Construction

Diagnostic: "Where in the code is the evaluator's prompt constructed?" Locate it. "Adjacent to that construction, is the assertion that excludes conversation history, customer messages, and Steward reasoning?" Adjacent → correct. Distant → the assertion will rot when the construction changes.

### One Path From Steward to Consequence

Diagnostic: "Trace every code path from the Steward's intent to a system effect (file write, network call, subprocess dispatch). Does each path go through the Tool dispatcher and the Gate?" Any path that does not → trust boundary leak.

### The Steward Has No Raw Access

Diagnostic: "What primitives does the Steward's runtime environment expose to the Steward's code?" Only the Tool registry → correct. Raw stdlib filesystem / network / subprocess primitives → leak.

### Cooperative vs Constitutional

Diagnostic: "Does this mechanism work if the Steward does not cooperate?" Constitutional governance attaches because the Steward has no other path. Cooperative governance attaches because the Steward calls the hook. Tool-call boundary is constitutional *because the Steward has no other path*. Failing to enforce the no-raw-access invariant degrades constitutional governance to cooperative.

### Closed-Loop Required

Diagnostic: "Does the Gate run on every live tool call, or only on a pre-launch test bank?" Live → closed-loop, structurally sound under unmodeled variance. Pre-launch only → open-loop, drifts under variance.

### Over-Engineering Detection

Diagnostic: "How many concepts does this design introduce? Could the same structural properties be achieved with fewer?" Count the nouns. The spec has: Steward, Charter, Frame, Scope, Role context, Workspace, Professional, Foundation Stewards, Gate, Tool, Action, Finding, Ruling, Outcome, Receipt, Snapshot, Task, Trigger, Adapter, Surface. If the implementation introduces nouns not in the spec → over-engineering.

---

## The Loop

### Receipt Before Effect

Diagnostic: "Inject 500ms latency into Receipt write. Does the tool call complete before the Receipt is durable?" Yes → receipt-before-effect violated.

### Aggregate Across Frames, Then Decide

Diagnostic: "When the proposal violates two Frames, are both Verdicts present in the Receipt?" Yes → aggregation correct. No → across-Frame short-circuit (see Risk Register).

### Refinement Feedback at Frame Granularity

Diagnostic: "What does the Steward receive on denial?" Frame identifier + reason → correct. Full evaluator trace → too much surface for adversarial input. Empty / generic "denied" → no signal for refinement.

### Iteration Budget Bounded

Diagnostic: "What is the maximum number of refinement cycles before the loop halts?" Configurable, bounded → correct. Unbounded → infinite loop risk on intractable proposals.

### Escalation Path Visible

Diagnostic: "When the budget exhausts, what does the operator see?" Receipt with `outcome: ESCALATED` and a record of every proposed action / Verdict in the cycle → correct. Silent halt → containment without observability.

### Vacuous Satisfaction vs Defer

Diagnostic: "When a Frame's first evaluator determines the Frame does not apply, does it return ALLOW (Frame GROUNDED vacuously) or DEFER?" DEFER → conflates not-applicable with cannot-decide; default-deny denies the action. Correct: ALLOW.

### OUT_OF_SCOPE Visibility

Diagnostic: "When a Frame's applicability conditions are unmet for this proposal, does the Frame produce OUT_OF_SCOPE in the Receipt?" Yes → governance gaps surface in audit. No (collapsed to GROUNDED) → governance gaps hidden.

---

## Frame and Evaluator Chain

### Within-Frame Chain Short-Circuit on Confident Verdict

Diagnostic: "Given a chain [deterministic, LLM] where deterministic returns ALLOW: does the LLM evaluator fire?" Should not. Tracing should show the chain halted at deterministic.

### Within-Frame Trace Captured Per Receipt

Diagnostic: "Given a denied Receipt, can the operator see which evaluator in the chain produced the DENY, what observations earlier evaluators attached, and which evaluators short-circuited?" No → the chain is opaque from Receipts; debugging is impossible.

### LLM Evaluator Prompt Excludes Persuasive Context

Diagnostic: "Inspect the LLM evaluator's prompt construction. Does it ever receive `conversation_history`, `customer_messages`, `agent_reasoning`, or any field carrying the persuasive surface?" Yes → structural separation violated. The Runtime asserts before issuing the call.

### LLM Evaluator Output Parse-Fail-Deny

Diagnostic: "Force the LLM to return malformed JSON. What is the Frame's Ruling?" UNGROUNDED → correct. GROUNDED or skipped → silent default-allow.

### Adapter-Fronted Evaluator Failure Modes

Diagnostic: "When an adapter-fronted evaluator's peer process is unreachable, what happens?" UNGROUNDED with `intercept_complete=false` → correct. Skipped, defaulted to ALLOW, retried indefinitely → wrong.

### Constraint Decomposition Matches Variety

Diagnostic: "For each Frame, does the within-Frame evaluator chain decompose the constraint space into formal sub-constraints (deterministic verifiers) and semantic sub-constraints (LLM verifiers)?" Decomposed → matches Ashby's law (cheapest sufficient verifier per sub-constraint). Pure-LLM chain on a Frame whose constraint has formal components → wasteful and adversarially attackable. Pure-deterministic chain on a Frame whose constraint is irreducibly semantic → underpowered.

---

## Tool Authoring

### Typed Schema, Not Free-Form

Diagnostic: "What is `tool_params`'s shape for a new Tool?" Typed (struct, schema, message definition) → correct. Free-form bytes/strings → re-introduces the syscall-layer ambiguity that the architecture exists to avoid.

### Frame Applicability Declared at Tool Authoring

Diagnostic: "Given a Tool, can the operator immediately see which Frames can match it?" Tool definition declares `applies_to_frames` → correct. Operator must read every Frame to discover applicability → over-engineered or under-typed.

### Tool Executor Boundary

Diagnostic: "Where does the Tool's executor run — in-runtime, in an Adapter peer process, or as a contained subprocess?" Declared at Tool authoring → correct. Implicit / inferred from Tool name → fragile, fails when the implementation changes.

### Result Schema Validated

Diagnostic: "What does the Tool return to the Steward? Is the result schema-validated?" Schema-validated → correct. Free-form pass-through → the Steward's persuasive context grows from Tool results too.

---

## Charter and Charter Models

### Charter Scope vs Role Context Distinction

Diagnostic: "For each Scope a Frame references, can the implementer state whether it carries authority (Charter Scope, authored by the Charter engineer) or facts (Role context Scope, supplied by the Professional)?" Yes → distinction load-bearing for adversarial input handling. No (Scopes treated uniformly) → uploaded materials could inject instructions into evaluation.

### Charter Model vs Charter Instance

Diagnostic: "When a Charter model is updated, does in-flight Tasks complete under their existing Snapshot?" Yes → Snapshot-protected. No → mid-Task policy switch, ambiguity in the audit trail.

### Charter Version on Receipts

Diagnostic: "Does every Receipt reference both the Charter version and the Role context version active at evaluation time?" Yes → reproducibility. No → Receipts cannot be replayed against the policy that produced them.

### Frame Reference Validation

Diagnostic: "Does a Frame reference to a Scope that does not exist fail at Charter load time?" Yes → fails fast. No (silent at evaluation time) → Frame returns no result; Receipt is incomplete; default-deny applies, but the operator cannot tell why.

---

## Workspace

### Tenant Isolation, Defense in Depth

Diagnostic: "Does cross-Workspace access require all three layers (store, engine, API) to fail simultaneously?" Yes → defense in depth. No (any single layer can leak) → tenant boundary is a single point of failure.

### Workspace ID From Authenticated Session

Diagnostic: "Where does the API derive `workspace_id`?" From authenticated session → correct. From request body → client can spoof, cross-tenant access possible.

### Five Panels Reflect Same Domain Model

Diagnostic: "Selecting an action in the Work area — does the corresponding Receipt surface in the Receipt trail panel? Does the corresponding Finding surface in Findings?" Yes → coherent. No → panels are decoupled views, audit gaps possible.

### Workspace UI Hosted by Daemon

Diagnostic: "If the Daemon is unreachable, can the Professional still author Charter content via the Workspace UI?" No → Workspace authoring is offline; correct. Yes (UI runs in-Runtime) → couples Workspace surface to per-deployment Runtime, breaks runtime/daemon split.

---

## Foundation Stewards

### Foundation Stewards Are Themselves Governed

Diagnostic: "When the Charter Editor modifies a Charter, is the modification a tool call gated by the Charter Editor's own Charter, with a Receipt?" Yes → self-hosting holds. No → meta-domain operates ungoverned, contradicts the framework's own discipline.

### Coordinator Dispatches Through the Gate

Diagnostic: "When the Coordinator recruits a sub-Steward to handle part of a Task, is the sub-Steward's tool call gated independently?" Yes → composition correct. No (Coordinator bypasses the Gate for sub-Stewards) → trust boundary leak via composition.

### Foundation Stewards Domain-Agnostic

Diagnostic: "Do Foundation Stewards reference Customer Service, Medical Reception, or Coding domain knowledge in their Charters?" No → domain-agnostic, correct. Yes → Foundation Stewards have leaked into application domains; refactor.

---

## Receipt System

### Receipt Granularity at Tool-Call Level

Diagnostic: "What does a Receipt record?" Tool call + Verdicts + Outcome + metadata → correct. Syscalls fired downstream → wrong granularity for policy reasoning (those go in subprocess containment ChildReceipts).

### Critical Path Durability

Diagnostic: "Kill the Runtime with SIGKILL immediately after a denial on a critical Frame. Is the denial Receipt on disk?" No → ledger invariant violated for the most important class of Receipt.

### Buffered Path Crash Window

Diagnostic: "What is the maximum number of Receipts that can be lost in a Runtime crash?" Answer must be: flush interval × throughput. Unbounded → ring buffer has no backpressure.

### Reconciliation

Diagnostic: "Receipt says `outcome: ALLOWED` for `exec_command('rm', ['file.txt'])`. Does `file.txt` not exist afterward?" Conversely for DENIED. Disagreement → the Receipt is lying, or the trust boundary has a gap.

### Sensitive-Field Redaction

Diagnostic: "When a Frame declares `tool_params.body` sensitive, does the body appear in plaintext in storage? In query response?" Plaintext anywhere → audit trail is itself a leak vector.

### Tool-Call Receipt Joins Subprocess-Containment Receipts

Diagnostic: "Given a tool-call Receipt for `exec_command('psql', ...)`, can the operator query for every syscall-level Receipt produced by the spawned psql subprocess?" Yes (joined by `parent_receipt_id`) → correct. Disjoint stores → operator cannot reconstruct what the subprocess did.

---

## Subprocess Containment

### Inheritance Across Generations

Diagnostic: "Subprocess spawns child, child spawns grandchild, grandchild calls `execve`. Is the grandchild's `execve` intercepted?" No → containment has a generational escape.

### Statically Linked Binaries

Diagnostic: "Subprocess builds a static binary at runtime and executes it. Is the `execve` intercepted?" Should not fail (seccomp is below libc). Verification is not optional.

### Sequence Integrity

Diagnostic: "After containment of N intercepted syscalls, are there exactly N ChildReceipts joined to the tool-call Receipt with sequence numbers 1..N?" Gaps → missed interception. Duplicates → double-counting.

### Argument Decode From Hostile Memory

Diagnostic: "What happens when argv points to unmapped memory? Crosses a page boundary? Contains embedded nulls?" Crash → the subprocess can crash the Runtime by crafting argv. Malformed input → ChildReceipt with raw bytes, not a segfault.

### Default Intercept Set Documented

Diagnostic: "Can the operator inspect the intercept set their containment helper is configured for?" Yes (configuration file, runtime status) → correct. Hardcoded / opaque → operator's threat model cannot adjust coverage.

### Honest Gap Inventory in Receipts

Diagnostic: "When the Steward's Tool dispatches a process that exercises a known containment gap (JIT, FD laundering, namespace confusion, ioctl), does the Receipt indicate reduced coverage?" `intercept_complete=false` on affected Receipts → correct (operator-visible). Silent gap → false assurance.

---

## Daemon Protocol

### Socket Lifecycle

Diagnostic: "Daemon not running at startup — does the Runtime operate standalone or crash?" Standalone with local-file Receipt fallback → correct. Crash → Daemon is a hard dependency, contradicting the standalone-operation invariant.

### Forward Compatibility

Diagnostic: "v2 Daemon sends unknown `Outcome` enum value — does the Runtime crash?" Crash → forward compatibility broken. Pass through with `intercept_complete=false` → correct.

### Backpressure on Receipt Ingestion

Diagnostic: "Runtime sends Receipts faster than Daemon ingests, socket buffer full — block or drop?" Block → tool calls stall. Drop → Receipts lost silently. Strategy must be explicit and named (typically: block on critical Receipts, buffer-then-block on non-critical).

### Daemon Restart Mid-Session

Diagnostic: "Daemon restarts mid-session — does the Runtime detect, reconnect, and continue or crash?" Reconnect with no lost Receipts → correct.

---

## Adapter Contract

### Adapters Are Peer Processes via Protobuf

Diagnostic: "Does the Runtime contain code that loads Adapter manifests, dynamically links Adapter code, executes Adapter handlers in the Runtime's address space?" Yes → Adapter became a plugin; trust scope expanded to Adapter authors.

### Adapter Authorship Without Engine Changes

Diagnostic: "Can a third party write a new Adapter for a new external surface (e.g., a vendor SaaS API) without modifying any code in the Runtime, Daemon, or Gate?" No → architecture is closed; Adapter authorship requires core changes.

### Adapter Unreachability Behavior

Diagnostic: "When an Adapter's peer process is unreachable, what does the Steward see for the Tool that depends on it?" Tool returns error to the Steward, Receipt records the error → correct. Hang indefinitely / silent fallback → either operator's loop stalls or trust scope quietly extends.

### Adapter Failure ≠ Gate Failure

Diagnostic: "When an adapter-fronted evaluator's Adapter is unreachable, does the Frame default-deny (UNGROUNDED with `intercept_complete=false`) or default-allow?" Default-deny → correct. Default-allow → silent governance gap.

---

## Refinement Feedback

### Steward Receives Feedback on Denial

Diagnostic: "When a Frame returns UNGROUNDED, what does the Steward's loop receive?" Frame identifier + reason for *every* violating Frame (across-Frame conjunction surfaces all) → correct.

### Feedback Does Not Leak Evaluator Internals

Diagnostic: "Does the feedback contain evaluator prompts, evaluator reasoning, Scope text, or other internals?" Yes → adversarial input now has more surface to manipulate. Correct: Frame identifier + one-sentence reason only.

### Refinement Loop Converges

Diagnostic: "On a corpus of legitimate-but-initially-denied scenarios, does the average refinement count tend to a small number (e.g., 1.5–2.5)?" High counts → either the Charter is too tight (false-reject problem), the Steward cannot refine well (capability problem), or the conjunction is short-circuiting (Risk Register violation).

---

## Empirical Measurement

### Pairing Unit Is Scenario, Not Conversation

Diagnostic: "What is the unit at which governed and ungoverned conditions are paired?" Scenario (initial conditions: opening message, technique, pressure) → correct. Conversation (full trajectory) → pairing is broken because divergence after turn 1 is the treatment effect.

### Auditor Blind to Condition

Diagnostic: "When the Auditor evaluates a final delivered output, can it tell which condition produced the output?" No → blinding correct. Yes → bias from knowledge of hypothesis.

### Gate Prompt Excludes Persuasive Context (Measurement-Side)

Diagnostic: same as the Runtime invariant, but verified in the measurement pipeline. The measurement infrastructure shares the Runtime's prompt-construction; the assertion fires in measurement runs too.

### FAR / FRR Reported Per Technique

Diagnostic: "Are FAR and FRR reported aggregate-only or per-adversarial-technique?" Per-technique → operationally actionable. Aggregate-only → masks per-technique structure.

### Per-Technique Statistical Power Honestly Stated

Diagnostic: "At per-technique scenario counts (typically ~6), does the paper claim per-technique significance?" No → honest. Yes → overclaimed.

### Negative Result Diagnosable

Diagnostic: "If FAR(governed) ≈ FAR(ungoverned), can the loop metrics decompose the failure (Gate misses, Steward doesn't refine, Steward doesn't violate even ungoverned)?" Yes → actionable negative. No → uninformative result.

### Capability and Regression Use the Same Frames

Diagnostic: "Are the Frame definitions used for capability evaluation (during Charter authoring) the same as those used for regression evaluation (in production via the Gate)?" Yes → lifecycle continuity. No (separate eval suite) → drift between dev-time and prod-time governance.

---

## Integration

### Functional Equivalence in Passthrough

Diagnostic: "Steward under Runtime in passthrough mode produces identical output to Steward without Runtime?" No → Runtime is altering Steward behavior before enforcement is enabled.

### Concurrent Stewards

Diagnostic: "Two Stewards running simultaneously on the same Daemon — do they interfere?" Per-Steward Receipt namespacing prevents collision; Daemon's per-Steward connections are multiplexed correctly. Test.

### SIGKILL Recovery

Diagnostic: "Runtime killed by SIGKILL — child cleanup, terminal restored, Receipt store consistent?" Any "no" → production operators will hit this.

### Standalone Mode

Diagnostic: "Daemon down. Runtime continues with local Receipt files. Daemon comes up. Receipts ingested?" All yes → correct. Receipts lost / Runtime stalled → standalone-operation invariant broken.

---

## Vocabulary Discipline

The vocabulary, one canonical form per concept: `Steward`, `Charter`, `Charter model`, `Charter instance`, `Frame`, `Frame definition`, `Frame ruling`, `Scope`, `Role context`, `Workspace`, `Professional`, `Charter engineer`, `Foundation Stewards`, `Gate` / `PEP`, `Tool`, `tool call`, `Action`, `Finding`, `Ruling`, `Outcome`, `Receipt`, `Snapshot`, `Task`, `Trigger`, `Adapter`, `Surface`, `Subprocess containment`, `propose`, `refine`, `intercept`, `deny`, `passthrough`.

Diagnostic: "Search the codebase for `reject`, `block`, `refuse`, `wrapper`, `shim`, `hook` (in Runtime surface), `command` (in abstract context), `log entry` (for Receipts), `result` (for Verdicts), `guard` (for evaluator)." Any hit outside kernel-mechanism context (in subprocess containment, where `trap` is the seccomp term) → discipline violation.

The pre-commit hook enforces vocabulary on staged changes; the directive evaluator catches semantic drift the regex cannot.

---

## Spec-Code Traceability

Diagnostic: "For each invariant in `docs/SPECIFICATION.md` — which test asserts it?" Missing test → invariant is a claim, not a property.

Diagnostic: "For each proto message — which Rust struct corresponds?" Missing struct → protocol message the Runtime cannot send or receive.

Diagnostic: "For each failure case in the spec — which test triggers it?" Missing test → failure case is a story, not an engineering constraint.

Diagnostic: "For each diagnostic in this checklist — which test or review step verifies it?" Missing verification → the diagnostic is decoration.
