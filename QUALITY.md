# Quality

jesterky is a public workflow substrate. It stays small, contract-pinned,
replayable, and explicit about what each workflow proves.

## Contract and replay

- Rust contract types are the source of truth. Emitted JSON schemas and
  generated Python models must match them before a release lands.
- Replay identity is the logical address, event kind, and payload. Wall time,
  model latency, terminal rendering, and local scheduling are metadata, not
  identity.
- Closed public vocabularies use typed variants. Unknown contract variants and
  invalid overlays fail with a classified error instead of being normalized.

## Evidence

- Every published claim maps to a committed manifest, score file, ablation
  table, or report under [`proof/`](proof/README.md).
- Screenshots may demonstrate rendering, but they are not primary evidence for
  a run or benchmark result.
- Negative results remain in the main result. "Wired, no uplift", rubric
  gameability, model stalls, and fake-actor limitations are reported as
  measured rather than rewritten as wins.
- Fake actors prove workflow shape only. Live actors, deterministic replay, and
  measured benchmarks are distinct evidence tiers.

## Public boundary

- `jesterky-quality` contains public demo workloads and public-corpus scans.
  Private customer data, internal corpora, and private release audits do not
  enter published crates or proof artifacts.
- Public files contain no contributor-specific absolute paths, secrets, or
  links to private planning material.
- Rust installation uses `cargo install jesterky-cli`. Python contract types use
  `uv add jesterky`.

## Ship gate

Before landing a public release:

```bash
cargo test --workspace
cargo build --all-targets
```

The schema drift guard, generated Python import/rebuild check, public-path and
secret scan, changelog review, and committed proof map must also pass. An
attestation without the corresponding command output or artifact is not a
pass.
