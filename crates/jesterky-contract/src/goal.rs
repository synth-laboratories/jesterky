//! Formal **goals / work products** for a jesterky run — the semantic dual of
//! resource [`crate::budget`]s.
//!
//! Budgets cap *spend* and answer "will I hit the token/wall cap?". Goals set
//! *targets on achievement* and answer "have I already produced the thing I
//! wanted?". A run that structurally finished its graph may still have *not*
//! produced its deliverable; a run that produced it early can stop. Goals make
//! that termination **semantic**, not just structural (graph-finished).
//!
//! # Declared in workflow JSON
//!
//! Goals live on [`crate::topology::RunPlan::goals`] and may be overridden per
//! run via `--args '{"goals":{...}}'` using [`GoalPlan::overlay_json`] (deep
//! merge; the `goals` array **replaces** when present).
//!
//! ```json
//! "runplan": {
//!   "goals": {
//!     "terminate_on_met": true,
//!     "fail_on_unmet": true,
//!     "goals": [
//!       { "id": "found", "kind": "ledger_pred", "path": "summary.found", "equals": true },
//!       { "id": "quality", "kind": "metric_threshold", "path": "summary.score", "min": 0.8, "required": false }
//!     ]
//!   }
//! }
//! ```
//!
//! # Semantics (important)
//!
//! | Surface | Meaning |
//! |---|---|
//! | **Progress** | Per goal in `[0,1]`: checklist hit (0/1) or `value/min` for a threshold. |
//! | **State** | `met` / `unmet` / `unknown` (path missing / wrong type). |
//! | **Run state** | `met` iff **every `required` goal** is met. Non-required goals never block. |
//! | **terminate_on_met** | When all required goals are met, the runner *may* skip remaining entrypoints (early success wrap-up). The pure engine only computes met-ness; the host/runner sets [`GoalSnapshot::terminated_early`]. |
//! | **fail_on_unmet** | A required goal still unmet at run end fails the run (dual of `fail_on_hard_exhaust`). |
//!
//! # Typed surface
//!
//! | Type | Role |
//! |---|---|
//! | [`GoalPlan`] | Full JSON config (`runplan.goals`) + [`GoalPlan::overlay_json`] |
//! | [`GoalSpec`] / [`GoalKind`] | One goal (id/required + predicate) |
//! | [`GoalEngine::snapshot`] | Pure projection: ledger JSON + plan → [`GoalSnapshot`] |
//! | [`GoalSnapshot`] | On [`crate::artifact::RunManifest::goals`] |
//!
//! Host/programs supply the ledger; engine only projects. Full field tables:
//! `docs/GOALS.md`. Shape aligned with [`crate::budget::BudgetSnapshot`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const GOAL_ENGINE_VERSION: &str = "goal_engine.v1";

// ───────────────────────────── goals & plan ────────────────────────────────

fn default_true() -> bool {
    true
}

/// What a goal checks against the run ledger.
///
/// Internally tagged (`"kind": "..."`) so a [`GoalSpec`] flattens to a flat JSON
/// object. v1 ships two predicate kinds; adding a kind is a contract change
/// (mirrors the closed [`crate::budget::BudgetKind`] discipline).
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoalKind {
    /// Met when the ledger value at `path` deep-equals `equals`.
    LedgerPred { path: String, equals: Value },
    /// Met when the numeric ledger value at `path` is `>= min`. Progress = `value/min`.
    MetricThreshold { path: String, min: f64 },
}

impl GoalKind {
    /// Stable discriminant string for the snapshot (`ledger_pred` / `metric_threshold`).
    pub fn tag(&self) -> &'static str {
        match self {
            Self::LedgerPred { .. } => "ledger_pred",
            Self::MetricThreshold { .. } => "metric_threshold",
        }
    }

    /// Dotted ledger path this goal reads (segments split on `.`; numeric
    /// segments index into arrays).
    pub fn path(&self) -> &str {
        match self {
            Self::LedgerPred { path, .. } => path,
            Self::MetricThreshold { path, .. } => path,
        }
    }
}

/// One declared goal on a run.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct GoalSpec {
    /// Stable identifier (unique within a plan).
    pub id: String,
    /// Predicate over the ledger.
    #[serde(flatten)]
    pub kind: GoalKind,
    /// Required goals gate the run state and `fail_on_unmet` (default true).
    /// Non-required goals record progress only and never block.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Optional display name (panel + snapshot). Defaults to [`Self::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Include this goal on the progress viz line (default true).
    #[serde(default = "default_true")]
    pub show_progress: bool,
}

/// Terminal / panel rendering knobs for goals.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct GoalVizConfig {
    /// Master switch (default true). When false, no goal line is drawn.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Show the per-goal progress line (`goals 1/2 met · quality 0.72/0.80`).
    #[serde(default = "default_true")]
    pub show_progress: bool,
    /// Use short labels (goal id) instead of the full label.
    #[serde(default = "default_true")]
    pub short_labels: bool,
}

impl Default for GoalVizConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_progress: true,
            short_labels: true,
        }
    }
}

/// Full goal configuration for a workflow run (part of [`crate::topology::RunPlan`]).
///
/// This is the **typed JSON interface**: serialize/deserialize as `runplan.goals`.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct GoalPlan {
    /// Declared goals. Empty plan ⇒ no evaluation / no panel line.
    #[serde(default)]
    pub goals: Vec<GoalSpec>,
    /// When every required goal is met, allow the runner to skip remaining
    /// entrypoints (early success wrap-up). Default true.
    #[serde(default = "default_true")]
    pub terminate_on_met: bool,
    /// Fail the run when a `required` goal is still unmet at run end. Default true.
    #[serde(default = "default_true")]
    pub fail_on_unmet: bool,
    /// Optional finalize node id to run on success wrap-up. Recorded in v1;
    /// runner wiring is a follow-up (see `docs/GOALS.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalize: Option<String>,
    /// Future control-plane knob: when true, hosts/runners may cancel queued
    /// in-flight map work after success wrap-up. Default false preserves v1
    /// entrypoint-skip semantics.
    #[serde(default)]
    pub cancel_in_flight: bool,
    /// Terminal panel configuration.
    #[serde(default)]
    pub viz: GoalVizConfig,
}

impl Default for GoalPlan {
    fn default() -> Self {
        Self {
            goals: Vec::new(),
            terminate_on_met: true,
            fail_on_unmet: true,
            finalize: None,
            cancel_in_flight: false,
            viz: GoalVizConfig::default(),
        }
    }
}

impl GoalPlan {
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }

    /// Display label for a goal (explicit label or its id).
    pub fn label_for(&self, goal: &GoalSpec) -> String {
        goal.label.clone().unwrap_or_else(|| goal.id.clone())
    }

    /// Deep-merge a partial JSON object onto this plan (see
    /// [`crate::budget::BudgetPlan::overlay_json`] for the merge rules). The
    /// `goals` array **replaces** when present.
    pub fn overlay_json(&self, overlay: &Value) -> Self {
        let Ok(base) = serde_json::to_value(self) else {
            return self.clone();
        };
        let merged = deep_merge_json(base, overlay);
        serde_json::from_value(merged).unwrap_or_else(|_| self.clone())
    }
}

/// Recursively merge `overlay` into `base`; non-objects in `overlay` replace.
fn deep_merge_json(mut base: Value, overlay: &Value) -> Value {
    match (base.as_object_mut(), overlay.as_object()) {
        (Some(base_map), Some(over_map)) => {
            for (k, v) in over_map {
                match base_map.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        *existing = deep_merge_json(existing.clone(), v);
                    }
                    _ => {
                        base_map.insert(k.clone(), v.clone());
                    }
                }
            }
            Value::Object(base_map.clone())
        }
        _ => overlay.clone(),
    }
}

// ───────────────────────────── snapshot ─────────────────────────────────────

/// Whether a goal (or the whole run) is satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    /// Predicate satisfied.
    Met,
    /// Predicate evaluated but not satisfied.
    Unmet,
    /// Path missing or wrong type — cannot evaluate.
    Unknown,
}

/// Result of evaluating one goal against the ledger.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct GoalStatus {
    pub id: String,
    pub label: String,
    /// `ledger_pred` / `metric_threshold`.
    pub kind: String,
    pub required: bool,
    pub state: GoalState,
    /// Partial credit in `[0, 1]`: `1.0` when met, `value/min` for a threshold,
    /// `0.0` for an unmet predicate, `0.0` when unknown.
    pub progress: f64,
    /// Human-readable evaluation (`summary.score 0.72 < min 0.80`).
    pub detail: String,
    /// The observed ledger value at the goal's path (echoed for consumers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Value>,
    pub show_progress: bool,
}

/// Full goal projection for a run (live follow + final manifest).
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct GoalSnapshot {
    pub schema_version: String,
    pub run_id: String,
    /// Plan knobs used for this projection (echoed for consumers / viz).
    pub plan: GoalPlan,
    pub items: Vec<GoalStatus>,
    /// `met` iff every required goal is met; else `unmet`.
    pub state: GoalState,
    pub required_total: u32,
    pub required_met: u32,
    /// Set by the host/runner when it stopped remaining work because the goals
    /// were met. The pure engine always emits `false`.
    #[serde(default)]
    pub terminated_early: bool,
}

impl GoalSnapshot {
    /// Every required goal is met.
    pub fn all_required_met(&self) -> bool {
        self.state == GoalState::Met
    }

    /// The runner should skip remaining entrypoints: plan opts in and all
    /// required goals are met.
    pub fn should_terminate(&self) -> bool {
        self.plan.terminate_on_met && self.all_required_met()
    }

    /// A required goal is still unmet and the plan fails on that.
    pub fn should_fail(&self) -> bool {
        self.plan.fail_on_unmet && !self.all_required_met()
    }
}

// ───────────────────────────── engine ──────────────────────────────────────

/// Pure goal engine: plan + ledger → snapshot.
pub struct GoalEngine;

impl GoalEngine {
    /// Project goal progress from the run `ledger` (a JSON object of ledger
    /// keys → values, e.g. the seeded args + recorded node outputs).
    pub fn snapshot(run_id: impl Into<String>, plan: &GoalPlan, ledger: &Value) -> GoalSnapshot {
        let run_id = run_id.into();
        if plan.is_empty() {
            return GoalSnapshot {
                schema_version: GOAL_ENGINE_VERSION.into(),
                run_id,
                plan: plan.clone(),
                items: Vec::new(),
                state: GoalState::Met, // no required goals ⇒ vacuously met
                required_total: 0,
                required_met: 0,
                terminated_early: false,
            };
        }

        let mut items = Vec::with_capacity(plan.goals.len());
        let mut required_total = 0u32;
        let mut required_met = 0u32;
        let mut all_required_met = true;

        for goal in &plan.goals {
            let observed = resolve_path(ledger, goal.kind.path()).cloned();
            let (state, progress, detail) = evaluate(&goal.kind, observed.as_ref());
            if goal.required {
                required_total += 1;
                if state == GoalState::Met {
                    required_met += 1;
                } else {
                    all_required_met = false;
                }
            }
            items.push(GoalStatus {
                id: goal.id.clone(),
                label: plan.label_for(goal),
                kind: goal.kind.tag().into(),
                required: goal.required,
                state,
                progress,
                detail,
                observed,
                show_progress: goal.show_progress,
            });
        }

        GoalSnapshot {
            schema_version: GOAL_ENGINE_VERSION.into(),
            run_id,
            plan: plan.clone(),
            items,
            state: if all_required_met {
                GoalState::Met
            } else {
                GoalState::Unmet
            },
            required_total,
            required_met,
            terminated_early: false,
        }
    }
}

/// Resolve a dotted path into a JSON value. Empty segments are skipped; numeric
/// segments index into arrays. Returns `None` on any missing key / type mismatch.
fn resolve_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path.split('.') {
        if seg.is_empty() {
            continue;
        }
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn evaluate(kind: &GoalKind, observed: Option<&Value>) -> (GoalState, f64, String) {
    match kind {
        GoalKind::LedgerPred { path, equals } => match observed {
            None => (
                GoalState::Unknown,
                0.0,
                format!("{path} missing (want {equals})"),
            ),
            Some(v) if v == equals => (GoalState::Met, 1.0, format!("{path} == {equals}")),
            Some(v) => (GoalState::Unmet, 0.0, format!("{path} = {v} != {equals}")),
        },
        GoalKind::MetricThreshold { path, min } => match observed.and_then(Value::as_f64) {
            None => (
                GoalState::Unknown,
                0.0,
                format!("{path} not numeric (want >= {min})"),
            ),
            Some(v) => {
                let progress = if *min > 0.0 {
                    (v / min).clamp(0.0, 1.0)
                } else if v >= *min {
                    1.0
                } else {
                    0.0
                };
                if v >= *min {
                    (GoalState::Met, 1.0, format!("{path} {v} >= min {min}"))
                } else {
                    (
                        GoalState::Unmet,
                        progress,
                        format!("{path} {v} < min {min}"),
                    )
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan_with(goals: Vec<GoalSpec>) -> GoalPlan {
        GoalPlan {
            goals,
            ..GoalPlan::default()
        }
    }

    #[test]
    fn ledger_pred_met_and_unmet() {
        let plan = plan_with(vec![GoalSpec {
            id: "found".into(),
            kind: GoalKind::LedgerPred {
                path: "summary.found".into(),
                equals: json!(true),
            },
            required: true,
            label: None,
            show_progress: true,
        }]);
        let met = GoalEngine::snapshot("r", &plan, &json!({"summary": {"found": true}}));
        assert_eq!(met.state, GoalState::Met);
        assert_eq!(met.required_met, 1);
        assert!(met.should_terminate());
        assert!(!met.should_fail());

        let unmet = GoalEngine::snapshot("r", &plan, &json!({"summary": {"found": false}}));
        assert_eq!(unmet.state, GoalState::Unmet);
        assert_eq!(unmet.items[0].state, GoalState::Unmet);
        assert!(!unmet.should_terminate());
        assert!(unmet.should_fail());
    }

    #[test]
    fn metric_threshold_progress_and_state() {
        let plan = plan_with(vec![GoalSpec {
            id: "quality".into(),
            kind: GoalKind::MetricThreshold {
                path: "summary.score".into(),
                min: 0.8,
            },
            required: true,
            label: Some("quality".into()),
            show_progress: true,
        }]);
        let partial = GoalEngine::snapshot("r", &plan, &json!({"summary": {"score": 0.6}}));
        assert_eq!(partial.items[0].state, GoalState::Unmet);
        assert!((partial.items[0].progress - 0.75).abs() < 1e-9);

        let done = GoalEngine::snapshot("r", &plan, &json!({"summary": {"score": 0.9}}));
        assert_eq!(done.items[0].state, GoalState::Met);
        assert!((done.items[0].progress - 1.0).abs() < 1e-9);
    }

    #[test]
    fn missing_path_is_unknown_and_blocks_required() {
        let plan = plan_with(vec![GoalSpec {
            id: "found".into(),
            kind: GoalKind::LedgerPred {
                path: "summary.found".into(),
                equals: json!(true),
            },
            required: true,
            label: None,
            show_progress: true,
        }]);
        let snap = GoalEngine::snapshot("r", &plan, &json!({"summary": {}}));
        assert_eq!(snap.items[0].state, GoalState::Unknown);
        assert_eq!(snap.state, GoalState::Unmet);
        assert_eq!(snap.required_met, 0);
    }

    #[test]
    fn non_required_goal_never_blocks_run_state() {
        let plan = plan_with(vec![
            GoalSpec {
                id: "found".into(),
                kind: GoalKind::LedgerPred {
                    path: "found".into(),
                    equals: json!(true),
                },
                required: true,
                label: None,
                show_progress: true,
            },
            GoalSpec {
                id: "stretch".into(),
                kind: GoalKind::MetricThreshold {
                    path: "score".into(),
                    min: 100.0,
                },
                required: false,
                label: None,
                show_progress: true,
            },
        ]);
        let snap = GoalEngine::snapshot("r", &plan, &json!({"found": true, "score": 1.0}));
        assert_eq!(snap.state, GoalState::Met); // required met; stretch unmet but non-required
        assert_eq!(snap.required_total, 1);
        assert!(snap.should_terminate());
    }

    #[test]
    fn empty_plan_is_vacuously_met() {
        let snap = GoalEngine::snapshot("r", &GoalPlan::default(), &json!({}));
        assert_eq!(snap.state, GoalState::Met);
        assert!(snap.items.is_empty());
        assert_eq!(snap.required_total, 0);
        assert!(!snap.should_fail());
    }

    #[test]
    fn array_index_path_resolves() {
        let plan = plan_with(vec![GoalSpec {
            id: "first_reward".into(),
            kind: GoalKind::MetricThreshold {
                path: "results.0.reward".into(),
                min: 1.0,
            },
            required: true,
            label: None,
            show_progress: true,
        }]);
        let snap = GoalEngine::snapshot("r", &plan, &json!({"results": [{"reward": 2.0}]}));
        assert_eq!(snap.items[0].state, GoalState::Met);
    }

    #[test]
    fn plan_roundtrips_json_flat_kind() {
        let raw = r#"{
          "terminate_on_met": true,
          "fail_on_unmet": false,
          "goals": [
            { "id": "found", "kind": "ledger_pred", "path": "summary.found", "equals": true },
            { "id": "q", "kind": "metric_threshold", "path": "summary.score", "min": 0.8, "required": false }
          ]
        }"#;
        let plan: GoalPlan = serde_json::from_str(raw).unwrap();
        assert_eq!(plan.goals.len(), 2);
        assert!(!plan.fail_on_unmet);
        assert!(matches!(plan.goals[0].kind, GoalKind::LedgerPred { .. }));
        assert!(!plan.goals[1].required);
        let back = serde_json::to_value(&plan).unwrap();
        let again: GoalPlan = serde_json::from_value(back).unwrap();
        assert_eq!(plan, again);
    }

    #[test]
    fn overlay_json_replaces_goals_and_merges_knobs() {
        let base = plan_with(vec![GoalSpec {
            id: "found".into(),
            kind: GoalKind::LedgerPred {
                path: "found".into(),
                equals: json!(true),
            },
            required: true,
            label: None,
            show_progress: true,
        }]);
        let overlay = json!({
            "fail_on_unmet": false,
            "goals": [
                { "id": "score", "kind": "metric_threshold", "path": "s", "min": 0.5 }
            ]
        });
        let merged = base.overlay_json(&overlay);
        assert_eq!(merged.goals.len(), 1);
        assert_eq!(merged.goals[0].id, "score");
        assert!(!merged.fail_on_unmet);
        assert!(merged.terminate_on_met); // untouched default preserved
    }
}
