//! Limits / semaphores — bounded concurrency and the central-serialization joint
//! (the DungeonGrid "poll under a center logic" pattern). A `session_group` names
//! a limit; `permits = 1` forces turns through a shared resource.
//!
//! Implemented as a runtime-agnostic async semaphore: `acquire` returns a future
//! that is `Pending` until a permit is free, registering the caller's waker so a
//! `release` (guard drop) can wake it. No busy-spin, no runtime dependency —
//! composes with the crate's `join_all_ordered` cooperative poller.

use std::collections::HashMap;
use std::future::poll_fn;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

/// Runtime state of one named limit: how many permits, how many held, and the
/// wakers of futures waiting for a permit.
#[derive(Debug, Default)]
struct LimitState {
    permits: u32,
    active: u32,
    waiters: Vec<Waker>,
}

/// The set of limits for a run, built from the merged `RunPlan`. Held as an
/// `Arc` so a [`LimitGuard`] can release on drop.
#[derive(Default)]
pub struct LimitSet {
    states: Mutex<HashMap<String, LimitState>>,
}

impl LimitSet {
    /// Build from `name → permits` (permits floored at 1).
    pub fn from_permits(permits: &std::collections::BTreeMap<String, u32>) -> Self {
        let states = permits
            .iter()
            .map(|(name, &permits)| {
                (
                    name.clone(),
                    LimitState { permits: permits.max(1), active: 0, waiters: Vec::new() },
                )
            })
            .collect();
        Self { states: Mutex::new(states) }
    }

    /// Acquire a permit on `name`, waiting (cooperatively) if the limit is
    /// saturated. Returns a guard that releases on drop. Errors if `name` was
    /// never configured — no silent auto-provisioning (house rule: no fallbacks).
    pub async fn acquire(self: &Arc<Self>, name: &str) -> Result<LimitGuard, LimitError> {
        if !self.states.lock().unwrap().contains_key(name) {
            return Err(LimitError::Unknown(name.to_string()));
        }
        let key = name.to_string();
        poll_fn(|cx| {
            let mut states = self.states.lock().unwrap();
            let state = states.get_mut(&key).expect("limit existence checked above");
            if state.active < state.permits {
                state.active += 1;
                Poll::Ready(())
            } else {
                if !state.waiters.iter().any(|w| w.will_wake(cx.waker())) {
                    state.waiters.push(cx.waker().clone());
                }
                Poll::Pending
            }
        })
        .await;
        Ok(LimitGuard { set: Arc::clone(self), name: name.to_string() })
    }

    fn release(&self, name: &str) {
        let mut states = self.states.lock().unwrap();
        if let Some(state) = states.get_mut(name) {
            state.active = state.active.saturating_sub(1);
            // Wake everyone waiting; whoever polls first takes the freed permit,
            // the rest re-register. Correct, if not maximally fair.
            for waker in state.waiters.drain(..) {
                waker.wake();
            }
        }
    }
}

/// RAII permit. Dropping frees the permit and wakes any waiter. The runner emits
/// `SemaphoreAcquired`/`Released` around acquire/drop (events are the runner's
/// job, not the LimitSet's).
pub struct LimitGuard {
    set: Arc<LimitSet>,
    name: String,
}

impl Drop for LimitGuard {
    fn drop(&mut self) {
        self.set.release(&self.name);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    #[error("limit not configured: {0}")]
    Unknown(String),
}
