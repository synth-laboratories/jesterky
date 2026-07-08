# jesterky

The pinned Rust workflow substrate for the Synth stack — *ještěrky*, "lizards"
(they regrow tails: replay/resume). Fresh core that **supersedes** `workflow-rs`
and `rust_backend/graph` (mines them for ideas, doesn't fork them).

The core runtime is implemented for the M1 fake-actor surface: it runs the
closed node taxonomy, emits deterministic `Addr`-ordered events, records actor
calls, builds a process tree, and replays recorded runs byte-for-byte. Full
design + ADR: `../mloky/HANDOFF_jesterky_rust_rebuild.md`, roadmap:
`../mloky/ROADMAP_jesterky.md`.

## Crates
- **`jesterky-contract`** — the four pinned schemas as Rust types (source of
  truth; `schemars` emits `jesterky.schema.json`, from which the Python contract
  types are generated — no pyo3, see the roadmap's pyo3 decision).
  - `event.rs` — `Addr` (the logical clock, ADR #5) · `Event` · `EventKind`.
  - `topology.rs` — `WorkflowSpec` · `Node` · closed `NodeKind` (ADR #3).
  - `artifact.rs` — `Artifact`/`ArtifactRef` · `ProcessNode` trace · `RunManifest`.
- **`jesterky-core`** — pure orchestration (ADR #6), zero IO.
  - `traits.rs` — the seam: `Actor` · `Resource` · `EventSink` · `Clock` · `ArtifactStore`.
  - `runner.rs` — `Runner::run` + the `emit` logical-clock joint + `execute_map` (skeletal).
  - `ledger.rs` — declared-edge I/O (ADR #3).
- **`jesterky-actor`** — host-side SDK: `ReplayActor` (fully implemented — proves
  the record/replay joint), `FakeActor`, and in-memory sink/store/clock doubles.
- **`jesterky-model`** — the first *real* host actor (M2): `ModelActor<M>` adapts
  the `Actor` seam to a `Model` (one async completion), and `CodexModel` drives
  `codex exec` with ChatGPT-bundle auth (never an API key). `StubModel` makes the
  actor logic testable with no network.
- **`jesterky-quality`** — the reference M2 workload: a structured code quality
  scan (expand → map 8 audit dimensions → reduce report) with the actor roles.
- **`jesterky-cli`** — `jesterky run` / `replay` / `validate` / `schema`.

## Quickstart
```bash
# Fake actor — deterministic, no network. Run → manifest → replay (byte-identical).
cargo run -p jesterky-cli -- run examples/quality_scan.json --out /tmp/quality_scan.manifest.json
cargo run -p jesterky-cli -- replay /tmp/quality_scan.manifest.json --spec examples/quality_scan.json
cargo run -p jesterky-cli -- validate examples/quality_scan.json
cargo run -p jesterky-cli -- schema workflow
```

Real model scan via codex (`--actor codex`), parameterized by target:
```bash
# ChatGPT-bundle route (gpt-5.5):
cargo run -p jesterky-cli -- run examples/quality_scan.json \
  --actor codex --cd /path/to/repo --args '{"target":"/path/to/repo"}' \
  --out /tmp/scan.manifest.json

# DeepSeek-proxy route: point --codex-home at a dir holding the proxy config.toml.
cargo run -p jesterky-cli -- run examples/quality_scan.json \
  --actor codex --model deepseek/deepseek-v4-pro-direct \
  --codex-home /tmp/jesterky_codex_home --cd /path/to/repo \
  --args '{"target":"/path/to/repo"}' --out /tmp/scan.manifest.json
```
See `HANDOFF_jesterky_round6_live_scan.md` for the proxy `config.toml` setup and
the live-run checklist. A real run records the model outputs, so `replay`
re-drives it through `ReplayActor` with no model.

## What's locked vs skeletal
LOCKED: every type in `jesterky-contract`; the five seam traits; `Addr` + its
`Ord`; `Runner::emit` (per-node logical-clock allocation); `ReplayActor`; the
`RunManifest` shape. IMPLEMENTED: schema emission, topology validation/hash,
ledger resolution, all node execution bodies, serial/parallel map dispatch,
session limits, trace-tree rendering, CLI run/replay, and starter conformance
checks.

## Remaining gates
1. **M2 live run** — a real DeepSeek-proxy quality scan + replay (deferred to the
   runner; see `HANDOFF_jesterky_round6_live_scan.md`).
2. **mloky parity gate** — same topology + recorded outputs → matching event
   stream under the agreed `seq`→`Addr` mapping.
3. Seed the conformance suite from mloky's captured event streams.
4. Publish/package `jesterky-contract` (Rust + schema-generated Python) and the
   runtime surfaces. (No pyo3 — Python is contract types + a thin HTTP client.)

Read order: `traits.rs` → `runner.rs` → `ledger.rs`.
