#!/usr/bin/env bash
# Bulk-run GEPA + GELO trace annotation workflows over a v4 trace corpus.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRACE_DIR="${TRACE_DIR:-$ROOT/proof/craftax_v4_traces}"
OUT_DIR="${OUT_DIR:-$ROOT/proof/craftax_trace_annotate}"
ACTOR="${ACTOR:-codex}"
MODEL="${MODEL:-gpt-5.5}"
JESTERKY_BIN="${JESTERKY_BIN:-$ROOT/target/release/jesterky}"

mkdir -p "$OUT_DIR"

if [[ ! -d "$TRACE_DIR" ]]; then
  echo "trace_dir missing: $TRACE_DIR (run scripts/capture_craftax_v4_traces.py first)" >&2
  exit 1
fi

if [[ ! -x "$JESTERKY_BIN" ]]; then
  echo "building jesterky release binary..."
  (cd "$ROOT" && cargo build --release -p jesterky-cli)
fi

ARGS_JSON="{\"trace_dir\":\"$TRACE_DIR\"}"
CD_ARG="${CD_ARG:-$ROOT}"

run_workflow() {
  local spec="$1"
  local name="$2"
  local manifest="$OUT_DIR/${name}.manifest.json"
  echo "==> $name (actor=$ACTOR model=$MODEL)"
  if [[ "$ACTOR" == "codex" ]]; then
    "$JESTERKY_BIN" run "$ROOT/examples/${spec}.json" \
      --actor codex \
      --model "$MODEL" \
      --cd "$CD_ARG" \
      --args "$ARGS_JSON" \
      --out "$manifest" \
      --no-follow
  else
    "$JESTERKY_BIN" run "$ROOT/examples/${spec}.json" \
      --actor fake \
      --args "$ARGS_JSON" \
      --out "$manifest"
  fi
  echo "manifest -> $manifest"
}

run_workflow gepa_trace_annotate gepa_trace_annotate
run_workflow gelo_trace_annotate gelo_trace_annotate

echo "done. inspect manifests under $OUT_DIR"
