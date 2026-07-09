---
type: feature_release
surface: blog
product_area: jesterky
claim_tier: measured
proof: /resources/proof/jesterky-workflows
release: jesterky@0.1.1
audience: builder
status: draft
owner: josh
title: "jesterky: dynamic workflows you can replay"
---

# jesterky: dynamic workflows you can replay

The strongest pattern to come out of agent harnesses this year is the dynamic
workflow: instead of prompting one model to do everything, you author a small
deterministic script — fan out, map, verify, reduce — and the script, not the
model, drives the run. Claude Code's workflow orchestration is the inspiration
here, and the pattern deserves to exist as an open substrate. jesterky is that
substrate, built around the one property the pattern needs to be trustworthy:
the run is a pinned artifact, and you can replay it.

A workflow run fans out across models, tools, and retries; the interesting
failure happens on shard 7 of 12; and by the time you look, the run is gone.
Logging more does not fix this — logs describe what happened, they are not a
thing you can re-execute. In jesterky the run *is* the artifact. A run emits a
pinned event stream and a manifest; feed the manifest back in and the runtime
re-drives the same orchestration against the recorded outputs,
deterministically.

jesterky is Rust, open source, and small. The core orchestrates — fan-out,
fan-in, sessions, concurrency limits, budgets and their dual, goals — and does
no IO. Everything that touches the world (a model call, a tool, a sandbox, a
clock) crosses a typed seam into the host.

## The contract is the product

The center of jesterky is a pinned contract, defined once as Rust types in the
`jesterky-contract` crate: the workflow topology, the event stream, the run
artifact, and the replay semantics. Those types are the single source of truth.
The crate emits a JSON Schema (`jesterky.schema.json`) from the same types, and
a drift guard fails the build if the schema and the types disagree. The Python
package is generated from that schema — client-only, no second implementation
to keep in sync.

```bash
cargo run -q -p jesterky-contract --example emit_schema workflow > jesterky.schema.json
cargo test -p jesterky-contract        # schema_drift: artifact matches the types
./python/gen.sh                        # Python types regenerate from the same schema
```

Event identity is a logical address — `(run_id, node_path, iteration, local_seq)` —
not an emit-time counter. The canonical order of a run is its events sorted by
that address, so two runs of the same workflow are comparable even when the
wall-clock interleaving differs. Wall time is recorded as metadata and is
explicitly not part of a run's identity. That decision is what makes replay
hold up under real parallelism, where emission order is nondeterministic.

## The core does orchestration; the host does IO

The core knows how to run a graph and nothing else. Actors, resources, the
event sink, the clock, the artifact and checkpoint stores are all host traits.
The default host actor echoes its inputs — deterministic, no network — which is
what the tests and the fake end-to-end path run on. Swap in the codex actor and
the same workflow drives a real model through `codex exec`, on ChatGPT-bundle
auth, with no API key in the process. The workflow spec does not change; only
the actor behind the seam does.

Two host-side pieces are new in 0.1.1 and carry most of this post's results:

**Sandboxes.** A workflow node can hand its model a seeded execution workspace:
`jesterky-sandbox` seeds a directory (local or Docker) with whatever the task
needs — source trees, oracles, verifiers — the model works inside it with real
tools, and capture globs pull the built artifacts back into the run manifest.
The environment stops being a prompt description and becomes something the
model actually runs.

**The proxy.** `jesterky-proxy` gives chat-only models the full agentic loop:
it translates Responses-style tool calls to chat tool calls and back, serves
the model catalog codex expects, and round-trips Gemini's thought signatures.
`jesterky run --actor codex --model gemini/gemini-3.1-pro-preview` spawns it
automatically. One agentic harness, any model behind it.

## Record and replay, proven live

The quality scan is the reference workload: expand a target into per-dimension
audit jobs, map an auditor over them concurrently, reduce the verdicts into a
report. It runs live in this release, not just under the fake actor. Pointed at
our production blog corpus, gpt-5.5 audited every published post:

```bash
jesterky run examples/quality_scan_blogs.json --actor codex \
  --args '{"blog_dir":".../content/blog"}' --out proof/quality_scan_blogs.live.manifest.json
# status=completed events=52 recorded=9
jesterky replay proof/quality_scan_blogs.live.manifest.json --spec examples/quality_scan_blogs.json
# replay ok: events=52 recorded=9
```

Eight posts audited, two judged SOUND, six FRAGILE with named blockers —
mostly measured claims missing visible proof metadata. The committed manifest
(`proof/quality_scan_blogs.live.manifest.json`) carries the real model
verdicts, and replay re-executes the orchestration against them and confirms it
lands identically on address, kind, and payload. This is not a diff of two log
files; it is the runtime re-executing its own decisions.

## The refactor bench: models run a real environment

The sharpest thing the sandbox enables is a benchmark where the model runs the
environment. The dev-port bench seeds a workspace with a working Python game
engine, its scenario set, a train-subset event-log oracle, and a verifier. The
model ports the engine to a Rust crate, builds it, diffs its output against the
train oracle, and iterates. The captured crate is then scored on *all*
scenarios against the full oracle — the model saw oracle logs for only the 4
train scenarios, so held-out scenarios measure faithful-port generalization,
not memorization.

Five engines form a difficulty ladder (all-scenario pass rate, one run per
cell; DeepSeek runs under a 10-minute wall-clock cap and is scored on its
partial crate):

| engine | gold LOC | scenarios | gpt-5.5 | gemini-3.1-pro-preview | deepseek-v4 (capped) |
|--------|---------:|----------:|--------:|-----------------------:|---------------------:|
| tictactoe | 1634 | 20 | 1.0 | 0.8 | 0.8 |
| sokoban | 1206 | 15 | 1.0 | 0.333 | 0.333 |
| minihack | 1036 | 27 | 0.148 | 0.148 | 0.0 |
| crafter | 3723 | 34 | 0.118 | 0.0 | 0.0 |
| earthborne (train=2) | 1916 | 4 | 1.0 | 0.0 | 1.0 |

The stable result across sessions is the cliff, and the cliff is mechanic
complexity, not lines of code: minihack has the fewest lines on the ladder and
defeats every model across its 27 nethack-derived scenarios, while earthborne,
the largest mid-tier engine, is fully ported because it is big but regular.
The second stable result is the overfit signature: on engines it cannot fully
port, gpt-5.5 passes exactly the 4 seeded train scenarios — minihack 4/27,
crafter 4/34 — a runnable crate that reproduces only what it can diff, with
zero held-out generalization. Gemini's failure mode is the under-port: crates
that build and then panic or mismatch at runtime. DeepSeek finishes small
regular engines faithfully and is killed mid-port on the hard ones. Mid-ladder
cells swing between runs (sokoban flipped for two models across two sessions);
the packet reports single-run cells as indicative and holds per-model ranking
claims for multi-seed runs.

Every cell cites a committed score artifact
(`gamebench/tasks/dev-port-singleplayer/score.sandbox.<model>.<engine>.json`),
and the bench itself is a jesterky workflow — the ports above are manifests you
can replay.

## The manifest is what an optimizer reads

A run's manifest is not just for replay. Its trace is a process tree: per node,
the typed inputs, the typed outputs, and an outcome score. That is exactly the
object an optimizer needs to walk a run and propose a change. The dependency
runs one way — optimizers consume jesterky, jesterky never depends on them.

The headline case is GEPA proposing a Craftax agent prompt. Two arms,
identical except for the system prompt under test: arm A is the base ReAct
prompt, arm B is the prompt GEPA proposed. Same container, same seeds, same
model, same output contract. The powered ablation at n=64 paired seeds: base
ReAct mean reward 2.328, GEPA prompt mean 3.006, uplift +0.678, 95% CI
[0.205, 1.151], exact two-sided sign-flip p=0.007, wins/ties/losses 40/4/20
(`proof/gepa_craftax_ablation.md`).

The with/without-jesterky claim is GELO. Same optimizer loop, one change: arm B
exports rollout traces, runs the `gelo_trace_annotate` workflow to cluster
failure themes, and materializes that context into the proposer before each
propose step. Arm A, without the workflow, improved Craftax reward over
baseline by +0.339; arm B improved it by +0.977
(`proof/gelo_jesterky_workflow_ablation.md`).

The same intervention inside GEPA did not clear its bar, and we report that as
measured: the hook is validated end-to-end — arm B materialized non-empty
themes before every propose, arm A stayed clean at zero workflow evidence —
but arm B's best held-out mean (1.0) did not beat arm A's (1.5) at either
budget. GEPA's workflow arm ships as wired, no measured uplift on this taskset
(`proof/gepa_jesterky_workflow_ablation.md`).

## Every product consumes the same seam

**Optimizers.** GELO and GEPA take a top-level `[jesterky_workflow]` config
block; when enabled, the annotate→materialize hook above runs before each
propose, and every run records its workflow evidence in the result manifest.

**Stack.** Agent workers reach these workflows over MCP: five verbs —
`stack_jesterky_{register,launch,inspect,replay,compare}` — let a worker
register a spec, launch it, inspect the manifest, replay it, and diff two runs
without holding the Rust runtime in its own context. All four launch workflows
are verified worker-invocable end-to-end. 0.1.1 exists partly so workers can
`cargo install jesterky-cli` and have the binary on PATH.

**Cloud.** SMR ReportBench runs are graded through the same trace-evaluate
workflow: real graded run artifacts are transformed into the v4 trace shape
(`scripts/build_reportbench_traces.py`), `smr_reportbench_trace_evaluate` maps
a rubric-grading actor over them, and a scorer joins the workflow's verdicts
against each run's autograde reward into a single report score. Run live over
the four graded ReportBench lanes, gpt-5.5's verdicts agreed with autograde on
all four — it failed the one lane at 12/17 checks and passed the three at
18/18 — for a report score of 0.926 and replay confirmed
(`proof/smr_reportbench_trace_evaluate.md`). The A/B — baseline report vs
trace-evaluate-guided report — is the next rung; the scorer that will grade it
is the one that just ran.

## Parity with the runtime it descends from

jesterky is a from-scratch Rust rebuild of an earlier Python runtime, mloky.
The two do not share an event vocabulary, so the parity gate does not compare
event bytes. It projects both runtimes onto the two properties any workflow
substrate must guarantee on a fan-out/fan-in run: conservation — every job that
starts completes, and the count survives the reduce with nothing silently
dropped — and termination — the run reaches a terminal `completed` state with
every job ok. The gate reads mloky's projection from a real recorded run and
jesterky's from a fresh deterministic scan, and asserts both are faithful.

## Where it falls short

The live path is only as reliable as the model behind it: a hosted model can
wrap its JSON in prose or stall mid-generation, and while jesterky extracts the
last well-formed object and classifies the failure, a flaky model still
produces a flaky run. The GEPA workflow arm is reported exactly as measured —
wired, no uplift on Craftax at two budgets. DeepSeek's dev-port scores measure
DeepSeek under a wall-clock cap, which is a statement about throughput as much
as ability. The terminal panel renders a finished run post-hoc; live redraw
during a run is not wired. The ReportBench evaluation ran live and agreed with
autograde, but the A/B uplift number does not exist yet, and we say so rather
than print one.

## Try it

```bash
cargo install jesterky-cli
jesterky run examples/quality_scan.json --actor fake --out run.json
jesterky visualize run.json --spec examples/quality_scan.json
jesterky replay run.json --spec examples/quality_scan.json
```

Every claim above maps to a committed artifact under the repo's `proof/`
directory — ablation tables, run manifests, score files, and an append-only
run registry for the optimizer arms. Nothing here is a screenshot of something
that only ran once.
