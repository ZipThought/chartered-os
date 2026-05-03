# Context: wire protocol

Versioned by field numbering. Inherits root `CONTEXT.md`.

## Spec section

§The Protocol.

## Layout

- `v1/tool_call.proto` — the proposal packet.
- `v1/verdict.proto` — per-Frame Ruling + within-chain trace,
  `EvaluatorEntry`, `Metrics`.
- `v1/receipt.proto` — append-only Gate-step record, `Outcome` and
  `EnforcementLevel` enums.
- `v1/evaluate.proto` — Runtime ↔ Daemon ↔ Adapter messages.

Future versions ship under `v2/`, `v3/`, etc. Renumbering or removing
fields is a wire-breaking change and requires a new version directory;
adding fields is forward-compatible (new fields ignored by old readers,
absent fields default).

## Authority over `core/`

The Rust types in `core/` MUST stay wire-compatible with these proto
messages. CHECKLIST §Spec-Code Traceability mandates one Rust struct
per proto message. Drift between spec §Receipts content list and
`receipt.proto` was caught and fixed earlier; the alignment principle
is general.

## What does NOT live here

- The Rust generated bindings. Step-1 hand-codes the kernel types in
  `core/`; protoc-generated Rust ships when the Daemon and Adapters
  land at the layered phase.
- Application-level message types (Charter file format, scenario JSON
  for the Runtime binary). Those are tooling formats, not the wire
  protocol.
