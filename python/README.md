# jesterky (Python)

Python contract types for the [jesterky](https://github.com/jesterky/jesterky)
workflow substrate. This package is **client-only**: it gives you typed,
validated request/response shapes (`WorkflowSpec`, `RunManifest`) for talking to
a jesterky host. It runs no workflows — orchestration lives in the Rust core.

The models are generated from the pinned JSON Schema emitted by the Rust
`jesterky-contract` crate, which is the single source of truth (ADR #1). The Rust
types define the contract; this package mirrors them for Python callers.

```python
from jesterky import WorkflowSpec, RunManifest

spec = WorkflowSpec.model_validate_json(open("workflow.json").read())
manifest = RunManifest.model_validate_json(run_output)
```

## Install

```bash
pip install jesterky
```

## Regenerate

The types are codegen'd, not hand-written. After the contract changes:

```bash
./python/gen.sh   # re-emits schema from Rust, regenerates spec.py + manifest.py
```

Licensed under MIT OR Apache-2.0.
