# CharteredOS

> Today's intelligent agents are open-loop systems. Open-loop systems cannot be made reliable by tuning the plant — better models, better prompts, more guardrails, more training. Cybernetics established this in the mid-20th century: reliability is a property of the enclosure, not the cognition inside it. The AI field has spent its agentic-era effort on the plant; the missing engineering move is the enclosure. **CharteredOS is that enclosure for intelligent agents.**

The enclosure places a comparator between every proposed action and its effect. Each proposal is evaluated against Charter-defined setpoints — Steward-owned Frames, in Minsky's 1974 sense, with applicability conditions, declared Scopes, and an evaluator chain. The error signal is projected back to the agent as a Refinement signal. Refinement iterates inside a Task until the action is grounded or escalated. Attempts record each proposal; Receipts are the immutable audit evidence for every Gate step and controller event.

A *Steward* is a chartered-resident agent operating under this enclosure. Frames are weak entities under their owning Steward; cross-boundary identity is a FrameRef `{ steward_id, frame_id }`. Tasks, Attempts, the Gate, and the Receipt trail are the architecture; nothing else differs between a test deployment and a production deployment.

![The M&A diligence scenario, side-by-side](docs/assets/m-and-a-loop.png)

For operators who need governed agents in domains (medical, financial, legal, gov, sensitive customer service) where Tasks show what work was attempted and the Receipt trail proves what was admitted, denied, or escalated. Does *not* wrap unmodified third-party agents at the syscall layer; that trust property cannot be retrofitted, and well-trodden options exist for syscall-level isolation (Docker, gVisor, AppArmor, network policies).

## Status

Open-source contribution from an internal research prototype. Run locally — clone the repo, point an OpenAI-compatible LLM endpoint at it (hosted OpenAI, or a local server such as LM Studio, llama.cpp, vLLM, SGLang), and exercise the `examples/demo/` deployment.

No warranty whatsoever. The MIT and Apache 2.0 licenses (`LICENSE-MIT`, `LICENSE-APACHE`) are explicit: the software is provided as-is, without warranty of any kind.

Not for production use yet. To pilot CharteredOS in your organisation, contact [ZipThought](https://www.zipthought.com.au/).

## Embedding the runtime

The runtime publishes a `chartered_runtime::Agent` library type. Construct
once from a `.chartered/` directory; call `run(brief)` per invocation.
Each call is atomic — opens a fresh run dir, dispatches the loop, writes
Receipts, returns a categorical `AgentOutcome` (`Externalized` / `Quiet`
/ `Escalated` / `Failed`). Stateless across calls; holding an Agent in a
host process across many calls is a performance choice (warmed config,
pooled LLM clients), not a correctness one. See
`runtime/examples/agent_embed.rs` for the consumer pattern.

## Reproducing the result

Three CLI surfaces back the v1 demo, all local-only:

- `chartered-runtime --user-message …` (or `--selection-…`) runs one
  governed Task and prints the run summary as JSON.
- `chartered-runtime --scenario-suite <corpus_dir>` iterates a corpus
  against one in-process Agent and emits a per-scenario report plus
  aggregations by technique and failure class.
- `scripts/compare-mode.sh <chartered_dir> <corpus_dir> [<out_dir>]`
  fans one corpus across the three native governance configurations
  (naked / evaluation-only / full) and emits side-by-side reports plus
  a compact summary. The contrast between configurations is the
  ablation the verification harness produces.
- `scripts/passive-mode.sh <chartered_dir> <watch_dir>` watches a
  workspace subtree and fires one governed Runtime invocation per new
  file. Restraint is the load-bearing behavioral property — the trail
  shows how many arrivals stayed Quiet versus how many surfaced a
  finding.

`examples/demo/run.sh --mode active|passive|both` boots the included
M&A diligence deployment in either flavour against an
OpenAI-compatible LLM endpoint.

## Repository

- `docs/SPECIFICATION.md` — the architecture, sovereign.
- `docs/IMPLEMENTATION_CHECKLIST.md` — verification diagnostics.
- `proto/v1/` — wire protocol.
- `core/` — kernel library (Task, Attempt, Gate, Receipt, LoopRunner, canonical LLM-backed role implementations).
- `dispatch/` — OS-touching ToolExecutor implementations; the only crate that touches `std::fs`, `std::process`, `tokio::net`.
- `runtime/` — per-deployment Runtime binary AND `chartered_runtime::Agent` library API. Spec §The Runtime.
- `tracer/` — standalone syscall-trace tool; operator's choice for post-dispatch subprocess observability, peer to Docker/gVisor/strace, not part of the Gate architecture.
- `dashboard/` — local workspace console.
- `daemon/` — placeholder for the cross-deployment Receipt store.
- `examples/charters/` — Charter templates per domain plus the verification-pipeline Charters (synthetic-data, gold-labeler, same-context-baseline).
- `examples/scenarios/` — checked-in corpora per Charter.
- `examples/demo/` — runnable M&A diligence deployment with both operating modes.
- `scripts/` — operator helpers (Compare-mode runner, passive-mode runner, CI checks).

## License

Dual-licensed under MIT and Apache 2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
