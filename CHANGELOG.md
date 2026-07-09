# Changelog

All notable changes to jesterky are recorded here. Versions follow the three
independent trains (contract / runtime / CLI); this release pins them together at
`0.1.0`.

## 0.1.1 — 2026-07-09

The launch release: agentic actors get real environments, and chat-only models
get the full agentic loop. Ships with the launch blog and the proof packet.

### Added
- **`jesterky-sandbox` 0.1.1** (new crate) — seeded execution workspaces for
  agentic actors: a `Sandbox` trait with Local and Docker providers,
  `SandboxConfig` (backend / mode / seed / capture) on the contract,
  `HostConfig.sandboxes`, and per-call wiring in `ModelActor`. Capture globs
  pull built artifacts back into the run manifest.
- **`jesterky-proxy` 0.1.1** (new crate) — the agentic loop for chat-only
  routes: Responses⇄chat tool-call translation, a `/v1/models` catalog for
  codex, and Gemini `thought_signature` round-trip. `jesterky run --actor codex
  --model <provider>/<model>` spawns it automatically.
- **Goals engine v1** in `jesterky-core` — goals as the dual of budgets, with
  contract types (`goal.rs`, `budget.rs`) and enforcement in the run loop.
- **Quality workloads** in `jesterky-quality` — blog/docs corpus scans, trace
  annotate/evaluate (GEPA, GELO, SMR ReportBench), obliq math verify/retrieve,
  dungeongrid multiplayer; example specs and output schemas for each.
- **Terminal viz** — `jesterky visualize` renders a finished run's process tree.

### Changed
- CLI: `--model`, `--codex-home`, `--cd`, `--args-file`, `--events-out`; replay
  fidelity is addr+kind+payload (wall time is metadata, never identity).

### Breaking changes
- None. The new crates and CLI flags are additive for existing 0.1.x users.

## 0.1.0 — 2026-07-08

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
uv add jesterky                 # Python contract types
```

### Out of scope (post-launch)
Hosted service, Stack cockpit integration, the workflow optimizer, and the
GEPA/GELO loop. This release is the substrate.

### Breaking changes
- Initial public release.
