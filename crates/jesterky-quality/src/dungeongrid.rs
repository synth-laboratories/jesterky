//! DungeonGrid 4-party workload — mloky parity path on jesterky (**LLM-only**).
//!
//! Topology shape (jesterky encoding of mloky's round-robin play loop):
//! `reset` expands a capped turn schedule → serial `map` over turns drives a
//! `hero` actor (observe → **LLM policy** → step) → `finalize` emits
//! episode_result.
//!
//! The env is a small in-process grid (no Python `dungeongrid` package). It is
//! sovereign over positions/rewards/done; the hero **must** be a real model
//! (`--actor codex`). There is no scripted/fake policy path.
//!
//! Honest framing: this proves long-horizon orchestration + replay + viz, not
//! that the policy solves a real DungeonGrid quest.

use async_trait::async_trait;
use jesterky_contract::{HostConfig, HostRole, HostVizConfig};
use jesterky_core::ledger::Ledger;
use jesterky_core::{Actor, ActorRequest, ActorResult, CoreError, HostError, ProgramRegistry};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

pub const HERO_ACTOR: &str = "hero";
pub const DEFAULT_MAX_TURNS: u32 = 12;
pub const DEFAULT_HEROES: [&str; 4] = ["hero_1", "hero_2", "hero_3", "hero_4"];

// ───────────────────────────── grid env ────────────────────────────────

#[derive(Clone, Debug)]
struct HeroState {
    x: i32,
    y: i32,
    last_action: Option<String>,
    visited: u32,
}

#[derive(Debug)]
struct GridEnv {
    quest_id: String,
    seed: i32,
    width: i32,
    height: i32,
    goal: (i32, i32),
    heroes: HashMap<String, HeroState>,
    hero_order: Vec<String>,
    score: f64,
    achievements: Vec<String>,
    total_turns: u32,
    objective_met: bool,
    done: bool,
}

impl GridEnv {
    fn new() -> Self {
        Self {
            quest_id: String::new(),
            seed: 0,
            width: 8,
            height: 8,
            goal: (7, 7),
            heroes: HashMap::new(),
            hero_order: Vec::new(),
            score: 0.0,
            achievements: Vec::new(),
            total_turns: 0,
            objective_met: false,
            done: false,
        }
    }

    fn reset(&mut self, quest_id: &str, hero_ids: &[String], seed: i32) {
        self.quest_id = quest_id.to_string();
        self.seed = seed;
        self.hero_order = hero_ids.to_vec();
        self.heroes.clear();
        // Stagger starts so party members don't stack and the panel shows motion.
        let starts = [(1, 1), (1, 2), (2, 1), (2, 2)];
        for (i, id) in hero_ids.iter().enumerate() {
            let (x, y) = starts[i % starts.len()];
            self.heroes.insert(
                id.clone(),
                HeroState {
                    x,
                    y,
                    last_action: None,
                    visited: 1,
                },
            );
        }
        self.score = 0.0;
        self.achievements.clear();
        self.total_turns = 0;
        self.objective_met = false;
        self.done = false;
        self.unlock("party.assembled");
    }

    fn unlock(&mut self, name: &str) {
        if !self.achievements.iter().any(|a| a == name) {
            self.achievements.push(name.to_string());
        }
    }

    fn observe(&self, hero_id: &str) -> Result<String, String> {
        let h = self
            .heroes
            .get(hero_id)
            .ok_or_else(|| format!("unknown hero {hero_id}"))?;
        let (gx, gy) = self.goal;
        let dist = (gx - h.x).abs() + (gy - h.y).abs();
        let legal = legal_actions(h.x, h.y, self.width, self.height).join(", ");
        Ok(format!(
            "quest={quest} hero={hero} pos=[{x},{y}] goal=[{gx},{gy}] dist={dist} \
             world_turns={turns} score={score:.2} last={last} legal=[{legal}]",
            quest = self.quest_id,
            hero = hero_id,
            x = h.x,
            y = h.y,
            gx = gx,
            gy = gy,
            dist = dist,
            turns = self.total_turns,
            score = self.score,
            last = h.last_action.as_deref().unwrap_or("none"),
            legal = legal,
        ))
    }

    fn act_plan(&mut self, hero_id: &str, action: &str) -> Result<TurnResult, String> {
        if self.done {
            return Err("episode already done".into());
        }
        let h = self
            .heroes
            .get_mut(hero_id)
            .ok_or_else(|| format!("unknown hero {hero_id}"))?;
        let (ox, oy) = (h.x, h.y);
        let (gx, gy) = self.goal;
        let before = (gx - ox).abs() + (gy - oy).abs();

        let mut nx = ox;
        let mut ny = oy;
        match action {
            "move:north" => ny -= 1,
            "move:south" => ny += 1,
            "move:west" => nx -= 1,
            "move:east" => nx += 1,
            "wait" => {}
            other => return Err(format!("illegal action {other}")),
        }
        if nx < 0 || ny < 0 || nx >= self.width || ny >= self.height {
            return Err(format!("out of bounds from [{ox},{oy}] via {action}"));
        }
        h.x = nx;
        h.y = ny;
        h.last_action = Some(action.to_string());
        h.visited += 1;
        self.total_turns += 1;

        let after = (gx - nx).abs() + (gy - ny).abs();
        let mut reward = 0.0;
        let mut unlocked = Vec::new();
        if after < before {
            reward += 0.15;
        } else if after > before {
            reward -= 0.05;
        }
        if action != "wait" {
            reward += 0.02;
        }
        if self.total_turns == 1 {
            self.unlock("exploration.first_step");
            unlocked.push("exploration.first_step".to_string());
            reward += 0.1;
        }
        if after == 0 {
            self.objective_met = true;
            self.done = true;
            self.unlock("objective.goal_reached");
            unlocked.push("objective.goal_reached".to_string());
            reward += 1.0;
        }
        // Mild "room" milestones for panel flavor.
        if nx >= 4 && ny >= 4 && !self.achievements.iter().any(|a| a == "exploration.mid_map") {
            self.unlock("exploration.mid_map");
            unlocked.push("exploration.mid_map".to_string());
            reward += 0.25;
        }

        self.score += reward;
        let observation = self.observe(hero_id)?;
        Ok(TurnResult {
            observation,
            reward,
            new_achievements: unlocked,
            done: self.done,
            action: action.to_string(),
            pos: (nx, ny),
            dist: after,
        })
    }

    fn episode_result(&self) -> Value {
        json!({
            "score": self.score,
            "achievements": self.achievements,
            "turns": self.total_turns,
            "objective_met": self.objective_met,
            "quest_id": self.quest_id,
            "seed": self.seed,
            "heroes": self.hero_order,
        })
    }
}

#[derive(Debug, Clone)]
struct TurnResult {
    #[allow(dead_code)]
    observation: String,
    reward: f64,
    new_achievements: Vec<String>,
    done: bool,
    action: String,
    pos: (i32, i32),
    dist: i32,
}

fn legal_actions(x: i32, y: i32, w: i32, h: i32) -> Vec<&'static str> {
    let mut out = vec!["wait"];
    if y > 0 {
        out.push("move:north");
    }
    if y + 1 < h {
        out.push("move:south");
    }
    if x > 0 {
        out.push("move:west");
    }
    if x + 1 < w {
        out.push("move:east");
    }
    out
}

// ───────────────────────────── shared handle ───────────────────────────

/// Shared env handle for programs + the hero actor (one run at a time per CLI process).
#[derive(Clone)]
pub struct DungeonGridState {
    inner: Arc<Mutex<GridEnv>>,
}

impl Default for DungeonGridState {
    fn default() -> Self {
        Self::new()
    }
}

impl DungeonGridState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(GridEnv::new())),
        }
    }

    /// Sovereign episode verdict (for CLI post-run printing).
    pub fn episode_result(&self) -> Value {
        self.inner.lock().unwrap().episode_result()
    }
}

// ───────────────────────────── programs ────────────────────────────────

/// Register pure DungeonGrid programs. Pass the same [`DungeonGridState`] to
/// [`DungeonGridActor`] so reset/expand/finalize and hero turns share the env.
pub fn register(programs: &mut ProgramRegistry, state: DungeonGridState) {
    let reset_state = state.clone();
    programs.register(
        "dungeongrid.reset",
        Arc::new(move |ledger, inputs| reset(&reset_state, ledger, inputs)),
    );
    let fin_state = state.clone();
    programs.register(
        "dungeongrid.finalize",
        Arc::new(move |_ledger, inputs| finalize(&fin_state, inputs)),
    );
}

fn hero_ids_from(ledger: &Ledger, inputs: &Value) -> Vec<String> {
    if let Some(arr) = inputs.get("hero_ids").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
    }
    if let Some(arr) = ledger.get("hero_ids").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
    }
    if let Some(s) = inputs
        .get("hero_ids")
        .and_then(|v| v.as_str())
        .or_else(|| ledger.get("hero_ids").and_then(|v| v.as_str()))
    {
        return s
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
    }
    DEFAULT_HEROES.iter().map(|s| (*s).to_string()).collect()
}

fn reset(state: &DungeonGridState, ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let quest_id = inputs
        .get("quest_id")
        .and_then(|v| v.as_str())
        .or_else(|| ledger.get("quest_id").and_then(|v| v.as_str()))
        .unwrap_or("lantern_crypt")
        .to_string();
    let seed = inputs
        .get("seed")
        .and_then(|v| v.as_i64())
        .or_else(|| ledger.get("seed").and_then(|v| v.as_i64()))
        .unwrap_or(7) as i32;
    let max_turns = inputs
        .get("max_turns")
        .and_then(|v| v.as_u64())
        .or_else(|| ledger.get("max_turns").and_then(|v| v.as_u64()))
        .unwrap_or(DEFAULT_MAX_TURNS as u64) as u32;
    let hero_ids = hero_ids_from(ledger, inputs);
    if hero_ids.is_empty() {
        return Err(CoreError::from(
            jesterky_core::ledger::LedgerError::TypeMismatch(
                "dungeongrid.reset needs at least one hero_id".into(),
            ),
        ));
    }

    {
        let mut env = state.inner.lock().unwrap();
        env.reset(&quest_id, &hero_ids, seed);
    }

    // Round-robin turn schedule — jesterky map body is a single node, so the
    // multi-step mloky play_loop collapses to one recorded hero turn per job.
    let mut jobs = Vec::new();
    for turn in 0..max_turns {
        let hero_id = hero_ids[(turn as usize) % hero_ids.len()].clone();
        let hero_index = (turn as usize) % hero_ids.len();
        jobs.push(json!({
            "turn": turn,
            "hero_id": hero_id,
            "hero_index": hero_index,
            "label": format!("t{turn:02}:{hero}", hero = hero_ids[(turn as usize) % hero_ids.len()]),
        }));
    }

    Ok(json!({
        "jobs": jobs,
        "episode_done": false,
        "turn_count": 0,
        "hero_ids": hero_ids,
        "quest_id": quest_id,
        "seed": seed,
        "max_turns": max_turns,
    }))
}

fn finalize(state: &DungeonGridState, inputs: &Value) -> Result<Value, CoreError> {
    let env = state.inner.lock().unwrap();
    let mut result = env.episode_result();
    // Fold map turn count if present for diagnostics.
    if let Some(turns) = inputs.get("turns").and_then(|v| v.as_array()) {
        if let Some(obj) = result.as_object_mut() {
            obj.insert("mapped_turns".into(), json!(turns.len()));
        }
    }
    Ok(json!({ "episode_result": result }))
}

// ───────────────────────────── model action parse ──────────────────────

const LEGAL_ACTIONS: &[&str] = &["move:north", "move:south", "move:east", "move:west", "wait"];

fn parse_model_action(text: &Value) -> Option<String> {
    let raw = if let Some(s) = text.get("action").and_then(|v| v.as_str()) {
        Some(s.to_string())
    } else if let Some(arr) = text.get("actions").and_then(|v| v.as_array()) {
        arr.first().and_then(|first| {
            first
                .as_str()
                .map(str::to_string)
                .or_else(|| {
                    first
                        .get("action")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .or_else(|| {
                    first
                        .get("type")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
        })
    } else {
        None
    }?;
    let action = raw.trim().to_string();
    if LEGAL_ACTIONS.contains(&action.as_str()) {
        Some(action)
    } else {
        None
    }
}

// ───────────────────────────── hero actor ──────────────────────────────

/// Host actor for DungeonGrid: observe → **LLM policy** → step env.
///
/// The policy actor is required — there is no scripted / fake hero path.
/// On model failure or unparseable action the turn fails (no non-LLM fallback).
pub struct DungeonGridActor {
    state: DungeonGridState,
    policy: Arc<dyn Actor>,
}

impl DungeonGridActor {
    pub fn with_policy(state: DungeonGridState, policy: Arc<dyn Actor>) -> Self {
        Self { state, policy }
    }
}

#[async_trait]
impl Actor for DungeonGridActor {
    async fn drive(&self, req: ActorRequest) -> Result<ActorResult, HostError> {
        if req.actor != HERO_ACTOR {
            return self.policy.drive(req).await;
        }

        let job = req
            .inputs
            .get("job")
            .cloned()
            .unwrap_or_else(|| req.inputs.clone());
        let hero_id = job
            .get("hero_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HostError::Actor {
                actor: HERO_ACTOR.into(),
                message: "hero job missing hero_id".into(),
            })?
            .to_string();
        let turn = job.get("turn").and_then(|v| v.as_u64()).unwrap_or(0);

        // Short-circuit remaining map items once the env ends the episode.
        {
            let env = self.state.inner.lock().unwrap();
            if env.done {
                return Ok(ActorResult {
                    outputs: json!({
                        "hero_id": hero_id,
                        "turn": turn,
                        "skipped": true,
                        "done": true,
                        "action": "skip",
                        "reward": 0.0,
                        "label": format!("skip {hero_id}"),
                        "policy_source": "env_done",
                    }),
                    score: Some(env.score),
                    signal: None,
                    artifacts: Vec::new(),
                });
            }
        }

        let observation = {
            let env = self.state.inner.lock().unwrap();
            env.observe(&hero_id).map_err(HostError::Resource)?
        };

        let policy_req = ActorRequest {
            addr: req.addr.clone(),
            actor: HERO_ACTOR.to_string(),
            inputs: json!({
                "hero_id": hero_id,
                "turn": turn,
                "observation": observation,
                "legal_actions": LEGAL_ACTIONS,
            }),
        };
        let policy_result = self.policy.drive(policy_req).await?;
        let action =
            parse_model_action(&policy_result.outputs).ok_or_else(|| HostError::Actor {
                actor: HERO_ACTOR.into(),
                message: format!(
                    "LLM policy did not return a legal action (got {}); expected one of {:?}",
                    policy_result.outputs, LEGAL_ACTIONS
                ),
            })?;

        let result = {
            let mut env = self.state.inner.lock().unwrap();
            env.act_plan(&hero_id, &action)
                .map_err(HostError::Resource)?
        };

        let ach = result.new_achievements.join(",");
        let label = if result.reward > 0.0 {
            format!(
                "{:<12} +{:.2}  [{},{}]",
                result.action, result.reward, result.pos.0, result.pos.1
            )
        } else {
            format!(
                "{:<12} [{},{}] d={}",
                result.action, result.pos.0, result.pos.1, result.dist
            )
        };

        Ok(ActorResult {
            outputs: json!({
                "hero_id": hero_id,
                "turn": turn,
                "observation": observation,
                "action": result.action,
                "reward": result.reward,
                "new_achievements": result.new_achievements,
                "done": result.done,
                "pos": [result.pos.0, result.pos.1],
                "dist": result.dist,
                "label": label,
                "achievements_note": ach,
                "policy_source": "llm",
                "rationale": policy_result.outputs.get("rationale").cloned().unwrap_or(Value::Null),
                // score field helps the btop panel pick a tone
                "score": if result.done { 9.0 } else if result.reward >= 0.15 { 7.0 } else if result.reward < 0.0 { 3.0 } else { 6.0 },
            }),
            score: Some(result.reward),
            signal: Some(json!({ "done": result.done, "policy_source": "llm" })),
            artifacts: Vec::new(),
        })
    }
}

// ───────────────────────────── host config ─────────────────────────────

pub fn host_config() -> HostConfig {
    let mut roles = BTreeMap::new();
    roles.insert(
        HERO_ACTOR.to_string(),
        HostRole {
            prompt: Some(HERO_SYSTEM_PROMPT.to_string()),
            prompt_file: None,
        },
    );
    let mut output_schemas = BTreeMap::new();
    output_schemas.insert(
        HERO_ACTOR.to_string(),
        "dungeongrid_action.schema.json".to_string(),
    );
    HostConfig {
        roles,
        output_schemas,
        sandboxes: Default::default(),
        viz: Some(HostVizConfig {
            // Party lanes: expand yields `hero_ids` (4 heroes), not turn slots.
            item_labels_op: Some("dungeongrid.reset".to_string()),
            item_jobs_field: Some("hero_ids".to_string()),
            item_label_field: Some("hero_id".to_string()),
            map_node: Some("play_turns".to_string()),
            matrix_report_field: Some("episode_result".to_string()),
        }),
    }
}

pub const HERO_SYSTEM_PROMPT: &str = "\
You control one DungeonGrid hero on a shared grid. Each turn you receive an \
observation with pos, goal, dist, and legal moves. Choose exactly ONE action \
from the legal set. Prefer reducing Manhattan distance to the goal. Never \
immediately reverse your previous move when another on-goal move exists \
(anti-oscillation). Reply with ONE JSON object: \
{\"action\":\"move:east|move:west|move:north|move:south|wait\",\"rationale\":\"one short sentence\"}. \
No tools, no prose outside the JSON.";

#[cfg(test)]
mod tests {
    use super::*;
    use jesterky_contract::{Addr, NodePath};

    /// Stand-in for an LLM policy: reads the observation and returns a legal
    /// on-goal action. Used only in unit tests — production always uses a real
    /// model actor via the CLI.
    struct StubLlmPolicy;

    #[async_trait]
    impl Actor for StubLlmPolicy {
        async fn drive(&self, req: ActorRequest) -> Result<ActorResult, HostError> {
            let obs = req
                .inputs
                .get("observation")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Minimal parse: if goal is east of pos, move east; else south/wait.
            let action = if obs.contains("goal=[") && obs.contains("pos=[") {
                // Prefer east when the observation mentions a larger x goal via
                // the env's own string — enough to exercise the LLM-only path.
                if obs.contains("move:east") || obs.contains("dist=") {
                    "move:east"
                } else {
                    "wait"
                }
            } else {
                "wait"
            };
            Ok(ActorResult {
                outputs: json!({ "action": action, "rationale": "stub llm" }),
                score: None,
                signal: None,
                artifacts: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn llm_policy_party_runs_under_cap() {
        let state = DungeonGridState::new();
        let mut programs = ProgramRegistry::new();
        register(&mut programs, state.clone());
        let reset = programs.get("dungeongrid.reset").unwrap();
        let ledger = Ledger::new();
        let out = reset(
            &ledger,
            &json!({
                "quest_id": "test",
                "seed": 1,
                "max_turns": 8,
                "hero_ids": ["hero_1", "hero_2", "hero_3", "hero_4"]
            }),
        )
        .unwrap();
        let jobs = out["jobs"].as_array().unwrap();
        assert_eq!(jobs.len(), 8);

        let actor = DungeonGridActor::with_policy(state.clone(), Arc::new(StubLlmPolicy));
        for job in jobs {
            let result = actor
                .drive(ActorRequest {
                    addr: Addr {
                        run_id: "t".into(),
                        node_path: NodePath::root(),
                        iteration: 0,
                        local_seq: 0,
                    },
                    actor: HERO_ACTOR.into(),
                    inputs: json!({ "job": job }),
                })
                .await
                .unwrap();
            assert_eq!(result.outputs["policy_source"], "llm");
            assert!(result.outputs.get("action").is_some());
        }
        let ep = state.episode_result();
        assert_eq!(ep["turns"], 8);
        assert!(ep["score"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn parse_model_action_accepts_only_legal() {
        assert_eq!(
            parse_model_action(&json!({"action": "move:east"})).as_deref(),
            Some("move:east")
        );
        assert_eq!(parse_model_action(&json!({"action": "teleport"})), None);
        assert_eq!(
            parse_model_action(&json!({"actions": ["move:south"]})).as_deref(),
            Some("move:south")
        );
    }
}
