//! `ModelActor` logic tested against a stub model — no network, deterministic.
//! The one real codex round-trip is `#[ignore]`d (needs the CLI + auth + net).

use jesterky_contract::{Addr, NodePath};
use jesterky_core::{Actor, ActorRequest};
use jesterky_model::{
    build_prompt, extract_json, validate_json, CodexModel, ModelActor, ModelError, ModelRequest,
    StubModel,
};
use serde_json::json;
use std::path::PathBuf;

fn request(actor: &str, inputs: serde_json::Value) -> ActorRequest {
    ActorRequest {
        addr: Addr {
            run_id: "test".to_string(),
            node_path: NodePath::root(),
            iteration: 0,
            local_seq: 0,
        },
        actor: actor.to_string(),
        inputs,
    }
}

#[tokio::test]
async fn parses_json_reply_into_outputs() {
    let actor = ModelActor::new(StubModel::replying(
        r#"{"verdict":"pass","confidence":0.9}"#,
    ));
    let out = actor
        .drive(request("quality_auditor", json!({ "file": "a.rs" })))
        .await
        .expect("drive ok");
    assert_eq!(out.outputs["verdict"], json!("pass"));
    // score/signal are optimizer slots the actor does NOT synthesize.
    assert!(out.score.is_none() && out.signal.is_none());
}

#[tokio::test]
async fn tolerates_prose_and_code_fences_around_json() {
    let actor = ModelActor::new(StubModel::replying(
        "Sure — here is the result:\n```json\n{\"ok\":true}\n```\nHope that helps.",
    ));
    let out = actor
        .drive(request("x", json!({})))
        .await
        .expect("drive ok");
    assert_eq!(out.outputs["ok"], json!(true));
}

#[tokio::test]
async fn non_json_reply_becomes_a_classified_actor_error() {
    let actor = ModelActor::new(StubModel::replying("I could not complete that."));
    let err = actor.drive(request("x", json!({}))).await.unwrap_err();
    assert!(err.to_string().contains("JSON"), "got: {err}");
    // The re-ask is bounded — the message names the attempt count, not an endless loop.
    assert!(err.to_string().contains("attempts"), "got: {err}");
}

#[tokio::test]
async fn parse_flake_is_recovered_by_a_re_ask() {
    use std::sync::atomic::{AtomicU32, Ordering};
    // A model that flakes (prose) on the first call, then returns valid JSON —
    // the ~few-percent non-determinism a live run hits. The actor must recover it.
    let calls = std::sync::Arc::new(AtomicU32::new(0));
    let c = calls.clone();
    let actor = ModelActor::new(StubModel::new(move |_: &ModelRequest| {
        let n = c.fetch_add(1, Ordering::SeqCst);
        Ok(if n == 0 {
            "Sorry, here are my thoughts...".to_string()
        } else {
            r#"{"verdict":"pass"}"#.to_string()
        })
    }));
    let out = actor
        .drive(request("x", json!({})))
        .await
        .expect("recovers");
    assert_eq!(out.outputs["verdict"], json!("pass"));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "one flake + one recovery");
}

#[tokio::test]
async fn model_error_class_survives_into_the_host_error() {
    let actor = ModelActor::new(StubModel::new(
        |_: &ModelRequest| -> Result<String, ModelError> {
            Err(ModelError::Quota("usage limit reached".to_string()))
        },
    ));
    let err = actor
        .drive(request("scanner", json!({})))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("quota"), "got: {err}");
}

#[test]
fn prompt_carries_role_system_and_inputs() {
    let prompt = build_prompt(&ModelRequest {
        actor: "auditor".to_string(),
        system: Some("Be terse.".to_string()),
        inputs: json!({ "n": 1 }),
        output_schema: None,
        node_path: None,
        live: None,
        sandbox: None,
    });
    assert!(prompt.contains("Be terse."));
    assert!(prompt.contains("auditor"));
    assert!(prompt.contains("\"n\""));
}

#[test]
fn extract_json_handles_bare_prose() {
    assert_eq!(extract_json("noise {\"a\":1} tail").unwrap()["a"], json!(1));
    assert!(extract_json("no object here").is_err());
}

#[test]
fn extract_json_takes_the_last_object_past_reasoning_braces() {
    // The real failure mode: a reasoning prelude contains its own braces, then the
    // answer follows. First-`{`-to-last-`}` straddles both and fails; we must take
    // the last balanced object.
    let reply = "<reasoning>Return a JSON like {\"example\": 1} then decide.</reasoning>\n\
                 ```json\n{\"verdict\": \"fail\", \"score\": 3}\n```";
    let v = extract_json(reply).expect("finds the answer object");
    assert_eq!(v["verdict"], json!("fail"));
    assert_eq!(v["score"], json!(3));
}

#[test]
fn extract_json_ignores_braces_inside_strings() {
    // A brace inside a string literal must not break balance tracking.
    let v = extract_json("prefix {\"note\": \"has a } brace\", \"ok\": true} suffix").unwrap();
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["note"], json!("has a } brace"));
}

// ── output-schema validation (draft-07 subset, dependency-free) ──────────────

fn verdict_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["dimension", "verdict"],
        "properties": {
            "dimension": { "type": "string" },
            "verdict": { "type": "string", "enum": ["pass", "fail"] },
            "count": { "type": "integer" }
        }
    })
}

#[test]
fn validate_json_accepts_conforming_object() {
    let ok = json!({ "dimension": "security", "verdict": "pass", "count": 3 });
    assert!(validate_json(&ok, &verdict_schema()).is_ok());
}

#[test]
fn validate_json_flags_missing_required_wrong_type_and_bad_enum() {
    let schema = verdict_schema();
    let missing = json!({ "verdict": "pass" });
    assert!(validate_json(&missing, &schema)
        .unwrap_err()
        .contains("dimension"));
    let bad_enum = json!({ "dimension": "x", "verdict": "maybe" });
    assert!(validate_json(&bad_enum, &schema)
        .unwrap_err()
        .contains("enum"));
    let wrong_type = json!({ "dimension": "x", "verdict": "pass", "count": "three" });
    assert!(validate_json(&wrong_type, &schema)
        .unwrap_err()
        .contains("integer"));
    let extra = json!({ "dimension": "x", "verdict": "pass", "surprise": 1 });
    assert!(validate_json(&extra, &schema)
        .unwrap_err()
        .contains("surprise"));
}

fn committed_schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("quality_verdict.schema.json")
}

#[tokio::test]
async fn off_schema_reply_fails_the_node_after_retries() {
    // A model that always returns a shape-valid JSON object that is MISSING the
    // schema's required `severity`/`rationale`. The actor must re-ask and, after
    // the bounded budget, fail the node with a schema-violation reason.
    use std::sync::atomic::{AtomicU32, Ordering};
    let calls = std::sync::Arc::new(AtomicU32::new(0));
    let c = calls.clone();
    let actor = ModelActor::new(StubModel::new(move |_: &ModelRequest| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(r#"{"dimension":"security","verdict":"pass"}"#.to_string())
    }))
    .with_output_schema("quality_scanner", committed_schema_path());

    let err = actor
        .drive(request("quality_scanner", json!({ "job": {} })))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("schema"), "got: {err}");
    assert!(
        calls.load(Ordering::SeqCst) >= 2,
        "must re-ask, not accept once"
    );
}

#[tokio::test]
async fn on_schema_reply_passes_first_try() {
    let actor = ModelActor::new(StubModel::replying(
        r#"{"dimension":"security","verdict":"pass","severity":"none","rationale":"ok"}"#,
    ))
    .with_output_schema("quality_scanner", committed_schema_path());
    let out = actor
        .drive(request("quality_scanner", json!({ "job": {} })))
        .await
        .expect("conforming reply accepted");
    assert_eq!(out.outputs["verdict"], json!("pass"));
}

/// Real codex round-trip. Ignored — needs the codex CLI, ChatGPT-bundle auth,
/// and network. Run manually:
///   cargo test -p jesterky-model -- --ignored codex_live_round_trip
#[tokio::test]
#[ignore = "requires codex CLI + ChatGPT-bundle auth + network"]
async fn codex_live_round_trip() {
    let actor = ModelActor::new(CodexModel::gpt55());
    let out = actor
        .drive(request("echo", json!({ "say": "hello" })))
        .await
        .expect("codex drive ok");
    assert!(out.outputs.is_object(), "codex returned a JSON object");
}
