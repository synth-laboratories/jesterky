//! The ledger — typed run state and declared-edge I/O (ADR #3).
//!
//! A node reads its `inputs` bindings from the ledger, runs, and writes its
//! `outputs` back. An "edge" is just an output key of one node lining up with an
//! input binding of another. There is NO expression language: a `Ref` is a typed
//! path (`ledger.jobs`, `item`, `item.target`), resolved here — never `eval`'d.

use jesterky_contract::{Bindings, Ref};
use std::collections::HashMap;

/// Keyed store of typed values for one run (and, scoped, one map item via the
/// `item` binding). Values are `serde_json::Value` in the skeleton; M0 tightens
/// this toward the contract's typed value envelope.
#[derive(Debug, Default, Clone)]
pub struct Ledger {
    slots: HashMap<String, serde_json::Value>,
    /// The current map/for_each element, bound as `item` (and `item.field`).
    current_item: Option<serde_json::Value>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the per-item binding before resolving a map body's inputs. In the
    /// PARALLEL map path this is done on a per-item CLONE of the ledger so items
    /// never race on `current_item` (ADR #5) — see `runner::execute_map`.
    pub fn with_item(&self, item: serde_json::Value) -> Ledger {
        let mut l = self.clone();
        l.current_item = Some(item);
        l
    }

    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        self.slots.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.slots.get(key)
    }

    /// Resolve a single [`Ref`] against ledger state / the current item.
    ///
    /// TODO(M0): parse the ref into `{source, path}` and walk it. Supported
    /// sources: `ledger.<key>[.path]`, `item[.path]`, literals. Total and
    /// checkable — no fallbacks (house rule).
    pub fn resolve(&self, _r: &Ref) -> Result<serde_json::Value, LedgerError> {
        todo!("M0: typed ref resolution (ledger.* / item.* / literal)")
    }

    /// Resolve every binding into a concrete `{name: value}` inputs object.
    /// In the parallel map path this runs on the main thread per item, BEFORE
    /// dispatch, so the threaded body gets concrete inputs (ADR #5).
    pub fn resolve_bindings(&self, _b: &Bindings) -> Result<serde_json::Value, LedgerError> {
        todo!("M0: map each binding via `resolve`, collect into a JSON object")
    }

    /// Write a node's result back under its `outputs` bindings.
    pub fn store_outputs(
        &mut self,
        _outputs: &Bindings,
        _result: &serde_json::Value,
    ) -> Result<(), LedgerError> {
        todo!("M0: for each output binding, copy result[field] → ledger[key]")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("unresolved reference: {0}")]
    Unresolved(String),
    #[error("type mismatch resolving {0}")]
    TypeMismatch(String),
}
