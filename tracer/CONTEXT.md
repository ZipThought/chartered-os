# Context: companion syscall tracer

NOT part of the CharteredOS Gate architecture. Inherits root
`CONTEXT.md` only for the prohibition list (silent failure, finite-spec
verdicts on LLM output) — none of which apply here because this binary
does not participate in the loop.

## Spec section that mentions this

§Tools — listed alongside Docker / gVisor / strace as one option for
operators who want post-dispatch syscall observability of subprocesses
dispatched via `exec_command`. Provides reconciliation evidence
(CHECKLIST §Receipt System > Reconciliation) by capturing what the
subprocess actually did at the syscall boundary.

## CLI

    chartered-trace <cmd> [args...]
    CHARTERED_LOG=path overrides the default log location.

Wraps the target with a seccomp-notify filter (passthrough). Logs
intercepted syscalls to `chartered.log`. Allow-only — observation,
never enforcement. TOCTOU on argv decoding makes this unsafe for
enforcement; use as a tracer only.

## Why this lives in this repo

The Gate evaluates the proposed `exec_command(cmd, args, env)` at the
tool-call boundary. What the subprocess does after dispatch is outside
the Gate's view. Operators that need post-dispatch observability
deploy a tracer; this binary is one option, shipped here for
convenience but architecturally independent.

## What does NOT live here

Anything that participates in the Gate, the Receipt trail, the loop,
the Charter, or any LLM-backed role. This binary has no dependency on
`core/` and shall not gain one.
