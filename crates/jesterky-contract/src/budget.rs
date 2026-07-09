//! Formal **resource budgets** for a jesterky run — distinct from concurrency
//! [`crate::topology::Limit`]s (semaphores).
//!
//! # Declared in workflow JSON
//!
//! Budgets live on [`crate::topology::RunPlan::budgets`] and may be overridden
//! per run via `--args '{"budgets":{...}}'`. Overrides use
//! [`BudgetPlan::overlay_json`] (deep merge: nested `eta`/`viz` fields merge;
//! `caps` replaces when present).
//!
//! ```json
//! "runplan": {
//!   "budgets": {
//!     "warning_percent": 80,
//!     "fail_on_hard_exhaust": true,
//!     "eta": {
//!       "enabled": true,
//!       "mode": "all",
//!       "min_wall_secs": 1.0,
//!       "sample_interval_secs": 0.45
//!     },
//!     "viz": {
//!       "enabled": true,
//!       "show_progress": true,
//!       "show_eta": true,
//!       "show_nearest_tag": true,
//!       "short_labels": true
//!     },
//!     "caps": [
//!       { "kind": "actor_calls", "max": 8, "hard": true, "label": "turns" },
//!       { "kind": "tokens", "max": 100000, "hard": false },
//!       { "kind": "wall_seconds", "max": 120, "hard": false }
//!     ]
//!   }
//! }
//! ```
//!
//! # Semantics (important)
//!
//! | Surface | Meaning |
//! |---|---|
//! | **Progress** | How much of each cap is used *right now* (`spent/max`, `%`, state). |
//! | **ETA** | Estimated **time until that cap is exhausted** at the current burn rate — **not** time until the workflow finishes. |
//!
//! To make ETA ≈ “time until this episode ends”, set caps near the episode size
//! (e.g. `actor_calls.max == max_turns`). A high cap (e.g. 64) with a short run
//! (8 turns) yields a large, slowly changing ETA.
//!
//! # Typed surface
//!
//! | Type | Role |
//! |---|---|
//! | [`BudgetPlan`] | Full JSON config (`runplan.budgets`) + [`BudgetPlan::overlay_json`] |
//! | [`BudgetCap`] | One dimension (`kind`/`max`/`hard`/display flags) |
//! | [`BudgetEtaConfig`] / [`BudgetEtaMode`] | Forecast knobs |
//! | [`BudgetVizConfig`] | Panel knobs |
//! | [`BudgetEngine::snapshot`] | Pure projection → [`BudgetSnapshot`] |
//! | [`BudgetSnapshot`] | On [`crate::artifact::RunManifest::budgets`] |
//!
//! Host meters; engine only projects. Full field tables: `docs/BUDGETS.md`.
//! Shape aligned with optimizers `LimitEngine` + SMR progress-toward-limits.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::topology::ContractError;

pub const BUDGET_ENGINE_VERSION: &str = "budget_engine.v1";

// ───────────────────────────── kinds & caps ────────────────────────────────

/// What resource is being capped.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, JsonSchema, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum BudgetKind {
    /// Impure actor invocations (model turns, env-coupled hero acts, …).
    ActorCalls,
    /// Total tokens (in+out) observed on the live bus / recorded usage.
    Tokens,
    /// Wall-clock seconds since the host started the run.
    WallSeconds,
}

impl BudgetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActorCalls => "actor_calls",
            Self::Tokens => "tokens",
            Self::WallSeconds => "wall_seconds",
        }
    }

    /// Short panel label (`calls` / `tok` / `wall`).
    pub fn short_str(self) -> &'static str {
        match self {
            Self::ActorCalls => "calls",
            Self::Tokens => "tok",
            Self::WallSeconds => "wall",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::ActorCalls => "calls",
            Self::Tokens => "tokens",
            Self::WallSeconds => "seconds",
        }
    }
}

/// One declared cap on a run.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetCap {
    pub kind: BudgetKind,
    /// Hard ceiling; must be > 0.
    pub max: f64,
    /// When true, exceeding the cap fails the run if [`BudgetPlan::fail_on_hard_exhaust`].
    #[serde(default)]
    pub hard: bool,
    /// Optional display name (panel + snapshot). Defaults to [`BudgetKind::as_str`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Per-cap warning threshold override (percent of cap). Falls back to plan default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_percent: Option<f64>,
    /// Include this cap on the progress viz line (default true).
    #[serde(default = "default_true")]
    pub show_progress: bool,
    /// Include this cap on the ETA viz line when a forecast exists (default true).
    #[serde(default = "default_true")]
    pub show_eta: bool,
}

fn default_true() -> bool {
    true
}

fn default_warning_percent() -> f64 {
    80.0
}

fn default_min_wall_secs() -> f64 {
    1.0
}

fn default_sample_interval() -> f64 {
    0.45
}

// ───────────────────────────── plan (JSON config) ──────────────────────────

/// How ETA lines are composed on the terminal panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetEtaMode {
    /// Do not show or compute ETA forecasts.
    Off,
    /// Show only the soonest-to-exhaust cap.
    NearestOnly,
    /// Show every cap that has a finite forecast (default).
    #[default]
    All,
}

/// ETA / burn-rate forecast knobs (host metering + engine).
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetEtaConfig {
    /// Master switch for ETA forecasting (default true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which ETAs to surface on the panel.
    #[serde(default)]
    pub mode: BudgetEtaMode,
    /// Minimum wall seconds before burn-rate ETA is trusted (default 1.0).
    #[serde(default = "default_min_wall_secs")]
    pub min_wall_secs: f64,
    /// Host sampling cadence hint for live follow (default 0.45s).
    #[serde(default = "default_sample_interval")]
    pub sample_interval_secs: f64,
}

impl Default for BudgetEtaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: BudgetEtaMode::All,
            min_wall_secs: default_min_wall_secs(),
            sample_interval_secs: default_sample_interval(),
        }
    }
}

/// Terminal / panel rendering knobs for budgets.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetVizConfig {
    /// Master switch (default true). When false, no budget lines are drawn.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Show the progress line (`calls 4/64 (6%) · …`).
    #[serde(default = "default_true")]
    pub show_progress: bool,
    /// Show the ETA line (`ETA calls 3m24s · …`).
    #[serde(default = "default_true")]
    pub show_eta: bool,
    /// Append `nearest <kind>` on the ETA line.
    #[serde(default = "default_true")]
    pub show_nearest_tag: bool,
    /// Use short labels (`calls`/`tok`/`wall`) instead of full kind names.
    #[serde(default = "default_true")]
    pub short_labels: bool,
}

impl Default for BudgetVizConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_progress: true,
            show_eta: true,
            show_nearest_tag: true,
            short_labels: true,
        }
    }
}

/// Full budget configuration for a workflow run (part of [`crate::topology::RunPlan`]).
///
/// This is the **typed JSON interface**: serialize/deserialize as `runplan.budgets`.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetPlan {
    /// Caps by dimension. Empty plan ⇒ no metering / no panel lines.
    #[serde(default)]
    pub caps: Vec<BudgetCap>,
    /// Default warning threshold as percent of cap (0–100). Default 80.
    #[serde(default = "default_warning_percent")]
    pub warning_percent: f64,
    /// Fail the run when a `hard` cap is exhausted (default true).
    #[serde(default = "default_true")]
    pub fail_on_hard_exhaust: bool,
    /// ETA forecast configuration.
    #[serde(default)]
    pub eta: BudgetEtaConfig,
    /// Terminal panel configuration.
    #[serde(default)]
    pub viz: BudgetVizConfig,
}

impl Default for BudgetPlan {
    fn default() -> Self {
        Self {
            caps: Vec::new(),
            warning_percent: default_warning_percent(),
            fail_on_hard_exhaust: true,
            eta: BudgetEtaConfig::default(),
            viz: BudgetVizConfig::default(),
        }
    }
}

impl BudgetPlan {
    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }

    pub fn get(&self, kind: BudgetKind) -> Option<&BudgetCap> {
        self.caps.iter().find(|c| c.kind == kind)
    }

    /// Warning percent for a cap (per-cap override or plan default).
    pub fn warning_for(&self, cap: &BudgetCap) -> f64 {
        cap.warning_percent.unwrap_or(self.warning_percent)
    }

    /// Display label for a cap.
    pub fn label_for(&self, cap: &BudgetCap) -> String {
        cap.label
            .clone()
            .unwrap_or_else(|| cap.kind.as_str().to_string())
    }

    /// Short or full label depending on viz settings.
    pub fn panel_label_for(&self, cap: &BudgetCap) -> String {
        if let Some(label) = &cap.label {
            return label.clone();
        }
        if self.viz.short_labels {
            cap.kind.short_str().to_string()
        } else {
            cap.kind.as_str().to_string()
        }
    }

    /// Deep-merge a partial JSON object onto this plan.
    ///
    /// Used by CLI `--args.budgets` so callers can override knobs without
    /// re-declaring the full plan:
    ///
    /// ```json
    /// { "eta": { "mode": "nearest_only" }, "warning_percent": 70 }
    /// ```
    ///
    /// Rules:
    /// - Object keys in `overlay` replace / deep-merge into the base.
    /// - Nested objects (`eta`, `viz`) merge field-by-field.
    /// - Arrays (`caps`) **replace** when present (not element-wise merge).
    /// - Non-object overlays and deserialize failures return a typed contract error.
    pub fn overlay_json(&self, overlay: &serde_json::Value) -> Result<Self, ContractError> {
        if !overlay.is_object() {
            return Err(ContractError::InvalidOverlay {
                target: "budget",
                message: format!("expected JSON object, got {}", json_type(overlay)),
            });
        }

        let base = serde_json::to_value(self).map_err(|e| ContractError::InvalidOverlay {
            target: "budget",
            message: format!("failed to serialize base plan: {e}"),
        })?;
        let merged = deep_merge_json(base, overlay);
        serde_json::from_value(merged).map_err(|e| ContractError::InvalidOverlay {
            target: "budget",
            message: e.to_string(),
        })
    }
}

fn json_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Recursively merge `overlay` into `base` (both should be JSON objects for
/// nested merge; non-objects in overlay replace base).
fn deep_merge_json(mut base: serde_json::Value, overlay: &serde_json::Value) -> serde_json::Value {
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
            serde_json::Value::Object(base_map.clone())
        }
        _ => overlay.clone(),
    }
}

// ───────────────────────────── observations & snapshot ─────────────────────

/// One metered sample: how much of a budget was spent at a wall-clock offset.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetObservation {
    pub kind: BudgetKind,
    /// Seconds since run start when this sample was taken.
    pub t_secs: f64,
    /// Cumulative spent (same unit as the cap).
    pub spent: f64,
    /// Cumulative reserved-but-not-yet-settled amount. Hosts use this before
    /// wide parallel dispatch so caps account for in-flight work.
    #[serde(default)]
    pub reserved: f64,
}

/// How close a budget is to its cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetState {
    Ok,
    Warning,
    Exhausted,
    Unlimited,
    Unknown,
}

/// Forecast confidence (interval width).
#[derive(Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastConfidence {
    High,
    Medium,
    Low,
    Unknown,
}

/// ETA to exhaust one budget from observed burn rate.
///
/// **Not** “time until the workflow finishes” — see module docs.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetForecast {
    pub model: String,
    #[serde(default)]
    pub seconds_to_limit: Option<f64>,
    #[serde(default)]
    pub seconds_to_limit_low: Option<f64>,
    #[serde(default)]
    pub seconds_to_limit_high: Option<f64>,
    #[serde(default)]
    pub rate_per_second: Option<f64>,
    pub confidence: ForecastConfidence,
    pub sample_count: u64,
}

/// Progress + forecast for one declared cap.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub kind: BudgetKind,
    /// Effective display label (from cap or kind).
    pub label: String,
    pub unit: String,
    pub max: f64,
    pub spent: f64,
    #[serde(default)]
    pub reserved: f64,
    pub remaining: f64,
    /// `spent / max` in \[0, ∞); values ≥ 1 mean exhausted.
    pub utilization: f64,
    /// Percent of cap used.
    pub used_percent: f64,
    pub state: BudgetState,
    pub hard: bool,
    pub show_progress: bool,
    pub show_eta: bool,
    pub forecast: BudgetForecast,
}

/// Full budget projection for a run (live follow + final manifest).
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    pub schema_version: String,
    pub run_id: String,
    /// Wall seconds used for this snapshot.
    pub wall_secs: f64,
    /// Plan knobs used for this projection (echoed for consumers / viz).
    pub plan: BudgetPlan,
    pub items: Vec<BudgetStatus>,
    /// Soonest ETA among items that have a finite forecast (when ETA enabled).
    #[serde(default)]
    pub nearest: Option<BudgetStatus>,
    /// Aggregate run state: exhausted if any hard item is exhausted, else warning, else ok.
    pub state: BudgetState,
}

// ───────────────────────────── engine ──────────────────────────────────────

/// Pure budget engine: caps + observation series → snapshot.
pub struct BudgetEngine;

impl BudgetEngine {
    /// Project progress + ETA.
    ///
    /// `observations` should be chronological; multiple kinds may be interleaved.
    /// Warning threshold and ETA enablement come from `plan`.
    pub fn snapshot(
        run_id: impl Into<String>,
        plan: &BudgetPlan,
        observations: &[BudgetObservation],
        wall_secs: f64,
    ) -> BudgetSnapshot {
        let run_id = run_id.into();
        let wall_secs = wall_secs.max(0.0);
        if plan.is_empty() {
            return BudgetSnapshot {
                schema_version: BUDGET_ENGINE_VERSION.into(),
                run_id,
                wall_secs,
                plan: plan.clone(),
                items: Vec::new(),
                nearest: None,
                state: BudgetState::Unlimited,
            };
        }

        let eta_enabled = plan.eta.enabled && !matches!(plan.eta.mode, BudgetEtaMode::Off);

        let mut items = Vec::with_capacity(plan.caps.len());
        for cap in &plan.caps {
            if !(cap.max.is_finite() && cap.max > 0.0) {
                continue;
            }
            let series: Vec<(f64, f64, f64)> = observations
                .iter()
                .filter(|o| o.kind == cap.kind)
                .map(|o| (o.t_secs.max(0.0), o.spent.max(0.0), o.reserved.max(0.0)))
                .collect();
            let spent = series.last().map(|(_, s, _)| *s).unwrap_or(0.0);
            let spent = if cap.kind == BudgetKind::WallSeconds {
                wall_secs.max(spent)
            } else {
                spent
            };
            let reserved = series.last().map(|(_, _, r)| *r).unwrap_or(0.0);
            let committed = spent + reserved;
            let forecast_series: Vec<(f64, f64)> = series
                .iter()
                .map(|(t, spent, reserved)| (*t, spent + reserved))
                .collect();
            let remaining = (cap.max - committed).max(0.0);
            let utilization = if cap.max > 0.0 {
                committed / cap.max
            } else {
                0.0
            };
            let used_percent = (utilization * 100.0).clamp(0.0, 9999.0);
            let warn_at = plan.warning_for(cap);
            let state = if committed >= cap.max {
                BudgetState::Exhausted
            } else if used_percent >= warn_at {
                BudgetState::Warning
            } else {
                BudgetState::Ok
            };
            let forecast = if eta_enabled {
                forecast_eta(
                    remaining,
                    &forecast_series,
                    wall_secs,
                    plan.eta.min_wall_secs,
                )
            } else {
                BudgetForecast {
                    model: "disabled".into(),
                    seconds_to_limit: None,
                    seconds_to_limit_low: None,
                    seconds_to_limit_high: None,
                    rate_per_second: None,
                    confidence: ForecastConfidence::Unknown,
                    sample_count: forecast_series.len() as u64,
                }
            };
            items.push(BudgetStatus {
                kind: cap.kind,
                label: plan.label_for(cap),
                unit: cap.kind.unit().into(),
                max: cap.max,
                spent,
                reserved,
                remaining,
                utilization,
                used_percent,
                state,
                hard: cap.hard,
                show_progress: cap.show_progress,
                show_eta: cap.show_eta,
                forecast,
            });
        }

        let nearest = if eta_enabled {
            items
                .iter()
                .filter_map(|item| {
                    item.forecast
                        .seconds_to_limit
                        .map(|secs| (secs, item.clone()))
                })
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, item)| item)
        } else {
            None
        };

        let state = if items
            .iter()
            .any(|i| i.state == BudgetState::Exhausted && i.hard)
        {
            BudgetState::Exhausted
        } else if items.iter().any(|i| i.state == BudgetState::Exhausted) {
            BudgetState::Exhausted
        } else if items.iter().any(|i| i.state == BudgetState::Warning) {
            BudgetState::Warning
        } else if items.is_empty() {
            BudgetState::Unlimited
        } else {
            BudgetState::Ok
        };

        BudgetSnapshot {
            schema_version: BUDGET_ENGINE_VERSION.into(),
            run_id,
            wall_secs,
            plan: plan.clone(),
            items,
            nearest,
            state,
        }
    }
}

fn forecast_eta(
    remaining: f64,
    series: &[(f64, f64)],
    wall_secs: f64,
    min_wall_secs: f64,
) -> BudgetForecast {
    if remaining <= 0.0 {
        return BudgetForecast {
            model: "exhausted".into(),
            seconds_to_limit: Some(0.0),
            seconds_to_limit_low: Some(0.0),
            seconds_to_limit_high: Some(0.0),
            rate_per_second: None,
            confidence: ForecastConfidence::High,
            sample_count: series.len() as u64,
        };
    }
    let rates = positive_rates(series);
    let (mut model, rate, mut confidence) = pick_rate(&rates);
    let rate = match (rate, series.first(), series.last()) {
        (Some(r), Some((t0, s0)), Some((t1, s1))) if t1 > t0 && s1 >= s0 => {
            let elapsed_avg = (s1 - s0) / (t1 - t0);
            if elapsed_avg > 0.0 {
                Some(r.min(elapsed_avg * 1.5))
            } else {
                Some(r)
            }
        }
        (r, _, _) => r,
    };
    let rate = rate.or_else(|| {
        if wall_secs > min_wall_secs {
            let spent = series.last().map(|(_, s)| *s).unwrap_or(0.0);
            if spent > 0.0 {
                model = "elapsed_average".into();
                confidence = ForecastConfidence::Low;
                return Some(spent / wall_secs);
            }
        }
        None
    });

    match rate.filter(|r| *r > 0.0 && r.is_finite()) {
        Some(r) => {
            let seconds = (remaining / r).max(0.0);
            let (lo_m, hi_m) = interval_multipliers(confidence);
            BudgetForecast {
                model,
                seconds_to_limit: Some(seconds),
                seconds_to_limit_low: Some((seconds * lo_m).max(0.0)),
                seconds_to_limit_high: Some((seconds * hi_m).max(0.0)),
                rate_per_second: Some(r),
                confidence,
                sample_count: series.len() as u64,
            }
        }
        None => BudgetForecast {
            model: "insufficient_samples".into(),
            seconds_to_limit: None,
            seconds_to_limit_low: None,
            seconds_to_limit_high: None,
            rate_per_second: None,
            confidence: ForecastConfidence::Unknown,
            sample_count: series.len() as u64,
        },
    }
}

fn positive_rates(series: &[(f64, f64)]) -> Vec<f64> {
    let mut rates = Vec::new();
    for w in series.windows(2) {
        let (t0, s0) = w[0];
        let (t1, s1) = w[1];
        let dt = t1 - t0;
        let ds = s1 - s0;
        if dt >= 1.0 && ds > 0.0 {
            rates.push(ds / dt);
        }
    }
    rates
}

fn pick_rate(rates: &[f64]) -> (String, Option<f64>, ForecastConfidence) {
    if rates.is_empty() {
        return (
            "insufficient_samples".into(),
            None,
            ForecastConfidence::Unknown,
        );
    }
    let mut sorted = rates.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted[sorted.len() / 2];
    let confidence = match sorted.len() {
        0 => ForecastConfidence::Unknown,
        1..=2 => ForecastConfidence::Low,
        3..=5 => ForecastConfidence::Medium,
        _ => ForecastConfidence::High,
    };
    ("median_rate".into(), Some(mid), confidence)
}

fn interval_multipliers(confidence: ForecastConfidence) -> (f64, f64) {
    match confidence {
        ForecastConfidence::High => (0.85, 1.25),
        ForecastConfidence::Medium => (0.75, 1.5),
        ForecastConfidence::Low => (0.5, 2.0),
        ForecastConfidence::Unknown => (0.5, 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_and_eta_from_linear_burn() {
        let plan = BudgetPlan {
            caps: vec![
                BudgetCap {
                    kind: BudgetKind::ActorCalls,
                    max: 10.0,
                    hard: true,
                    label: Some("turns".into()),
                    warning_percent: None,
                    show_progress: true,
                    show_eta: true,
                },
                BudgetCap {
                    kind: BudgetKind::Tokens,
                    max: 1000.0,
                    hard: false,
                    label: None,
                    warning_percent: Some(50.0),
                    show_progress: true,
                    show_eta: true,
                },
            ],
            warning_percent: 80.0,
            fail_on_hard_exhaust: true,
            eta: BudgetEtaConfig::default(),
            viz: BudgetVizConfig::default(),
        };
        let obs = vec![
            BudgetObservation {
                kind: BudgetKind::ActorCalls,
                t_secs: 0.0,
                spent: 0.0,
                reserved: 0.0,
            },
            BudgetObservation {
                kind: BudgetKind::ActorCalls,
                t_secs: 10.0,
                spent: 5.0,
                reserved: 0.0,
            },
            BudgetObservation {
                kind: BudgetKind::Tokens,
                t_secs: 0.0,
                spent: 0.0,
                reserved: 0.0,
            },
            BudgetObservation {
                kind: BudgetKind::Tokens,
                t_secs: 10.0,
                spent: 400.0,
                reserved: 0.0,
            },
        ];
        let snap = BudgetEngine::snapshot("run-1", &plan, &obs, 10.0);
        assert_eq!(snap.items.len(), 2);
        let calls = snap
            .items
            .iter()
            .find(|i| i.kind == BudgetKind::ActorCalls)
            .unwrap();
        assert_eq!(calls.label, "turns");
        assert!((calls.spent - 5.0).abs() < 1e-9);
        let eta = calls.forecast.seconds_to_limit.unwrap();
        assert!((eta - 10.0).abs() < 0.5, "eta={eta}");
        assert!(snap.nearest.is_some());
        // tokens at 40% with warning_percent 50 → ok; at 80% plan default would warn
        let tok = snap
            .items
            .iter()
            .find(|i| i.kind == BudgetKind::Tokens)
            .unwrap();
        assert_eq!(tok.state, BudgetState::Ok);
    }

    #[test]
    fn eta_off_disables_forecasts() {
        let mut plan = BudgetPlan::default();
        plan.caps.push(BudgetCap {
            kind: BudgetKind::WallSeconds,
            max: 30.0,
            hard: true,
            label: None,
            warning_percent: None,
            show_progress: true,
            show_eta: true,
        });
        plan.eta.enabled = false;
        let snap = BudgetEngine::snapshot("r", &plan, &[], 10.0);
        assert!(snap.nearest.is_none());
        assert_eq!(snap.items[0].forecast.model, "disabled");
        assert!(snap.items[0].forecast.seconds_to_limit.is_none());
    }

    #[test]
    fn exhausted_state_when_over_cap() {
        let mut plan = BudgetPlan::default();
        plan.caps.push(BudgetCap {
            kind: BudgetKind::WallSeconds,
            max: 30.0,
            hard: true,
            label: None,
            warning_percent: None,
            show_progress: true,
            show_eta: true,
        });
        let snap = BudgetEngine::snapshot("r", &plan, &[], 45.0);
        assert_eq!(snap.items[0].state, BudgetState::Exhausted);
        assert_eq!(snap.state, BudgetState::Exhausted);
    }

    #[test]
    fn plan_roundtrips_json() {
        let raw = r#"{
          "warning_percent": 75,
          "fail_on_hard_exhaust": false,
          "eta": { "enabled": true, "mode": "nearest_only", "min_wall_secs": 2.0 },
          "viz": { "show_nearest_tag": false, "short_labels": false },
          "caps": [
            { "kind": "actor_calls", "max": 8, "hard": true, "label": "turns" }
          ]
        }"#;
        let plan: BudgetPlan = serde_json::from_str(raw).unwrap();
        assert_eq!(plan.warning_percent, 75.0);
        assert!(!plan.fail_on_hard_exhaust);
        assert_eq!(plan.eta.mode, BudgetEtaMode::NearestOnly);
        assert_eq!(plan.eta.min_wall_secs, 2.0);
        assert!(!plan.viz.show_nearest_tag);
        assert_eq!(plan.caps[0].label.as_deref(), Some("turns"));
        let back = serde_json::to_value(&plan).unwrap();
        let again: BudgetPlan = serde_json::from_value(back).unwrap();
        assert_eq!(plan, again);
    }

    #[test]
    fn overlay_json_partial_merge_preserves_caps() {
        let base = BudgetPlan {
            caps: vec![BudgetCap {
                kind: BudgetKind::ActorCalls,
                max: 64.0,
                hard: true,
                label: Some("calls".into()),
                warning_percent: None,
                show_progress: true,
                show_eta: true,
            }],
            warning_percent: 80.0,
            fail_on_hard_exhaust: true,
            eta: BudgetEtaConfig {
                mode: BudgetEtaMode::All,
                ..BudgetEtaConfig::default()
            },
            viz: BudgetVizConfig::default(),
        };
        let overlay = serde_json::json!({
            "warning_percent": 70,
            "eta": { "mode": "nearest_only", "min_wall_secs": 3.0 },
            "viz": { "show_nearest_tag": false }
        });
        let merged = base.overlay_json(&overlay).expect("valid budget overlay");
        assert_eq!(merged.caps.len(), 1);
        assert_eq!(merged.caps[0].max, 64.0);
        assert_eq!(merged.warning_percent, 70.0);
        assert_eq!(merged.eta.mode, BudgetEtaMode::NearestOnly);
        assert_eq!(merged.eta.min_wall_secs, 3.0);
        // Untouched eta fields keep base defaults.
        assert!(merged.eta.enabled);
        assert!((merged.eta.sample_interval_secs - 0.45).abs() < 1e-9);
        assert!(!merged.viz.show_nearest_tag);
        assert!(merged.viz.show_progress);
    }

    #[test]
    fn overlay_json_caps_array_replaces() {
        let base = BudgetPlan {
            caps: vec![BudgetCap {
                kind: BudgetKind::Tokens,
                max: 1_000_000.0,
                hard: false,
                label: None,
                warning_percent: None,
                show_progress: true,
                show_eta: true,
            }],
            ..BudgetPlan::default()
        };
        let overlay = serde_json::json!({
            "caps": [{ "kind": "actor_calls", "max": 8, "hard": true, "label": "turns" }]
        });
        let merged = base.overlay_json(&overlay).expect("valid budget overlay");
        assert_eq!(merged.caps.len(), 1);
        assert_eq!(merged.caps[0].kind, BudgetKind::ActorCalls);
        assert_eq!(merged.caps[0].label.as_deref(), Some("turns"));
    }
}
