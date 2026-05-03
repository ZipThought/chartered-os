# Project Citadel — M&A diligence demo

Self-contained demo of the OS-contract architecture against an M&A
diligence persona. The workspace is a directory on disk (Vesper
Capital's Project Citadel data room + buy-side memos); the Charter
declares a diligence Steward with three Frames (citation grounding,
diligence discipline, severity calibration); the kernel dispatches
through the kind-typed artifact substrate the same way every other
persona will.

## What this demo proves

- A real-shape persona workspace (M&A) is just `(directory + Charter
  + Steward graph)` running on the unchanged kernel.
- The Tool ABI is fixed at `read_artifact` / `modify_artifact` /
  `list_artifacts` / `record_finding`; substrate variation is below
  the line, in ArtifactBackends. The same dashboard, same runtime,
  same Receipts trail as the existing text-blob demo.
- Findings flow into a `kind=findings-store` artifact; text artifacts
  are `kind=text`; both reached through the same Tools, dispatched by
  kind explicitly, no ownership heuristic.
- The Charter's Steward-owned Frames make the diligence Steward auditable: every
  finding cites quoted text, addresses a concrete deal-relevant
  issue, and carries a calibrated severity. Tasks group the professional
  action, Attempts record each proposal, and Receipts record every Gate
  decision or controller event.

The substrate-swap claim: replace the local-directory backing of
`workspace/` with a live VDR backend (Datasite, Intralinks, Box) and
nothing in the Charter, behavioral spec, Steward-owned Frames, or
Steward changes. Same Task/Attempt/Receipt shape; same audit trail.

## Running the demo

Prerequisites:
- Rust toolchain (the launcher builds `chartered-runtime` once).
- Node.js (the launcher boots the dashboard).
- An OpenAI-compatible LLM endpoint. Default is a local server at
  `http://localhost:1234/v1` with model `openai/gpt-oss-20b`. Override
  via env vars (see below).

```bash
./examples/demo/run.sh
```

Then open http://127.0.0.1:5177.

Environment overrides:
```bash
PORT=5180 ./examples/demo/run.sh                          # alternate port
LLM_BASE_URL=http://<host-ip>:1234/v1 ./examples/demo/run.sh  # dynamic host endpoint
LLM_BASE_URL=https://api.openai.com/v1 \
  LLM_MODEL=gpt-4o-mini \
  LLM_API_KEY=sk-... \
  ./examples/demo/run.sh                                   # OpenAI hosted
PROFILE=release ./examples/demo/run.sh                     # release build
```

The launcher resets `findings.jsonl` and the per-run Receipt trail on
each invocation so the demo starts from a clean state.

## Walkthrough

The dashboard's left rail shows the workspace tree, Charter, Tasks,
Receipts, and Findings store. Tasks are the user-facing work units;
Receipts are nested audit evidence. The center pane renders the
selected artifact.

### Try a generative diligence action

1. Open `memos/deal-thesis.md`.
2. Select a sentence in the Risks section
   (e.g., "The largest unresolved risks remain customer concentration…").
3. Click **Refine**.
4. Observe in the right rail:
   - The Steward's proposed Tool call (`modify_artifact` with
     `kind=text`).
   - The Gate's per-Frame Verdicts (citation grounding, diligence
     discipline).
   - The Outcome (`Allowed` / `Denied` / `Escalated`).
   - The Task containing the Attempt and resulting Receipt persisted
     to `.chartered/runs/<id>/`.

The memo updates in place if the Outcome is `Allowed`.

### Try an evaluative diligence action

1. Open `data-room/customer-northstar.md`.
2. Select Section 11.1 (the change-of-control assignment clause).
3. Click **Review**.
4. The Steward proposes a `record_finding` Tool call. The Gate evaluates
   citation grounding, diligence discipline, and severity calibration.
   On `Allowed`, a Finding is appended to the `findings-store` artifact
   (visible in the left-rail Findings node).

Try the same on `data-room/apa-draft-v3.md` Section 8.1 (the MAC
carve-outs) and `data-room/employment-cto-singh.md` Section 7.1
(Good Reason triggers). Each should surface a different deal-relevant
issue.

### Inspect the task and audit trail

The left rail's Tasks node shows each professional action as one work
unit. Open a Task to see its Attempts, denied Receipts, accepted Receipt,
or controller event such as budget exhaustion. The Receipts node remains
available for audit inspection. Click a Receipt to see the Tool call,
per-Frame Verdicts (with FrameRef and reasons), Charter version +
RoleContext version + Snapshot ID, and cognition trace from the Actor
turn that produced it.

The Findings node shows every accepted finding. Each carries the
authoring Steward, the source artifact, the cited range, and a link
back to the Task and Receipt that admitted it.

## What the diligence Steward should find

The demo's data-room documents have specific deal-relevant issues
seeded into them. A well-Charter-bound Steward will surface them; a
loose one will not. Use these as ground truth when reviewing Receipts:

**`apa-draft-v3.md`**
- §7.2 indemnity cap is "commercially reasonable amount" — undefined.
- §8.1 MAC carve-outs include pandemic and climate events
  (broader than market).
- §9.1 Reference Working Capital pinned to a stale date (12/31/2024).
- Article 3 lacks a representation regarding pending litigation.

**`customer-northstar.md`**
- §11.1 Change of Control allows NorthStar to withhold consent in its
  sole discretion (gating; this is the largest customer).
- §14.3 Customer holds an asymmetric termination-for-convenience right.

**`employment-cto-singh.md`**
- §4.3 references Schedule A definitions of Cause and Good Reason that
  are not in the data room.
- §7.1(d) "Good Reason" includes a change in reporting line — Vesper's
  portfolio plan would activate this trigger.
- §9.1 24-month US-wide non-compete on a CA-resident employee
  (likely unenforceable in California).

A Steward operating without the citation-grounding Frame will quote
nothing; one without diligence discipline will produce vague concerns
("clause could be clearer"); one without severity calibration will
mark everything `high`. The Receipts trail makes each failure
inspectable.

## Layout

```
examples/demo/
├── README.md              this file
├── run.sh                 launcher (builds runtime, boots dashboard)
├── workspace/             the M&A workspace (text artifacts on disk)
│   ├── data-room/         seller-disclosed documents
│   │   ├── apa-draft-v3.md
│   │   ├── customer-northstar.md
│   │   └── employment-cto-singh.md
│   └── memos/             buy-side work product
│       └── deal-thesis.md
└── .chartered/            CharteredOS deployment
    ├── chartered.toml     governance toggles (full grounding + evaluation)
    ├── steward.toml       Steward system prompt + backends
    ├── charter.toml       points at ./charter, version 1
    ├── role_context.md    deal context (parties, exclusivity, today)
    ├── charter/
    │   ├── behavioral_spec.md   human authoring surface for conduct/actions
    │   ├── frames.toml          three Frames + permitted_tools
    │   └── scopes.md            Charter scopes the Frames evaluate against
    └── tools/
        ├── modify_artifact.toml
        ├── record_finding.toml
        ├── read_artifact.toml
        └── list_artifacts.toml
```

## Architectural pointers

- `docs/SPECIFICATION.md` §The Loop, §Tools, §The Charter, §Vocabulary.
- `core/src/artifact.rs` (`ArtifactStore`, `ArtifactBackend`,
  kind-typed dispatch).
- `dispatch/src/artifact.rs` (`FilesystemTextBackend`,
  `FilesystemFindingsBackend`).
