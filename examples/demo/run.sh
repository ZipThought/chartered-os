#!/usr/bin/env bash
# Project Citadel diligence demo launcher.
#
# Boots the workspace console (dashboard/) against examples/demo/'s
# self-contained M&A workspace + Charter, with the runtime built once
# and reused per Tool dispatch.
#
# Two operating modes share the same Charter and the same Receipt
# trail:
#   active   Dashboard with select-and-click triggers. The analyst
#            drives each Task explicitly.
#   passive  Data-room watcher. Each new file under the data-room
#            subtree fires one governed Runtime invocation; restraint
#            is the load-bearing behavioral property — most arrivals
#            produce no externally visible finding.
#
# Usage:
#   ./examples/demo/run.sh                       # default mode=active
#   ./examples/demo/run.sh --mode passive        # passive watcher
#   ./examples/demo/run.sh --mode both           # both, dashboard fg, watcher bg
#   PORT=5180 ./examples/demo/run.sh
#   LLM_BASE_URL=https://api.openai.com/v1 LLM_MODEL=gpt-4o-mini \
#     LLM_API_KEY=sk-... ./examples/demo/run.sh
#
# Environment overrides:
#   PORT          dashboard port (default 5177)
#   LLM_BASE_URL  OpenAI-compatible endpoint
#   LLM_MODEL     model identifier
#   LLM_API_KEY   API key (optional for local servers)
#   PROFILE       cargo build profile: debug | release (default debug)

set -euo pipefail

MODE="active"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="$2"
      shift 2
      ;;
    *)
      echo "usage: $0 [--mode active|passive|both]" >&2
      exit 64
      ;;
  esac
done
if [[ "$MODE" != "active" && "$MODE" != "passive" && "$MODE" != "both" ]]; then
  echo "--mode must be active | passive | both" >&2
  exit 64
fi

# Resolve repo root from this script's location, regardless of CWD.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEMO_ROOT="$SCRIPT_DIR"
WORKSPACE_ROOT="$DEMO_ROOT/workspace"
CHARTERED_DIR="$DEMO_ROOT/.chartered"

PROFILE="${PROFILE:-debug}"
PORT="${PORT:-5177}"
LLM_BASE_URL="${LLM_BASE_URL:-http://localhost:1234/v1}"
LLM_MODEL="${LLM_MODEL:-openai/gpt-oss-20b}"
LLM_API_KEY="${LLM_API_KEY:-}"

echo "==> Building chartered-runtime ($PROFILE)"
if [[ "$PROFILE" == "release" ]]; then
  cargo build --quiet --manifest-path "$REPO_ROOT/runtime/Cargo.toml" --release --bin chartered-runtime
  RUNTIME_BIN="$REPO_ROOT/runtime/target/release/chartered-runtime"
else
  cargo build --quiet --manifest-path "$REPO_ROOT/runtime/Cargo.toml" --bin chartered-runtime
  RUNTIME_BIN="$REPO_ROOT/runtime/target/debug/chartered-runtime"
fi

if [[ ! -x "$RUNTIME_BIN" ]]; then
  echo "Runtime binary not found at $RUNTIME_BIN" >&2
  exit 1
fi

# Reset per-run state (receipts, cognition trace) so each demo invocation
# starts from a clean trail.
rm -rf "$CHARTERED_DIR/runs"
rm -f "$CHARTERED_DIR/records.jsonl"

echo "==> Workspace : $WORKSPACE_ROOT"
echo "==> Charter   : $CHARTERED_DIR"
echo "==> LLM       : $LLM_BASE_URL ($LLM_MODEL)"
echo "==> Mode      : $MODE"

export WORKSPACE_ROOT CHARTERED_DIR PORT
export LLM_BASE_URL LLM_MODEL LLM_API_KEY
export CHARTERED_RUNTIME_BIN="$RUNTIME_BIN"

WATCH_DIR="$WORKSPACE_ROOT/data-room"
PASSIVE_PIDS=()
cleanup() {
  for pid in "${PASSIVE_PIDS[@]:-}"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT INT TERM

case "$MODE" in
  active)
    echo "==> Dashboard : http://127.0.0.1:$PORT"
    echo
    exec node "$REPO_ROOT/dashboard/local-api.mjs"
    ;;
  passive)
    echo "==> Watching : $WATCH_DIR"
    echo
    exec bash "$REPO_ROOT/scripts/passive-mode.sh" "$CHARTERED_DIR" "$WATCH_DIR"
    ;;
  both)
    echo "==> Dashboard : http://127.0.0.1:$PORT"
    echo "==> Watching : $WATCH_DIR"
    echo
    bash "$REPO_ROOT/scripts/passive-mode.sh" "$CHARTERED_DIR" "$WATCH_DIR" &
    PASSIVE_PIDS+=("$!")
    exec node "$REPO_ROOT/dashboard/local-api.mjs"
    ;;
esac
