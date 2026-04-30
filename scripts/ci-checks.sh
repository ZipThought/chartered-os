#!/bin/bash
# CI sanity checks — single source of truth for cargo check, clippy, test.
# Called by: .github/workflows/ci.yml and .claude/hooks/pre-commit-audit.mjs
#
# Each Rust crate is its own self-contained project under its own
# subdirectory; there is no workspace at the repository root.
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

# Discover first-level crate directories (./<crate>/Cargo.toml).
mapfile -t CRATES < <(
    find . -mindepth 2 -maxdepth 2 -name Cargo.toml \
        -not -path "./.git/*" \
        -not -path "*/target/*" \
        -printf "%h\n" | sed 's|^\./||' | sort
)

if [ ${#CRATES[@]} -eq 0 ]; then
    echo "=== no Rust crates present yet — skipping cargo checks ==="
    exit 0
fi

for crate in "${CRATES[@]}"; do
    echo "=== $crate: cargo check ==="
    (cd "$crate" && cargo check)
    echo "=== $crate: cargo clippy ==="
    (cd "$crate" && cargo clippy -- -D warnings)
    echo "=== $crate: cargo test ==="
    (cd "$crate" && cargo test)
done
