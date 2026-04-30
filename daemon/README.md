# charteredd

The Chartered Daemon. A long-running per-host (or per-organization) process that owns the Receipt store and serves the Workspace UI across multiple Steward deployments.

## Role

A Steward runs in its own per-deployment Runtime. The Runtime hosts the Gate and writes Receipts. The Daemon receives Receipts from one or more Runtimes, persists them in the Receipt store, and exposes the Professional's audit query surface via the Workspace UI.

Two reasons for the Runtime/Daemon split rather than a single combined process:

- **Cross-deployment audit.** Operators running multiple Stewards (a customer-service Steward, a medical-reception Steward, a coding Steward) want a single Receipt store to query against. The Daemon is the shared store.
- **Operator surface persistence.** The Workspace UI, the CLI, the streaming Receipt feed — these belong on a process that is up regardless of which Steward is running.

The Runtime can also operate standalone when the Daemon is unavailable (writes Receipts to local per-process files; the Daemon ingests them when it returns). The Runtime is not blocked by Daemon unreachability; Receipts are durable on either path.

## What the Daemon Does

- **Receipt ingestion** from one or more Runtimes over the protobuf protocol on a Unix domain socket.
- **Receipt storage.** SQLite by default with per-day partitioning; append-only file backend available; tamper-evidence (hash chaining or append-only with checksum) on the chain.
- **Access-controlled query.** "Show me UNGROUNDED Receipts for Frame X this week." "Show me every tool call from Steward Y reaching endpoint Z." Per-field redaction of declared-sensitive fields at query response.
- **Workspace UI.** Web app the Professional uses for Charter authoring, Role context confirmation, work execution, Findings review, Receipt query, and Steward configuration. Foundation Stewards operate here.
- **Adapter-fronted evaluators.** When a Frame's chain references an adapter-fronted evaluator, the Daemon coordinates the call (Adapter peer process, protobuf request/response, evaluator trace into the Receipt).

## What the Daemon Does Not Do

- Does not own the Steward loop. The Runtime owns the propose-evaluate-refine cycle.
- Does not own the Gate's allow/deny decision. Decisions are made by the Runtime's local Gate; the Daemon's role over Receipts is post-hoc audit and async receipt-corpus analysis.
- Does not watch the host environment. It receives Receipts; it does not introspect arbitrary processes.

## References

`docs/SPECIFICATION.md > Receipts and Audit` and `docs/SPECIFICATION.md > The Runtime > Operator Surface` for the architecture; `proto/v1/` for the protocol definitions.
