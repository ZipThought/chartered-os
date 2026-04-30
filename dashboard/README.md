# Workspace UI

The Professional's surface. A local web application served by `charteredd` that exposes Charter authoring, Role context confirmation, audit query, refinement metrics, and live Receipt streaming for Stewards.

## Five Panels

- **Scope selection.** The Professional's operational world — which Scopes apply, which Stewards are bound, which artifacts are in play.
- **Work area.** Artifact editing with inline action triggers. Where generative work happens. Findings and Receipts surface in adjacent panels as actions occur.
- **Findings.** Review-Steward output anchored to artifact concerns. The concern is the Finding's identity; artifact location is navigational metadata.
- **Receipt trail.** Every governance decision: tool call, every Frame's Verdict, the Outcome, the Charter version + Role context version active at evaluation time. Allowed, escalated, denied — unified.
- **Steward configuration.** Charter Scopes (read-only by Professional), Role context Scopes (editable), Frame definitions (read-only), behavioral specification (read-only), governance mode toggles, version display.

## What Is Surfaced

- **Charter authoring.** Foundation Stewards (Charter Review, Charter Editor, Frame Decomposition) operate on the Charter through the Workspace under their own Charters' governance.
- **Audit query.** Receipts queryable by Tool, Frame, Outcome, time range, Steward identity, content patterns. Per-field redaction respected at query response.
- **Refinement metrics.** First-pass authorization rate, mean iterations to grounded, escalation rate, recovery rate — exposed as a dashboard and as CSV export for external compliance reporting.
- **Live observation.** Streaming view of Receipts as they're written. The "watch the Steward run" view.
- **Per-Receipt drill-down.** Which evaluator in which Frame's chain produced which Decision, with metrics.

The Workspace UI is governance-side admin tooling. Application-layer monitoring of business KPIs (task throughput, cost per task, customer satisfaction) is the operator's separate concern.

## References

`docs/SPECIFICATION.md > The Workspace` for the surface; `docs/SPECIFICATION.md > The Runtime > Operator Surface` for what the Daemon hosts; `proto/v1/` for the streaming Receipt protocol.
