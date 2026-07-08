---
type: feature_release
surface: blog
product_area: jesterky
claim_tier: measured
proof: /resources/proof/jesterky-quality-scan
release: jesterky@0.1.0
audience: builder
status: draft
owner: josh
title: "jesterky: a workflow substrate you can replay"
---

# jesterky: a workflow substrate you can replay

Agent workflows are hard to trust because they are hard to reproduce. A run
fans out across models, tools, and retries; the interesting failure happens on
shard 7 of 12; and by the time you look, the run is gone. You can log more, but
logs are a description of what happened, not a thing you can re-execute. jesterky
is a workflow substrate built the other way around: the run *is* the artifact,
and you can replay it.

jesterky is Rust, open source, and small. The core orchestrates — fan-out,
fan-in, sessions, concurrency limits — and does no IO. Everything that touches
the world (a model call, a tool, a clock) crosses a typed seam into the host. A
run emits a pinned event stream and a manifest; feed the manifest back in and it
re-drives the same orchestration against the recorded outputs, deterministically.

## The contract is the product

The center of jesterky is a pinned contract, defined once as Rust types in the
`jesterky-contract` crate: the workflow topology, the event stream, the run
artifact, and the replay semantics. Those types are the single source of truth.
The crate emits a JSON Schema (`jesterky.schema.json`) from the same types, and a
drift guard fails the build if the schema and the types disagree. The Python
package is generated from that schema — client-only, no second implementation to
keep in sync.

```bash
cargo run -q -p jesterky-contract --example emit_schema workflow > jesterky.schema.json
cargo test -p jesterky-contract        # schema_drift: artifact matches the types
./python/gen.sh                        # Python types regenerate from the same schema
```

Event identity is a logical address — `(run_id, node_path, iteration, local_seq)` —
not an emit-time counter. The canonical order of a run is its events sorted by
that address, so two runs of the same workflow are comparable even when the
wall-clock interleaving differs. Wall time is recorded as metadata and is
explicitly not part of a run's identity. That decision is what makes replay hold
up under real parallelism, where emission order is nondeterministic.

## The core does orchestration; the host does IO

The core knows how to run a graph and nothing else. Actors, resources, the event
sink, the clock, the artifact and checkpoint stores are all host traits. The
default host actor echoes its inputs — deterministic, no network — which is what
the tests and the fake end-to-end path run on. Swap in `CodexModel` and the same
workflow drives a real model through `codex exec`, on ChatGPT-bundle auth, with
no API key in the process. The workflow spec does not change; only the actor
behind the seam does.

## Record and replay, proven

The quality scan is the reference workload: expand a target into per-dimension
audit jobs, map an auditor over them concurrently, reduce the verdicts into a
report. Run it with the fake actor and it is fully deterministic — a manifest you
can replay:

```bash
jesterky run examples/quality_min.json --actor fake --run-id demo --out run.json
# status=completed events=5 recorded=1
jesterky replay run.json --spec examples/quality_min.json
# replay ok: events=5 recorded=1
```

Replay re-runs the pure orchestration and matches against the recorded actor
outputs on the fidelity fields — address, kind, payload. It ignores wall time by
construction. This is not a diff of two log files; it is the runtime re-executing
its own decisions and confirming they land the same way.

## Parity with the runtime it descends from

jesterky is a from-scratch Rust rebuild of an earlier Python runtime, mloky. The
two do not share an event vocabulary and were never meant to — mloky emits domain
lifecycle events, jesterky emits the pinned `Addr`-keyed contract stream. So the
parity gate does not compare event bytes. It projects both runtimes onto the two
properties any workflow substrate must guarantee on a fan-out/fan-in run:
conservation — every job that starts completes, and the count survives the reduce
with nothing silently dropped — and termination — the run reaches a terminal
`completed` state with every job ok. The gate reads mloky's projection from a
real recorded run and jesterky's from a fresh deterministic scan, and asserts
both are faithful.

```bash
cargo test -p jesterky-quality --test conformance
# mloky_reference_run_is_faithful ... ok
# jesterky_scan_matches_mloky_contract ... ok
```

## Where it falls short

The live path is only as reliable as the model behind it. A real scan through a
hosted model can return prose around its JSON or stall mid-generation; jesterky
extracts the last well-formed object and classifies the failure, but a flaky
model still produces a flaky run, and that is a property of the world, not
something the substrate hides. The terminal panel renders a finished run
post-hoc; live redraw during a run is not wired yet. And the hosted service,
the Stack cockpit, and the optimizer loop that consume these manifests are
deliberately out of scope for this release — this is the substrate, not the
platform on top of it.

## Try it

```bash
cargo install --path crates/jesterky-cli
jesterky run examples/quality_scan.json --actor fake --out run.json
jesterky visualize run.json --spec examples/quality_scan.json
jesterky replay run.json --spec examples/quality_scan.json
```

Every claim above maps to a command in the repo's `proof/` directory. Nothing
here is a screenshot of something that only ran once.

## What it is for

A replayable run is the unit the rest of the stack is built to consume. Stack
runs jesterky workflows as its cockpit substrate. Hosted optimizers treat a
manifest as a training example — GEPA and reward-model loops need runs they can
re-execute and score, not logs they can only read. Managed research and agent
infrastructure need the same guarantee for a different reason: an audit trail
that is also an executable. jesterky ships the guarantee first. The things built
on it come next.
