# Context: kernel library

The negative-feedback loop's mechanism. Inherits root `CONTEXT.md`.

## Spec sections that govern this subtree

§The Loop, §Cognition Layer, §Structural Separation, §Requisite
Variety, §Default Deny, §Conjunction, §Frames, §The Charter,
§Role Context, §Receipts, §Vocabulary.

## What lives here

The components that close the negative-feedback loop:

- **Setpoint**: `Frame` (frame.rs). Declared scopes carry provenance
  via `ScopeKind { Charter, RoleContext }` — the spec's adversarial-
  input distinction is type-enforced, not flattened. `FrameId` is local
  to one Steward; cross-boundary identity is `FrameRef`.
- **Sensor**: `trait Evaluator` + canonical `LlmEvaluator` (verdict.rs).
  Backend swap = `CognitionBackend` impl swap.
- **Comparator**: `Gate` (gate.rs). Capability check → parallel Frame
  eval (no across-Frame short-circuit) → typed Scope resolution →
  Evaluator → aggregation → Receipt.
- **Plant**: `trait Actor` + canonical `LlmActor` (actor.rs).
  `Action::Fail` carries cognitive failure into the loop with operator
  visibility (Receipt with `intercept_complete=false`).
- **Controller**: `LoopRunner` (loop_runner.rs). Composes plant +
  comparator + budgeted feedback path + Tool dispatch. Creates Tasks
  and Attempts; Receipts hang under them.
- **Coherence freeze**: `Snapshot` (snapshot.rs). Content-addressed;
  identical Charter+RoleContext yields identical id.
- **Binding**: `Workspace` (workspace.rs). Configuration-time
  validation: returns `Result<Self, WorkspaceValidationError>`.
  Rejects missing scope references and missing tool executors before
  the loop runs.
- **Cognition abstraction**: `cognition::CognitionBackend` +
  `FakeCognitionBackend` (cognition.rs). The fake backend is a queue;
  empty queue → `CognitionError`, never silent success. The Runtime
  binary wraps every backend with `runtime::persistence::LoggingBackend`
  to record (request, response) pairs to `cognition.jsonl`.
- **Multi-Steward orchestration**: `ScenarioRunner` (scenario.rs)
  with `Tester`, `Judge`, `ActorFactory`. Canonical `LlmTester` and
  `LlmJudge` consume the same `CognitionBackend` trait.
- **Test-grade Tool implementations**: `harness` module. In-memory
  only — `MessageLog`, `InMemoryFs`, `ReviewQueue`. Used by kernel
  tests and the Runtime binary's scenario mode.
- **Charter parsers**: `charter_loader` (charter_loader.rs). Two-stage
  construction — `parse_charter_def` / `parse_role_context_def` accept
  already-loaded text (no filesystem dependency) and produce data-only
  shapes; `build_charter` / `build_role_context` materialize the runtime
  types by attaching Evaluator instances and version numbers. Markdown
  sections become slugified scope names. Filesystem IO that reads
  `frames.toml` / `scopes.md` / `behavioral_spec.md` / `role_context.md`
  / `skills/*.md` lives in `runtime::charter_loader`. Format documented
  in `examples/charters/CONTEXT.md`.

## What does NOT live here

- Stdlib effect surfaces (`std::fs`, `std::process`, `tokio::fs`,
  `tokio::process`, `tokio::net`). This crate is in-memory only;
  filesystem IO for Charter loading lives in `runtime::charter_loader`;
  OS-touching ToolExecutors live in `dispatch/`; the file-backed
  Receipt store and the cognition log live in `runtime/src/persistence.rs`.
- Real LLM backends. Step 2 adds OpenAI / Anthropic / vLLM / SGLang
  as sibling implementations of `CognitionBackend`.
- E2E scenario tests. Those live in `runtime/tests/` and target the
  binary.
- `.chartered/` config loading (chartered.toml, steward.toml,
  charter.toml, tools/*.toml). The Charter loader handles the per-Charter
  files (`frames.toml` + `scopes.md`); deployment-level config loading
  lives in `runtime/`.

## Tests in this crate

- `tests/gate_invariants.rs` — structural property tests for the Gate.
- `tests/reconciliation.rs` — Receipt outcome ↔ observed effect.
- `tests/passthrough.rs` — passthrough-mode functional equivalence
  to a baseline.
- `tests/charter_loader.rs` — exercises the loader against the
  shipped `examples/charters/*/` artifacts.
