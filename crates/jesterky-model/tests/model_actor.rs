//! `ModelActor` logic tested against a stub model — no network, deterministic.
//! The one real codex round-trip is `#[ignore]`d (needs the CLI + auth + net).

use jesterky_contract::{Addr, NodePath};
use jesterky_core::{Actor, ActorRequest};
use jesterky_model::{
    build_prompt, extract_json, CodexModel, ModelActor, ModelError, ModelRequest, StubModel,
};
use serde_json::json;

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
    let actor = ModelActor::new(StubModel::replying(r#"{"verdict":"pass","confidence":0.9}"#));
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
    let out = actor.drive(request("x", json!({}))).await.expect("drive ok");
    assert_eq!(out.outputs["ok"], json!(true));
}

#[tokio::test]
async fn non_json_reply_becomes_a_classified_actor_error() {
    let actor = ModelActor::new(StubModel::replying("I could not complete that."));
    let err = actor.drive(request("x", json!({}))).await.unwrap_err();
    assert!(err.to_string().contains("JSON"), "got: {err}");
}

#[tokio::test]
async fn model_error_class_survives_into_the_host_error() {
    let actor = ModelActor::new(StubModel::new(|_: &ModelRequest| -> Result<String, ModelError> {
        Err(ModelError::Quota("usage limit reached".to_string()))
    }));
    let err = actor.drive(request("scanner", json!({}))).await.unwrap_err();
    assert!(err.to_string().contains("quota"), "got: {err}");
}

#[test]
fn prompt_carries_role_system_and_inputs() {
    let prompt = build_prompt(&ModelRequest {
        actor: "auditor".to_string(),
        system: Some("Be terse.".to_string()),
        inputs: json!({ "n": 1 }),
        output_schema: None,
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
