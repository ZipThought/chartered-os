# Context

CharteredOS is the cybernetic enclosure for intelligent agents — see
`README.md` for the diagnostic frame. In implementation terms: the LLM
is a conditional generator (predictor); this framework is the
controller. Setpoint = Frame, plant = Actor, sensor = Evaluator,
comparator = Gate, error signal = Refinement signal. Stability is a
property of the enclosure, not of the Actor's cognition.

`docs/SPECIFICATION.md` is sovereign; agent inference derives from it,
not the reverse. `docs/IMPLEMENTATION_CHECKLIST.md` is the verification
surface — answer its diagnostics for the changed scope before claiming
any phase complete.

## Subtree map

- `docs/` — sovereign documents.
- `proto/` — wire protocol (versioned by field numbering).
- `core/` — kernel library. Cargo: `chartered-core`. In-memory only;
  no stdlib effect surfaces.
- `dispatch/` — OS-touching ToolExecutor implementations. Cargo:
  `chartered-dispatch`. Quarantines `std::fs` / `std::process` /
  `tokio::fs` / `tokio::process` / `tokio::net`. The only path from
  the Runtime's Tool dispatcher to the operating system.
- `runtime/` — per-deployment Runtime binary AND library API. Cargo:
  `chartered-runtime`. Spec §The Runtime. Exposes
  `chartered_runtime::Agent` for in-process embedding; the binary is
  a thin wrapper. E2E may target either surface. Per-run persistence
  (`receipts.jsonl`, `cognition.jsonl`) lives under
  `<chartered_dir>/runs/<run_id>/` — grep-able, isolated per
  invocation.
- `tracer/` — companion syscall-trace tool. Cargo: `chartered-tracer`.
  NOT part of the Gate architecture; peer to Docker / gVisor / strace.
- `dashboard/` — local workspace console. Node-served static UI + thin
  local API that subprocess-spawns the Runtime per invocation. Not a
  Cargo crate; not on the trust boundary. Spec §User-Facing
  Integration Boundary.
- `daemon/` — placeholder for the cross-deployment Receipt store and
  operator surface (spec §The Runtime). Not implemented; single-
  deployment runs use the Runtime alone.
- `examples/charters/` — Reference Charter templates per domain.
- `examples/demo/` — runnable M&A diligence deployment.
- `examples/scenarios/` — committed eval corpora per Charter.

## Naming convention

Cargo package names carry the `chartered-` prefix (global identity on
crates.io). Directory names do not — the repo is `chartered-os` and
the prefix would be redundant inside it.

## Backend swap rule

The boundary between fake and real LLM backends is `CognitionBackend`
(`core/src/cognition.rs`), never the role trait. One canonical role
implementation per role (`LlmActor`, `LlmEvaluator`, `LlmTester`,
`LlmJudge`); fake/real swap is the backend impl. Tests enqueue what
the LLM "would say" into `FakeCognitionBackend`; prompt assembly,
parsing, and decision derivation run identically against fake and
real backends. Bugs in those paths surface in fake-mode CI rather
than live.

Real-backend impl: `runtime::openai_backend::OpenAiCompatibleBackend`.
Same impl serves real OpenAI (`https://api.openai.com/v1` + API key)
and local OpenAI-compatible servers (LM Studio, llama.cpp, vLLM,
SGLang; key optional). Picked per-role via `steward.toml` `backend =
"openai"` + env (`OPEN_AI_BASE_URL`, `OPEN_AI_MODEL`, `OPEN_AI_API_KEY`);
`OPEN_AI_BASE_URL` is the versioned API base.

Frames are Steward-owned weak entities. Any serialized cross-boundary
reference uses `FrameRef { steward_id, frame_id }`; bare `FrameId`
is local to one Steward's Charter. User-facing work is represented by
Tasks and Attempts; Receipts are audit evidence under those objects.
Markdown can be an authoring surface, but runtime identity comes from
compiled typed config.

The Actor parser tolerates raw JSON, markdown-fenced JSON, and gpt-oss
harmony envelopes (with `to=` recipient in the header and params in
the body). The Evaluator parser tolerates the same envelopes around
`DECISION: REASON` lines. The role logic does not branch on backend
kind.

## Forbidden across the repo

- Finite-specification Verdicts on LLM-authored content (CHECKLIST
  §Risk Register names this as the structural anti-pattern).
- Silent governance degradation: every partial-coverage condition
  flips `intercept_complete=false`; every Actor cognitive failure
  produces a Receipt with `outcome: Escalated`.
- Library-based E2E reconstruction. End-to-end runs against the
  Runtime binary, parameterized by a scenario JSON.
- Parallel role implementations (e.g., `Fake<Role>` + `Llm<Role>`).
  Branch only at the backend trait.

## Spec sections that govern the repo as a whole

§The Loop, §Where Governance Applies, §Cognition Layer, §Vocabulary,
§Structural Separation, §Requisite Variety, §Default Deny,
§Conjunction, §Frames, §Tools, §The Charter, §Role Context,
§The Runtime, §Receipts, §The Protocol.
