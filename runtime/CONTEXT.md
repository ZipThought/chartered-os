# Context: Runtime binary

The per-deployment process from spec §The Runtime: hosts the Actor
loop, runs the Gate, dispatches Tools, writes Receipts. Inherits root
`CONTEXT.md`.

## Spec sections that govern this subtree

§The Runtime, §The Loop, §Receipts.

## ONE entry, ONE code path

There is no `scenario` vs `production` split. The binary has one
invocation:

    chartered-runtime [--chartered-dir <dir>]
                      [--workspace-root <dir>]
                      [--user-message <text>]
                      [--refinement-budget <n>]

`.chartered/` walk-up resolves config; `steward.toml` per-role
`backend` value selects fake or real `CognitionBackend`; `tools/*.toml`
`executor` strings select OS-touching `ToolExecutor` impls from
`dispatch::ExecutorRegistry`. A test deployment differs from a
production deployment only in those values. ZERO test-only code paths.

See feedback memory: "Single code path across test and production —
at every level."

## Drives the loop with

- **Charter + Role context**: loaded from the Charter directory that
  `.chartered/charter.toml::path` points at, via
  `runtime::charter_loader::load_charter_def` and `load_role_context_def`
  (deployment-side filesystem IO; the kernel parses pre-loaded text
  through `chartered_core::parse_charter_def` / `parse_role_context_def`).
- **Per-role CognitionBackend**: per-role tables in `steward.toml`
  (`[actor]`, `[evaluator]`, `[tester]?`, `[judge]?`) declare
  `backend = "fake"` or `backend = "openai"`. For `fake`, inline
  `fake_responses` arrays (per-Frame for evaluator) drive the queue.
- **Tool dispatch**: `tools/*.toml` `executor` strings are mapped to
  `dispatch::ExecutorRegistry` impls scoped to `--workspace-root`
  (default: parent of `.chartered/`).
- **Tester**: `[tester]` in `steward.toml` (multi-turn,
  `max_turns`-bounded) OR `--user-message` (single-task, max_turns=1).
- **Judge**: `[judge]` in `steward.toml` OR a no-op judge that emits
  a default `JudgeOutput`.

Stdout: pretty JSON `{ workspace_id, run_id, run_dir, receipts_log,
cognition_log, tasks, attempts, receipts, judge, turns,
terminated_by_budget }`. Exit code: 0 on successful run regardless of
Judge verdict (the verdict is in the JSON), 1 on file/parse/validation
failure, 64 on usage error.

## CognitionBackend selection

`steward.toml`'s per-role `backend` value picks a `CognitionBackend`:

- `backend = "fake"` — `FakeCognitionBackend` driven by inline
  `fake_responses` in the same TOML table. The only test path; CI
  runs entirely on this.
- `backend = "openai"` — `OpenAiCompatibleBackend` (HTTP). Reads
  `OPEN_AI_BASE_URL`, `OPEN_AI_MODEL`, `OPEN_AI_API_KEY` from environment
  (loaded by `dotenvy::dotenv()` from `.env` in CWD or any ancestor
  on binary startup). Per-role `model = "..."` in `steward.toml`
  overrides `OPEN_AI_MODEL`. Same impl serves both real OpenAI
  (`OPEN_AI_BASE_URL=https://api.openai.com/v1`, key required) and local
  OpenAI-compatible servers (LM Studio, llama.cpp, vLLM, SGLang;
  key optional). `OPEN_AI_BASE_URL` is the versioned API base; the backend
  appends `/chat/completions`. The `OpenAiCompatibleBackend` does not
  differ between them — only the env values do.

The role implementations (`LlmActor`, `LlmEvaluator`, `LlmTester`,
`LlmJudge`) consume the trait and never branch on backend kind. The
parser in `core::actor::parse_actor_response` accepts:
- raw JSON Action objects,
- markdown-fenced JSON,
- gpt-oss harmony envelopes (with `to=` recipient in the header and
  params in the body).
The parser in `core::verdict::parse_evaluator_response` accepts the
same `DECISION: REASON` lines whether wrapped in a harmony envelope
or emitted plainly.

## Per-run persistence

Each binary invocation writes two grep-able JSON Lines files under
`<chartered_dir>/runs/<run_id>/`:

- **`receipts.jsonl`** — one Receipt per line. Append-only; written
  by the file-backed `AppendOnlyFileReceiptStore` (in
  `runtime::persistence`) injected into the Workspace via
  `Workspace::with_store`. The store keeps an in-memory mirror so
  Frame `prior_receipt_queries` do not pay disk I/O on every Gate
  evaluation; the on-disk file is the durable record.
  Each Receipt carries `task_id`; proposal Receipts also carry
  `attempt_id`. Controller events such as `<budget_exhausted>` share
  the Task and omit Attempt.
- **`cognition.jsonl`** — one (request, response) pair per LLM call.
  Every per-role `CognitionBackend` is wrapped in `LoggingBackend`
  and shares one `CognitionLogFile`. Each entry has `started_ns`,
  `finished_ns`, `backend_id` (`actor`, `eval-<frame_id>`,
  `tester`, `judge`), the full request, and either a response or an
  error. Operators grep prompts and responses by role.

Run identifier: `r-<unix_nanos>-<pid>`. Lex-sortable; collision-free
within one host.

`.gitignore` excludes `**/.chartered/runs/` so per-run trails never
end up in the repo. `target/` (also gitignored) holds tempdirs from
the test suite indirectly via `tempfile::TempDir`, which uses system
temp by default.

## Binary-level integration targets THIS binary

`tests/binary_integration.rs` constructs a complete `.chartered/`
deployment in an isolated tempdir per test and runs the binary against
it. Asserts on both the stdout JSON AND on real filesystem state in the
workspace. The deployments are real production deployments — same
loader, same loop, same `dispatch::*` Tools. The only distinction from
the e2e LLM deployments is `backend = "fake"` in the per-role config.

Per `AGENTS.md §Verification`, this file is integration (vertical cut
across the binary boundary, fake-LLM side of the test pair). E2e
requires an actual LLM and lives in `tests/llm_e2e.rs`, local-only.

Tempdirs are system temp (outside the repo, naturally gitignored).
Each test gets its own tempdir for isolation.

Coverage: happy path (file written + content verified), capability
denial, frame denial then refinement convergence, budget exhaustion
(escalation), actor parse failure (intercept_complete=false),
path-traversal denial via dispatch, across-frame conjunction,
multi-turn with [tester], workspace validation failures, unknown
executor errors, read-after-write roundtrip, exec_command real
subprocess.

## Library surface

`runtime/src/lib.rs` exposes:
- `config` — `.chartered/` loader.
- `persistence` — file-backed Receipt store and CognitionBackend logger.
- `openai_backend` — `OpenAiCompatibleBackend` HTTP client.
- `run` — the single execution path.

Tests:
- `tests/config.rs` — loader exercised directly (integration).
- `tests/persistence.rs` — disk-backed Receipt store + Cognition log
  (integration; one tempdir per test).
- `tests/binary_integration.rs` — binary exercised with `backend =
  "fake"` deployments (integration).
- `tests/llm_e2e.rs` — binary exercised with real LLM (`backend =
  "openai"` or `"openai"`-over-local-LM). E2e per
  `AGENTS.md §Verification`: every test `#[ignore]`d, opt-in via
  `cargo test -- --ignored`. Local-only.

Module unit tests inside `openai_backend` (stateless canonicalization
and URL building). The binary entry (`src/main.rs`) consumes the lib
the same way external consumers would.

## Example deployment

`examples/deployments/coding-agent-min/.chartered/` is the canonical
fixture: a complete `.chartered/` directory pointing at the
`examples/charters/coding-agent/` Charter, configured for `openai`
backends (real-LLM target — not yet runnable until the OpenAI
backend ships). Used by `tests/config.rs` for the loader; not yet
runnable end-to-end.

## Architectural constraints

- Cognition backend is the only fake/real swap point at the role
  level; ToolExecutor registry is the only fake/real swap point at
  the effect level.
- Receipts cross stdout as JSON. `core::Receipt` and components must
  remain serde-Serialize.
- `Workspace::new` validation runs at binary startup; misconfigurations
  exit nonzero with a structured error. Never silently degrade.
- No subcommands. No alternative orchestration paths. One loader, one
  runner, one Receipt format.
