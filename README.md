# jesterky

The pinned Rust workflow substrate for the Synth stack — *ještěrky*, "lizards"
(they regrow tails: replay/resume). Fresh core that **supersedes** `workflow-rs`
and `rust_backend/graph` (mines them for ideas, doesn't fork them).

**This is an interface skeleton, not a runtime yet.** The joints and seams are
locked; the bodies are documented `todo!()` for the implementing engineer. Full
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

## What's locked vs skeletal
LOCKED: every type in `jesterky-contract`; the five seam traits; `Addr` + its
`Ord`; `Runner::emit` (per-node logical-clock allocation); `ReplayActor`; the
`RunManifest` shape. SKELETAL (`todo!()` with algorithm docstrings): validation/
hash, ref resolution, the per-kind execution bodies, and parallel map dispatch.

## First tasks for the implementing engineer (M0 → M1)
1. `schemars` derive on the contract → emit `jesterky.schema.json`; stand up the
   conformance suite (seed = mloky's captured event streams).
2. `WorkflowSpec::validate_and_hash` (cycle-DFS + canonicalize→SHA-256).
3. `Ledger` ref resolution + binding store (`ledger.*` / `item.*` / literal).
4. `Runner::execute_node` bodies + `execute_map` serial/parallel (keep `emit`'s
   `Addr` scheme — never a global emit counter).
5. Fold `recorded` + structure into the `ProcessNode` trace.

Read order: `traits.rs` → `runner.rs` → `ledger.rs`.
