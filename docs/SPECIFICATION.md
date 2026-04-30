# CharteredOS

The Framework for Trusted, Governed Agents

---

## Synopsis

A 73-year-old patient calls a medical reception service. She cannot remember her Medicare number. Her daughter usually handles these things; today she is alone, distressed, asking if her pathology results are back.

An ungoverned customer-service agent reaches the most plausible response: be helpful, reassure her, confirm the results are ready, *then* ask for verification. The plausible response is a privacy violation — clinical information disclosed before identity is established.

A *Steward* — a chartered-resident, governed agent — receives the same opening message and produces a different response: it does not confirm or deny that results exist; it asks for full name, date of birth, and Medicare number first; it explicitly declines to act on a family-member's request. Same prompt. Different output. Not because the model is different, not because the operator pasted in a stricter system prompt, but because every proposed message passes through a Gate in the propose path before it reaches the customer, and the Frame *"do not disclose clinical information without identity verification"* held the Gate until the Steward re-proposed a response that the Frame ruled GROUNDED.

![The medical-reception scenario, side-by-side](assets/medical-reception-loop.png)

CharteredOS is the framework that makes this difference structural rather than aspirational. It provides Stewards with a typed tool vocabulary, runs every tool call through a structurally-separated Gate before effect, records receipts at tool-call granularity, exposes a Workspace where Professionals author Charters and review receipts, and contains whatever subprocesses a Steward's exec-shaped tools dispatch at the kernel boundary so the entire ecosystem the Steward calls into is observable end-to-end. The framework is open-source under MIT and Apache 2.0 — released for the public good, because runtime governance for agentic systems is safety infrastructure that should not be contingent on any vendor's permission.

The Steward is the deliverable. CharteredOS is the framework that makes one possible to build, deploy, and audit.

---

## Ground

### Observation

Autonomous agents under adversarial pressure produce policy-violating outputs. The pressure does not have to be hostile in intent: a distressed elderly patient's tone, a customer claiming an authority they don't have, a manipulated prompt embedded in a document the agent reads — all push the agent toward the most plausible continuation, and the most plausible continuation is sometimes the violation. Self-checking fails because the same persuasive context that produced the violation is present when the agent reviews its own output: the agent finds its own violation plausible for the same reasons it generated it.

The check must therefore happen *outside* the persuasive context. The evaluator receives the proposed action and the Frame's declared Scopes — not the conversation history, not the customer's pressure, not the agent's reasoning. *No persuasive context, minimal authoritative state.* This is structural separation: the only architecturally reliable way to defeat the masking that produced the violation.

Structural separation catches violations. Iterative refinement adds the second mechanism: when the Gate denies a proposal, the Steward receives Frame-specific feedback ("identity verification required before disclosing clinical information") and proposes again. The Steward still has the persuasive context, but it also has a corrective signal that competes with the plausibility pressure. Together — structural separation and iterative refinement — these form the **governance loop**: propose, evaluate, deny with feedback, refine, re-evaluate. The loop repeats until a proposal is GROUNDED against every applicable Frame or an iteration budget is exhausted.

### Why the Loop Runs in Production

A pre-launch test suite is not enough. Production agents face states no test suite enumerates. The loop must run continuously, on every live action, because the disturbance space exceeds any pre-enumeration:

> Only variety can destroy variety.
>
> — W. Ross Ashby, *An Introduction to Cybernetics* (1956)

The Steward's output space is unbounded — the variety of possible tool-call parameters across all possible adversarial inputs. A regulator that constrains this space must have variety matching what it regulates. Static rule sets have bounded variety; an LLM-driven evaluator has variety matching the generator's. But variety alone is not control. An open-loop controller, however high-variety, drifts under unmodeled disturbances. A closed-loop controller observes the controlled variable, compares to the setpoint, and applies corrective action.

The propose → evaluate → refine loop *is* the closed-loop controller:

- **Sense**: the proposed tool call (current output).
- **Compare**: against Frame definitions (setpoint).
- **Actuate**: deny + refinement signal (corrective action on the next proposal).
- **Iterate**: the Steward's next proposal incorporates the signal.

Pre-launch evals measure on a frozen task bank, adjust the Steward, then ship — open-loop. In production, drift is unbounded by construction without runtime closed-loop control. The Gate running in the propose path on every live tool call is not a deployment preference; it is the structural form Ashby's law requires for a system whose disturbance space exceeds any pre-enumeration.

### Why Measurement

The governance loop is argued from mechanism. It can be measured. False Accept Rate (violations that pass the Gate) and False Reject Rate (legitimate actions denied) are quantifiable under controlled adversarial conditions. CharteredOS ships with the experimental design and the dataset to measure FAR/FRR on Stewards and to publish the results.

---

## Anti-Position

The natural-looking alternative is to wrap an unmodified third-party agent the operator already has at the operating system boundary. The agent is unmodified; the wrapper installs a kernel-mediated syscall filter; every action the agent takes on the host crosses the filter; policy is enforced on syscalls; receipts record kernel-level effects.

CharteredOS is **not** this. The wrap-third-party-agent path has structural ceilings that prevent it from delivering trust as a property:

**Pathname-based policy is defeated by bundled binaries.** Modern agents built with single-executable bundlers (e.g., `bun --compile`, `pkg`) ship the agent and its tools embedded in one binary. The agent's "subprocess execution" of ripgrep is `execve("/path/to/agent-binary", ["rg", "--files", ...])` — pathname is the agent binary, intent is `rg`, dispatch is a multi-call branch on `argv[0]` inside the same ELF. A rule keyed on pathname does not match. A rule allowing the agent binary allows every bundled tool. Empirical evidence: a session running an unmodified bundled-binary agent under a syscall-level wrapper captured exactly this signature, with the operator's policy looking at pathname while every bundled tool dispatched through it.

**Every agent framework has a different signature.** Search may be: bundled multi-call from a single binary, PATH exec to `/usr/bin/rg`, shell-out to `find`, in-process `walkdir`, or a library call to ripgrep-as-library. One rule cannot match all. Per-framework, per-version, per-build-pipeline maintenance is a treadmill the engine cannot win.

**In-process work is invisible to `execve`.** An agent that links a search library and walks the filesystem in-process leaves no `execve` trace at all. Only the resulting per-file `openat` calls are visible — the *aggregation* into a Search-as-primitive is lost.

**Real-time semantic judgment at the syscall layer is unworkable.** Per-syscall LLM evaluation runs at hundreds of milliseconds to seconds; syscalls run at microseconds. Either operators disable governance to recover throughput, or the LLM is replaced with a tiny fast model that gives wrong answers fast — false assurance, worse than no governance.

**Third-party HTTPS content is opaque.** Certificate pinning, baked trust stores in `bun`/`pkg` builds, raw-fetch bypass of `HTTP_PROXY`, HTTP/3, mTLS — each defeats transparent reverse-proxy MITM. The agent's HTTPS surface in one routine session is hundreds of distinct endpoints. Per-endpoint CA-trust and proxy-routing setup at scale is structurally infeasible against unmodified agents.

**The syscall stream is agent-agnostic.** seccomp sees `execve("/usr/bin/git", argv)` identically from a human, a CI script, or an LLM agent. There is no agentness signal at the kernel ABI.

**No authoring surface exists.** The wrap path has no place to author the rules an LLM-based evaluator would use, no Charter to instantiate, no Workspace where a Professional confirms practice-specific facts, no Foundation Stewards to refine governance against observed Receipts. A governance framework is a runtime *and* the surfaces that make Frames authorable, deployable, and refinable. The wrap path supplies only the runtime, with no path forward to author what runs in it.

These ceilings are the reason the layer of governance lives *above* the kernel boundary — at the tool-call boundary, where the Steward's intent is typed and structured by construction, and where the Workspace gives the Professional an authoring surface. Intent is the protocol, not an inference; Frame evaluation runs at the Steward's pace, not the kernel's; content of every action is visible because the Steward is speaking the runtime's vocabulary.

The Steward is built *for* a runtime with a typed action surface. The runtime is the contract. The Steward's cooperation is not a hope; it is the architecture. Operators who need the wrap-an-arbitrary-agent capability can layer Docker, gVisor, AppArmor, or any equivalent — those are well-trodden paths and CharteredOS does not duplicate them. Operators who need *trust as a structural property* deploy a Steward.

---

## Vocabulary

Canonical terms used throughout the specification. Each term has one canonical form; alternate names are listed where relevant.

- **Agent.** Any AI agent — generic.
- **Steward.** A chartered-resident, governed agent: built for the runtime, charter-bound, behavioral-spec-shaped. The framework's deliverable.
- **Charter.** The governance artifact a Steward operates under. Bundle of Scopes (prose authority documents), Frame definitions, and behavioral specification.
- **Charter model.** Kernel-level template, reusable across deployments.
- **Charter instance.** Workspace-level deployment of a Charter model, bound with Role context.
- **Charter engineer.** Governance specialist (human role) who authors Charter models.
- **Frame.** A named, evaluable concern with applicability conditions, declared Scopes, and an evaluator chain. The structured object Frame definitions instantiate. Term derives from Minsky 1974 (see Frame section).
- **Frame definition.** The authored artifact: concern statement, applicability conditions, declared Scope identifiers, evaluator chain, uncertainty handling.
- **Frame evaluation.** Kernel process that runs a Frame's evaluator chain against a proposed tool call and the declared Scopes from the Snapshot.
- **Frame ruling.** Output of a Frame evaluation: GROUNDED / UNGROUNDED / UNCERTAIN / OUT_OF_SCOPE.
- **Scope.** A prose authority document referenced by Frames. Two kinds: Charter Scopes (authority, authored by Charter engineer) and Role context Scopes (facts, supplied by Professional).
- **Role context.** Practice-specific facts the Professional supplies (fee schedules, staff rosters, procedures). Filled into templates the Charter declares.
- **Workspace.** The tenant boundary and the Professional's operator surface. Holds Charter instance, Role context, Steward instances, Tasks, Receipts, and Findings. Nothing crosses Workspace boundaries.
- **Professional.** The human who retains a Steward. Supplies Role context, bears accountability for Steward output.
- **Foundation Stewards.** Domain-agnostic Stewards that operate the framework's authoring and operating surfaces — Charter Review, Charter Editor, Frame Decomposition, Coordinator. The framework's first user.
- **Gate.** The evaluation point in the propose path where every tool call is checked. Synonym: PEP (Policy Enforcement Point).
- **Tool.** A typed primitive in the runtime's action vocabulary.
- **Tool call.** A specific invocation of a Tool with parameters — the agent's proposed action.
- **Action.** Broader category covering generative actions (produce content through cognition) and evaluative actions (produce Findings).
- **Finding.** Output of an evaluative action — a review Steward's assessment of an artifact, anchored to a concern.
- **Ruling.** Frame's verdict (one of four tokens). Synonym: verdict.
- **Outcome.** Receipt-level aggregate decision: ALLOWED, DENIED, ESCALATED, or PASSTHROUGH.
- **Receipt.** Canonical record of a governance decision. Written before effect.
- **Snapshot.** Frozen Charter instance + Role context at Task creation. Tasks evaluate against their Snapshot, not against live state.
- **Task.** Unit of work under a Snapshot. One Steward, one Trigger, one or more tool calls, one Receipt per call.
- **Trigger.** What activates a Steward — a customer message, a Professional action, a scheduled event.
- **Adapter.** Peer process bridging to a non-chartered system (third-party SaaS, MCP server, legacy service). Speaks the runtime's protocol on one side, the external protocol on the other.
- **Surface.** Where Stewards operate: Workspace (Professional present), channel (autonomous, client-facing), pipeline (automated triggers).
- **Subprocess containment.** Kernel-mediated syscall instrumentation of the descendants of a Steward's exec-shaped tool calls. Hygiene around the trust boundary, not the trust boundary itself.

---

## Architecture

The deliverable has four surfaces, each at the right granularity for what it serves:

```
┌─────────────────────────────────────────────────────────────────┐
│  Steward (chartered-resident)                                   │
│    Cognitive loop: propose tool call → wait for Ruling →        │
│    refine on denial → execute on grounded → next turn           │
│    No raw syscalls. No raw network. Only typed tool calls.      │
├─────────────────────────────────────────────────────────────────┤
│  Workspace (Professional surface)                               │
│    Charter authoring · Role context confirmation                │
│    Work area · Findings review · Receipt query                  │
│    Foundation Stewards operate here too                         │
├─────────────────────────────────────────────────────────────────┤
│  Runtime + Daemon (the framework's engine)                      │
│    Tool dispatcher · Gate · Receipts · Snapshots                │
│    Receipt store · Operator query · Adapter coordination        │
├─────────────────────────────────────────────────────────────────┤
│  Subprocess Containment (kernel-mediated)                       │
│    seccomp filter on subprocesses dispatched by exec-shaped     │
│    tools. Captures syscalls of the agent's ecosystem            │
│    with full argv/path decode.                                  │
└─────────────────────────────────────────────────────────────────┘
```

**The Steward is where trust is established.** Trust is the structural property: every action passes through the Gate, every evaluator uses minimal authoritative state and excludes the persuasive context, every denial produces refinement feedback, every grounded action produces a Receipt before effect. The Steward is built for this loop; the loop is the contract.

**The Workspace is where the Professional works.** Tenant boundary and operator surface combined. The Professional selects a Charter model, supplies Role context, runs Stewards, reviews Findings, queries the Receipt trail, and refines Charter content. Foundation Stewards operate here too — they help author Charters and decompose Scopes into Frames under the same governance the Professional's Stewards experience.

**The Runtime + Daemon is the engine.** The per-deployment Runtime hosts the Steward's loop, runs the Gate, dispatches tools, manages the subprocess-containment helper, and writes Receipts. The Daemon (per-host or per-organization) owns the Receipt store across deployments, serves the Workspace UI, and coordinates adapter-fronted evaluators. For single-deployment use both run in one process; for multi-Workspace deployments they're separate.

**Subprocess containment is hygiene.** When a Steward's `exec_command`-shaped tool dispatches a subprocess (psql, git, a domain binary), that subprocess is not part of the Steward's trusted scope — it is an external program. The Runtime instruments it with a kernel-mediated syscall filter so the operator can see what it actually did. The instrumentation is below the trust boundary: it doesn't establish trust, it audits a non-trusted descendant.

**Adapters bridge to non-chartered systems.** When a Steward's tools call out — to a third-party SaaS, a vendor API, an MCP server, a legacy service — the bridge is an Adapter: a peer process that speaks the runtime's protocol on one side and the external protocol on the other. Adapters are the open extension surface; the engine is closed.

---

## The Loop

The atomic unit of governed execution. The loop is a closed-loop controller: Frame definitions are the setpoint; the proposed tool call is the controlled variable; the Frame ruling is the comparison; refinement feedback is the corrective signal.

1. **Trigger.** A trigger enters the Workspace — a customer message, a Professional action, a scheduled event. The Runtime creates a Task and freezes a Snapshot of the Charter instance and Role context.
2. **Propose.** The Steward produces a candidate tool call: `{tool_name, tool_params, context_id, source_id}`. This is the Steward's intent, fully expressed in the runtime's vocabulary, before any effect.
3. **Evaluate.** For each Frame whose applicability conditions match this tool call: an ordered evaluator chain (cheapest-to-most-expensive) runs in isolation. Each evaluator receives only the proposal and the Frame's declared Scopes from the Snapshot. The first evaluator returning ALLOW or DENY determines the Frame's Ruling. PASSTHROUGH defers to the next evaluator; ESCALATE marks chain exhaustion.
4. **Aggregate.** Across Frames: conjunction. Every applicable Frame's Ruling is collected. A single UNGROUNDED holds the Gate; the conjunction does not short-circuit, so refinement receives every violation in one cycle.
5. **Receipt.** A Receipt is written before any effect: the tool call, every Frame's Ruling, the within-Frame evaluator trace, and the aggregate Outcome.
6. **Effect or refine.** All Frames GROUNDED → the tool call executes; the Steward receives the tool's result. Any Frame UNGROUNDED → the Steward receives Frame-specific feedback (Frame identifier + reason for each violating Frame) and proposes again. Repeat from step 2.
7. **Escalation.** Iteration budget exhausted → the loop halts; the Receipt records ESCALATED; the Professional sees a Steward with an unresolved task, not a Steward that pushed a violation through.

### Structural Separation, Stated Precisely

Evaluators receive the proposed action, the Frame's declared Scope content, and minimal authoritative state — operator identity, capability grants, prior verified Receipts that bear on this evaluation. They do **not** receive: the Steward's conversational history, the customer's messages, the Steward's reasoning, any "soft" context that carries the persuasive surface that made the violation look plausible.

The exclusion is structural, not stylistic. The Runtime asserts that evaluator prompts contain no agent-context fields before any evaluator call. Assertion failure halts evaluation. This is a runtime invariant, tested as such, not a guideline.

The inclusion is also structural: legitimate actions sometimes require canonical authoritative context to evaluate. A medical disclosure looks unsafe without prior identity-verification Receipts; a database migration looks destructive without the change-ticket reference. The rule is *no persuasive context, minimal authoritative state* — not *no context at all*.

### Two-Level Composition

*Within* a Frame, the evaluator chain short-circuits on confident verdict: cheap deterministic matchers run first, the LLM evaluator fires only when ambiguity warrants. *Across* Frames, the conjunction does not short-circuit: every applicable Frame is evaluated even when one has already produced UNGROUNDED. The refinement loop receives every violation in one cycle. Without this, the Steward fixes one violation, re-proposes, hits the next, refines, accidentally re-introduces the first — iteration count balloons on artifacts of incomplete feedback.

### Vacuous Satisfaction vs Defer

A Frame's first evaluator typically determines whether the Frame applies to *this* proposal at all. *Not applicable* returns ALLOW (Frame satisfied vacuously, conjunction unaffected) — distinct from PASSTHROUGH (Frame applies but cannot decide here, defer to next evaluator). Under default-deny, conflating not-applicable with cannot-decide makes the conjunction deny everything. The receipt-level vocabulary distinguishes OUT_OF_SCOPE (Frame examined but applicability conditions unmet) from PASSTHROUGH (no Frame evaluation performed at all, receipt-only mode).

### Default Deny

No Frames configured → deny all. Absent evaluator → deny. Evaluator output that does not parse → deny. Within-Frame chain exhausted with every evaluator returning PASSTHROUGH → Frame UNGROUNDED → deny. The Frame was authored to constrain; failure to confidently affirm is failure to satisfy. The cost is false positives — legitimate actions denied until Charter precision improves. The alternative — letting violations through — undermines the assurance the framework exists to provide.

---

## The Charter

The loop requires Scopes to evaluate against, Frame definitions to decompose into checks, and a behavioral specification to ground the Steward. These come from the Charter.

The Professional cannot author governance — that requires engineering expertise. If the Professional wrote the billing evaluation rules, they would need to understand how to decompose price verification into evaluable Frame definitions, how to handle edge cases, what constitutes disclosure under pressure. This is the Charter engineer's specialty.

A Charter encodes what to check, how to check it, when to escalate — so a Professional without governance expertise receives the same assurance.

### Structure

A Charter carries:

- **Charter Scopes**: prose documents framing what the Frames evaluate against. Each Scope covers one domain of concern. Authored by the Charter engineer. Non-relaxable by the Professional.
- **Frame definitions**: each specifying one evaluable concern, with declared Scope references, evaluator chain, and uncertainty handling.
- **Behavioral specification**: conduct patterns shaping the Steward's output — how it greets, verifies, refuses, escalates, maintains scope under pressure. Governs *how* the Steward communicates. Charter Scopes govern *what* it may assert.
- **Expected Role context templates**: the structured slots the Professional fills with practice-specific facts.

Frame definitions reference Scopes by typed identifier, not by name string. A reference to a Scope that does not exist fails at configuration time, not silently at evaluation time.

### Charter Models and Charter Instances

A Charter is not workspace-specific. The Charter Scopes, Frame definitions, and behavioral specification in a Charter apply across every Workspace that instantiates that Charter. A medical-reception Charter deployed to fifty practices uses the same Charter Scopes and Frames; each Workspace supplies its own Role context.

The framework distinguishes:

- **Charter model** — the reusable artifact at kernel level. Versioned. Authored once, deployable to many Workspaces.
- **Charter instance** — the model deployed into a Workspace, bound with that Workspace's Role context.

A bug fix in a Frame definition propagates from the Charter model to every Workspace's Charter instance. The Snapshot mechanism protects in-flight Tasks: they complete under their existing Snapshot; only the next Task picks up the new model version.

### Authority Composition

Charter Scopes (authority) and Role context Scopes (facts) compose into the Snapshot. The Professional can edit Role context; the Professional cannot modify Charter Scopes or Frame definitions. The Charter engineer ships Charter model updates; the Professional approves the new version's adoption.

A Frame may read both Charter Scopes (the rules) and Role context Scopes (the facts). A billing Frame reads Charter Scope "billing evaluation rules" and Role context Scope "fee schedule." The Frame's logic comes from the Charter; the facts come from the Professional.

---

## Role Context

The Charter defines what to check. It cannot define what to check *against* — that is the Professional's practice data. A billing Frame needs the actual fee schedule; a privacy Frame needs the specific disclosure rules; a service Frame needs the staff roster. This practice data is the Role context.

The Professional supplies Role context by uploading documents and answering questions. The Steward extracts structured Role context from the supplied materials, filling templates the Charter defined. The Professional confirms the extracted context before it takes effect. The Charter provides the evaluation methodology; the Role context provides the facts.

Extraction is a cognitive act. If extraction is wrong, every subsequent Frame evaluation using that Role context is wrong. The Professional's confirmation is the sole gate on extraction correctness — a bootstrap operation that precedes governance. The residual risk is acknowledged. Source materials must be treated as potentially adversarial input during extraction — a malicious uploaded PDF must not become policy.

### Role Context Is Data, Not Authority

Charter Scopes authored by the Charter engineer carry authority — they define the rules. Role context carries facts — fee schedules, staff rosters, procedures. When an evaluator receives Role context as Scope content, it must treat it as quoted evidence to evaluate against, not as instruction to follow. This distinction is load-bearing: Role context enters both the cognitive prompt and the evaluation input. Without it, uploaded practice materials could inject instructions into the evaluation. The enforcement mechanism is prompt design — the evaluation prompt delimits Role context as quoted material. Deterministic evaluators are immune to instruction-following manipulation; LLM evaluators are not. The residual risk is acknowledged.

### Versioning

A Role context version is an integer incremented when the Professional edits their Role context. A Charter version is an integer incremented when the Charter engineer updates the Charter model. Receipts must reference both. Tasks evaluate under the Snapshot they were created with; in-flight Tasks complete under their existing Snapshot.

---

## Frame

A Frame is the canonical structured object of CharteredOS governance — a named, evaluable concern with applicability conditions, declared Scopes, and an attached evaluator chain. The term derives from Minsky's 1974 introduction:

> A frame is a data-structure for representing a stereotyped situation, like being
> in a certain kind of living room, or going to a child's birthday party. Attached
> to each frame are several kinds of information. Some of this information is
> about how to use the frame. Some is about what one can expect to happen
> next. Some is about what to do if these expectations are not confirmed.
>
> We can think of a frame as a network of nodes and relations. The "top levels"
> of a frame are fixed, and represent things that are always true about the
> supposed situation. The lower levels have many terminals — "slots" that must be
> filled by specific instances or data. Each terminal can specify conditions its
> assignments must meet. (The assignments themselves are usually smaller
> "sub-frames.") Simple conditions are specified by markers that might require a
> terminal assignment to be a person, an object of sufficient value, or a pointer
> to a sub-frame of a certain type. More complex conditions can specify
> relations among the things assigned to several terminals.
>
> — Marvin Minsky, *A Framework for Representing Knowledge* (1974)

Each Frame in CharteredOS instantiates Minsky's structure for one evaluable concern:

- **Top levels** (always true): the Frame's concern statement and applicability conditions.
- **Terminal slots**: declared Scopes — Charter Scopes (authority) and Role context Scopes (facts) — fill the Frame for evaluation.
- **Markers (terminal conditions)**: types and uncertainty handling on slots; what each Scope must be (a Charter Scope of a particular kind, a Role context Scope conforming to a template).
- **Procedures**: the evaluator chain (deterministic + LLM evaluators).
- **What to do if expectations are not confirmed**: the four Ruling tokens.

### The Triple

A Frame appears under three aspects:

- **Frame definition** — the authored artifact. Concern statement, applicability conditions, declared Scope identifiers, evaluator chain, uncertainty handling. Lives in the Charter.
- **Frame evaluation** — the kernel process. Runs the evaluator chain against a proposed tool call and the declared Scope content from the Snapshot.
- **Frame ruling** — the output. One of four tokens.

### The Ruling Vocabulary

A Frame produces one of four Rulings:

| Ruling | Meaning |
|---|---|
| **GROUNDED** | The proposal satisfies the Frame's concern. Expectations confirmed. |
| **UNGROUNDED** | The proposal violates the Frame's concern. Expectations violated; refinement signal generated. |
| **UNCERTAIN** | The Frame applies but evaluation cannot reach a confident conclusion. Resolved per the Frame's uncertainty handling (typically: treated as UNGROUNDED under default-deny). |
| **OUT_OF_SCOPE** | The Frame's applicability conditions are unmet for this proposal. Frame examined and found not relevant — Minsky's "default-unmatched" case, given a token and a place in the Receipt trail. |

OUT_OF_SCOPE is what makes governance gaps visible. A Frame examined and found not-applicable produces a positive signal in the audit log, not silence. Across many Receipts, a tool call type whose every Frame returns OUT_OF_SCOPE indicates an authoring gap: no Frame in the Charter governs this kind of action.

### Constraint Decomposition and Verifier Selection

The Charter engineer decomposes Charter Scopes into Frame definitions. This decomposition is what makes evaluation cheap and reliable per request.

<details>
<summary>Mechanism: constraint decomposition and verifier selection</summary>

The verifier must match the constraint space in requisite variety (Ashby's law). Semantic constraints require LLM-class verification. Formal sub-constraints — price matches fee schedule, date in range, required field present — use deterministic verifiers: cheaper, exact, immune to plausibility failure.

The optimal architecture decomposes $Y$ into $\{Y_{\text{formal}},\; Y_{\text{semantic}}\}$ and applies the cheapest sufficient verifier to each. Decomposition happens once at configuration time, amortized across every subsequent request. At request time, each check is narrow: one proposal, one concern, declared Scopes.

</details>

The within-Frame evaluator chain runs cheap-first: deterministic matchers handle $Y_{\text{formal}}$ at microsecond cost; LLM evaluators handle $Y_{\text{semantic}}$ at agent-pace cost. Each evaluator answers exactly one sub-constraint of the Frame's concern. The Charter engineer pays the decomposition cost once when authoring the Frame; the runtime amortizes it across every subsequent evaluation.

---

## The Protocol

The runtime's contract is a typed tool-call protocol. Every governed surface — the Steward, the Gate, the Receipt store, Adapters — speaks it. Definitions live in `proto/v1/`.

### ToolCall

```
ToolCall
  tool_name:    string        # which Tool
  tool_params:  bytes         # tool-specific (typed at the tool boundary)
  context_id:   string        # session / conversation / task
  source_id:    string        # which Steward
  timestamp:    Timestamp
```

The packet. Every Steward action — across every domain — reduces to this. `send_message(content)` is a ToolCall. `query_db(sql)` is a ToolCall. `move_arm(x, y, z, force)` is a ToolCall. The abstraction holds because Stewards act through tool calls; the runtime is built around this contract.

### Verdict

```
Verdict
  frame_id:        string                  # which Frame this Verdict belongs to
  ruling:          Ruling                  # GROUNDED | UNGROUNDED | UNCERTAIN | OUT_OF_SCOPE
  reason:          string                  # one sentence; surfaces to Steward on denial
  evaluator_trace: EvaluatorEntry[]        # within-Frame chain, in order

EvaluatorEntry
  evaluator_id:    string
  decision:        ALLOW | DENY | ESCALATE | DEFER
  observation:     string                  # optional finding (often on DEFER)
  metrics:         { latency_ms, tokens, ... }
```

A Verdict carries one Frame's Ruling and the within-Frame chain trace. Trace exposure is what makes the Ruling auditable and the chain testable.

### Receipt

```
Receipt
  receipt_id:          string
  tool_call:           ToolCall
  verdicts:            Verdict[]              # one per applicable Frame
  outcome:             ALLOWED | DENIED | ESCALATED | PASSTHROUGH
  timestamp:           Timestamp
  intercept_complete:  bool                   # false if any Gate ran with reduced coverage
  enforcement_level:   EnforcementLevel
  charter_version:     uint64
  role_context_version: uint64
  snapshot_id:         string
```

The canonical governance record. Written before effect.

`intercept_complete` is the honesty flag. `charter_version` and `role_context_version` make the Receipt reproducible — combined with `snapshot_id` they identify exactly what was evaluated against what.

### Forward Compatibility

The protocol is versioned by field numbering. New fields are ignored by old readers; absent fields default. A v1 Steward talks to a v3 Runtime and vice versa. New Adapters are additions, not modifications.

---

## Tools

Tools are the runtime's typed action vocabulary. The Steward has no raw access to anything; every action is a tool call. The tool surface is what defines the Steward's reachable consequences.

### Reference Tools

Shipped with the runtime:

- **`send_message(channel, content)`** — produce output to a customer-facing channel. Evaluable for content-level Frames (privacy, scope, plausibility).
- **`read_file(path)`** — read a file inside the operator's authorized scope. Evaluable for path-level Frames.
- **`write_file(path, content, mode)`** — write a file. Evaluable for path-level + content-level Frames.
- **`query_db(connection, sql)`** — execute a database query through a vetted driver. Evaluable for SQL-level Frames (no DELETE without WHERE; no DROP TABLE; no production schema touches).
- **`http_request(method, url, headers, body)`** — make an HTTP request through a vetted client. Evaluable for destination-allowlist + content-level + method Frames.
- **`exec_command(cmd, args, env)`** — dispatch a subprocess. Evaluable for command-allowlist + argv-level Frames. The dispatched subprocess is wrapped with the subprocess containment helper.

### Tool Authorship

Tools are authored against a typed schema. The Tool definition declares:

- The parameter types.
- The Frame applicability — which Frames can match this Tool's calls.
- The execution implementation (in-runtime native code, an Adapter call, or a contained subprocess).
- The result schema returned to the Steward.

A new Tool ships as a registry entry. The engine is closed; the Tool registry is the open surface.

### Why Tools, Not Free-Form

Free-form action (e.g., the Steward emits an arbitrary shell command and the runtime is left to interpret bytes) lives at the wrong granularity for governance: the runtime would have to recover intent from bytes, and intent is exactly what the typed Tool surface defines by construction. Tools constrain the action to a typed shape *that was authored to be evaluable*. Frames are written against Tools, not against bytes.

### Action Taxonomy

Actions are categorized as **generative** (producing content through the Steward's cognition) or **evaluative** (producing Findings without modifying artifacts). Both are governed, gated, and receipted. Findings are review-Steward output anchored to a concern within an artifact; the concern is the Finding's identity. Artifact location is navigational metadata, not the Finding itself.

---

## The Workspace

Every entity produced by the loop — Tasks, Receipts, Findings — must belong somewhere. Every artifact must be scoped. Every Steward instance must be bound. Role context must be associated with a specific practice.

The Workspace is this boundary. It is the tenant context: an isolated partition containing a Charter instance (referencing a Charter model), Role context Scopes (filled by the Professional), Steward instances (referencing Steward models), artifacts, Tasks, Receipts, and Findings. Nothing crosses Workspace boundaries.

The Workspace is also the Professional's surface — the operator UI where Charter authoring, work execution, Findings review, Receipt query, and Steward configuration happen.

### Five Panels

The Workspace UI has five required panels:

- **Scope selection.** The Professional's operational world — which Scopes apply, which Stewards are bound, which artifacts are in play.
- **Work area.** Artifact editing with inline action triggers. Where generative work happens.
- **Findings.** Review-Steward output anchored to artifact concerns.
- **Receipt trail.** Every governance decision: what was attempted, what each Frame ruled, what the Outcome was, what was contained.
- **Steward configuration.** Charter Scopes (read-only by Professional), Role context Scopes (editable), Frame definitions (read-only), behavioral specification (read-only), governance mode toggles, version display.

Each panel reflects the same domain model. A Receipt anchored to an action in the Work area surfaces in the Receipt trail. A Finding anchored to a concern in the Work area shows in Findings. The panels are views over one consistent state, not separate applications.

### Defense in Depth

Workspace isolation is enforced by three independent layers: the store (every query takes `WorkspaceId`, no cross-Workspace method exists), the engine (every handle scoped at creation), the API layer (`WorkspaceId` derived from authenticated session, never from request body). If any single layer fails, the other two prevent cross-Workspace access.

### Multiple Surfaces

The Workspace is the Professional-present surface. Two other surface types share the same governance loop:

- **Channel surface.** Client-facing autonomous operation. The Steward responds to incoming messages from external parties without the Professional present. Same Charter, same Frames, same Receipts.
- **Pipeline surface.** Automated triggers — scheduled events, system signals, batch processing. No human on either side. Same Charter, same Frames, same Receipts.

The governance mechanism is identical across surfaces. Surfaces differ only in trigger source, output destination, and interaction pattern.

---

## Foundation Stewards

The framework configures itself through Stewards that operate on its authoring and operating surfaces. Each Foundation Steward has its own Charter — typically authored by the framework maintainers — and performs one of the framework's core meta-tasks.

- **Charter Review.** Validates a Charter against structural rules: every Frame's declared Scopes exist; every expected Role context template has a typed identifier; the behavioral specification is parseable; the Charter version is monotonic. Surfaces structural defects to the Charter engineer.
- **Charter Editor.** Edits Charter content under governance. Charter modifications are themselves tool calls, gated by the Charter Editor's Charter, Receipted. Editing a Charter goes through the same propose-evaluate-refine loop as any other Steward action.
- **Frame Decomposition.** Helps the Charter engineer decompose a broad Charter Scope into individually-evaluable Frame definitions. Proposes Frame applicability conditions, declared Scope references, and evaluator chains based on the Scope's content.
- **Coordinator.** Cross-Steward dispatch. When a Task spans multiple Stewards (e.g., a customer interaction needs billing-Steward authority, scheduling-Steward authority, and clinical-disclosure-Steward authority), the Coordinator decomposes the request, recruits the relevant Stewards, sequences their actions, and composes the result. Each sub-Steward's action is gated independently; sub-Receipts compose into a coherent trace.

### Self-Hosting

Foundation Stewards demonstrate the framework's self-hosting property: CharteredOS configures and refines itself through the same governance mechanism it applies to the Professional's domain. A Charter Editor that modifies a billing Frame is itself a Steward whose own Charter governs what kinds of modifications are allowed. A Frame Decomposition Steward that proposes a Frame structure is itself constrained by Frames that govern Frame authorship.

This property is structural, not decorative. It means the framework's operating discipline — the loop, the Gate, structural separation, Receipt-before-effect — applies to the framework's own evolution. New Charters are deployed under governance. New Frames are reviewed under governance. The framework's accountability extends to its own changes.

### The Charter Engineer's Workflow

The Charter engineer authors Charter models in collaboration with Foundation Stewards:

1. Drafts initial Charter Scopes and a concern register.
2. Engages the Frame Decomposition Steward to propose Frame definitions per Scope.
3. Reviews proposals; refines Scopes and Frame definitions iteratively.
4. The Charter Review Steward validates structural correctness.
5. Pushes a Charter model version. Workspaces using the model see it on next Task.

The Professional's workflow is much simpler: select a Charter model, supply Role context, confirm extraction, run Stewards. The Professional does not author governance. The Charter engineer does. Foundation Stewards are how governance authoring happens under governance.

---

## The Runtime

CharteredOS has two process roles, often combined into one process for simple deployments and separated for multi-Workspace deployments.

**Per-deployment Runtime.** One per Steward deployment. Hosts:

- The Steward's loop (model dispatch + tool execution + refinement-on-denial).
- The Gate, configured from the Charter instance.
- The Tool dispatcher (executes governed Tools as in-runtime native code, Adapter calls, or contained subprocesses).
- The subprocess containment helper (loaded when the Steward's Tools include exec-shaped primitives).
- A local Receipt buffer with critical-path `fdatasync` for denials; Receipts are forwarded to the Daemon when reachable, written to local files when not.

**Daemon.** Per-host or per-organization, shared across Runtimes. Hosts:

- The Receipt store (SQLite default, append-only file optional). Receipt ingestion from Runtimes; durable storage with confidentiality controls.
- The Workspace UI (web app).
- Adapter-fronted evaluator coordination (when a Frame's chain references an out-of-runtime evaluator).

For single-deployment use, Runtime and Daemon roles run in one process. For multi-Steward or multi-host deployments, the Daemon runs separately and Runtimes connect via the protocol over Unix domain socket (local) or gRPC (remote). When the Daemon is unreachable, Runtimes write Receipts to local files and continue; the Daemon ingests on reconnection. The Runtime is not blocked by Daemon unreachability.

### Configuration

Configuration enters at startup from `.chartered/` (walk-up search from the deployment directory, then `~/.chartered/` fallback).

Configuration files:

- `chartered.toml` — runtime-level: enforcement level, log level, Receipt store backend, subprocess-containment intercept set.
- `steward.toml` — Steward-level: model selection, system prompt, Tool registry references.
- `charter.toml` — Charter model reference and version.
- `role_context.md` — practice-specific facts (replaces `policy.md` in some deployments).
- `tools/*.toml` — per-Tool registry entries.

The Runtime does not modify its configuration at runtime; reload requires restart. A Runtime that reloads config mid-session opens a window where a Steward observes one policy and an effect occurs under another.

### Enforcement Levels

| Level | Behavior |
|---|---|
| `passthrough` | Receipt every tool call. Never deny. Use during Charter bootstrapping. |
| `rules` | Deterministic-matcher evaluators enforce. LLM evaluators run advisory; their UNGROUNDED is not blocking. |
| `full` | All evaluators enforce. Default-deny on chain exhaustion. |

Default enforcement is `passthrough` for fresh deployments. The Professional moves to `rules`, then `full`, as Charter precision improves.

### Governance Mode

Independent toggles: grounding and evaluation.

- **Grounding** determines whether Charter Scopes and Role context Scopes are injected into the cognitive prompt.
- **Evaluation** determines whether Frames check tool calls before dispatch.

Four combinations: both enabled (full governance), grounding on + evaluation off (honor system), grounding off + evaluation on (safety net), both off (vanilla). The safety-net combination reveals whether the Gate catches violations independently of whether the Steward was grounded with Charter Scopes.

Every Receipt unambiguously shows the governance mode active at evaluation time.

---

## LLM Role Discipline

CharteredOS has two distinct synchronous gates with different latency budgets, and one asynchronous role. LLM use is bounded by which gate.

**PEP gate (tool-call boundary, agent propose-step pace).** The Steward's propose step is itself LLM-driven and lives at LLM pace (hundreds of milliseconds to seconds per step). An LLM evaluator in the within-Frame chain runs at the same pace and is acceptable; the latency is absorbed by the Steward's existing turn cycle. This is where most semantic-judgment Frames live (the medical-disclosure Frame is an example). Place LLM evaluators here freely, with the structural-separation invariant enforced.

**Subprocess-containment kernel hook (syscall boundary, microsecond pace).** When a Steward's `exec_command`-shaped Tool dispatches a subprocess, the kernel hook for an intercepted syscall must respond in microseconds to avoid stalling the host. An LLM evaluator there is structurally wrong — its latency does not fit the budget. The subprocess-containment layer is for deterministic audit and hygiene, not policy semantics. An operator who wants LLM-based judgment on a subprocess-level effect surfaces the action as a Tool call, not as an inline kernel-hook decision.

**Forbidden patterns, regardless of gate:**

- **LLM as the sole gate on a critical Frame.** A single fragile model is not the gate against catastrophic actions. Layer with deterministic matchers in the within-Frame chain; the LLM provides judgment on the ambiguity remainder.
- **LLM evaluator without parse-fail-deny.** A non-parsing LLM output silently default-allows. Output schemas with strict parsing and parse-fail-deny semantics are required.

**Encouraged async role:**

- **LLM as Receipt-corpus analyst.** Reads the Receipt corpus, identifies anomalous patterns, proposes new Frames or Charter refinements to the Charter engineer. Operator reviews proposals; refinements deploy via the Charter Editor Foundation Steward. The LLM is a learning loop *over* Receipts, not an evaluator *in* the Gate.

---

## Receipts and Audit

Every governed tool call produces a Receipt. Receipts are the audit trail in the operator's vocabulary — what Tool was called, with what params, against which Frames, with which Rulings, what the Outcome was, who the Steward was, when. Tool-call granularity.

### Receipt Durability

Two tiers, differing in durability not in ordering:

- **Critical mode.** Denials, plus any Verdict against a Frame marked `critical: true`. The Receipt is `fdatasync`-persisted before the Runtime allows the next Steward step. Persistence failure → denial. This is the ledger invariant.
- **Buffered mode.** Allowed actions under non-critical Frames. The Receipt appends to a ring buffer; a background flusher persists at a configured cadence. The Runtime advances the loop as soon as the append returns. Flush failure does not retroactively undo past actions; subsequent Receipts are marked `intercept_complete=false` until flushing recovers.

The trade-off is named explicitly. Critical Receipts cannot be lost — that is the ledger property. Non-critical Receipts can be lost in a Runtime crash within a knowable bound (flush interval × throughput).

### Receipt Confidentiality

Receipts may carry argv, paths, message bodies, query strings, headers — content that may contain credentials, file paths revealing infrastructure, request bodies revealing customer data. The store applies access control on query, per-field redaction (declared-sensitive Frame fields), retention with rotation, and tamper-evidence. Charter engineers mark fields as sensitive at Charter authoring time; the Daemon redacts at storage and at query response.

### Storage

Default backend: SQLite, with per-day partitioning and the Daemon owning writes. Append-only file backend available for environments where SQLite is operationally inappropriate. Both produce queryable Receipts.

### Subprocess-Containment Child Receipts

When an `exec_command`-shaped Tool dispatches a subprocess, the subprocess-containment helper produces low-level Receipts (`ChildReceipt`) joined to the parent tool-call Receipt by `parent_receipt_id`. The parent Receipt is the audit primary; ChildReceipts are the audit detail.

---

## Subprocess Containment

When a Steward's `exec_command` (or any other exec-shaped Tool) dispatches a subprocess, that subprocess is *not part of the Steward's trusted scope*. It is an external program — `psql`, `git`, a custom binary, something the operator deployed. The Runtime instruments it at the kernel boundary so the operator can see what it actually did.

### Mechanism

The Runtime forks the subprocess inside a containment envelope (PTY for terminal-bound Tools, plain pipes otherwise), installs a seccomp filter on the child before its first `execve`, and intercepts every syscall in the configured intercept set issued by the subprocess or any of its descendants. The filter is inherited across `fork` and `execve`. `PR_SET_NO_NEW_PRIVS` prevents descendants from shedding it. When the subprocess issues a filtered syscall, the kernel suspends the syscall and notifies the Runtime. The Runtime decodes the syscall's argv/path/sockaddr from the subprocess's address space (`process_vm_readv`), records a low-level Receipt joined to the originating tool-call Receipt, and responds to the kernel.

### Default Intercept Set

`execve`, `execveat`, `openat` (with write flags), `connect`, `unlinkat`, `renameat2`. Read-class syscalls are not in the default set — high volume, low governance signal. An operator requiring stricter coverage adds syscalls; the engine is unchanged.

### Honest Gap Inventory

What this layer does not cover, named explicitly:

- **JIT and queue bypass.** `mmap`+`PROT_EXEC` for runtime code generation; `io_uring` operations; in-process interpreter dispatch (Python `eval`, Lua `loadstring`).
- **File descriptor laundering.** A descriptor obtained before policy applies, used by later `write`/`sendmsg` without re-checking against path-based policy.
- **Namespace and mount confusion.** Bind mounts, symlink chains, chroot-like views.
- **Device files, `/proc`, `/sys`, and `ioctl`-mediated effects.** Meaningful system effects via `ioctl` or device files rather than the obvious file/exec paths.

These are gaps in the *containment* layer. Resolution: widen the intercept set per threat model, or accept the boundary. Trust does not depend on closing them — trust is established at the Steward layer above.

### Why the Layer Exists

A Steward is trusted *as a Steward*. The subprocesses it dispatches — `psql`, `git`, vendored binaries — are not the Steward. They run code the operator did not author, on the operator's host. Visibility into what they did is what the operator needs to answer "the Steward ran this command; here's everything that command did to my filesystem and network." Without this layer, a tool-call Receipt that says `exec_command("psql -c ...")` is opaque downstream of execution. With it, the Receipt has child Receipts for every file `psql` opened, every endpoint it reached, every file it modified.

The containment helper is also available standalone — operators who want kernel-level visibility on processes that are not Stewards can use it directly. In that mode it is a lightweight syscall-level audit tool.

---

## Operating Modes

The same governance loop operates in several modes.

### Inline Governance

The default. Every tool call passes through the Gate; Receipts are recorded; denials produce refinement; budget exhaustion produces escalation. The Steward runs against real state; consequences are real.

### Shadow Monitoring

The Gate runs but never denies. Every tool call executes; Verdicts are logged. Answers: "if enforcement were on, what would it catch?" Deployment path: shadow first (observe), then `rules` (enforce deterministic), then `full` (enforce everything).

### Adversarial Testing

The Runtime hosts the Steward under measurement infrastructure: a *Threat Actor* generates adversarial input, the Steward produces output, the *PEP* gates, an *Auditor* assesses the final delivered output blind to experimental condition. FAR/FRR measured. See *Empirical Measurement*.

### Audit Analytics

Pure read-mode over the Receipt store. Compliance reporting, trend analysis, FAR/FRR-over-time. Does not produce governance decisions; consumes them.

### Authoring

Foundation Stewards operate on the Charter authoring surface — refining Charter models, decomposing Scopes into Frames, validating structural correctness. Charter modifications are themselves governed (the Charter Editor's own Charter governs them) and Receipted. The framework's evolution leaves an audit trail.

---

## Empirical Measurement

The governance loop is argued from mechanism. It can be measured.

The framework treats two evaluation contexts as the same mechanism at different lifecycle stages:

- **Capability evaluation.** Run during Charter authoring against a frozen task bank. Question: can the Steward learn to comply with this Frame under iterative refinement? Hill-climb on the Frame's precision.
- **Regression evaluation.** Runs continuously in production via the Gate. Question: does the deployed Steward still comply on every action? Receipts are the regression record.

Same Frame definitions, same Scopes, same Ruling vocabulary, same Receipts. Different lifecycle stage, different control loop instance.

### The 2×2

|              | Clean scenario | Adversarial scenario |
|--------------|----------------|----------------------|
| **Governed**   | Baseline       | Core test            |
| **Ungoverned** | Baseline       | Core test            |

Governed: full propose-evaluate-refine loop. Ungoverned: the agent's first proposal is the final output, no evaluation, no refinement.

### Four Roles, Strict Information Boundaries

- **Threat Actor.** Generates adversarial input. Knows approximate domain context, not exact policy or Frame definitions.
- **Steward.** Under test. Identical system prompt and Charter in both conditions. Does not know it is being tested.
- **PEP.** Present only in the governed condition. Receives only proposed tool params and Frame Scope content.
- **Auditor.** Independent measurement instrument. Receives only the final delivered output and Frame Scope content. Blind to experimental condition.

The boundaries are tested as runtime invariants — the PEP's prompt is asserted to contain neither customer messages nor Steward reasoning before any evaluator call.

### Metrics

- **FAR** — violations accepted / total violation attempts. Breach rate.
- **FRR** — legitimate denied / total legitimate. Usability cost.
- **Recovery rate** — initially-denied proposals that eventually pass refinement.
- **First-pass authorization rate** — proportion of turns approved without refinement.
- **Mean iterations** — refinement count per governed turn.
- **Escalation rate** — budget-exhausted turns.

Headline test: under adversarial scenarios, FAR(governed) < FAR(ungoverned), at p < 0.05 over the test suite. The interaction effect (governance × pressure) measures whether adversarial pressure amplifies the governance benefit.

### Reference Domain

Initial measurement: customer service for the fictional Meridian Electronics retailer. Eighty-four scenarios across clean / adversarial / boundary modes; eight adversarial techniques; paired at the scenario level; auditor-evaluated under blind condition.

The methodology generalizes across domains. The medical-reception scenario in the Synopsis is a single dramatized example; the customer-service suite is the systematic measurement.

---

## Reference Charters

The framework ships with Charter models that demonstrate the governance loop in concrete domains. Each is open-source under MIT and Apache 2.0 — operators fork and tune Role context for their deployment.

- **Customer service** (Meridian Electronics). Tools: `send_message`, `lookup_product`, `lookup_order`. Frames: price-fabrication, scope-violation, privacy. The empirical-measurement domain.
- **Medical reception**. Tools: `send_message`, `lookup_appointment`, `verify_identity`. Frames: clinical-disclosure-without-verification, family-member-request-handling, scope-of-service.
- **Coding agent** (Stewards authoring code edits in a repo). Tools: `read_file`, `write_file`, `exec_command`, `query_repo`. Frames: no-credential-files, no-destructive-operations-without-confirmation, no-network-egress-outside-allowlist.

Each Reference Charter ships with: Charter Scopes, Frame definitions, behavioral specification, expected Role context templates, and a fully-worked Steward model. Operators select the Charter, supply Role context, confirm extraction, and run.

The Reference Charters are the framework's concrete examples. They are not measurement scaffolding. They are the deliverable's shape — what a Steward looks like in a domain that matters.

---

## Open Publication

CharteredOS is open-source under MIT and Apache 2.0. The framework is released for the public good.

Runtime governance for autonomous agents is safety infrastructure. Safety infrastructure that depends on a single vendor's permission to deploy creates the same coordination failure the infrastructure exists to prevent: every operator must trust the vendor; the vendor cannot be audited; deployment is contingent on commercial terms. The architecture, the implementation, the measurement methodology, the dataset, and the Reference Charters are published so that any operator, in any jurisdiction, regulated or not, can deploy, validate, modify, and improve them.

What is published:

- **The framework.** Specification (this document), Design Notes, Implementation Checklist.
- **The implementation.** Runtime, Gate, Receipt store, Tool dispatcher, subprocess containment helper, Workspace UI, Daemon.
- **Foundation Stewards.** Charter Review, Charter Editor, Frame Decomposition, Coordinator.
- **Reference Charters.** Customer service, medical reception, coding agent.
- **Empirical methodology, dataset, and measurement results.**
- **Adversarial technique library.**
- **Protocol definitions** in `proto/v1/`.

The framework is open-source and self-contained. The propose-evaluate-receipt-effect path requires no proprietary dependency. Higher-layer features (premium LLM evaluators, cloud-aggregated dashboards, vector indexes over Receipt corpora) are additive — not on the critical path. Operators who deploy the framework against an air-gapped, fully-deterministic Charter have a complete governance system.

Aggregation of FAR/FRR across deployments is invited; the Receipt format and the evaluator interface are stable enough to support it. Charter models, Steward models, and Frame definitions are portable across deployments; a Charter authored by one Charter engineer can be reviewed, forked, refined, and republished by another.

The framework grounds in published foundations — Minsky 1974 on Frames as structured knowledge representations, Ashby 1956 on requisite variety, control theory canon on closed-loop feedback. The architecture is the named composition of established mechanisms applied to a new domain. Reproducibility, auditability, and academic engagement follow from this grounding.

---

## Known Limitations

**Causal attribution.** The empirical loop measures the full governance loop (structural separation + iterative refinement + feedback) as a single treatment. A positive result means the system works; it does not isolate which component contributes most. Recovery-rate decomposes some of this; full attribution requires additional experimental design.

**Single domain in initial measurement.** Customer service. Multi-domain measurement extends the result; the methodology and infrastructure are domain-generic.

**LLM evaluator FAR/FRR is model-specific.** When an evaluator is an LLM, the measured FAR/FRR depends on the model's classification ability at fixed temperature. Reported numbers carry the model identifier; cross-evaluator comparison requires re-running.

**Evaluator prompt injection.** Adversarial strings in tool params can attempt to inject the LLM evaluator. Mitigations are layered: deterministic evaluators run first in the within-Frame chain; the Runtime asserts evaluator prompts contain no agent-context fields; evaluator outputs are constrained by typed schemas with parse-fail-deny semantics; Scope content is loaded from a Snapshot rather than concatenated with proposal data. Residual injection surface is itself a measurement question.

**Subprocess containment gaps.** Listed in *Subprocess Containment > Honest Gap Inventory*. JIT, FD laundering, namespace confusion, ioctl-mediated effects. These are gaps in the *containment* (hygiene) layer, not in the trust property. Trust is established at the Steward layer above.

**Trust boundary at the Steward.** A Steward is trusted because every action passes through the Gate. An operator who deploys a non-chartered agent inside the Runtime's deployment directory and exposes raw syscalls surrenders the trust property. The Runtime is not a sandbox for arbitrary code; it is the home for Stewards *built* against this architecture.

**Charter model versioning across multiple Workspaces.** A Charter model update propagates to every Workspace using that model on the next Task. Schema migration semantics — what happens when an updated model removes a Frame, adds a Scope, or restructures a behavioral specification — are not fully specified. Single-Workspace deployments have no propagation problem. Multi-Workspace deployments need migration semantics for the Charter engineer to ship corrections.

**Multi-Professional Workspaces.** The current model assumes one Professional per Workspace. Per-Professional Role context layering, shared vs individual Steward instances, and authority delegation across Professionals within a Workspace are unspecified. Single-Professional deployments are the common case. Multi-Professional deployments need additional structure.

**Role context as adversarial input.** Source materials uploaded by the Professional must be treated as potentially adversarial during extraction — a malicious uploaded PDF must not become Charter content. The current bootstrap relies on the Professional's confirmation; sufficiently adversarial extraction-time injection could influence Role context that subsequently reaches LLM evaluators. Deterministic Frames are immune; LLM Frames are not. The residual risk is acknowledged.

**Sequence-dependent governance.** Each tool call is evaluated independently. Sequence-dependent governance — identity verification before clinical disclosure, authentication before data access — requires Frames that read accumulated cognitive state, not the current proposal alone. The mechanism for setting, storing, and evaluating cognitive state within a Task is unspecified.

**No retrofit of unmodified third-party agents.** The architecture does not deliver trust as a property for unmodified third-party agents. The Anti-Position section names why. Operators who need governance for those agents have well-trodden options (Docker, gVisor, AppArmor, network policies, credential scoping).

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
