//! The ledger — typed run state and declared-edge I/O (ADR #3).
//!
//! A node reads its `inputs` bindings from the ledger, runs, and writes its
//! `outputs` back. An "edge" is just an output key of one node lining up with an
//! input binding of another. There is NO expression language: a `Ref` is a typed
//! path (`ledger.jobs`, `item`, `item.target`), resolved here — never `eval`'d.

use jesterky_contract::{Bindings, Ref};
use std::collections::HashMap;

/// Keyed store of typed values for one run (and, scoped, one map item via the
/// `item` binding). Values are raw JSON at this IO seam; public replay identity
/// is still carried by typed contract events/manifests.
#[derive(Debug, Default, Clone)]
pub struct Ledger {
    slots: HashMap<String, serde_json::Value>,
    /// The current map/for_each/session element, bound as `item_as` and, for
    /// compatibility with the cycle-1 map path, also as `item`.
    current_items: HashMap<String, serde_json::Value>,
}

pub(crate) struct ItemBindingRestore {
    previous: Vec<(String, Option<serde_json::Value>)>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the per-item binding before resolving a map body's inputs. In the
    /// PARALLEL map path this is done on a per-item CLONE of the ledger so items
    /// never race on `current_item` (ADR #5) — see `runner::execute_map`.
    pub fn with_item(&self, item: serde_json::Value) -> Ledger {
        self.with_item_as("item", item)
    }

    pub(crate) fn with_item_as(&self, name: &str, item: serde_json::Value) -> Ledger {
        let mut l = self.clone();
        l.bind_item(name, item);
        l
    }

    pub(crate) fn bind_item(&mut self, name: &str, item: serde_json::Value) -> ItemBindingRestore {
        let mut names = vec![name.to_string()];
        if name != "item" {
            names.push("item".to_string());
        }

        let mut previous = Vec::with_capacity(names.len());
        for name in names {
            previous.push((name.clone(), self.current_items.insert(name, item.clone())));
        }
        ItemBindingRestore { previous }
    }

    pub(crate) fn restore_item(&mut self, restore: ItemBindingRestore) {
        for (name, previous) in restore.previous {
            match previous {
                Some(value) => {
                    self.current_items.insert(name, value);
                }
                None => {
                    self.current_items.remove(&name);
                }
            }
        }
    }

    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        self.slots.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.slots.get(key)
    }

    /// Snapshot the top-level ledger slots (seeded args + node outputs written
    /// back) as a JSON object. This is the surface goals evaluate against —
    /// a `GoalKind` path like `summary.score` reads slot `summary` then `.score`.
    /// The per-item `item` bindings are transient and deliberately excluded.
    pub fn snapshot_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.slots
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }

    /// Resolve a single [`Ref`] against ledger state / the current item.
    ///
    /// Supported sources: `ledger.<key>[.path]`, `item[.path]`, named map
    /// item bindings, and JSON literals. Total and checkable — no fallbacks
    /// (house rule).
    pub fn resolve(&self, r: &Ref) -> Result<serde_json::Value, LedgerError> {
        let raw = r.0.trim();
        if let Some((source, path)) = item_source_and_path(raw) {
            if let Some(item) = self.current_items.get(source) {
                return walk_path(item.clone(), &path, &r.0);
            }
            return Err(LedgerError::UnknownSource(source.to_string()));
        }

        if raw.starts_with("ledger.") {
            let (key, path) = ledger_key_and_path(raw)?;
            let value = self
                .slots
                .get(key)
                .cloned()
                .ok_or_else(|| LedgerError::MissingSlot(key.to_string()))?;
            return walk_path(value, &path, &r.0);
        }

        serde_json::from_str(raw).map_err(|e| LedgerError::MalformedLiteral {
            raw: r.0.clone(),
            message: e.to_string(),
        })
    }

    /// Resolve every binding into a concrete `{name: value}` inputs object.
    /// In the parallel map path this runs on the main thread per item, BEFORE
    /// dispatch, so the threaded body gets concrete inputs (ADR #5).
    pub fn resolve_bindings(&self, b: &Bindings) -> Result<serde_json::Value, LedgerError> {
        let mut resolved = serde_json::Map::new();
        for (name, r) in b {
            resolved.insert(name.clone(), self.resolve(r)?);
        }
        Ok(serde_json::Value::Object(resolved))
    }

    /// Write a node's result back under its `outputs` bindings.
    pub fn store_outputs(
        &mut self,
        outputs: &Bindings,
        result: &serde_json::Value,
    ) -> Result<(), LedgerError> {
        for (field, dest) in outputs {
            let value = result
                .as_object()
                .and_then(|object| object.get(field))
                .cloned()
                .ok_or_else(|| LedgerError::MissingSlot(format!("result.{field}")))?;
            let (key, path) = ledger_key_and_path(dest.0.trim())?;
            store_ledger_path(&mut self.slots, key, &path, value)?;
        }
        Ok(())
    }
}

fn item_source_and_path(raw: &str) -> Option<(&str, Vec<String>)> {
    let mut parts = raw.split('.');
    let source = parts.next()?;
    if source.is_empty() || source == "ledger" || !is_ref_source(source) {
        return None;
    }
    let path = parts.map(str::to_string).collect::<Vec<_>>();
    if path.iter().any(String::is_empty) {
        return None;
    }
    Some((source, path))
}

fn is_ref_source(source: &str) -> bool {
    let mut chars = source.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("unresolved reference: {0}")]
    Unresolved(String),
    #[error("unknown reference source: {0}")]
    UnknownSource(String),
    #[error("missing ledger slot: {0}")]
    MissingSlot(String),
    #[error("bad path resolving {reference}: {message}")]
    BadPath { reference: String, message: String },
    #[error("malformed JSON literal `{raw}`: {message}")]
    MalformedLiteral { raw: String, message: String },
    #[error("type mismatch resolving {0}")]
    TypeMismatch(String),
}

fn ledger_key_and_path(raw: &str) -> Result<(&str, Vec<String>), LedgerError> {
    let rest = raw
        .strip_prefix("ledger.")
        .ok_or_else(|| LedgerError::TypeMismatch(raw.to_string()))?;
    if rest.is_empty() {
        return Err(LedgerError::BadPath {
            reference: raw.to_string(),
            message: "missing ledger key".to_string(),
        });
    }

    let mut parts = rest.split('.');
    let key = parts.next().unwrap_or_default();
    if key.is_empty() {
        return Err(LedgerError::BadPath {
            reference: raw.to_string(),
            message: "missing ledger key".to_string(),
        });
    }
    let path = parts.map(str::to_string).collect::<Vec<_>>();
    if path.iter().any(String::is_empty) {
        return Err(LedgerError::BadPath {
            reference: raw.to_string(),
            message: "empty path segment".to_string(),
        });
    }
    Ok((key, path))
}

fn walk_path(
    mut current: serde_json::Value,
    path: &[String],
    original: &str,
) -> Result<serde_json::Value, LedgerError> {
    for segment in path {
        current = match current {
            serde_json::Value::Object(map) => map
                .get(segment)
                .cloned()
                .ok_or_else(|| LedgerError::BadPath {
                    reference: original.to_string(),
                    message: format!("missing object key `{segment}`"),
                })?,
            serde_json::Value::Array(values) => {
                let index = segment
                    .parse::<usize>()
                    .map_err(|_| LedgerError::BadPath {
                        reference: original.to_string(),
                        message: format!("array segment `{segment}` is not an index"),
                    })?;
                values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| LedgerError::BadPath {
                        reference: original.to_string(),
                        message: format!("array index {index} out of bounds"),
                    })?
            }
            _ => return Err(LedgerError::TypeMismatch(original.to_string())),
        };
    }
    Ok(current)
}

fn store_ledger_path(
    slots: &mut HashMap<String, serde_json::Value>,
    key: &str,
    path: &[String],
    value: serde_json::Value,
) -> Result<(), LedgerError> {
    if path.is_empty() {
        slots.insert(key.to_string(), value);
        return Ok(());
    }

    let root = slots
        .entry(key.to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    store_json_path(root, path, value)
}

fn store_json_path(
    current: &mut serde_json::Value,
    path: &[String],
    value: serde_json::Value,
) -> Result<(), LedgerError> {
    if path.is_empty() {
        *current = value;
        return Ok(());
    }

    if !current.is_object() {
        return Err(LedgerError::TypeMismatch(path.join(".")));
    }

    let object = current.as_object_mut().unwrap();
    if path.len() == 1 {
        object.insert(path[0].clone(), value);
        return Ok(());
    }

    let next = object
        .entry(path[0].clone())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    store_json_path(next, &path[1..], value)
}
