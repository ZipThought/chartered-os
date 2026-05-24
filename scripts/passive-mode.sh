#!/usr/bin/env bash
# Passive-mode runner: watch a workspace directory and fire one
# governed Runtime invocation per new file. Demonstrates the
# environment-embedded agent pattern using existing primitives — no
# new ArtifactBackend; the Steward receives each arrival as a Brief.
#
# Usage:
#   ./scripts/passive-mode.sh <chartered_dir> <watch_dir> [<idle_seconds>]
#
# Arguments:
#   chartered_dir  A real `.chartered/` deployment.
#   watch_dir      Directory to watch for new files (typically the
#                  data-room subtree of the workspace).
#   idle_seconds   Optional. Exit when no new file arrives within
#                  this many seconds. Default: keep running.
#
# Each new file produces one invocation of `chartered-runtime` with a
# Brief::Prompt naming the arrival. The runtime's per-call run dir
# captures the receipts; the receipts trail shows how many invocations
# externalized (a finding surfaced) versus how many stayed Quiet (the
# agent observed and chose not to externalize). The signal-to-noise
# ratio across many arrivals is the passive-mode demo's claim.

set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <chartered_dir> <watch_dir> [<idle_seconds>]" >&2
  exit 64
fi

CHARTERED_DIR="$(cd "$1" && pwd)"
WATCH_DIR="$(cd "$2" && pwd)"
IDLE_SECONDS="${3:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

RUNTIME_BIN="$REPO_ROOT/runtime/target/debug/chartered-runtime"
if [[ ! -x "$RUNTIME_BIN" ]]; then
  RUNTIME_BIN="$REPO_ROOT/runtime/target/release/chartered-runtime"
fi
if [[ ! -x "$RUNTIME_BIN" ]]; then
  echo "==> Building chartered-runtime (debug)"
  (cd "$REPO_ROOT/runtime" && cargo build --quiet --bin chartered-runtime)
  RUNTIME_BIN="$REPO_ROOT/runtime/target/debug/chartered-runtime"
fi

WORKSPACE_ROOT="$(dirname "$CHARTERED_DIR")"

echo "==> Watching $WATCH_DIR"
echo "==> Chartered $CHARTERED_DIR"
echo "==> Idle exit  ${IDLE_SECONDS}s (0 = keep running)"

# Snapshot the initial set of files so existing artifacts don't
# trigger spurious invocations on startup.
mapfile -t SEEN < <(find "$WATCH_DIR" -type f 2>/dev/null | sort)
declare -A SEEN_MAP=()
for p in "${SEEN[@]:-}"; do
  [[ -n "$p" ]] && SEEN_MAP["$p"]=1
done

LAST_EVENT=$(date +%s)
while true; do
  # Polling loop — no inotify dependency, works in containers and
  # WSL. The polling cadence (1s) is the slowest the dashboard
  # would update its passive-mode feed; tighten if needed.
  sleep 1
  mapfile -t CURRENT < <(find "$WATCH_DIR" -type f 2>/dev/null | sort)
  NEW_FOUND=false
  for p in "${CURRENT[@]:-}"; do
    [[ -z "$p" ]] && continue
    if [[ -z "${SEEN_MAP[$p]:-}" ]]; then
      SEEN_MAP["$p"]=1
      rel="${p#"$WATCH_DIR"/}"
      brief="A new document arrived in the data room at \`$rel\`. Consider it under the diligence Charter; halt if no finding is warranted."
      echo "==> New: $rel"
      "$RUNTIME_BIN" \
        --chartered-dir "$CHARTERED_DIR" \
        --workspace-root "$WORKSPACE_ROOT" \
        --user-message "$brief" \
        | python3 -c "import json,sys; d=json.load(sys.stdin); print(f'    outcome={d.get(\"outcome\",{}).get(\"kind\")} run_id={d.get(\"run_id\")} receipts={len(d.get(\"receipts\",[]))}')" \
        || echo "    (runtime exited nonzero or produced non-JSON output)"
      LAST_EVENT=$(date +%s)
      NEW_FOUND=true
    fi
  done
  if [[ "$IDLE_SECONDS" -gt 0 && "$NEW_FOUND" == "false" ]]; then
    NOW=$(date +%s)
    if (( NOW - LAST_EVENT >= IDLE_SECONDS )); then
      echo "==> Idle for ${IDLE_SECONDS}s; exiting."
      exit 0
    fi
  fi
done
