# Context: dispatcher (OS-touching ToolExecutors)

The Tools that touch the operating system. Inherits root `CONTEXT.md`.

## Spec section

§Tools — "executor (in-Runtime native, peer process, contained
subprocess)". This crate provides the in-Runtime-native executors.

## Why this exists separate from `core/`

Per root `CONTEXT.md`: stdlib effect surfaces (`std::fs`,
`std::process`, `tokio::fs`, `tokio::process`, `tokio::net`) are
**quarantined** here. The kernel (`chartered-core`) stays in-memory;
the harness Tools there are pure in-memory stand-ins for kernel tests.
This crate is the only path from the Runtime's Tool dispatcher to the
operating system.

## What lives here

- `NativeFsRead` (`fs.rs`) — `read_file(path)`. Reads UTF-8.
- `NativeFsWrite` (`fs.rs`) — `write_file(path, content)`. Creates
  intermediate directories within the workspace root.
- Native artifact executors (`artifact.rs`) — `read_artifact`,
  `modify_artifact`, `list_artifacts`. Artifact ids are workspace-relative
  UTF-8 file paths; findings are recorded by Charters as
  `modify_artifact` calls with `kind=record-store`, appending to
  `.chartered/<artifact_id>.jsonl` (the default record store registers
  artifact_id `records`, persisting at `.chartered/records.jsonl`).
- `NativeExec` (`exec.rs`) — `exec_command(cmd, args)`. Spawns,
  captures stdout/stderr, reports exit code.
- `ExecutorRegistry` (`registry.rs`) — maps the deployment config's
  `executor` strings (`"native_fs_read"`, `"native_fs_write"`,
  `"native_exec"`, artifact executor names) to ToolExecutor instances keyed by the
  deployment's `tool_id`.

## Workspace-root containment

Every executor is constructed with a workspace root and rejects any
path that resolves outside it after canonicalization. Symlink escapes
are caught because canonicalize follows symlinks. Path traversal
denials surface as `ToolResult::Err`, never silently allow.

## What does NOT live here

- Tools that are not in-Runtime-native: peer-process Adapters and
  contained-subprocess executors live in their own crates (future).
- Charter / RoleContext loading: `runtime::charter_loader` (deployment
  IO) + `chartered_core::parse_charter_def` / `parse_role_context_def`
  (pure kernel parsers); deployment-config loading: `runtime::config`.
- Network tools: future.

## Tests

- `tests/fs_executors.rs` — write/read roundtrip, path traversal
  rejection (relative `..` and absolute outside root), missing fields,
  intermediate directory creation.
- `tests/exec_executor.rs` — echo captures stdout, nonzero exit code
  reported, missing `cmd` field rejected, unknown command spawn
  failure surfaces as `ToolResult::Err`.
- `tests/artifact_executors.rs` — selection read, governed splice,
  finding append, artifact listing, traversal rejection.
- `tests/registry.rs` — known executor names build, unknown names
  produce a structured error.
