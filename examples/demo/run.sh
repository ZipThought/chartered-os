#!/usr/bin/env bash
# Project Citadel diligence demo launcher.
#
# Boots the workspace console (dashboard/) against examples/demo/'s
# self-contained M&A workspace + Charter, with the runtime built once
# and reused per Tool dispatch.
#
# Usage:
#   ./examples/demo/run.sh              # default: local OpenAI-compatible server at localhost:1234/v1
#   PORT=5180 ./examples/demo/run.sh    # alternate port
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
rm -f "$CHARTERED_DIR/findings.jsonl"

echo "==> Workspace : $WORKSPACE_ROOT"
echo "==> Charter   : $CHARTERED_DIR"
echo "==> LLM       : $LLM_BASE_URL ($LLM_MODEL)"
echo "==> Dashboard : http://127.0.0.1:$PORT"
echo

export WORKSPACE_ROOT CHARTERED_DIR PORT
export LLM_BASE_URL LLM_MODEL LLM_API_KEY
export CHARTERED_RUNTIME_BIN="$RUNTIME_BIN"

exec node "$REPO_ROOT/dashboard/local-api.mjs"
