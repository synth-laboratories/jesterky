#!/usr/bin/env bash
# jesterky publish runbook — the finish-line crossing. IRREVERSIBLE.
#
# Publishes the six crates to crates.io in dependency order (a dependent cannot
# publish until its deps are indexed) and the Python types to PyPI. Requires an
# explicit --confirm; a dry run without it prints the plan and stops.
#
# Prereqs (do once):
#   - crates.io: `cargo login <token>` (token from https://crates.io/settings/tokens)
#   - PyPI:      set UV_PUBLISH_TOKEN, or `uv publish --token <pypi-token>`
#   - claim the org/repo: https://github.com/jesterky
#
# Usage:
#   scripts/publish.sh            # dry run: preflight + plan, publishes NOTHING
#   scripts/publish.sh --confirm  # actually publish (after review)
set -euo pipefail
cd "$(dirname "$0")/.."

CONFIRM="${1:-}"
# Dependency order: a crate must be indexed before anything that depends on it.
ORDER=(jesterky-contract jesterky-core jesterky-actor jesterky-model jesterky-quality jesterky-cli)

echo "== preflight =="
cargo test --workspace
cargo publish --dry-run -p jesterky-contract
echo "preflight ok"
echo
echo "== plan (dependency order) =="
for c in "${ORDER[@]}"; do echo "  cargo publish -p $c"; done
echo "  (then) cd python && uv build && uv publish"
echo

if [ "$CONFIRM" != "--confirm" ]; then
  echo "DRY RUN — nothing published. Re-run with --confirm to cross the line."
  exit 0
fi

wait_indexed() {  # poll crates.io until the just-published version is queryable (<=60s)
  local crate="$1" tries=0
  until curl -s -A "jesterky-publish (jmvpurtell@gmail.com)" \
        "https://crates.io/api/v1/crates/$crate/0.1.0" | grep -q '"num":"0.1.0"'; do
    tries=$((tries + 1)); [ "$tries" -ge 12 ] && { echo "timeout waiting for $crate to index"; exit 1; }
    echo "  waiting for $crate@0.1.0 to index ($((tries * 5))s)…"; sleep 5
  done
  echo "  $crate@0.1.0 indexed"
}

echo "== publishing crates =="
for c in "${ORDER[@]}"; do
  echo "-- $c --"
  cargo publish -p "$c"
  wait_indexed "$c"
done

echo "== publishing python (PyPI) =="
( cd python && uv build && uv publish )

echo
echo "DONE. Crates + PyPI published at 0.1.0."
echo "Finish line remaining (manual): make github.com/jesterky public; flip blog status: draft -> live."
