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
  truth; M0 adds `schemars` → `jesterky.schema.json` + pyo3 Python types).
  - `event.rs` — `Addr` (the logical clock, ADR #5) · `Event` · `EventKind`.
  - `topology.rs` — `WorkflowSpec` · `Node` · closed `NodeKind` (ADR #3).
  - `artifact.rs` — `Artifact`/`ArtifactRef` · `ProcessNode` trace · `RunManifest`.
- **`jesterky-core`** — pure orchestration (ADR #6), zero IO.
  - `traits.rs` — the seam: `Actor` · `Resource` · `EventSink` · `Clock` · `ArtifactStore`.
  - `runner.rs` — `Runner::run` + the `emit` logical-clock joint + `execute_map` (skeletal).
  - `ledger.rs` — declared-edge I/O (ADR #3).
- **`jesterky-actor`** — host-side SDK: `ReplayActor` (fully implemented — proves
  the record/replay joint), `FakeActor`, and in-memory sink/store/clock doubles.
- **`jesterky-cli`** — `jesterky run`, `jesterky replay`, and schema emission for
  the M1 fake-actor ship surface.

## Quickstart
```bash
cargo run -p jesterky-cli -- run examples/quality_scan.json --out /tmp/quality_scan.manifest.json
cargo run -p jesterky-cli -- replay /tmp/quality_scan.manifest.json --spec examples/quality_scan.json
cargo run -p jesterky-cli -- schema workflow
```

`examples/quality_scan.json` uses host-side demo programs in the CLI plus
`FakeActor`; real model/process actors stay outside the core behind the `Actor`
trait.

## What's locked vs skeletal
LOCKED: every type in `jesterky-contract`; the five seam traits; `Addr` + its
`Ord`; `Runner::emit` (per-node logical-clock allocation); `ReplayActor`; the
`RunManifest` shape. IMPLEMENTED: schema emission, topology validation/hash,
ledger resolution, all node execution bodies, serial/parallel map dispatch,
session limits, trace-tree rendering, CLI run/replay, and starter conformance
checks.

## Remaining M0/M1 gates
1. Seed the conformance suite from mloky's captured event streams and parity
   fixtures.
2. Publish/package `jesterky-contract` and the runtime surfaces.
3. Add pyo3 Python bindings over the Rust core.
4. Run the mloky parity gate: same topology + recorded outputs → matching
   event stream under the agreed mapping.

Read order: `traits.rs` → `runner.rs` → `ledger.rs`.
