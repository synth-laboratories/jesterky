//! `jesterky-core` — pure workflow orchestration (ADR #6). Parses a
//! [`jesterky_contract::WorkflowSpec`], walks the graph, emits the event stream,
//! records impure calls, and produces a [`jesterky_contract::RunManifest`]. It
//! contains no model, HTTP, subprocess, or clock code; everything IO crosses the
//! seam in [`traits`].
//!
//! Read order for the implementing engineer: `traits` (the seam) → `runner`
//! (the joints; note `emit` and the `execute_map` docstring) → `ledger`. All node
//! kinds, the async limit-semaphore, sessions, and the trace tree are
//! implemented; the seam traits are where a real host (Stack, Cloud) plugs in.

pub mod ledger;
pub mod limits;
pub mod mailbox;
pub mod runner;
pub mod session;
pub mod traits;

pub use limits::{LimitError, LimitGuard, LimitSet};
pub use mailbox::Mailbox;
pub use runner::{CoreError, ProgramFn, ProgramRegistry, Runner};
pub use session::Session;
pub use traits::{
    Actor, ActorRequest, ActorResult, ArtifactStore, CheckpointStore, Clock, EventSink, HostError,
    Resource,
};
