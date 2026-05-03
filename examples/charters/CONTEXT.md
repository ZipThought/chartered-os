# Context: Reference Charter templates

Concrete Charter templates per domain. Spec §Tools and §Reference
Charters point here. Inherits root `CONTEXT.md`. Loaded by
`core::charter_loader::load_charter_def`.

## Domains

- `medical-reception/` — fictional Australian general practice
  (Greenmount Medical Centre); Frames over billing accuracy, privacy
  disclosure, scope of procedures. All declared scopes are Role
  context (operator-supplied facts). The worked example throughout
  `docs/SPECIFICATION.md` (§The Loop, §Structural Separation).
- `customer-service/` — fictional appliance retailer (Meridian
  Electronics); Frames over pricing accuracy, returns/warranty
  compliance, scope adherence. All declared scopes are Role context
  (operator-supplied facts).
- `coding-agent/` — filesystem scope, no privilege escalation, no
  destructive ops without confirmation, no exfiltration. All declared
  scopes are Charter (framework authority). The framework's primary
  deliverable target.

## File layout per domain

- `frames.toml` — `permitted_tools` plus `[[frames]]` tables. Each
  Frame has `id`, `concern`, `applies_to_tools`, typed
  `declared_scopes` (each entry: `{ name, kind = "Charter"|"RoleContext" }`),
  optional `prior_receipt_queries` (each entry: `{ frame_id_filter?,
  limit }`).
- `scopes.md` — Charter Scopes only. Each `## Heading` becomes a scope
  name via slugify (lowercase, non-alphanumeric runs collapsed to `_`).
  May be empty for domains where all evaluation evidence is
  operator-supplied (see customer-service).
- `role_context_template.md` (when present) — reference template for
  Role context Scopes the Professional fills in at deployment. Same
  markdown-section format as `scopes.md`.

## Authority

These files are the Charter engineer's reference shape. The Step-1
Runtime binary scenario mode constructs Charters from JSON; the TOML
loader (`core::charter_loader`) produces a `CharterDef` /
`RoleContextDef` from these files for the production-mode binary
(when it ships).

## What is missing

Per spec §The Charter, a complete Charter ships Charter Scopes, Frame
definitions, **behavioral specification**, **expected Role context
templates**, and **a fully-worked Steward model**. Examples currently
ship `frames.toml` + `scopes.md` + (where relevant)
`role_context_template.md`. Behavioral spec and a fully-worked Steward
model land as the relevant kernel surfaces stabilize.
