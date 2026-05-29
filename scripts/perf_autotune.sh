#!/usr/bin/env bash
#
# Gated perf-autotune runner.
#
# Searches kernel candidates with perf-lab, runs the hard correctness gates, and
# — only if everything is green — commits the refreshed PERF_REPORT.md and pushes
# directly to main. Intended to be driven on a loop by a local Claude Code
# session (e.g. `/loop 30m /perf-autotune`) or by cron.
#
# Safety model: the engine's *shipped* kernels are pure safe Rust; perf-lab's
# inline-assembly candidates are only measured/verified in the harness and never
# enter the build path here, so an autonomous run can only update the report.
# Promoting a champion into the engine is a separate change that must pass the
# same `cargo test --all` gate (which includes the differential kernel tests).

set -euo pipefail
cd "$(dirname "$0")/.."

ITERATIONS="${1:-10000}"
SEED="${RANDOM}${RANDOM}"

echo "==> perf-lab search (${ITERATIONS} iterations, seed ${SEED})"
cargo run --release -q -p glasssheet-perf-lab -- \
    --iterations "${ITERATIONS}" --seed "${SEED}" --out PERF_REPORT.md

echo "==> correctness gates (fmt / clippy / tests incl. differential kernels)"
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all

echo "==> bench (informational; regressions are judged against the prior report)"
cargo bench -p glasssheet-engine >/dev/null 2>&1 || true

if git diff --quiet -- PERF_REPORT.md; then
    echo "==> report unchanged; nothing to push"
    exit 0
fi

echo "==> gates green and report changed; committing + pushing to main"
git add PERF_REPORT.md
git commit -m "perf: autotune report $(date -u +%Y-%m-%dT%H:%MZ) (seed ${SEED})"
for n in 1 2 3 4; do
    if git push origin HEAD:main; then
        exit 0
    fi
    sleep $((2 ** n))
done
echo "!! push failed after retries" >&2
exit 1
