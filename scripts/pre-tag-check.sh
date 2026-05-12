#!/usr/bin/env bash
# Run the same fast lint gates that the Tests workflow's `lint` and
# `api-docs-drift` jobs run. Designed to be invoked manually before
# `git tag vX.Y.Z` or wired into a `pre-push` git hook for branches
# that push to main / tags.
#
# This catches:
#   - cargo fmt drift (Tests workflow `lint` job)
#   - untagged ```typescript fences in docs/src (Tests `doc-tests` job's
#     --lint pass)
#   - obvious cargo build / type errors via `cargo check` (Tests
#     `cargo-test` + `compile-smoke` builds)
#
# What it does NOT catch (still needs full CI):
#   - doc-test runtime behavior
#   - parity vs `node --experimental-strip-types`
#   - cross-compile builds, harmonyos smoke, etc.
#
# Exit 0 = clear to tag. Non-zero = fix what's reported and re-run.
# All checks run; we print every failure before exiting, so one run
# surfaces every issue instead of trickling one per `git push`.
#
# Usage:
#   ./scripts/pre-tag-check.sh
#   ./scripts/pre-tag-check.sh --quick   # skips cargo check (much faster)

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

quick=0
if [[ "${1:-}" == "--quick" ]]; then
    quick=1
fi

failures=()

step() {
    printf '\n\033[1;36m==>\033[0m %s\n' "$1"
}

run_check() {
    local label="$1"; shift
    step "$label"
    if "$@"; then
        printf '\033[1;32m   ok\033[0m: %s\n' "$label"
    else
        printf '\033[1;31m   FAIL\033[0m: %s\n' "$label"
        failures+=("$label")
    fi
}

# 1. cargo fmt --all -- --check (Tests `lint` job)
run_check "cargo fmt --all --check" cargo fmt --all -- --check

# 2. docs/src linter (Tests `doc-tests` matrix --lint pass)
run_check "perry-doc-tests --lint docs/src" \
    cargo run --release --quiet -p perry-doc-tests -- --lint docs/src

# 3. cargo check (catches type errors fast; Tests `cargo-test` builds
#    everything anyway). Skipped under --quick.
if [[ $quick -eq 0 ]]; then
    run_check "cargo check --release --workspace" \
        cargo check --release --workspace \
            --exclude perry-ui-ios --exclude perry-ui-tvos \
            --exclude perry-ui-watchos --exclude perry-ui-visionos \
            --exclude perry-ui-android --exclude perry-ui-windows \
            --exclude perry-ui-gtk4
fi

echo
if [[ ${#failures[@]} -eq 0 ]]; then
    printf '\033[1;32mAll pre-tag checks passed.\033[0m Safe to tag.\n'
    exit 0
fi

printf '\033[1;31mPre-tag checks FAILED:\033[0m\n'
for f in "${failures[@]}"; do
    printf '  - %s\n' "$f"
done
exit 1
