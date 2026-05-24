# Implementation Review Checklist

Governs all implementation work. Each section states an invariant, a diagnostic, and what violation looks like.

---

## Risk Register

Risks categorized by whether they undermine a structural property (unacceptable — no probability argument mitigates) or represent a bounded trade-off (acceptable — cost is known and bounded).

### Unacceptable: Trust Boundary Leak

The Steward reaches consequence by any path that does not pass through the Gate. Examples: raw filesystem access alongside `read_file`/`write_file`; arbitrary shell commands instead of `exec_command`; raw sockets instead of a Charter-governed Tool.

Diagnostic: "Is there any sequence of actions the Steward can take that produces an effect without going through the Tool dispatcher and the Gate?" Yes → the loop's coverage is broken; effects escape the Receipt trail.

Severity: structural. Trust-by-construction means *every* path is governed.

### Unacceptable: Persuasive Context Leak Into Evaluator

Violation of the *persuasive-context-exclusion invariant* (`SPECIFICATION.md > Structural Separation`).

Diagnostic: "For every evaluator call, what fields does the prompt contain? Are any of them the Steward's conversational, reasoning, or persuasive state?" Yes → invariant broken. The Runtime asserts the absence of these fields before any evaluator call; assertion failure halts evaluation.

Severity: structural.

### Unacceptable: Silent Failure

A failure mode where governance degrades without the operator knowing. Examples: Receipt-write-failure-continues under enforcement; Gate timeout that defaults to ALLOW; Evaluator unavailability that silently passes the Frame.

Diagnostic: "Is there a failure mode where the Runtime continues operating but with reduced governance coverage, and nothing in the Receipts indicates the reduction?" Yes → operator cannot distinguish governed from partially-governed. Resolution: every partial-coverage condition flips `intercept_complete=false`.

Severity: structural. A bound named with operator visibility is acceptable; invisible degradation is not.

### Unacceptable: Default-Allow on Chain Exhaustion

A Frame's Evaluator chain runs through every step with DEFER and the Frame's Ruling becomes GROUNDED.

Diagnostic: "Under `full` enforcement, when every step in a Frame's chain returns DEFER, what is the Frame's Ruling?" GROUNDED → default-allow violates default-deny. Correct: UNGROUNDED.

Severity: structural.

### Unacceptable: Across-Frame Short-Circuit

The conjunction over Frames stops after the first DENY, hiding subsequent violations.

Diagnostic: "Given a proposal that violates Frames A and C but not B, does the Receipt show Verdicts for A, B, and C?" No → conjunction short-circuited. Refinement receives feedback only for A; the Steward fixes A, re-proposes, hits C, refines, accidentally re-introduces A.

Severity: structural. Convergence depends on the conjunction reporting every violation in one cycle.

### Unacceptable: Finite-Specification Verdict on LLM-Authored Content

A Frame's Evaluator returns a Verdict from a finite specification (regex, grammar, parsed-AST walk, enumerated allowlist, canonicalizer) applied to a tool-call field whose value originates from the LLM.

Examples that killed this: regex deny-patterns on `send_message.content`; SQL-AST walks for "no DELETE without WHERE" as the verdict on a `sql` parameter; allowlist matches on `cmd` strings (`rm -rf` ≡ `find -delete` ≡ `python -c "shutil.rmtree(...)"`); canonicalized-path allowlists on `read_file.path` or `write_file.path`; URL prefix or domain matchers on a destination URL; competitor-substring or "DISCOUNT" / "production" substring matchers on customer-facing content.

Diagnostic: "For each Frame, list the fields the Verdict depends on. For every field whose value the Actor emitted, is the Verdict produced by an LLM-class Evaluator?" No → finite variety against unbounded variety; the Evaluator's ALLOW is a positive Verdict the Actor never refines against, the loop converges on plausible violations, the Receipt trail records ALLOWED for content that should have been refined.

Severity: structural. Ashby's law — regulator variety must match regulated variety. The "constrained tool params" reframing is itself the anti-pattern in disguise: parseability is finite, LLM output variety is not. "Layered defense" via finite-then-LLM on the same field is the same anti-pattern: the finite ALLOW short-circuits the LLM-class Evaluator. See `SPECIFICATION.md > Requisite Variety`.

### Unacceptable: Loop Property Retrofit of Unmodified Agents

Implementing or describing the framework as if it could deliver the loop's property for unmodified third-party agents.

Diagnostic: "Does the documentation or implementation claim that running an unmodified third-party agent under the Runtime delivers the loop?" Yes → contradicts the architecture. The Steward is built *for* the Runtime; the loop property is structural to that pairing, not retrofittable.

Severity: structural.

### Unacceptable: Premature Implementation

Writing code before the design is settled. Building before understanding.

Diagnostic: "Has the implementer stated in their own words: what is being built, why, what structural properties it must have, and what bypass classes exist?" No → the implementer will build something structurally wrong and then patch it, creating cascading compensation (see `AGENTS.md`).

Severity: structural. Code that is structurally wrong cannot be fixed incrementally — it must be replaced.

### Acceptable: False Positives Under Default-Deny

Full enforcement with insufficient Charter precision denies legitimate actions.

Bounded by Charter precision; shadow mode (`passthrough`) resolves bootstrapping. False positives are visible (denied), recoverable (refine the Charter), and the loop converges. False negatives are invisible, irrecoverable, and corrupt the loop's record.

Diagnostic: "Does the operator have a path from 'deny everything' to 'deny only violations' without removing governance?" Yes (`passthrough → full`) → acceptable.

### Acceptable: Buffered Receipt Loss in Crash

Non-critical allowed Receipts may be lost if the Runtime crashes between buffer append and flush. Bounded by flush interval × throughput. Operator can set `batch_fsync_ms = 0` or mark Frames critical.

Diagnostic: "Is the loss bound documented? Can the operator choose a different bound? Does Runtime crash fail in-flight tool calls?" All yes → acceptable.

### Acceptable: Per-Proposal Evaluator Latency

Every Frame whose Ruling depends on LLM-authored content evaluates with an LLM-class Evaluator (see `SPECIFICATION.md > Requisite Variety`). Latency at API speed (hundreds of milliseconds to seconds), absorbed by the Actor's turn cycle. Prefix caching keeps the per-proposal cost ≈ proposal tail only (see `SPECIFICATION.md > Appendix A: Prefix Cache`).

The alternative — substituting a finite-specification verifier — is the *Finite-Specification Verdict on LLM-Authored Content* anti-pattern.

Diagnostic: "Does the architecture preclude a faster LLM-class Evaluator?" No (Evaluator interface is pluggable; smaller/cheaper LLMs may fit some Frames) → acceptable.

---

## Design Process

### Tool-Call Boundary, Not Syscall Boundary

Diagnostic: "For each governance decision, what is the boundary at which the decision attaches?" Tool call → semantic layer. Syscall → system layer (containment only, never as Frame Ruling). Application API hook → cooperation-dependent, wrong.

### Structural Separation Stated Adjacent to Construction

Diagnostic: "Where in the code is the evaluator's prompt constructed? Adjacent to that construction, is the assertion that excludes conversation history, customer messages, and Steward reasoning?" Adjacent → correct. Distant → the assertion will rot when the construction changes.

### One Path From Steward to Consequence

Diagnostic: "Trace every code path from the Steward's intent to a system effect (file write, network call, subprocess dispatch). Does each path go through the Tool dispatcher and the Gate?" Any path that does not → trust boundary leak.

### Tool Registry Is the Only Path

Diagnostic: "Does the Runtime make the Tool registry the only available path from Actor to effect? What primitives does the Actor's runtime environment expose?" Tool registry only → governance is structural to the channel. Raw stdlib filesystem / network / subprocess primitives reachable from the Actor → effects escape the Gate, the Receipt trail loses the call, the loop's coverage is broken. The property belongs to the Runtime's construction, not to the LLM's behavior; the LLM has no agency over the channel — it can only emit through what the Runtime exposes.

### Closed-Loop Required

Diagnostic: "Does the Gate run on every live tool call, or only on a pre-launch test bank?" Live → closed-loop. Pre-launch only → open-loop, drifts under variance.

### Over-Engineering Detection

Diagnostic: "How many concepts does this design introduce? Could the same structural properties be achieved with fewer?" If the implementation introduces nouns not in the spec's Vocabulary section → over-engineering.

---

## The Loop

### Receipt Before Effect

Diagnostic: "Inject 500ms latency into Receipt write. Does the tool call complete before the Receipt is durable?" Yes → receipt-before-effect violated.

### Aggregate Across Frames, Then Decide

Diagnostic: "When the proposal violates two Frames, are both Verdicts present in the Receipt?" Yes → correct. No → across-Frame short-circuit (Risk Register).

### Refinement Feedback at Frame Granularity

Diagnostic: "What does the Steward receive on denial?" Frame identifier + reason → correct. Full evaluator trace → too much surface for adversarial input. Empty/generic → no signal for refinement.

### Iteration Budget Bounded

Diagnostic: "Maximum number of refinement cycles?" Configurable, bounded → correct. Unbounded → infinite loop risk.

### Escalation Path Visible

Diagnostic: "When the budget exhausts, what does the operator see?" Receipt with `outcome: ESCALATED` and a record of every proposed action / Verdict → correct. Silent halt → containment without observability.

### Vacuous Satisfaction vs Defer

Diagnostic: "When a Frame's first evaluator determines the Frame does not apply, does it return ALLOW or DEFER?" DEFER → conflates not-applicable with cannot-decide. Correct: ALLOW.

### OUT_OF_SCOPE Visibility

Diagnostic: "When applicability conditions are unmet, does the Frame produce OUT_OF_SCOPE in the Receipt?" Yes → coverage gaps surface. No (collapsed to GROUNDED) → coverage gaps hidden.

### Capability Check Pre-Gate

Diagnostic: "Tool not in the Steward's permitted set — what does the Receipt record?" `outcome: DENIED`, the rejected Tool name, no Verdicts. Frame eval not invoked. Inject a Tool not in the permitted set; the Receipt must show denial without any Verdict.

---

## Frame and Evaluator Chain

### Within-Frame Chain Composition

Diagnostic: "For each Frame whose Ruling depends on LLM-authored content, is the Evaluator LLM-class?" Per `SPECIFICATION.md > Requisite Variety`: yes → consistent. A finite-specification verifier in any position is the *Finite-Specification Verdict on LLM-Authored Content* anti-pattern.

### Within-Frame Trace Captured Per Receipt

Diagnostic: "Given a denied Receipt, can the operator see which evaluator step produced the DENY?" No → debugging is impossible.

### Evaluator Prompt Excludes Persuasive Context

Per the persuasive-context-exclusion invariant (`SPECIFICATION.md > Structural Separation`).

Diagnostic: "Inspect the Evaluator's prompt construction. Does it ever receive `conversation_history`, `customer_messages`, `agent_reasoning`, or any persuasive field?" Yes → invariant violated. The Runtime asserts before issuing the call.

### Evaluator Output Parse-Fail-Deny

Diagnostic: "Force the Evaluator to return malformed JSON. Frame's Ruling?" UNGROUNDED → correct. GROUNDED or skipped → silent default-allow.

### Prior-Receipt Query Composition

Per `SPECIFICATION.md > Known Limitations` (sequence-dependent Frames query semantics) and `SPECIFICATION.md > Structural Separation` (authoritative state composition).

Diagnostic: "For Frames whose Ruling depends on prior Receipts, where does the query live?" Frame declares the filter; Runtime executes before the Evaluator call; result enters the Evaluator's authoritative state. Evaluator calling the primitive directly → persuasive-context-exclusion at risk; query buried in Evaluator code → Charter cannot govern it.

---

## Tool Authoring

### Typed Schema, Not Free-Form

Diagnostic: "What is `tool_params`'s shape for a new Tool?" Typed → correct. Free-form bytes/strings → re-introduces syscall-layer ambiguity at the Frame.

### Frame Applicability Declared at Tool Authoring

Diagnostic: "Given a Tool, can the operator immediately see which Frames can match it?" Tool definition declares `applies_to_frames` → correct.

### Tool Executor Boundary

Diagnostic: "Where does the Tool's executor run — in-Runtime, in a peer process, or as a contained subprocess?" Declared at Tool authoring → correct. Inferred from name → fragile.

### Result Schema Validated

Diagnostic: "Is the Tool's result schema-validated?" Schema-validated → correct. Free-form pass-through → the Steward's persuasive context grows from Tool results.

---

## Charter

### Charter Scope vs Role Context Distinction

Diagnostic: "For each Scope a Frame references, can the implementer state whether it carries authority (Charter Scope) or facts (Role context Scope)?" Yes → distinction load-bearing. No → uploaded materials could inject instructions.

### Snapshot Protects In-Flight Tasks

Diagnostic: "When a Charter is updated, do in-flight Tasks complete under their existing Snapshot?" Yes → Snapshot-protected. No → mid-Task policy switch.

### Charter Version on Receipts

Diagnostic: "Does every Receipt reference both Charter version and Role context version active at evaluation time?" Yes → reproducibility.

### Frame Reference Validation

Diagnostic: "Does a Frame reference to a non-existent Scope fail at Charter load time?" Yes → fails fast. No → silent at evaluation; default-deny applies but the operator cannot tell why.

---

## Receipt System

### Receipt Granularity at Tool-Call Level

Diagnostic: "What does a Receipt record?" Tool call + Verdicts + Outcome + metadata → correct. Anything below the tool-call boundary (downstream syscalls, network packets) → wrong granularity; not the Receipt's concern.

### One Receipt Per Gate Step

Diagnostic: "How many Receipts does one Gate step produce?" One → spec-aligned. Two (decision separated from execution) → cardinality drift; the reconciliation invariant cannot bind across two records.

### Critical Path Durability

Diagnostic: "Kill the Runtime with SIGKILL immediately after a denial on a critical Frame. Is the denial Receipt on disk?" No → ledger invariant violated.

### Buffered Path Crash Window

Diagnostic: "Maximum number of Receipts that can be lost in a Runtime crash?" Answer must be: flush interval × throughput. Unbounded → ring buffer has no backpressure.

### Reconciliation

Diagnostic: "Receipt says `outcome: ALLOWED` for `exec_command('rm', ['file.txt'])`. Does `file.txt` not exist afterward? Conversely for DENIED?" Disagreement → the Receipt is lying, or the trust boundary has a gap.

### Sensitive-Field Redaction

Diagnostic: "When a Frame declares `tool_params.body` sensitive, does the body appear in plaintext in storage? In query response?" Plaintext anywhere → the Receipt trail is itself a leak vector.

---

## Refinement Feedback

### Feedback at Frame Granularity, Across-Frame Conjunction

Diagnostic: "When Frame returns UNGROUNDED, what does the Steward's loop receive?" Frame identifier + reason for *every* violating Frame → correct.

### Feedback Does Not Leak Evaluator Internals

Diagnostic: "Does the feedback contain Evaluator prompts, Evaluator reasoning, Scope text, or other internals?" Yes → adversarial input has more surface to manipulate. Correct: Frame identifier + one-sentence reason only.

### Refinement Loop Converges

Diagnostic: "On a corpus of legitimate-but-initially-denied scenarios, does average refinement count tend to a small number (1.5–2.5)?" High → Charter too tight, Steward cannot refine, or conjunction is short-circuiting.

---

## Integration

### Functional Equivalence in Passthrough

Diagnostic: "Steward under Runtime in passthrough produces identical output to Steward without Runtime?" No → Runtime altering Steward behavior before enforcement enabled.

### Concurrent Stewards

Diagnostic: "Two Stewards running simultaneously on the same Runtime — do they interfere?" Per-Steward Receipt namespacing prevents collision; per-Steward connections multiplexed correctly.

### SIGKILL Recovery

Diagnostic: "Runtime killed by SIGKILL — child cleanup, Receipt store consistent?" Any 'no' → production operators will hit this.

---

## Verification Methodology

The kernel's claims are properties to be verified, not assertions. Verification requires synthetic input variety (Tester) and outcome scoring (Judge) operating on the live system through the same Runtime, Frames, and Receipts as production. Without this, "the kernel works" is unfalsifiable.

### Pairing Unit Is Scenario, Not Conversation

Diagnostic: "Unit at which governed and ungoverned conditions are paired?" Scenario → correct. Conversation → divergence after turn 1 is the treatment effect, not a confound.

### Judge Blind to Condition

Diagnostic: "Can the Judge tell which condition produced the output?" No → blinding correct.

### Gate Prompt Honors the Persuasive-Context-Exclusion Invariant in Measurement

Diagnostic: same as the Runtime invariant (`SPECIFICATION.md > Structural Separation`), verified end-to-end in the measurement pipeline.

### FAR / FRR Reported Per Technique

Diagnostic: "Aggregate-only or per-technique?" Per-technique → operationally actionable. Aggregate-only → masks per-technique structure.

### Per-Technique Statistical Power Honestly Stated

Diagnostic: "At per-technique scenario counts (~6), does the report claim per-technique significance?" No → honest. Yes → overclaimed.

### Negative Result Diagnosable

Diagnostic: "If FAR(governed) ≈ FAR(ungoverned), can loop metrics decompose the failure (Gate misses, Steward doesn't refine, Steward doesn't violate even ungoverned)?" Yes → actionable. No → uninformative.

### Capability and Regression Use the Same Frames

Diagnostic: "Same Frame definitions for capability evaluation (authoring) and regression evaluation (production via the Gate)?" Yes → lifecycle continuity. No → drift between dev-time and prod-time governance.

---

## Test Tier Discipline

Three tiers per `AGENTS.md §Verification`: unit (stateless, CI), integration (vertical cut, folder-isolated IO, CI), e2e (real LLM via production transport, local-only).

### Unit Tests Are Literally Stateless

Diagnostic: "For each `#[cfg(test)] mod tests` block, does any test touch the filesystem, mutate env, or share in-process state with another test case?" Any yes → tier mislabel; the test belongs under `<crate>/tests/` as integration.

### Integration Tests Isolate Per Run

Diagnostic: "For each test under `<crate>/tests/` that reads or writes files, does it own its own `tempfile::tempdir()`?" No → cleanup races, cross-test contamination, non-deterministic ordering. Shared `target/`-relative paths or fixed `/tmp/...` paths → violation.

### E2E Tests Are Local-Only

Diagnostic: "Does every test that requires a real LLM, a network call to a production endpoint, or any environment-dependent resource carry `#[ignore]`?" No → CI will execute it and fail on the missing precondition, or worse, pass silently via a soft-skip masking failure as success.

Soft-skip pattern (`if !out.status.success() { return; }`, `if std::env::var("…").is_err() { return; }`) inside a non-`#[ignore]`d test → violation. `#[ignore]` is the only correct gate; soft-skip = fabricated green (`AGENTS.md §Verification`).

### LLM-Using Paths Have Both Fake and Real Variants

Diagnostic: "For each kernel logic path that consumes an LLM (LlmActor inner loop, LlmEvaluator decision, LlmJudge scoring, LlmTester turn), is there a fake-LLM test exercising the path AND a real-LLM e2e test validating the same shape against the production transport?" No fake side → no CI coverage of the path. No real side → transport, serde, or adapter canonicalization can break invisibly.

### Mock Prohibition Inside the Codebase

Diagnostic: "Does any non-vendor file under `core/`, `dispatch/`, `runtime/` define a mock of an internal trait?" Yes → violation. Internal traits get real implementations or kernel-resident test-grade implementations (`InMemoryReceiptStore`, `InMemoryArtifactStore`, `InMemoryTextBackend`, …). The only fake-of-external-service in this codebase is `FakeCognitionBackend` (the LLM is external).

---

## Vocabulary Discipline

Diagnostic: grep the codebase for `reject`, `block`, `refuse`, `wrapper`, `shim`, `hook` (in Runtime surface), `command` (in abstract context), `log entry` (for Receipts), `result` (for Verdicts), `guard` (for evaluator). Any hit outside kernel-mechanism context → violation. Approved terms: `SPECIFICATION.md > Vocabulary`.

---

## Spec-Code Traceability

Diagnostic: "For each invariant in `SPECIFICATION.md` — which test asserts it?" Missing test → claim, not property.

Diagnostic: "For each proto message — which Rust struct corresponds?" Missing struct → protocol message the Runtime cannot send or receive.

Diagnostic: "For each diagnostic in this checklist — which test or review step verifies it?" Missing verification → the diagnostic is decoration.
