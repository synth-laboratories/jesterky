# Changelog

All notable changes to jesterky are recorded here. Versions follow the three
independent trains (contract / runtime / CLI); this release pins them together at
`0.1.0`.

## 0.1.0 — unreleased

First public release: the jesterky workflow substrate — a Rust core that
orchestrates fan-out/fan-in workflows with no IO, a pinned contract that is the
single source of truth, and deterministic record/replay.

### Added
- **`jesterky-contract` 0.1.0** — pinned contract types (topology, event stream,
  run artifact, replay semantics). Emits `jesterky.schema.json` from the Rust
  types with a build-time drift guard. Event identity is a logical `Addr`
  (`run_id`, `node_path`, `iteration`, `local_seq`); wall time is metadata only.
- **`jesterky-core` 0.1.0** — the orchestration core: program/actor/map/reduce/
  session nodes, concurrency limits, the process trace, and replay. Zero IO; all
  side effects cross host traits.
- **`jesterky-actor` 0.1.0** — host-side SDK: in-memory sink/stores, the replay
  driver, and test doubles.
- **`jesterky-model` 0.1.0** — `ModelActor` seam adapter and `CodexModel`
  (`codex exec`, ChatGPT-bundle auth, no API key; per-actor `--output-schema`,
  route/model override for proxied backends).
- **`jesterky-quality` 0.1.0** — the reference quality-scan workload (expand →
  map audit dimensions → reduce report) and the mloky parity gate.
- **`jesterky-cli` 0.1.0** — `jesterky run | replay | validate | visualize |
  schema`. Btop-style terminal panel for finished runs.
- **`jesterky` (PyPI)** — client-only Python contract types, generated from the
  pinned schema.

### Proven
- Deterministic fake end-to-end: `run` → `replay ok` (`proof/`).
- mloky parity gate: conservation + termination hold for both runtimes.
- `cargo install --path crates/jesterky-cli` yields a working binary from a clean
  checkout.

### Install
```bash
cargo install jesterky-cli      # runtime + CLI
pip install jesterky            # Python contract types
```

### Out of scope (post-launch)
Hosted service, Stack cockpit integration, the workflow optimizer, and the
GEPA/GELO loop. This release is the substrate.
