//! Limits / semaphores — bounded concurrency and the central-serialization joint
//! (the DungeonGrid "poll under a center logic" pattern). A `map`/`session_group`
//! names a limit; `permits = 1` forces turns through a shared resource.
//!
//! Distinct from a `map`'s `concurrency` (ADR #5): concurrency bounds how many
//! items the runner *dispatches*; a limit is a named budget that bodies acquire
//! and release, and that serializes access to something shared (an env, a rate
//! cap). Shape ported from `rlm/v2/src/limits.rs` (§11), retyped.

use std::collections::HashMap;
use std::sync::Mutex;

/// Runtime state of one named limit.
#[derive(Debug)]
pub struct LimitState {
    pub permits: u32,
    pub active: u32,
}

/// The set of limits for a run, built from the merged [`RunPlan`]. Acquisition
/// WAITS for a free permit (that is what serializes turns) — the core stays
/// runtime-agnostic, so the waiting primitive is chosen by the implementer.
///
/// [`RunPlan`]: jesterky_contract::RunPlan
#[derive(Default)]
pub struct LimitSet {
    states: Mutex<HashMap<String, LimitState>>,
}

impl LimitSet {
    /// Build from `name → permits`.
    pub fn from_permits(permits: &std::collections::BTreeMap<String, u32>) -> Self {
        let states = permits
            .iter()
            .map(|(name, &permits)| {
                (name.clone(), LimitState { permits: permits.max(1), active: 0 })
            })
            .collect();
        Self { states: Mutex::new(states) }
    }

    /// Acquire a permit on `name`, waiting if the limit is saturated. Returns a
    /// guard that releases on drop (or an explicit `release`).
    ///
    /// TODO(impl): wait for a free permit (e.g. a per-limit `tokio::sync::
    /// Semaphore` or a notify loop), increment `active`, and have the runner
    /// emit `SemaphoreAcquired`. Never busy-spin. This is the joint that makes
    /// heroes take turns through the env.
    pub async fn acquire(&self, _name: &str, _owner: &str) -> Result<LimitGuard, LimitError> {
        todo!("wait for permit; active += 1; emit SemaphoreAcquired")
    }
}

/// RAII permit. Dropping (or `release`) frees the permit and emits
/// `SemaphoreReleased`.
pub struct LimitGuard {
    // TODO(impl): back-reference to the LimitSet + limit name so Drop releases.
    _private: (),
}

#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    #[error("limit not configured: {0}")]
    Unknown(String),
}
