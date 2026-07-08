//! Mailbox — message-passing between sessions (multi-agent coordination). This
//! is ORCHESTRATION, not IO, so it lives in the core and is fully implemented
//! (unlike the host seam). DungeonGrid heroes publish/drain here each turn.
//!
//! The runner emits `MessagePublished` on publish and `MessageAvailable` on a
//! non-empty drain; those events carry the coordination trace for the visualizer
//! and the optimizer.

use jesterky_contract::Message;
use std::collections::HashMap;
use std::sync::Mutex;

/// Per-recipient inboxes. A sender never receives its own message.
#[derive(Default)]
pub struct Mailbox {
    inboxes: Mutex<HashMap<String, Vec<Message>>>,
}

impl Mailbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deliver `msg` to every recipient except the sender. Returns the number of
    /// deliveries (the runner uses it to decide whether to emit).
    pub fn publish(&self, msg: &Message) -> usize {
        let mut inboxes = self.inboxes.lock().unwrap();
        let mut delivered = 0;
        for recipient in &msg.recipients {
            if recipient != &msg.sender {
                inboxes.entry(recipient.clone()).or_default().push(msg.clone());
                delivered += 1;
            }
        }
        delivered
    }

    /// Take and clear a recipient's pending messages.
    pub fn drain(&self, recipient: &str) -> Vec<Message> {
        self.inboxes
            .lock()
            .unwrap()
            .get_mut(recipient)
            .map(std::mem::take)
            .unwrap_or_default()
    }
}
