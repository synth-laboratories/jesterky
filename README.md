# jesterky

The pinned Rust workflow substrate for the Synth stack — *ještěrky*, "lizards"
(they regrow tails: replay/resume). Fresh core that **supersedes** `workflow-rs`
and `rust_backend/graph` (mines them for ideas, doesn't fork them).

The core runtime runs the closed node taxonomy, emits deterministic
`Addr`-ordered events, records actor/resource calls, builds a process tree, and
replays recorded runs against the same contract.

## Crates
- **`jesterky-contract`** — the four pinned schemas as Rust types (source of
  truth; `schemars` emits `jesterky.schema.json`, from which the Python contract
  types are generated — no pyo3, see the roadmap's pyo3 decision).
  - `event.rs` — `Addr` (the logical clock, ADR #5) · `Event` · `EventKind`.
  - `topology.rs` — `WorkflowSpec` · `Node` · closed `NodeKind` (ADR #3).
  - `artifact.rs` — `Artifact`/`ArtifactRef` · `ProcessNode` trace · `RunManifest`.
  - `budget.rs` — formal resource budgets (`BudgetPlan` / `BudgetEngine` /
    progress + ETA). See **`docs/BUDGETS.md`**.
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
For proxied routes, point `--codex-home` at a directory containing the desired
Codex `config.toml` and auth material. A real run records the model outputs, so
`replay` re-drives it through `ReplayActor` with no model.

## Resource budgets (progress + ETA)

Concurrency (`runplan.limits`) is **not** the same as resource budgets
(`runplan.budgets`). Budgets cap **actor calls / tokens / wall seconds**, meter
progress, forecast **time-to-exhaust** (not time-to-finish), and render on the
live panel.

Full JSON schema of knobs, typed API, and semantics: **`docs/BUDGETS.md`**.

```json
"runplan": {
  "budgets": {
    "warning_percent": 80,
    "fail_on_hard_exhaust": true,
    "eta": { "enabled": true, "mode": "all", "min_wall_secs": 1.0 },
    "viz": { "show_progress": true, "show_eta": true, "show_nearest_tag": true },
    "caps": [
      { "kind": "actor_calls", "max": 8, "hard": true, "label": "turns" },
      { "kind": "tokens", "max": 100000, "hard": false },
      { "kind": "wall_seconds", "max": 120, "hard": false }
    ]
  }
}
```

Typed entry points: `BudgetPlan`, `BudgetCap`, `BudgetEtaConfig`,
`BudgetVizConfig`, `BudgetEngine::snapshot`, `RunManifest.budgets`
(`BudgetSnapshot`). Partial per-run override via
`BudgetPlan::overlay_json` / `--args '{"budgets":{...}}'`. Episode-scale ETA
template: `examples/budgets_episode_scale.json`.

## What's Locked
LOCKED: every type in `jesterky-contract`; the five seam traits; `Addr` + its
`Ord`; `Runner::emit` (per-node logical-clock allocation); `ReplayActor`; the
`RunManifest` shape. IMPLEMENTED: schema emission, topology validation/hash,
ledger resolution, all node execution bodies, serial/parallel map dispatch,
session limits, trace-tree rendering, CLI run/replay, and starter conformance
checks.

## Remaining gates
1. Keep `proof/README.md` mapped from every public claim to a committed
   artifact and replay/run command.
2. Keep the contract schema and generated Python types aligned with the Rust
   contract version.
3. Publish/package `jesterky-contract` and the runtime surfaces together. Python
   remains contract types plus thin integration code; no second runtime.

Read order: `traits.rs` → `runner.rs` → `ledger.rs`.
