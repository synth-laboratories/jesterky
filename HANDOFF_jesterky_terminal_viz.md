# Handoff — jesterky terminal visualization (mloky parity)

**Date:** 2026-07-08. **Topic:** bring jesterky's terminal viz up to the mloky
quality-scan experience so a live `--actor codex` run shows the phase-grouped
progress tree users expect, not just a post-hoc indented skeleton.

**Depends on:** M1 substrate (committed). **Does not block:** M2 live scan /
DeepSeek proxy proof (`HANDOFF_jesterky_round6_live_scan.md`). Viz can land in
parallel once the live run path is green.

---

## North star (from mloky)

The reference is mloky's **terminal-first** line, not the parked mermaid/box
graph. See `../mloky/HANDOFF_swarm_visualization.md`.

Target shape for `quality_scan` (preserve this layout):

```text
quality swarm  8 agents · 2-wide · model deepseek/...  (1 varieties -> 8 shards)
  scan_quality

⠋ 01:13  2 live · 3/8 done   deepseek/...   12k tok (8k in·4k out)
▸ scan_jobs 3/8 done · 2 live · 12k tok
  ┆ 01  13 steps · latest command/action
  ┆ 02   6 steps · running
  ...
result completed · ok=true · 18k tok
```

Key properties:

- **Run header** — title, agent count, concurrency, model, symbolic fanout.
- **Phase row** — one row per map collection (`scan_jobs`), rollup of done/live/tokens.
- **Item rows** — one per map shard (`[0]`…`[7]`), status + steps/detail on the right.
- **ANSI** — colors, dim/bold, spinner glyph on the status line while running.
- **Live tail** — re-render in place until `WorkflowCompleted` / `WorkflowFailed`
  (mloky: `run_visual.py --follow`).
- **Post-hoc** — same renderer over a finished manifest or event list.

---

## What exists today (jesterky, committed)

### M1 skeleton — done

| Piece | Location | Notes |
|---|---|---|
| `ProcessNode` trace tree | `crates/jesterky-contract/src/artifact.rs` | Built in `Runner::build_trace` |
| Minimal renderer | `crates/jesterky-actor/src/viz.rs` | `render_tree(&ProcessNode) -> String` — label + score + artifact count, plain text |
| CLI hook | `crates/jesterky-cli/src/main.rs` `print_manifest` | Prints trace after `run` / `replay` |
| Snapshot test | `crates/jesterky-actor/src/viz.rs` | Deterministic `assert_eq!` against `const EXPECTED` |

Example output today (fake actor):

```text
workflow:quality_scan artifacts=0
  scan_jobs artifacts=0
    [0] artifacts=0
      actor:quality_scanner artifacts=0
    ...
status=completed events=52 recorded=9
```

### Event stream — partial fit for live viz

jesterky already emits structural map events (`crates/jesterky-contract/src/event.rs`):

- `WorkflowStarted`, `WorkflowCompleted`, `WorkflowFailed`
- `NodeStarted`, `NodeCompleted`
- `MapItemStarted`, `MapItemCompleted`, `MapItemFailed`, `MapCompleted`
- `ActorInvoked`, `ArtifactEmitted`, …

`MapItemStarted/Completed` payloads today carry only `{ "index": i }` — no
tokens, steps, or detail string.

### Gaps vs mloky

| mloky | jesterky today |
|---|---|
| `RunView` / `PhaseView` / `ItemView` | Flat `ProcessNode` indent |
| `adapt_quality_events` over JSONL | No event → view adapter |
| `run_visual.py --follow` | Post-hoc print only |
| `agent_progress` / `agent_usage` events | No per-actor progress events |
| ANSI layout + spinner | Plain stdout |
| Declarative hints in workflow JSON | No `rendering` block in spec |

**Do not** attempt the mloky parity gate (flat `seq` JSONL → `Addr`) in this
handoff. That is a separate M2 judgment task. This work consumes jesterky's
native `Event` + `RunManifest` shapes only.

---

## Architecture decisions (locked)

1. **Renderer stays host-side and IO-free.** Pure `view → String` (or `Vec<String>`
   for line-based redraw). Core/runner must not depend on ANSI or stdout.
2. **Two input modes, one view model:**
   - **Live / post-hoc from events** — fold `&[Event]` (+ optional spec hints)
     into a `RunView`, render.
   - **Static from trace** — optional fast path: `ProcessNode` → `RunView` for
     completed manifests (can reuse the same renderer).
3. **Crate placement:** extend `jesterky-actor::viz` (already exported) OR add
   `jesterky-viz` if the module grows past ~400 lines. Default: keep in
   `jesterky-actor` until it hurts.
4. **No contract changes in slice 1.** Do not add `EventKind` variants or reshape
   `event.rs` / `NodeKind` for viz. Use existing events + spec metadata.
5. **ANSI off switch.** `--no-color` and auto-disable when `NO_COLOR` or stdout
   is not a TTY (mirror mloky `set_color`).

---

## Implementation slices

### Slice 1 — view model + quality-scan adapter (post-hoc)

**Goal:** `adapt_quality_events(&[Event], &WorkflowSpec) -> RunView` + rich
`render_run_view(&RunView) -> String` that matches the mloky layout for a
*completed* quality scan.

**Files to add/change:**

- `crates/jesterky-actor/src/viz.rs` — split into:
  - `view.rs` — `RunView`, `PhaseView`, `ItemView` (mirror mloky fields)
  - `adapt.rs` — `adapt_events`, `adapt_quality_scan`
  - `render.rs` — ANSI `render_run_view`, `render_run_view_lines`
  - keep `render_tree(ProcessNode)` as thin wrapper or deprecate in favor of
    `ProcessNode → RunView → render`
- `crates/jesterky-actor/Cargo.toml` — only add deps if needed (`owo-colors` or
  hand-rolled ANSI constants like mloky; prefer hand-rolled to match mloky exactly)

**Adapter logic for `examples/quality_scan.json`:**

1. On `WorkflowStarted`, read `payload` args if present; seed run title from
   `spec.name` (`quality_scan`).
2. Find map node `scan_jobs` — phase name = node id; item count from
   `MapItemStarted` indices or from `quality.expand` output in recorded/events
   (8 jobs).
3. For each `MapItemStarted` at path `…/scan_jobs/[i]` → item `i` status
   `running`.
4. `MapItemCompleted` → `done`; `MapItemFailed` → `failed`.
5. Roll up `done` / `live` / `failed` on the phase row and run header.
6. On `WorkflowCompleted` / `WorkflowFailed`, set run status + elapsed from
   first/last `wall_ms` delta (display only — not identity).
7. **Model / concurrency** — read from CLI-injected metadata if available (see
   slice 2); until then, show `?` or parse from `WorkflowStarted` payload if you
   add it host-side.

**Hints (optional, slice 1b):** allow `examples/quality_scan.json` to carry a
top-level `"rendering": { "title": "quality swarm", "fanout": "(1 varieties -> 8 shards)" }`
block — host-only, not in `jesterky.schema.json` until an ADR says otherwise.
Mirror mloky's `workflow_render_hints()`.

**Tests:**

- Unit: hand-build 8-item event list from a fake run's `manifest.events`,
  `assert_eq!(render, EXPECTED)` (committed const, no `insta`).
- Integration: extend `crates/jesterky-cli/tests/cli.rs` — after
  `run_then_replay_quality_scan`, assert rendered output contains
  `8 agents` and `scan_jobs` (substring checks are fine for slice 1).

**Acceptance:** `cargo test` green; post-hoc render of a completed fake
`quality_scan` manifest matches the mloky tree shape (header + phase + 8 items).

---

### Slice 2 — live follow during `jesterky run`

**Goal:** `jesterky run … --follow` (or `jesterky visualize --follow <manifest>`)
re-renders the tree in place until the run finishes.

**Approach A (recommended):** in-process subscriber on `MemEventSink`:

1. Add `SharedEventSink` — `Arc<Mutex<Vec<Event>>>` implementing `EventSink`.
2. `Runner` already calls `sink.emit(event)` — swap sink when `--follow`.
3. Spawn a tokio task (or synchronous thread) that every 500ms:
   - clones current events
   - `adapt_events` → `render_run_view_lines`
   - ANSI redraw: save previous line count, `\033[{n}A`, `\033[2K` per line
     (copy mloky `_redraw` from `scripts/run_visual.py` lines 57–64).
4. On `WorkflowCompleted` / `WorkflowFailed`, final frame + restore cursor
   (`\033[?25h`).
5. After run, print status footer (keep today's `status=… events=…` line below
   the tree).

**CLI flags:**

```text
jesterky run <spec> [--follow] [--viz-interval 0.5] [--no-color] …
jesterky visualize <manifest.json> [--spec <spec.json>] [--follow] …
```

`visualize` reads `manifest.events` (and optional spec for hints). `--follow`
on an incomplete manifest tails a file if you later add `--events-out` (slice 3);
for slice 2, in-process only is enough.

**Inject run metadata for the header:** when `--actor codex`, pass model +
concurrency into `WorkflowStarted` payload host-side (CLI only, not a contract
change to the enum — just richer `payload` JSON):

```json
{ "model": "deepseek/deepseek-v4-pro-direct", "concurrency": 2, "title": "quality swarm" }
```

**Tests:**

- Fake-actor run with `--follow` in a test terminal (or test the redraw helper
  and adapter without TTY).
- Assert follow loop exits on `WorkflowCompleted`.

**Acceptance:** `jesterky run examples/quality_scan.json --follow` shows a live
updating tree during the fake run; final frame shows `8/8 done`.

---

### Slice 3 — per-shard progress (real codex runs)

**Goal:** item rows show steps/tokens/detail like mloky during live DeepSeek scans.

**Problem:** `CodexModel` is one blocking `codex exec` per actor call — no
streaming progress today.

**Options (pick one; do not do all):**

| Option | Effort | Notes |
|---|---|---|
| **A. Parse codex stderr/stdout incrementally** | Medium | Wrap `codex exec` in a line reader; emit host-side progress callbacks into a side channel the viz task reads. No `EventKind` change if progress stays host-local until complete. |
| **B. Add `ActorProgress` events** | Higher | New `EventKind` — needs ADR + schema bump. Runner would need a progress callback on `Actor` trait. |
| **C. Poll child PIDs** | Low | Count live `codex` children per map slot (mloky `watch_swarm.py` style). Shows "running" but not steps/tokens. |

**Recommendation:** **C for slice 3a** (live slot counts during parallel map),
**A for slice 3b** (steps/detail when codex streams are available). Ship 3a with
slice 2 if time is tight — header/phase rollups with spinners already help.

**Acceptance:** live `--actor codex` quality scan shows `N live · M/8 done` updating
in real time; item rows show at least `running` / `done` / `failed`.

---

### Slice 4 — generic adapters (later)

Port mloky's dispatch pattern:

```python
# mloky terminal_tree.adapt_events
if "run_started" in kinds and "agent_started" in kinds: …
```

jesterky equivalent:

```rust
pub fn adapt_events(events: &[Event], spec: &WorkflowSpec) -> RunView {
    if is_quality_scan(spec) { adapt_quality_scan(...) }
    else { adapt_generic_workflow(...) }
}
```

`adapt_generic_workflow` — one phase per top-level `map` node, items from
`MapItem*` events keyed by `addr.node_path`. Good enough for DungeonGrid later.

**Not in scope for first PR:** `replay_swarm.py` lane animation, web artifact,
mermaid `render_workflow.py`.

---

## Reference map (read first)

| What | Where |
|---|---|
| mloky terminal renderer | `../mloky/src/mloky/rendering/terminal_tree.py` |
| mloky follow CLI | `../mloky/scripts/run_visual.py` |
| mloky viz handoff | `../mloky/HANDOFF_swarm_visualization.md` |
| mloky quality scan topology | `../mloky/evals/quality_scan/quality_scan.workflow.json` |
| jesterky quality scan spec | `examples/quality_scan.json` |
| jesterky events | `crates/jesterky-contract/src/event.rs` |
| jesterky trace builder | `crates/jesterky-core/src/runner.rs` `build_trace` |
| jesterky current renderer | `crates/jesterky-actor/src/viz.rs` |
| jesterky CLI | `crates/jesterky-cli/src/main.rs` |
| Round 3 renderer spec | `../mloky/HANDOFF_jesterky_round3_codex.md` § Cycle 3c |
| Round 4 shipped state | `HANDOFF_jesterky_round4_codex.md` § Actor SDK / Trace Viz |

---

## Hard constraints (do not break)

- **No `event.rs` / `NodeKind` / `traits.rs` shape changes** in slice 1–2.
- **Core stays IO-free** — no `println!` in `jesterky-core`.
- **Addr ordering unchanged** — viz reads events sorted by `addr` if needed;
  never sort by `wall_ms` for identity.
- **Deterministic post-hoc output** — with `--no-color` and a fixed completed
  event list, render is byte-stable (for snapshot tests).
- **Do not block M2 live scan** on viz — viz PRs can merge independently.

---

## Definition of done (this handoff)

Minimum ship (slices 1 + 2):

- [ ] `RunView` / `PhaseView` / `ItemView` in Rust
- [ ] `adapt_quality_scan` from `&[Event]` + `WorkflowSpec`
- [ ] ANSI `render_run_view` matching mloky header/phase/item layout
- [ ] `jesterky run --follow` live redraw on fake `quality_scan`
- [ ] `jesterky visualize <manifest>` post-hoc command
- [ ] `--no-color` + TTY detection
- [ ] Unit snapshot test + CLI integration assertion
- [ ] `cargo test` / `cargo build` green, zero new warnings
- [ ] README one-liner under CLI section

Stretch (slice 3a):

- [ ] Live `N live · M/8 done` during real `--actor codex` parallel map

---

## Suggested PR order

1. **PR1:** view model + adapter + post-hoc render + `jesterky visualize` + tests
2. **PR2:** `--follow` in-process + `WorkflowStarted` metadata injection
3. **PR3:** live progress (option C, then A if needed)

---

## Quick validation commands

```bash
# Post-hoc (after PR1)
cargo run -p jesterky-cli -- run examples/quality_scan.json \
  --out /tmp/qs.manifest.json
cargo run -p jesterky-cli -- visualize /tmp/qs.manifest.json \
  --spec examples/quality_scan.json

# Live fake (after PR2)
cargo run -p jesterky-cli -- run examples/quality_scan.json --follow

# Live real (after M2 proxy + PR2/3)
cargo run -p jesterky-cli -- run examples/quality_scan.json \
  --actor codex --model deepseek/deepseek-v4-pro-direct \
  --codex-home /tmp/jesterky_codex_home --cd /path/to/repo \
  --args '{"target":"/path/to/repo"}' --follow \
  --out /tmp/quality_scan.live.manifest.json
```

---

## Open questions (eng judgment — not blockers)

1. **`render_tree(ProcessNode)`** — keep as legacy plain indent or reimplement via
   `ProcessNode → RunView`? Prefer unify on `RunView` so CLI has one code path.
2. **`rendering` block in workflow JSON** — host-only extension now, or wait for
   schema/ADR? Host-only is fine for slice 1b.
3. **Event export to JSONL** — `--events-out` for external `visualize --follow`
   is nice for Stack later; defer unless trivial alongside `SharedEventSink`.

Log decisions with `jsk decision` if impact ≥ med.
