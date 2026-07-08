# Handoff — jesterky M2 live scan (run + debug)

Date: 2026-07-08. The **core is ready**; what remains is a real run against the
DeepSeek proxy plus debugging, then the parity gate. All code below is committed
and the whole workspace is green (`cargo test`, zero warnings; the two live tests
are `#[ignore]`d). Do NOT need to touch the core to run this.

## What's ready (this session, committed)

- **`jesterky-quality`** — the scan workload: `quality.expand` (8 audit
  dimensions × target), `quality.aggregate` (verdicts → report), actor roles
  (system prompts). Deterministic stub test + program units green.
- **`CodexModel`** route flexibility — `with_codex_home(dir)` sets `CODEX_HOME`
  on the subprocess; the reasoning-effort flag is omitted when effort is empty
  (proxy routes reject it); never sets an OpenAI API key; inherits parent env
  (so `SYNTH_API_KEY` passes through).
- **CLI** — `jesterky run --actor codex` with `--model` / `--codex-home` / `--cd`.
  gpt\* models get `high` effort, other routes get none. Uses the real quality
  programs and applies the scan roles.
- **Target binding** — run args seed the ledger, so `--args '{"target":"…"}'`
  reaches `quality.expand` (via `ledger.target`). Previously ignored.
- **Replay fidelity fix** — the checker no longer compares `wall_ms` (ADR #5
  metadata). A parallel real-actor run's emission order is nondeterministic, so
  recorded timestamps can't be re-attached on replay; fidelity is addr+kind+
  payload. This is why the earlier live scan's replay failed on `wall_ms` alone.

## What remains for you (run + debug)

### 1. Stand up the DeepSeek proxy config
Point `--codex-home` at a dir holding an mloky-shaped codex `config.toml` that
routes to the local proxy. Sketch (fill in from mloky's working config):

```toml
# /tmp/jesterky_codex_home/config.toml
model_provider = "synth-proxy"
[model_providers.synth-proxy]
name = "synth-proxy"
base_url = "http://127.0.0.1:8001/v1"    # the Responses↔chat proxy
# auth via SYNTH_API_KEY in env (inherited by the subprocess)
```

- Preflight: `curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8001/health`
- Ensure `SYNTH_API_KEY` is set in the shell that runs the CLI (dev key ok).
- `CodexModel` already sets `CODEX_HOME` and inherits env — no code change
  needed; you supply the config.toml + env.

Optional convenience (not built): a `--proxy` preset that materializes this
config.toml under a temp `--codex-home` and sets the route. The primitives
(`--model`, `--codex-home`, `--cd`) are enough without it.

### 2. Live run + replay
```bash
cargo run -p jesterky-cli -- run examples/quality_scan.json \
  --actor codex --model deepseek/deepseek-v4-pro-direct \
  --codex-home /tmp/jesterky_codex_home --cd /path/to/repo \
  --args '{"target":"/path/to/repo"}' \
  --out /tmp/quality_scan.live.manifest.json

cargo run -p jesterky-cli -- replay /tmp/quality_scan.live.manifest.json \
  --spec examples/quality_scan.json
```
8 scanner calls + 1 summary. Replay should now pass (wall_ms fix). Confirm the
manifest's recorded verdicts look sane and `status=completed`.

### Likely debug points
- **`min_success: 1.0`** in `quality_scan.json` — all 8 dimensions must return
  parseable JSON or the map fails. If DeepSeek flakes or over-explores, relax to
  e.g. `0.75` while debugging.
- **JSON extraction** — `ModelActor` takes the last `{…}` span in the model
  reply (`extract_json`). If DeepSeek doesn't terminate cleanly on long audits
  (the Flash problem mloky hit) or wraps output oddly, that's where it breaks.
  The scanner role prompt already says "exactly one JSON object, no prose."
- **`summary_recorder`** also makes a model call (summarizes the report). Cheap,
  but it is a real call — factor it into cost/latency.
- **codex session preamble** — `codex exec` prints a session header to stdout;
  `extract_json` skips it by taking the last brace span. Verify with a 1-shot.

## Optional / not blocking
- `--output-schema` + force-emit phase (mloky) — guards non-termination; add if
  DeepSeek won't stop.
- Blog rubric prompts vs the 8 generic dimension prompts — different workload
  shape; fine for the substrate proof, tighten later.
- **mloky parity gate** (M2 DoD) — separate effort: map mloky's flat `seq` event
  log onto jesterky's `Addr` clock and assert equality. Judgment work, not wired
  yet.
