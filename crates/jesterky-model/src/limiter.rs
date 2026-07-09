//! [`AdaptiveLimiter`] — an AIMD concurrency controller for one model+provider.
//!
//! A fixed semaphore is the wrong tool against a rate-limited provider: pick it
//! too high and you eat 429s, too low and you leave throughput on the table. This
//! is TCP-style congestion control instead — **additive increase, multiplicative
//! decrease**. Every model call takes a permit; the effective ceiling floats:
//!
//! - **429 / rate-limit → halve** the ceiling (down to `min`) and reset progress.
//! - **sustained success → +1** the ceiling (up to `max`), one step per `limit`
//!   clean calls, so it climbs back gently rather than snapping straight to the
//!   wall that just rejected us.
//!
//! One limiter is shared (behind an `Arc`) by every shard hitting the same
//! model+provider, so all of them feel the same backpressure at once. It is pure
//! concurrency control — no IO, no knowledge of codex — so it is unit-testable.

use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Debug)]
struct Inner {
    /// Current AIMD ceiling — the most calls allowed in flight right now.
    limit: usize,
    /// Calls currently holding a permit.
    in_flight: usize,
    /// Clean calls since the last increase; at `>= limit` the ceiling grows.
    successes: usize,
}

/// A shared, self-tuning concurrency gate for one model+provider key.
#[derive(Debug)]
pub struct AdaptiveLimiter {
    inner: Mutex<Inner>,
    /// Woken whenever a permit frees or the ceiling rises.
    wake: Notify,
    min: usize,
    max: usize,
    /// Deterministic-ish spread for anything that reads it (kept internal).
    _pad: AtomicUsize,
}

impl AdaptiveLimiter {
    /// Start at `start` in flight, floating within `[min, max]`. All are clamped
    /// to at least 1 and ordered so `min <= start <= max`.
    pub fn new(start: usize, min: usize, max: usize) -> Arc<Self> {
        let min = min.max(1);
        let max = max.max(min);
        let start = start.clamp(min, max);
        Arc::new(Self {
            inner: Mutex::new(Inner {
                limit: start,
                in_flight: 0,
                successes: 0,
            }),
            wake: Notify::new(),
            min,
            max,
            _pad: AtomicUsize::new(0),
        })
    }

    /// Wait until the current ceiling has room, then take a permit. The returned
    /// guard releases on drop.
    pub async fn acquire(self: &Arc<Self>) -> Permit {
        loop {
            // Arm the notification *before* checking so a release/grow that lands
            // between the check and the await is never lost (tokio's documented
            // lost-wakeup guard).
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            {
                let mut g = self.inner.lock().unwrap();
                if g.in_flight < g.limit {
                    g.in_flight += 1;
                    return Permit {
                        limiter: Arc::clone(self),
                    };
                }
            }
            notified.await;
        }
    }

    /// Report a clean call: step the ceiling up by one per `limit` successes.
    pub fn on_success(&self) {
        let mut g = self.inner.lock().unwrap();
        g.successes += 1;
        if g.successes >= g.limit && g.limit < self.max {
            g.limit += 1;
            g.successes = 0;
            drop(g);
            // A higher ceiling may let a waiter proceed immediately.
            self.wake.notify_waiters();
        }
    }

    /// Report a rate-limit (429): halve the ceiling and forget accrued progress.
    pub fn on_rate_limited(&self) {
        let mut g = self.inner.lock().unwrap();
        g.limit = (g.limit / 2).max(self.min);
        g.successes = 0;
    }

    /// The current ceiling (for display / tests).
    pub fn limit(&self) -> usize {
        self.inner.lock().unwrap().limit
    }

    fn release(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            g.in_flight = g.in_flight.saturating_sub(1);
        }
        self.wake.notify_waiters();
    }
}

/// A held permit; drop returns it to the limiter.
pub struct Permit {
    limiter: Arc<AdaptiveLimiter>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.limiter.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplicative_decrease_then_additive_increase() {
        let lim = AdaptiveLimiter::new(8, 1, 8);
        assert_eq!(lim.limit(), 8);
        lim.on_rate_limited();
        assert_eq!(lim.limit(), 4, "429 halves the ceiling");
        lim.on_rate_limited();
        assert_eq!(lim.limit(), 2);
        // Additive increase: one step per `limit` clean calls.
        lim.on_success();
        lim.on_success();
        assert_eq!(lim.limit(), 3, "2 successes at limit 2 → +1");
        lim.on_success();
        lim.on_success();
        lim.on_success();
        assert_eq!(lim.limit(), 4, "3 successes at limit 3 → +1");
    }

    #[test]
    fn decrease_never_below_min_increase_never_above_max() {
        let lim = AdaptiveLimiter::new(2, 2, 3);
        lim.on_rate_limited();
        assert_eq!(lim.limit(), 2, "clamped at min");
        for _ in 0..20 {
            lim.on_success();
        }
        assert_eq!(lim.limit(), 3, "clamped at max");
    }

    #[tokio::test]
    async fn acquire_blocks_at_ceiling_and_frees_on_drop() {
        let lim = AdaptiveLimiter::new(1, 1, 4);
        let p1 = lim.acquire().await;
        // Second acquire cannot complete while the single permit is held.
        let pending =
            tokio::time::timeout(std::time::Duration::from_millis(50), lim.acquire()).await;
        assert!(pending.is_err(), "ceiling of 1 blocks the second acquire");
        drop(p1);
        // Now it proceeds.
        let got = tokio::time::timeout(std::time::Duration::from_millis(50), lim.acquire()).await;
        assert!(got.is_ok(), "freed permit unblocks the waiter");
    }
}
