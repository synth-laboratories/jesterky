# jesterky proof packet

Reproducible evidence backing the launch-blog claims. Every claim in the blog
maps to one command here. Run from repo root.

## 1 — Installs from a clean checkout (M1)

```bash
cargo install --path crates/jesterky-cli --root /tmp/jt && /tmp/jt/bin/jesterky --help
```

Produces a `jesterky` binary (~3 MB) with `run`, `replay`, `validate`,
`visualize`, `schema` subcommands.

## 2 — Deterministic fake E2E: run → replay (core + record/replay)

```bash
jesterky run examples/quality_min.json --actor fake --run-id demo-fake-001 --out proof/quality_min.manifest.json
# -> status=completed events=5 recorded=1

jesterky replay proof/quality_min.manifest.json --spec examples/quality_min.json
# -> replay ok: events=5 recorded=1
```

No network. The manifest in this directory (`quality_min.manifest.json`) is the
committed artifact; replay re-drives the orchestration against the recorded
actor outputs and matches on the fidelity fields (Addr / kind / payload; wall_ms
is metadata, excluded — ADR #5).

## 3 — Contract is the source of truth (M0)

```bash
cargo run -q -p jesterky-contract --example emit_schema workflow > jesterky.schema.json
cargo test -p jesterky-contract          # schema_drift guard: artifact matches emitter
./python/gen.sh                          # regenerates Python types from the same schema
```

## 4 — Publish-ready

```bash
cargo publish --dry-run -p jesterky-contract      # green
cd python && uv build                             # sdist + wheel
```

## 5 — mloky parity gate (M2 proof spine)

```bash
cargo test -p jesterky-quality --test conformance
# -> mloky_reference_run_is_faithful ... ok
# -> jesterky_scan_matches_mloky_contract ... ok
```

jesterky descends from the mloky reference runtime. They do NOT share an event
vocabulary — mloky emits domain lifecycle events, jesterky emits the pinned
`Addr`-keyed contract stream — so byte equality is the wrong assertion. The gate
instead projects both runtimes onto a canonical `RunOutcome` and asserts the two
properties any substrate must guarantee on a map→reduce run: **conservation**
(`jobs_started == jobs_completed == jobs_in_report`, no silent drops) and
**termination** (terminal `completed`, all jobs ok). The mloky projection is read
from a real recorded run (`crates/jesterky-quality/fixtures/mloky_scan_reference.jsonl`,
8 jobs); the jesterky projection from a fresh deterministic scan. Passing means
jesterky reproduces the reference contract.

## 6 — Live model E2E (M2, requires codex + proxy)

See `HANDOFF_jesterky_round6_live_scan.md`. Runs the real quality scan through
`codex exec` (DeepSeek proxy), then replays the live manifest.
