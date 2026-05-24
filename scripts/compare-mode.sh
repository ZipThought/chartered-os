#!/usr/bin/env bash
# Compare-mode runner: fan one corpus across four governance
# configurations and emit per-configuration scenario-suite reports
# side-by-side plus a compact summary aggregating totals.
#
# Usage:
#   ./scripts/compare-mode.sh <chartered_dir> <corpus_dir> [<out_dir>]
#
# Arguments:
#   chartered_dir   A real `.chartered/` deployment directory. The
#                   governed runs reuse this Charter, role context,
#                   steward.toml, tools/, and workspace; the only
#                   difference between the two governed configurations
#                   is `chartered.toml`'s governance toggles. The
#                   ungoverned configurations bypass the chartered
#                   kernel entirely.
#   corpus_dir      A directory containing `corpus.jsonl` per the
#                   scenario_suite shape.
#   out_dir         Optional. Where to write the per-config reports.
#                   Defaults to a tempdir; the script prints the path.
#
# Four configurations:
#   naked              No governance, no judge. The Actor's commitment
#                      IS the outcome. Ungoverned strawman; reveals
#                      the model's behaviour without any enclosure.
#   same_context_judge Actor + in-conversation judge in the same
#                      conversation. The judge sees the Actor's
#                      reasoning prefix. Ungoverned strawman the
#                      chartered kernel's persuasive-context-exclusion
#                      invariant defends against.
#   separated_judge    Chartered kernel with `governance.grounding =
#                      false` and `governance.evaluation = true`. The
#                      Gate runs; Charter scopes are NOT injected into
#                      the Actor's prompt. Isolates the separation
#                      contribution from grounding.
#   separated_grounded Full chartered kernel: grounding + evaluation
#                      both on. The production configuration.
#
# Each report lands at `<out_dir>/<config>.json`. A compact
# `summary.json` aggregates totals + per-failure-class + per-technique
# across the four configurations.

set -euo pipefail

GOVERNED_ONLY=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --governed-only)
      GOVERNED_ONLY=true
      shift
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -lt 2 ]]; then
  echo "usage: $0 [--governed-only] <chartered_dir> <corpus_dir> [<out_dir>]" >&2
  exit 64
fi

CHARTERED_DIR="$(cd "$1" && pwd)"
CORPUS_DIR="$(cd "$2" && pwd)"
OUT_DIR="${3:-$(mktemp -d)}"

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

UNGOVERNED_BIN="$REPO_ROOT/runtime/target/debug/examples/ungoverned_suite"
if [[ ! -x "$UNGOVERNED_BIN" ]]; then
  echo "==> Building ungoverned_suite example"
  (cd "$REPO_ROOT/runtime" && cargo build --quiet --example ungoverned_suite)
  UNGOVERNED_BIN="$REPO_ROOT/runtime/target/debug/examples/ungoverned_suite"
fi

mkdir -p "$OUT_DIR"

run_governed() {
  local label="$1"
  local grounding="$2"
  local evaluation="$3"

  local variant_dir="$OUT_DIR/$label"
  rm -rf "$variant_dir"
  cp -r "$CHARTERED_DIR" "$variant_dir"
  cat >"$variant_dir/chartered.toml" <<EOF
[governance]
grounding = $grounding
evaluation = $evaluation
EOF
  local workspace_root
  workspace_root="$(dirname "$variant_dir")"
  echo "==> $label (governed: grounding=$grounding, evaluation=$evaluation)"
  "$RUNTIME_BIN" \
    --chartered-dir "$variant_dir" \
    --workspace-root "$workspace_root" \
    --scenario-suite "$CORPUS_DIR" \
    > "$OUT_DIR/$label.json"
}

run_ungoverned() {
  local label="$1"
  local mode="$2"
  echo "==> $label (ungoverned: mode=$mode)"
  "$UNGOVERNED_BIN" "$mode" "$CORPUS_DIR" > "$OUT_DIR/$label.json"
}

if [[ "$GOVERNED_ONLY" != "true" ]]; then
  run_ungoverned naked              naked
  run_ungoverned same_context_judge same_context_judge
fi
run_governed   separated_judge    false true
run_governed   separated_grounded true  true

if [[ "$GOVERNED_ONLY" == "true" ]]; then
  COMPARE_LABELS='["separated_judge", "separated_grounded"]'
else
  COMPARE_LABELS='["naked", "same_context_judge", "separated_judge", "separated_grounded"]'
fi
export OUT_DIR COMPARE_LABELS
python3 - <<'PY' >"$OUT_DIR/summary.json"
import json, pathlib, os
out_dir = pathlib.Path(os.environ["OUT_DIR"])
labels = json.loads(os.environ["COMPARE_LABELS"])
summary = {"by_config": {}}
for label in labels:
    data = json.loads((out_dir / f"{label}.json").read_text())
    totals = data["totals"]
    summary["by_config"][label] = {
        "total": totals["total"],
        "passed": totals["passed"],
        "failed": totals["failed"],
        "by_failure_class": data.get("by_failure_class", {}),
        "by_technique": data.get("by_technique", {}),
        "by_cell": data.get("by_cell", {}),
    }
print(json.dumps(summary, indent=2))
PY

echo
echo "==> Reports in $OUT_DIR/"
echo "==> Summary: $OUT_DIR/summary.json"
