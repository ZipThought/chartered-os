# CharteredOS

> Today's intelligent agents are open-loop systems. Open-loop systems cannot be made reliable by tuning the plant — better models, better prompts, more guardrails, more training. Cybernetics established this in the mid-20th century: reliability is a property of the enclosure, not the cognition inside it. The AI field has spent its agentic-era effort on the plant; the missing engineering move is the enclosure. **CharteredOS is that enclosure for intelligent agents.**

The enclosure places a comparator between every proposed action and its effect. Each proposal is evaluated against Charter-defined setpoints — Steward-owned Frames, in Minsky's 1974 sense, with applicability conditions, declared Scopes, and an evaluator chain. The error signal is projected back to the agent as a Refinement signal. Refinement iterates inside a Task until the action is grounded or escalated. Attempts record each proposal; Receipts are the immutable audit evidence for every Gate step and controller event.

A *Steward* is a chartered-resident agent operating under this enclosure. Frames are weak entities under their owning Steward; cross-boundary identity is a FrameRef `{ steward_id, frame_id }`. Tasks, Attempts, the Gate, and the Receipt trail are the architecture; nothing else differs between a test deployment and a production deployment.

![The medical-reception scenario, side-by-side](docs/assets/medical-reception-loop.png)

For operators who need governed agents in domains (medical, financial, legal, gov, sensitive customer service) where Tasks show what work was attempted and the Receipt trail proves what was admitted, denied, or escalated. Does *not* wrap unmodified third-party agents at the syscall layer; that trust property cannot be retrofitted, and well-trodden options exist for syscall-level isolation (Docker, gVisor, AppArmor, network policies).

## Status

Open-source contribution from an internal research prototype. Run locally — clone the repo, point an OpenAI-compatible LLM endpoint at it (hosted OpenAI, or a local server such as LM Studio, llama.cpp, vLLM, SGLang), and exercise the `examples/demo/` deployment.

No warranty whatsoever. The MIT and Apache 2.0 licenses (`LICENSE-MIT`, `LICENSE-APACHE`) are explicit: the software is provided as-is, without warranty of any kind.

Not for production use yet. To pilot CharteredOS in your organisation, contact [ZipThought](https://www.zipthought.com.au/).

## Repository

- `docs/SPECIFICATION.md` — the architecture, sovereign.
- `docs/IMPLEMENTATION_CHECKLIST.md` — verification diagnostics.
- `proto/v1/` — wire protocol.
- `examples/charters/` — Charter templates per domain.
- `core/` — kernel library (Task, Attempt, Gate, Receipt, LoopRunner, canonical LLM-backed role implementations).
- `dispatch/` — OS-touching ToolExecutor implementations; the only crate that touches `std::fs`, `std::process`, `tokio::net`.
- `runtime/` — per-deployment Runtime binary (spec §The Runtime); E2E tests target this binary.
- `tracer/` — standalone syscall-trace tool; operator's choice for post-dispatch subprocess observability, peer to Docker/gVisor/strace, not part of the Gate architecture.

## License

Dual-licensed under MIT and Apache 2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
