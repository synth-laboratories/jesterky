"""jesterky — Python contract types for the jesterky workflow substrate.

These models are generated from the pinned JSON Schema emitted by the Rust
``jesterky-contract`` crate (the single source of truth, ADR #1). This package
is **client-only**: typed request/response shapes for talking to a jesterky
host. It runs no workflows — orchestration lives in the Rust core.

Regenerate with ``python/gen.sh`` after the contract changes.
"""

from .spec import WorkflowSpec
from .manifest import RunManifest

__all__ = ["WorkflowSpec", "RunManifest"]
__version__ = "0.1.0"
