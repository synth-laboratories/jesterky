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
//! ADR #1: these Rust types are the source of truth. The crate emits JSON
//! schemas from the same definitions so downstream generated artifacts can
//! detect contract drift.

pub mod artifact;
pub mod budget;
pub mod event;
pub mod goal;
pub mod grounding;
pub mod live;
pub mod sandbox;
pub mod topology;

/// The contract version. A major bump is a stack-wide event (ADR #4).
pub const CONTRACT_VERSION: &str = "0.1.2";

pub use artifact::{
    Artifact, ArtifactRef, CallKind, Checkpoint, InvariantCheck, InvariantReport, Message,
    ProcessNode, RecordedOutput, RunManifest, RunStatus, RunStopReason,
};
pub use budget::{
    BudgetCap, BudgetEngine, BudgetEtaConfig, BudgetEtaMode, BudgetForecast, BudgetKind,
    BudgetObservation, BudgetPlan, BudgetSnapshot, BudgetState, BudgetStatus, BudgetVizConfig,
    ForecastConfidence, BUDGET_ENGINE_VERSION,
};
pub use event::{Addr, Event, EventKind, NodePath, PathSeg};
pub use goal::{
    GoalEngine, GoalKind, GoalPlan, GoalSnapshot, GoalSpec, GoalState, GoalStatus, GoalVizConfig,
    GOAL_ENGINE_VERSION,
};
pub use grounding::{
    CitedSpan, Grounding, GroundingPlan, GroundingReport, GroundingSourceRef, GroundingStatus,
    GroundingTally, GroundingVerdict, GroundingVerdictRecord, ReviewState,
    GROUNDING_SCHEMA_VERSION,
};
pub use live::{LiveBus, LiveEvent, LiveStream, ShardProgress};
pub use sandbox::{
    SandboxBackend, SandboxCapture, SandboxConfig, SandboxMode, SandboxSeed,
};
pub use topology::{
    Bindings, ContractError, Diagnostic, HostConfig, HostRole, HostVizConfig, Limit, Node, NodeId,
    NodeKind, Ref, RunPlan, Severity, Verbosity, WorkflowSpec,
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
