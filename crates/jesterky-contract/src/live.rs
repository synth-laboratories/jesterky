//! Per-run **live event stream** — the push side-channel for a running shard's
//! tokens / steps / latest action.
//!
//! A workflow run creates one [`LiveBus`]; each shard's streaming model
//! *publishes* a [`LiveEvent`] as its subprocess emits, and the terminal renderer
//! *drains* the stream ([`LiveStream::fold`]) into the per-shard [`ShardProgress`]
//! it owns. This is transient: never serialized, never replayed, and outside the
//! [`Event`](crate::Event) contract (no new recorded `EventKind`) — so it changes
//! no schema and breaks no replay.
//!
//! **Push, not poll.** The old model was a shared `Mutex<HashMap>` the model wrote
//! and the renderer sampled every frame. Here the model *sends* when something
//! happens and the renderer folds what arrived — the renderer owns the state, and
//! there is no shared mutable map. std `mpsc` (multi-producer, single-consumer)
//! fits exactly: one sender clone per shard, one receiver in the render thread,
//! and no async-runtime dependency in this crate. A publish with no live receiver
//! (no `--follow`) is a silent no-op — progress is best-effort by construction.

use crate::NodePath;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

/// One shard's live counters, folded from the stream. Cheap to clone (the render
/// thread hands a snapshot to the view each frame).
#[derive(Debug, Clone)]
pub struct ShardProgress {
    /// Prompt / input tokens billed so far (summed across the shard's turns).
    pub tokens_in: u64,
    /// Completion / output tokens produced so far.
    pub tokens_out: u64,
    /// Discrete agent steps observed (tool calls, reasoning, messages).
    pub steps: u32,
    /// A short human label for the most recent action (`reading runner.rs`).
    pub last_action: String,
    /// When this shard first reported — its display elapsed is `started.elapsed()`.
    pub started: Instant,
}

impl ShardProgress {
    fn fresh() -> Self {
        Self {
            tokens_in: 0,
            tokens_out: 0,
            steps: 0,
            last_action: String::new(),
            started: Instant::now(),
        }
    }

    /// Total tokens (in + out) for compact rollups.
    pub fn tokens(&self) -> u64 {
        self.tokens_in + self.tokens_out
    }

    /// Seconds since this shard first reported.
    pub fn elapsed_secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

/// One progress publication for a single shard: the shard's current running
/// totals plus its latest action. The renderer folds the newest into
/// [`ShardProgress`]. An empty `action` means "unchanged" — don't clobber the
/// last real label with a blank.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    pub path: NodePath,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub steps: u32,
    pub action: String,
}

/// The **publish** end of a run's live stream, shared across shards behind an
/// `Arc`. Each shard publishes with its [`NodePath`]; sends are non-blocking and a
/// dropped [`LiveStream`] (no `--follow`) makes every publish a no-op.
#[derive(Debug, Clone)]
pub struct LiveBus {
    tx: Sender<LiveEvent>,
}

impl LiveBus {
    /// Create the run's stream: the [`LiveBus`] producers publish to, and the
    /// single [`LiveStream`] the render thread drains. One pair per run.
    pub fn channel() -> (Arc<LiveBus>, LiveStream) {
        let (tx, rx) = channel();
        (
            Arc::new(LiveBus { tx }),
            LiveStream {
                rx,
                state: HashMap::new(),
            },
        )
    }

    /// Publish a shard's current totals + latest action. Non-blocking; if the
    /// consumer is gone (no live render) the send fails and is ignored — live
    /// progress is best-effort and never blocks or errors the model.
    pub fn publish(
        &self,
        path: &NodePath,
        tokens_in: u64,
        tokens_out: u64,
        steps: u32,
        action: &str,
    ) {
        let _ = self.tx.send(LiveEvent {
            path: path.clone(),
            tokens_in,
            tokens_out,
            steps,
            action: action.to_string(),
        });
    }
}

/// The **consume** end: owns the folded per-shard state and drains the stream.
/// Single consumer (the render thread), so it is not `Clone`.
#[derive(Debug)]
pub struct LiveStream {
    rx: Receiver<LiveEvent>,
    state: HashMap<NodePath, ShardProgress>,
}

impl LiveStream {
    /// Drain every [`LiveEvent`] that has arrived since the last call into the
    /// folded state, then return a snapshot for this frame. Non-blocking: it
    /// applies whatever is pending and returns immediately (the render thread
    /// calls it once per tick). The first event for a path stamps its `started`
    /// clock, so elapsed counts from a shard's first report.
    pub fn fold(&mut self) -> HashMap<NodePath, ShardProgress> {
        // `try_recv` yields `Err` on both Empty (caught up this frame) and
        // Disconnected (every producer gone, run finished) — either way there is
        // nothing more to fold right now, so a plain `while let Ok` covers both.
        while let Ok(ev) = self.rx.try_recv() {
            let entry = self
                .state
                .entry(ev.path)
                .or_insert_with(ShardProgress::fresh);
            entry.tokens_in = ev.tokens_in;
            entry.tokens_out = ev.tokens_out;
            entry.steps = ev.steps;
            if !ev.action.is_empty() {
                entry.last_action = ev.action;
            }
        }
        self.state.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_applies_latest_totals_and_keeps_last_action() {
        let (bus, mut stream) = LiveBus::channel();
        let path = NodePath::root();
        bus.publish(&path, 100, 10, 1, "thinking");
        bus.publish(&path, 4000, 900, 3, "grep -rn unsafe src");
        // A later event with a blank action must not wipe the last real label.
        bus.publish(&path, 5000, 1200, 4, "");
        let snap = stream.fold();
        let shard = snap.get(&path).expect("shard folded");
        assert_eq!(shard.tokens_in, 5000);
        assert_eq!(shard.tokens_out, 1200);
        assert_eq!(shard.steps, 4);
        assert_eq!(shard.last_action, "grep -rn unsafe src");
        assert_eq!(shard.tokens(), 6200);
    }

    #[test]
    fn publish_after_stream_dropped_is_a_silent_noop() {
        let (bus, stream) = LiveBus::channel();
        drop(stream);
        // No panic, no error surfaced — best-effort by design.
        bus.publish(&NodePath::root(), 1, 2, 3, "x");
    }
}
