//! Workflow topology — how a workflow is declared (ADR #3).
//!
//! Authored as JSON/TOML, deserialized into [`WorkflowSpec`], validated, then
//! run. Node kinds are a CLOSED enum; data flows via DECLARED typed bindings
//! (`in`/`out` maps of local-name → [`Ref`]) — never an eval DSL over strings
//! (the `mapping.rs` trap in §11). A `Ref` is a typed path, resolved by the
//! ledger, e.g. `ledger.jobs`, `item`, `item.target`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub type NodeId = String;

/// A reference into run state, resolved by the ledger at execution time.
/// Kept a newtype over String in the skeleton; M0 replaces the inner form with
/// a parsed `{ source, path }` so resolution is total and checkable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ref(pub String);

/// `in`/`out` bindings: local name inside the node ⇄ a ledger [`Ref`].
pub type Bindings = BTreeMap<String, Ref>;

/// The closed set of node kinds. Adding a kind is a contract change (ADR #4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum NodeKind {
    /// Pure, deterministic, in-process op (e.g. `quality.expand`). Re-run on
    /// replay — NOT recorded (ADR #7). Resolved from the program registry.
    Program { op: String },
    /// Pure aggregation over a collection (a program that folds `results`).
    Reduce { op: String },
    /// A host actor call (a model, a subprocess). Impure → recorded for replay.
    Actor { actor: String },
    /// Fan `body` over `over`, binding each element as `item_as`. `concurrency`
    /// opts into parallel execution (ADR #5); `None`/`1` = serial. `min_success`
    /// is the reduce-gate (fraction of items that must succeed).
    Map {
        over: Ref,
        item_as: String,
        concurrency: Option<u32>,
        #[serde(default = "one")]
        min_success: f64,
        body: Box<Node>,
    },
    /// Serial iteration with side effects visible across items.
    ForEach {
        over: Ref,
        item_as: String,
        body: Box<Node>,
    },
    /// Loop `body` while `cond` resolves truthy, bounded by `max_iters`. Each
    /// pass increments the `Addr::iteration` at this node.
    While {
        cond: Ref,
        body: Box<Node>,
        max_iters: u32,
    },
    /// Take `then` if `cond` is truthy, else `otherwise`.
    Branch {
        cond: Ref,
        then: NodeId,
        otherwise: Option<NodeId>,
    },
    /// Spawn one session per element of `sessions`, each running `body` against
    /// the named `actor`. `limit` (permits) serializes shared-resource access —
    /// this is the "poll under a center logic" pattern (DungeonGrid): set
    /// `permits = 1` on the env resource so sessions take turns.
    SessionGroup {
        sessions: Ref,
        actor: String,
        body: Box<Node>,
        limit: Option<Limit>,
    },
    /// Resume a previously-checkpointed session and run `body`.
    ResumeSession { session: Ref, body: Box<Node> },
}

fn one() -> f64 {
    1.0
}

/// A named concurrency budget. Enforced by the runner; `permits` bounds how many
/// holders may hold it at once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    pub name: String,
    pub permits: u32,
}

/// A node = its kind plus its I/O wiring.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Inputs the node reads, resolved from the ledger before it runs.
    #[serde(default)]
    pub inputs: Bindings,
    /// Outputs the node writes back into the ledger when it completes.
    #[serde(default)]
    pub outputs: Bindings,
}

/// A whole workflow. `entrypoint` is the ordered list of top-level nodes to run
/// (matches mloky's model). `nodes` is the id → node map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    pub entrypoint: Vec<NodeId>,
    pub nodes: BTreeMap<NodeId, Node>,
    /// Execution budgets/defaults for this workflow.
    #[serde(default)]
    pub runplan: RunPlan,
}

/// Run-level configuration: concurrency budgets and event verbosity. A run may
/// override these via args.runplan (merged by the runner).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunPlan {
    /// Named limits → permits. A `map`/`session_group` `limit` names one of these.
    #[serde(default)]
    pub limits: BTreeMap<String, u32>,
    /// Default parallel width for `map` nodes lacking their own `concurrency`.
    /// `None`/`1` = serial (ADR #5).
    #[serde(default)]
    pub map_concurrency: Option<u32>,
    #[serde(default)]
    pub verbosity: Verbosity,
}

impl Default for RunPlan {
    fn default() -> Self {
        Self {
            limits: BTreeMap::new(),
            map_concurrency: None,
            verbosity: Verbosity::Standard,
        }
    }
}

/// Per-run event verbosity (adapted from `rlm_event_streaming_plan.md`, §11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verbosity {
    Minimal,
    #[default]
    Standard,
    Verbose,
}

/// Severity of a validation [`Diagnostic`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A typed validation result with a location (ported shape from
/// `graph_ir.rs::Diagnostic`, §11) — never a bare string.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// JSON-ish path to the offending element, e.g. `nodes.audit_jobs.over`.
    pub path: String,
    pub message: String,
}

impl WorkflowSpec {
    /// Parse + validate (schema, cycle-check) and return a canonical content
    /// hash — the stable graph identity a replay asserts against (ADR #5,
    /// mechanism ported from `graph_ir.rs::hash_graph_ir`, §11).
    ///
    /// TODO(M0): implement cycle-DFS, canonicalize (recursive key/edge sort),
    /// SHA-256 over canonical bytes; return typed `Diagnostic`s on failure.
    pub fn validate_and_hash(&self) -> Result<String, ContractError> {
        let errors = self
            .validate()
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            return Err(ContractError::Invalid(format_diagnostics(&errors)));
        }

        let mut canonical = serde_json::to_value(self)
            .map_err(|e| ContractError::Invalid(format!("failed to serialize topology: {e}")))?;
        canonicalize_value(&mut canonical);
        let bytes = serde_json::to_vec(&canonical).map_err(|e| {
            ContractError::Invalid(format!("failed to serialize canonical topology: {e}"))
        })?;
        let digest = Sha256::digest(bytes);
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Non-fatal validation pass: unknown refs, dangling entrypoints, unresolved
    /// `limit` names, unreachable nodes. Returns diagnostics (empty = clean).
    /// TODO(M0): implement; `validate_and_hash` calls this and fails on any
    /// `Severity::Error`.
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for (idx, id) in self.entrypoint.iter().enumerate() {
            if !self.nodes.contains_key(id) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    path: format!("entrypoint[{idx}]"),
                    message: format!("dangling entrypoint references unknown node `{id}`"),
                });
            }
        }

        for (id, node) in &self.nodes {
            validate_node(
                node,
                &format!("nodes.{id}"),
                &self.nodes,
                &self.runplan.limits,
                &mut diagnostics,
            );
        }

        let graph = named_node_edges(&self.nodes);
        detect_cycles(&graph, &mut diagnostics);

        let reachable = reachable_nodes(&self.entrypoint, &graph, &self.nodes);
        for id in self.nodes.keys() {
            if !reachable.contains(id) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: format!("nodes.{id}"),
                    message: format!("node `{id}` is not reachable from any entrypoint"),
                });
            }
        }

        diagnostics
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("invalid topology: {0}")]
    Invalid(String),
    #[error("cycle detected at node {0}")]
    Cycle(String),
}

fn validate_node(
    node: &Node,
    path: &str,
    nodes: &BTreeMap<NodeId, Node>,
    limits: &BTreeMap<String, u32>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &node.kind {
        NodeKind::Program { .. } | NodeKind::Reduce { .. } | NodeKind::Actor { .. } => {}
        NodeKind::Map {
            min_success, body, ..
        } => {
            if !(0.0..=1.0).contains(min_success) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    path: format!("{path}.min_success"),
                    message: format!("min_success must be between 0.0 and 1.0, got {min_success}"),
                });
            }
            validate_node(body, &format!("{path}.body"), nodes, limits, diagnostics);
        }
        NodeKind::ForEach { body, .. } => {
            validate_node(body, &format!("{path}.body"), nodes, limits, diagnostics);
        }
        NodeKind::While {
            body, max_iters, ..
        } => {
            if *max_iters == 0 {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    path: format!("{path}.max_iters"),
                    message: "max_iters must be greater than 0".to_string(),
                });
            }
            validate_node(body, &format!("{path}.body"), nodes, limits, diagnostics);
        }
        NodeKind::Branch {
            then, otherwise, ..
        } => {
            if !nodes.contains_key(then) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    path: format!("{path}.then"),
                    message: format!("branch references unknown node `{then}`"),
                });
            }
            if let Some(otherwise) = otherwise {
                if !nodes.contains_key(otherwise) {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        path: format!("{path}.otherwise"),
                        message: format!("branch references unknown node `{otherwise}`"),
                    });
                }
            }
        }
        NodeKind::SessionGroup { body, limit, .. } => {
            if let Some(limit) = limit {
                if !limits.contains_key(&limit.name) {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        path: format!("{path}.limit"),
                        message: format!(
                            "limit `{}` is not declared in runplan.limits",
                            limit.name
                        ),
                    });
                }
            }
            validate_node(body, &format!("{path}.body"), nodes, limits, diagnostics);
        }
        NodeKind::ResumeSession { body, .. } => {
            validate_node(body, &format!("{path}.body"), nodes, limits, diagnostics);
        }
    }
}

fn named_node_edges(nodes: &BTreeMap<NodeId, Node>) -> BTreeMap<NodeId, Vec<NodeId>> {
    nodes
        .iter()
        .map(|(id, node)| (id.clone(), node_edges(node)))
        .collect()
}

fn node_edges(node: &Node) -> Vec<NodeId> {
    match &node.kind {
        NodeKind::Branch {
            then, otherwise, ..
        } => {
            let mut refs = vec![then.clone()];
            if let Some(otherwise) = otherwise {
                refs.push(otherwise.clone());
            }
            refs
        }
        _ => Vec::new(),
    }
}

fn detect_cycles(graph: &BTreeMap<NodeId, Vec<NodeId>>, diagnostics: &mut Vec<Diagnostic>) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }

    fn visit(
        id: &str,
        graph: &BTreeMap<NodeId, Vec<NodeId>>,
        marks: &mut BTreeMap<NodeId, Mark>,
        stack: &mut Vec<NodeId>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if matches!(marks.get(id), Some(Mark::Done)) {
            return;
        }
        if matches!(marks.get(id), Some(Mark::Visiting)) {
            let cycle_at = id.to_string();
            if !diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error && d.path == format!("nodes.{cycle_at}"))
            {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    path: format!("nodes.{cycle_at}"),
                    message: format!("cycle detected at node `{cycle_at}`"),
                });
            }
            return;
        }

        marks.insert(id.to_string(), Mark::Visiting);
        stack.push(id.to_string());
        if let Some(edges) = graph.get(id) {
            for next in edges {
                if graph.contains_key(next) {
                    visit(next, graph, marks, stack, diagnostics);
                }
            }
        }
        stack.pop();
        marks.insert(id.to_string(), Mark::Done);
    }

    let mut marks = BTreeMap::new();
    let mut stack = Vec::new();
    for id in graph.keys() {
        visit(id, graph, &mut marks, &mut stack, diagnostics);
    }
}

fn reachable_nodes(
    entrypoint: &[NodeId],
    graph: &BTreeMap<NodeId, Vec<NodeId>>,
    nodes: &BTreeMap<NodeId, Node>,
) -> BTreeSet<NodeId> {
    let mut reachable = BTreeSet::new();
    let mut stack = entrypoint
        .iter()
        .filter(|id| nodes.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();

    while let Some(id) = stack.pop() {
        if !reachable.insert(id.clone()) {
            continue;
        }
        if let Some(edges) = graph.get(&id) {
            for next in edges.iter().rev() {
                if nodes.contains_key(next) {
                    stack.push(next.clone());
                }
            }
        }
    }

    reachable
}

fn canonicalize_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                canonicalize_value(value);
            }
        }
        serde_json::Value::Object(map) => {
            let mut sorted = map
                .iter_mut()
                .map(|(key, value)| {
                    canonicalize_value(value);
                    (key.clone(), std::mem::take(value))
                })
                .collect::<BTreeMap<_, _>>();
            map.clear();
            for (key, value) in std::mem::take(&mut sorted) {
                map.insert(key, value);
            }
        }
        _ => {}
    }
}

fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
    serde_json::to_string(diagnostics).unwrap_or_else(|_| {
        diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.path, d.message))
            .collect::<Vec<_>>()
            .join("; ")
    })
}
