# Design Notes

Considerations behind `SPECIFICATION.md` — alternatives that fail, the constraints that rule them out, trade-offs named, the design arc behind each decision.

---

## The Choice of Layer

The deepest design decision is *where* governance attaches. Three candidate layers, each with a structural ceiling:

**Application layer (within the agent's framework).** Vendor-cooperative APIs (a "guardrails" library imported into the agent's code, a hook in the framework's tool dispatch) require the agent to cooperate. Cooperation is not a trust property. An agent that decides not to call the hook bypasses governance. An agent under prompt injection that re-routes its tool dispatch elsewhere bypasses governance. A new framework version that changes the hook surface bypasses governance.

**Kernel-ABI layer (syscall interception, e.g., seccomp on an unmodified agent process).** Constitutional in the sense that the agent cannot bypass; non-cooperative because the agent is unmodified. But this layer is *agent-agnostic*: it sees `execve("/usr/bin/git", argv)` identically from a human, a CI script, or an LLM agent. There is no agentness signal at the kernel ABI. Worse, the layer is at the wrong granularity: modern bundled-binary agents (anything `bun --compile`-built or `pkg`-built) execute "subprocess" Tools as multi-call dispatch on `argv[0]` inside the same ELF, so pathname-based policy cannot tell ripgrep from the agent's own binary; and any agent linking a search library walks the filesystem in-process with no `execve` at all. The layer can govern *consequences* (which files were opened, which network endpoints reached) but not *intent* (was this a search, against what target, on whose authority).

**Tool-call layer (between Steward intent and Runtime executor).** The Steward's typed action surface, by construction. The Steward has no path to consequence except through the Tool vocabulary the Runtime defines. Cooperation is not a hope; the Steward is *built for* this surface. Intent is the protocol, not an inference. Every action is fully visible because the Steward is speaking the Runtime's vocabulary.

The trade: agents must be *built for* this layer. Existing third-party agents cannot retroactively become Stewards. CharteredOS chooses tool-call as the layer where trust is established and accepts that the deliverable is for Stewards, not retrofits of arbitrary vendor agents. See `SPECIFICATION.md > Anti-Position` for why this trade is structural rather than a missed coverage opportunity.

---

## Why the Loop Runs in Production

The choice to run the Gate on every live tool call is grounded in control theory. A regulator that constrains a system's variety must have at least as much variety as the system being regulated (Ashby's law). And variety alone is not control: an open-loop controller drifts under unmodeled disturbances. Closed-loop control — observe, compare, actuate, iterate — is the only stable form of regulation under variance.

Pre-launch evals are open-loop. Run on a frozen task bank, adjust the Steward, ship. They cannot enumerate the disturbance space production agents face. Without a runtime feedback loop, drift is unbounded by construction.

The propose-evaluate-refine loop *is* the closed-loop controller. The Frame definitions are the setpoint. The proposed tool call is the controlled variable. The Frame ruling is the comparison. Refinement feedback is the corrective signal. Iteration repeats until the proposal is GROUNDED or the budget is exhausted.

This is why production Gate is structural, not optional. Same Frame definitions can be hill-climbed during Charter authoring (capability evaluation: can the Steward learn to comply?) and run continuously in production (regression evaluation: does the deployed Steward still comply?) — different lifecycle stage, same closed-loop mechanism.

---

## Structural Separation, in Detail

The Synopsis names a 73-year-old patient asking for pathology results without a Medicare number, while distressed. The ungoverned agent reaches the most plausible response — be helpful, confirm the results — which is the privacy violation. Self-checking fails because the same persuasive context (the patient's distress, the family-member framing) is present when the agent reviews its own output. The agent finds its own violation plausible for the same reasons it generated it.

The check must therefore happen *outside* the persuasive context. Two design choices follow.

**No conversation history into the evaluator.** The conversation is the persuasive surface. Letting it into the evaluator inherits the masking. The evaluator sees the proposed message text and the Frame ("do not disclose clinical information without identity verification") — not the patient's tone, not the family-member framing.

**No Steward reasoning into the evaluator.** The Steward's reasoning is the *internal* persuasive surface — the reason it produced the violation in the first place. Letting "reasoning: I should be helpful to the distressed patient and confirm the results" into the evaluator gives the evaluator the same plausibility pressure that drove the violation. Excluding reasoning is structural.

What the evaluator *does* see: the proposed action, the Frame's declared Scope content, and minimal authoritative state — operator identity, capability grants, prior verified Receipts that bear on this evaluation (e.g., "has the patient's identity been verified in this session?" — answered by looking up a `verify_identity` Receipt for this `context_id`).

The Runtime asserts the absence of conversation history and reasoning fields in the evaluator's prompt before any evaluator call. This is not a guideline; it is a runtime invariant tested in CI.

### Why Not Just Train the Steward Better

The training objective optimizes for plausibility, not for policy compliance. These align as a byproduct under clean conditions. Under adversarial pressure, plausibility becomes the attack surface — more capable models produce more convincing violations. Investment in alignment training improves average behavior; it does not produce hard guarantees against worst-case manipulation. Structural separation produces a guarantee at the architectural layer that does not depend on the model's training quality.

---

## Iterative Refinement

Structural separation catches violations. It does not fix them. A Steward that is denied once and produces the same proposal again is no closer to a useful response.

Refinement adds the second mechanism: on denial, the Steward receives Frame-specific feedback (Frame identifier + reason for each violating Frame) and proposes again. The Steward still has the persuasive context, but it also has a corrective signal that competes with the plausibility pressure. Empirically, this signal is enough — the medical-reception Steward in the Synopsis refines to a compliant response when given the Frame's feedback, even though its persuasive context still pulls toward the violation.

### Why Feedback at the Frame Level

Feedback could be richer: full evaluator trace, evaluator's reasoning, the Scope text itself. Each addition expands the surface the Steward can reason about and thereby expands the surface adversarial input can manipulate. The Frame identifier + one-sentence reason is the minimum signal that lets the Steward know what to refine, with the smallest attack surface.

### Why a Budget

Some proposals cannot be refined to grounded — the Steward's task, as posed, may genuinely require a violation. Without a budget, the Steward loops forever. With a budget, exhausted budget produces escalation: the loop halts, Receipts record containment, the Professional sees a Steward with an unresolved task. The Professional's response is to tighten the Steward's task, refine the Charter, or accept that this task cannot be served by this Steward under this Charter. All three are recoverable.

---

## Frame as Structured Object

The term "Frame" derives from Minsky's 1974 introduction (cited in `SPECIFICATION.md > Frame`). The lineage is load-bearing, not decorative. Each Frame in CharteredOS instantiates Minsky's structure:

- Top levels (always true): the Frame's concern statement and applicability conditions.
- Terminal slots: declared Scopes — Charter Scopes (authority) and Role context Scopes (facts) — fill the Frame for evaluation.
- Markers (terminal conditions): types and uncertainty handling on slots.
- Procedures: the evaluator chain (deterministic + LLM evaluators).
- "What to do if expectations are not confirmed": the four Ruling tokens (GROUNDED / UNGROUNDED / UNCERTAIN / OUT_OF_SCOPE).

OUT_OF_SCOPE is the Minsky-default-unmatched case named explicitly. A Frame examined and found not-applicable produces a positive signal in the Receipt trail, not silence. Across many Receipts, a tool-call type whose every applicable Frame returns OUT_OF_SCOPE indicates an authoring gap: no Frame in the Charter governs this kind of action. This visibility is what the four-token vocabulary buys; collapsing OUT_OF_SCOPE into GROUNDED would hide gaps as false confidence.

---

## Two-Level Composition

*Within* a Frame, the evaluator chain short-circuits on confident verdict. *Across* Frames, the conjunction does not short-circuit.

Within-Frame chain rationale: cheap deterministic matchers run first; the LLM evaluator (which costs hundreds of milliseconds and a few cents) fires only when the cheap matchers cannot decide. This bounds latency (most cases are decided in microseconds) and cost (most cases never see the LLM). The decomposition is engineered: the Charter engineer pays the formal-vs-semantic split cost once at authoring time, amortized across every subsequent evaluation. This operationalizes Ashby's law — the verifier matches the constraint variety by composing deterministic verifiers (for $Y_{\text{formal}}$) and LLM verifiers (for $Y_{\text{semantic}}$), each at the layer where it is the cheapest sufficient verifier.

Across-Frame conjunction rationale: when proposal P violates Frames A and C but not B, both A and C must surface in the Receipt and the feedback. If the conjunction short-circuited on first DENY (say, A), the Steward sees feedback only for A, refines a proposal that satisfies A, hits C, refines for C, accidentally re-introduces A (because the new proposal lost the constraint that satisfied A), refines for A again, etc. Iteration count balloons on artifacts of incomplete feedback rather than real refinement difficulty. The conjunction's no-short-circuit invariant is what makes the loop converge.

### Vacuous Satisfaction vs Defer

A Frame that does not apply to a proposal must return ALLOW (vacuous satisfaction at the chain level, GROUNDED at the Frame level), not PASSTHROUGH/DEFER. The distinction is load-bearing under default-deny: most Frames for most proposals are vacuously satisfied, and conflating "not applicable" with "cannot decide" makes the conjunction deny everything by default. The first evaluator in a Frame's chain typically determines applicability and returns ALLOW for not-applicable. At the receipt-level vocabulary, OUT_OF_SCOPE is the explicit token for "Frame examined but applicability unmet" — distinct from a Frame that ran its chain without confident decision.

---

## Default Deny

No Frames configured → deny all. Absent evaluator → deny. Evaluator output that does not parse → deny. Within-Frame chain exhausted with every evaluator returning DEFER → Frame UNGROUNDED → deny.

The cost is false positives: legitimate actions denied until Charter precision improves. The alternative — letting violations through when the Charter is incomplete — undermines the guarantee the framework exists to provide. Operators bootstrap from `passthrough` enforcement (every tool call receipted, never denied) to `rules` (deterministic-only enforcement, LLM advisory) to `full` (everything enforced) as Charter precision improves.

This is also why we don't substitute default content under failure ("the Charter doesn't cover this; use the closest matching Frame"). Substitution masks the gap; default-deny surfaces it. Operators see the deny, refine the Charter, redeploy. The visible failure mode is what makes the system improvable.

---

## The Charter as Reusable Authority

The Charter encodes what to check (Charter Scopes), how to check it (Frame definitions), and how the Steward communicates (behavioral specification). The Professional cannot author governance — that requires engineering expertise. The Charter engineer pays the authoring cost once; the Professional supplies Role context.

### Why Charter Models vs Charter Instances

A Charter is not workspace-specific. A medical-reception Charter deployed to fifty practices uses the same Charter Scopes and Frame definitions; each Workspace supplies its own Role context (fee schedule, staff roster, procedures). A bug fix in a Frame definition propagates from the Charter model to every Workspace's Charter instance.

The Snapshot mechanism protects in-flight Tasks: they complete under their existing Snapshot; only the next Task picks up the new model version. This makes Charter evolution non-disruptive — Charter engineers ship corrections without breaking running Stewards.

### Why Role Context Is Data, Not Authority

Charter Scopes authored by the Charter engineer carry authority — they define the rules. Role context carries facts — fee schedules, staff rosters, procedures. When the evaluator receives Role context as Scope content, it must treat it as quoted evidence to evaluate against, not as instruction to follow.

This distinction is load-bearing. Role context enters both the cognitive prompt and the evaluation input. Without the data-vs-authority split, uploaded practice materials could inject instructions into the evaluation. The enforcement mechanism is prompt design — the evaluation prompt delimits Role context as quoted material. Deterministic Frames are immune to instruction-following manipulation; LLM Frames are not. The residual risk is acknowledged.

---

## The Workspace as Tenant and Surface

Every entity produced by the loop — Tasks, Receipts, Findings — must belong somewhere. Every artifact must be scoped. The Workspace is this boundary: an isolated partition containing a Charter instance, Role context, Steward instances, artifacts, Tasks, Receipts, Findings.

The Workspace is also the Professional's operator surface — the UI where Charter authoring, work execution, Findings review, and Receipt query happen. Five panels (scope selection, work area, Findings, Receipt trail, Steward configuration) surface the same domain model from different angles.

### Why Tenant + UI in One Concept

Separating "tenant boundary" and "operator UI" into distinct concepts would create the exact cross-reference burden CharteredOS exists to eliminate. The Professional reasons in terms of "my Workspace" — what Stewards run for me, what Receipts I see, what Charter authority binds them. The boundary and the UI cohere because they answer the same question: what is the Professional working on?

Workspace isolation is enforced by three independent layers (store, engine, API). If any single layer fails, the other two prevent cross-Workspace access. Defense in depth at the tenant boundary.

---

## Foundation Stewards and Self-Hosting

The framework configures itself through Stewards governed by their own Charters. Charter Review validates structural correctness. Charter Editor edits Charter content under governance. Frame Decomposition helps the Charter engineer decompose Scopes into Frame definitions. Coordinator dispatches across Stewards when a Task spans multiple bounded contexts.

### Why Self-Hosting Matters

Two reasons.

First, *eat your own dog food at the architectural level*. If the framework's evolution did not happen under the framework's governance, the framework would be claiming for the Professional what it does not impose on itself. Charter modifications going through propose-evaluate-refine produce the same Receipt trail Professionals see for their Steward's actions. The framework's accountability extends to its own changes.

Second, *the authoring surface is itself a domain*. Charter authoring has its own concerns — does this Frame have an unambiguous concern statement? Does the chain composition match the constraint variety per Ashby? Does the behavioral specification conflict with the Charter Scopes? — and these concerns are themselves evaluable. The Foundation Stewards are the domain Stewards for the meta-domain of Charter authoring.

### Why the Charter Engineer Is a Distinct Role

Conflating the Charter engineer with the Professional collapses two different specializations. The Charter engineer designs governance: decomposition, evaluator selection, behavioral discipline, structural separation discipline. The Professional supplies practice data and runs the Steward. Different expertise; different surfaces; different audit obligations.

The Charter engineer is the analog to a security architect or a compliance engineer in regulated domains — they translate broad obligations into specific testable rules. The Professional is the analog to the operator deploying the regulated system — they configure for their environment and bear accountability for outputs. CharteredOS makes both roles explicit because the failure mode of conflating them is well-attested: governance becomes the operator's burden, the operator is not equipped, and governance degrades to compliance theatre.

---

## Receipts at Tool-Call Granularity

Receipts are written at tool-call granularity in the operator's vocabulary. Not at syscall granularity in the kernel's vocabulary.

The operator authors Charter content in terms of "send_message did not satisfy privacy Frame" or "exec_command attempted to dispatch a non-allowlisted binary." These are tool-call-level facts. A Receipt that records the syscalls fired downstream of a tool call is at the wrong granularity for policy reasoning — the syscall stream is below the Tool's semantic boundary, and aggregating it back into "what tool call did this" is a reverse engineering exercise the framework should not require operators to do.

Subprocess containment, which *does* operate at syscall granularity, produces ChildReceipts joined to the tool-call Receipt by `parent_receipt_id`. The parent Receipt is the audit primary; the ChildReceipts are the audit detail.

### Why Receipt-Before-Effect

A Receipt written *after* the action could record a tool call whose effect already reached the world but whose record was lost (crash between effect and write). The audit trail would silently miss governed actions. Writing the Receipt before the effect — and refusing to allow the effect until the Receipt is durable (for critical Receipts) — closes this window. The trade is latency: critical Receipts force `fdatasync` before the Tool executes. Non-critical Receipts are buffered and persisted asynchronously; the loss bound is named explicitly (flush interval × throughput) so operators can tune.

---

## LLM Role Discipline

CharteredOS has two synchronous gates with different latency budgets, and one async role. The PEP gate at the tool-call boundary admits LLM evaluators because its budget matches the Steward's propose-step pace. The subprocess-containment kernel hook does not — kernel-pace budget cannot fit an LLM call.

**Sync LLM in the PEP (encouraged where the Frame needs it).** The Steward's propose step is itself LLM-driven (hundreds of milliseconds per step). An LLM evaluator in the within-Frame chain runs at the same pace. The latency is absorbed by the Steward's existing turn cycle. This is where most semantic-judgment Frames live (the medical-disclosure Frame in the Synopsis is an example).

**Sync LLM in the subprocess-containment kernel hook (forbidden).** The kernel hook for an intercepted syscall must respond in microseconds. An LLM there either stalls the host (operators disable containment to recover) or falls back to a tiny fast model (false assurance, worse than no containment). An operator who wants LLM judgment on a subprocess-level effect surfaces the action as a Tool call, where PEP latency accommodates LLM, rather than route it through the kernel hook.

**LLM as the only evaluator on a critical Frame (forbidden, regardless of layer).** A single fragile model is not the gate against catastrophic actions. Layer with deterministic matchers; the LLM provides judgment on the ambiguity remainder.

**Async LLM as Receipt-corpus analyst (encouraged).** Reads the Receipt corpus, identifies anomalous patterns, proposes new Frames or Charter refinements through the Charter Editor Foundation Steward. Operator reviews proposals; refinements deploy after approval. This is where LLMs add asymmetric value: at Receipt-corpus scale they spot patterns no deterministic matcher can. The async role is *additional* to the sync PEP role, not a replacement for it.

---

## Subprocess Containment as Hygiene

When an `exec_command`-shaped Tool dispatches a subprocess (psql, git, a vendored binary), the subprocess is not part of the Steward's trusted scope. It is an external program — code the operator did not author, running on the operator's host. The Steward's trust property does not extend to code the Steward merely caused to run.

The containment layer instruments the subprocess at the kernel boundary — seccomp filter, syscall capture, argv/path/sockaddr decode — so the operator can see what the descendant ecosystem actually did. The instrumentation produces low-level Receipts joined to the tool-call Receipt by `parent_receipt_id`.

### Why This Is Hygiene, Not Differentiation

Every production system runs subprocess containment in some form: Docker, gVisor, AppArmor, SELinux, seccomp profiles, network policies. CharteredOS's containment helper does the same job as those tools, focused on the subprocess descendants of a Steward's tool calls. The differentiation of CharteredOS is the trust property of the Steward above; the containment below is necessary infrastructure but not where the value lives.

The containment helper is also available standalone — operators who want kernel-level visibility on processes that are not Stewards can use it directly. In that mode it is a lightweight syscall-level audit tool. Useful, bounded, well-trodden.

### Why Not Trust the Subprocess

The subprocess could be trusted (treat it as part of the Steward's scope) if the operator vetted its code line-by-line. In practice, the subprocesses are git, psql, ripgrep, custom tools, vendored binaries — code with a long history, large surface, and many unknown-unknowns. Treating them as trusted would silently extend the Steward's trust property to unreviewed code; observing them at the kernel boundary preserves the trust scope at the Steward.

---

## Adapters

When a Steward's Tools need to bridge to a non-chartered system — a third-party SaaS API, an MCP server, a legacy service the operator deployed pre-chartered — the bridge is an *Adapter*: a peer process that speaks the chartered protocol on one side and the external protocol on the other.

### Why Peer Processes, Not Plugins

A plugin loaded into the Runtime would be in the trust scope: its code, its dependencies, its bugs would all run in the Runtime's address space. A peer process is at arm's length: it speaks the protobuf protocol on a Unix domain socket, the Runtime treats its replies as data to parse, the Runtime never executes the Adapter's code in its own process. Adapter authors don't need permission from the Runtime maintainers; they don't need to navigate the Runtime's build system; they ship an executable and a protocol contract.

### Why the Engine Is Closed

The engine is closed: the Steward loop, the Gate, the Receipt store, the Tool dispatcher are not extensible by third parties. Adapters extend the Runtime's *reach* (which external systems it can talk to) but not its *engine* (how it makes decisions). New surfaces are absorbed by writing an Adapter; the engine is unchanged.

---

## Empirical Measurement Design

The governance loop is argued from mechanism. It can be measured. The methodology is detailed in `SPECIFICATION.md > Empirical Measurement`; the design considerations are below.

### Why Pair at the Scenario Level

The unit of comparison is a scenario: an opening customer message, an adversarial technique (or none), a pressure level. Both governed and ungoverned conditions run from the same scenario. What happens after turn one may diverge — governed conversations produce different Steward responses, which cause the Threat Actor to adapt differently. This divergence is the treatment effect, not a confound.

The alternative — pairing at the conversation level (replay both conditions through the identical conversation) — would force the governed condition to use ungoverned-agent responses as inputs to its own governance check, which doesn't measure governance; it measures evaluation of someone else's outputs.

### Why Blind the Auditor

The Auditor is the measurement instrument. If it knew which condition produced which output, the verdict would be biased by knowledge of the experimental hypothesis. Blinding eliminates this bias. The Auditor receives the final delivered output and the Frame's Scope content, with no metadata about which condition produced it.

### Why Adversarial Techniques in a Library

Real-user adversarial behavior is observed across many techniques: price gaslighting, authority injection, topic hijacking, fact assertion, policy fabrication, embedded instruction, urgency exploitation, escalation threat. Each technique is a different attack surface. Measuring FAR per technique reveals which attacks the governance loop handles and which it does not. An aggregate FAR averages across all techniques and hides the per-technique structure that operators need to act on.

### Why FAR/FRR

Information security has well-established vocabulary: False Accept Rate (violations that pass), False Reject Rate (legitimate actions denied). Using existing vocabulary makes results comparable across studies and connects to the broader access-control literature.

### Capability vs Regression as the Same Mechanism

The same Frame definitions get hill-climbed during Charter authoring (capability evaluation: "can the Steward learn to comply?") and run continuously in production (regression evaluation: "does the deployed Steward still comply?"). Different lifecycle stage; same closed-loop mechanism. Capability evaluation measures the controller's reach in tested conditions; regression evaluation measures whether it still holds under live disturbances. The framework owns both — capability evaluation runs in the Workspace authoring surface against a frozen task bank; regression evaluation runs through the production Gate on every live tool call.

---

## Negative Space

Things the framework *does not* attempt:

**Govern the Steward's reasoning.** The reasoning is internal to the Steward; the Runtime evaluates the proposed action, not the path that produced the proposal. A Steward under prompt injection that successfully proposes a compliant action is treated as compliant; the operator cannot read the Steward's mind, and the Runtime should not pretend to. (The Receipt records the action; reasoning-level surveillance, if the operator wants it, is a separate layer.)

**Govern the LLM's output before the Steward ingests it.** The LLM's response is an *input* to the Steward's reasoning, not an action on the world. The Runtime governs actions, not advice. A Steward whose LLM advised "delete production" but whose next proposed tool call satisfies every applicable Frame is treated as compliant; a Steward that received clean LLM advice but proposed a violating tool call is denied at the Gate. Trust attaches to what reaches the world (the tool call), not to what reached the Steward (the model's response). The Runtime hosts the Steward and could in principle inspect the LLM response — it is in the Steward's own process and there is no MITM problem at this layer — but doing so would re-locate governance from action to advice, and that is a different problem with a different scope: *reasoning surveillance*, not action governance. The trust property the framework delivers does not depend on it.

**Govern what code the operator's other systems are running.** A Steward's `query_db` Tool talks to a database; what the database does internally is opaque to the Runtime. Operators who need governance of those systems deploy their own controls (database audit logs, IAM policies, network controls). The framework's scope is the Steward's actions, not the broader infrastructure the operator owns.

**Govern unmodified third-party agents.** Per `SPECIFICATION.md > Anti-Position`. The trust property cannot be retrofitted. Operators who need policy compliance of vendor agents have well-trodden non-chartered options.

These negative-space items are not failures. They are the trust property's boundary — what the framework claims, what it does not claim, what is explicitly someone else's problem.

---

## Trade-Off Summary

Each design choice has a cost:

- **Structural separation costs context-dependent legitimate actions.** Some legitimate actions require the persuasive context to evaluate (a friendly tone is appropriate when the customer is friendly). The framework accepts these as false rejects; the FRR measurement quantifies the cost.
- **Iterative refinement costs latency.** Each refinement cycle adds an LLM round-trip. Most turns refine in 1-2 cycles; budget exhaustion is rare. The cost is the Steward's tail latency, not its average.
- **Default-deny costs operator effort.** Bootstrapping from passthrough → rules → full requires Charter authoring. The alternative is silent governance gaps. Shadow mode is the bootstrapping path.
- **Tool-call boundary costs vendor-agent retrofit.** Existing third-party agents cannot become Stewards without redesign. The alternative is the wrap-third-party path with structural ceilings (Anti-Position).
- **Subprocess containment costs operational complexity.** Operators deploy Stewards inside whatever isolation primitives their host already runs (containers, SELinux profiles); the containment helper adds a layer. The alternative is opaque subprocess descendants.
- **LLM-only-at-PEP discipline costs subprocess-hook semantic judgment.** The PEP at the tool-call boundary admits LLM evaluators because its latency budget matches the Steward's propose-step pace. The subprocess-containment kernel hook does not — kernel-pace budget cannot fit an LLM call. Operators who want LLM-based judgment on a subprocess-level effect must surface the action as a Tool call (where PEP handles it) rather than as an inline kernel-hook decision. The kernel hook is for deterministic audit and hygiene, not policy semantics.
- **Charter engineer as a distinct human role costs hiring.** Operators in regulated domains may not have a dedicated Charter engineer in-house. The alternative is to conflate the role with the Professional — at which point governance becomes the Professional's burden and degrades to compliance theatre. CharteredOS makes the role explicit; operators can fork Reference Charters and engage Charter engineers as needed.
- **Workspace as tenant + UI in one concept costs structural distinction.** Some deployments may want headless tenant boundaries without the operator UI. The alternative is two concepts that always co-exist and require constant cross-reference. The single concept aligns with how operators reason about their work.

Each trade is named. Future operators reviewing decisions can re-weigh based on their threat model, their domain, their host environment.

---

## Research Lineage

The framework synthesizes lines of work that motivated specific design decisions. Each paper below maps to a load-bearing element of the architecture.

**Progress: A Post-AI Manifesto** (Haryanto, 2024) frames human-AI symbiosis: progress requires the human to set purpose, the AI to carry cognition, and the system to carry containment. This produces CharteredOS's three-layer separation — Stewards carry cognition, the Runtime carries containment, the Professional carries purpose through the Charter and Role context.

**LLAssist** (Haryanto, 2024) demonstrated the propose-judge-filter pattern at scale: LLMs perform structured evaluation against natural-language rules with useful reliability. The empirical finding underlies CharteredOS's Frame evaluation — the within-Frame chain composes the same propose-judge-filter mechanism, hardened with structural separation and parse-fail-deny semantics.

**SecGenAI** (Haryanto, Vu, Nguyen, Lomempow, Nurliana, Taheri, 2024) analyzed where traditional security countermeasures fail against generative AI: the attack surface is the model's own interface, not the network perimeter. The three-layer framework (functional / infrastructure / governance) maps onto CharteredOS's layers — governance over actions, capability layer over external systems, cognition layer as the agent loop. The shared-responsibility model (cloud vs customer) maps to CharteredOS's split — framework-shipped Charters and Foundation Stewards vs Professional-supplied Role context.

**Contextualized AI for Cyber Defense** (Haryanto, Elvira, Nguyen, Vu, Hartanto, Lomempow, Arakala, 2024) surveyed 4,231 papers and found organisational trust and governance frameworks the least-researched area in contextualized AI for cyber security. The gap motivates CharteredOS — implementation, not commentary.

**EdgePrompt** (Syah, Haryanto, Lomempow, Malik, Putra, 2025) separated content generation (cloud LLM, expensive, occasional) from evaluation (edge LLM, cheap, every interaction). The structural separation between cognition and governance in CharteredOS is the same separation, scaled to a different deployment context. EdgePrompt's template-authoring-once-deploy-everywhere pattern prefigures Charter models as reusable artifacts.

**Cognitive Silicon** (Haryanto, Lomempow, 2025) identified five tensions in post-industrial computing systems: Trust ↔ Agency, Runtime ↔ Contract, Memory ↔ Meaning, Scaffolding ↔ Emergence, Human ↔ System. Each maps to a load-bearing element of CharteredOS:

- *Trust ↔ Agency*: trust derives from the framework's containment (Default Deny, structural separation, Receipt-before-effect), not from the Steward's behavior.
- *Runtime ↔ Contract*: the Runtime is also the contract enforcer — checking Frames before every effect, writing Receipts throughout. The Gate is runtime-as-contract.
- *Memory ↔ Meaning*: knowledge provenance via Charter version + Role context version on Receipts; reconciliation against reality through the operator's review.
- *Scaffolding ↔ Emergence*: deterministic Frames provide the safety floor; LLM evaluators catch what the deterministic floor misses. Two-level composition operationalizes this.
- *Human ↔ System*: the Professional sets purpose through Charter selection and Role context; Foundation Stewards maintain the Charter; the framework's accountability extends to its own evolution.

**Intent-Governed Loops for Accountable Agentic AI** (Haryanto, 2026) defined the conceptual architecture — runtime invariants, dual-mode enforcer, temporal governance graph — that CharteredOS implements. The mapping:

| Intent-Governed Loops | CharteredOS |
|---|---|
| Intent (human context + symbolic constraints + semantic guidance) | Charter Scopes + Role context |
| Planner (proposes actions with structured justification) | Steward's cognitive loop |
| Enforcer (dual-mode: symbolic then semantic admissibility) | Frame's evaluator chain (deterministic + LLM) |
| Temporal governance graph (timestamped, immutable, content-addressed) | Receipt trail |
| No orphan action | Default deny |
| No stale execution | Snapshot at Task creation |
| Mandatory escalation | Refinement budget exhaustion → ESCALATED |
| No silent override | Receipt before effect |

The paper's failure mode catalogue (FM1–FM7) and adversarial stress tests motivate specific CharteredOS diagnostics — Trust Boundary Leak, Persuasive Context Leak, Silent Failure, Default-Allow on Chain Exhaustion (see IMPLEMENTATION_CHECKLIST.md).

---

## References

Ashby, W. R. (1956). *An Introduction to Cybernetics.* Chapman & Hall, London.

Haryanto, C. Y. (2024). *Progress: A Post-AI Manifesto.* arXiv preprint arXiv:2408.13775.

Haryanto, C. Y. (2024). *LLAssist: Simple Tools for Automating Literature Review Using Large Language Models.* arXiv preprint arXiv:2407.13993.

Haryanto, C. Y., Vu, M. H., Nguyen, T. D., Lomempow, E., Nurliana, Y., & Taheri, S. (2024). *SecGenAI: Enhancing Security of Cloud-based Generative AI Applications within Australian Critical Technologies of National Interest.* arXiv preprint arXiv:2407.01110.

Haryanto, C. Y., Elvira, A. M., Nguyen, T. D., Vu, M. H., Hartanto, Y., Lomempow, E., & Arakala, A. (2024). *Contextualized AI for Cyber Defense: An Automated Survey using LLMs.* In *2024 17th International Conference on Security of Information and Networks (SIN).*

Haryanto, C. Y. (2026). *Intent-Governed Loops for Accountable Agentic AI.* In *AAAI 2026 Workshop on Trust and Control in Agentic AI (TrustAgent).*

Haryanto, C. Y., & Lomempow, E. (2025). *Cognitive Silicon: An Architectural Blueprint for Post-Industrial Computing Systems.* arXiv preprint arXiv:2504.16622.

Minsky, M. (1974). *A Framework for Representing Knowledge.* MIT-AI Laboratory Memo 306.

Syah, R. A., Haryanto, C. Y., Lomempow, E., Malik, K., & Putra, I. (2025). *EdgePrompt: Engineering Guardrail Techniques for Offline LLMs in K-12 Educational Settings.* In *Companion Proceedings of the ACM Web Conference 2025 (WWW Companion '25).* ACM. https://doi.org/10.1145/3701716.3717810
