//! `jesterky-contract` — the pinned workflow contract, shared by the OSS core,
//! Stack, Synth Cloud, and the optimizers. THIS crate is the product; the
//! runtime is an implementation of it.
//!
//! Four schemas, versioned together but conceptually distinct (rebuild handoff
//! §2):
//!  * [`topology`] — how a workflow is declared.
//!  * [`event`]    — the event stream (the load-bearing one).
//!  * [`artifact`] — artifacts + the process-tree trace.
//!  * replay       — [`artifact::RunManifest`] carries the record needed to replay.
//!
//! ADR #1: these Rust types are the source of truth. M0 adds `schemars` to emit
//! `jesterky.schema.json` (the cross-language interop artifact) and pyo3 Python
//! types from the same definitions.

pub mod artifact;
pub mod event;
pub mod topology;

/// The contract version. A major bump is a stack-wide event (ADR #4).
pub const CONTRACT_VERSION: &str = "0.0.0-dev";

pub use artifact::{
    Artifact, ArtifactRef, CallKind, Checkpoint, Message, ProcessNode, RecordedOutput, RunManifest,
    RunStatus,
};
pub use event::{Addr, Event, EventKind, NodePath, PathSeg};
pub use topology::{
    Bindings, ContractError, Diagnostic, Limit, Node, NodeId, NodeKind, Ref, RunPlan, Severity,
    Verbosity, WorkflowSpec,
};

/// Emit the JSON Schema for workflow topology documents.
pub fn workflow_schema_json() -> String {
    serde_json::to_string_pretty(&schemars::schema_for!(WorkflowSpec))
        .expect("WorkflowSpec schema serializes")
}

/// Emit the JSON Schema for replay manifests.
pub fn manifest_schema_json() -> String {
    serde_json::to_string_pretty(&schemars::schema_for!(RunManifest))
        .expect("RunManifest schema serializes")
}
