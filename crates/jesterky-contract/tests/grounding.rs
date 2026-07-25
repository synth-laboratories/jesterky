//! Grounding contract: verdict serde round-trips exactly, legacy rows read as
//! `ungraded` (never grounded), and run completion is independent of the
//! grounding projection.

use jesterky_contract::{
    Addr, CallKind, CitedSpan, Grounding, GroundingPlan, GroundingReport, GroundingSourceRef,
    GroundingStatus, GroundingVerdict, GroundingVerdictRecord, NodePath, RecordedOutput,
    ReviewState, RunManifest, RunStatus, RunStopReason, GROUNDING_SCHEMA_VERSION,
};
use serde_json::json;

fn addr(seq: u32) -> Addr {
    Addr {
        run_id: "grounding-run".to_string(),
        node_path: NodePath::root().child("annotate"),
        iteration: 0,
        local_seq: seq,
    }
}

fn graded(verdict: GroundingVerdict, trace_body_read: bool) -> Grounding {
    Grounding::Graded(GroundingVerdictRecord {
        verdict,
        trace_body_read,
        cited_event_ids: vec!["evt-craftax-0017".to_string()],
        cited_spans: vec![CitedSpan {
            source: "blob/ab12cd".to_string(),
            start: 128,
            end: 512,
        }],
        annotator: "openai/gpt-5.5".to_string(),
        schema_version: GROUNDING_SCHEMA_VERSION.to_string(),
        confidence: 0.9,
        review_state: ReviewState::Unreviewed,
        sources: vec![GroundingSourceRef::Run {
            run_id: "smr-run-a4948abe".to_string(),
        }],
    })
}

fn output(seq: u32, grounding: Grounding) -> RecordedOutput {
    RecordedOutput {
        addr: addr(seq),
        call: CallKind::Actor {
            actor: "trace_annotator".to_string(),
        },
        outputs: json!({ "annotation": "agent looped on wood collection" }),
        score: None,
        signal: None,
        artifacts: vec![],
        grounding,
    }
}

fn manifest(recorded: Vec<RecordedOutput>) -> RunManifest {
    RunManifest {
        run_id: "grounding-run".to_string(),
        workflow_name: "trace_annotate".to_string(),
        spec_hash: "0".repeat(64),
        args: json!({}),
        events: vec![],
        recorded,
        checkpoints: vec![],
        trace: None,
        status: RunStatus::Completed,
        stop_reason: RunStopReason::Completed,
        budgets: None,
        goals: None,
        invariants: None,
        grounding: None,
    }
}

#[test]
fn graded_verdict_round_trips_exactly() {
    let row = output(0, graded(GroundingVerdict::Grounded, true));
    let json = serde_json::to_value(&row).expect("serializes");
    // The graded record is internally tagged and carries every evidence field.
    assert_eq!(json["grounding"]["state"], "graded");
    assert_eq!(json["grounding"]["verdict"], "grounded");
    assert_eq!(json["grounding"]["trace_body_read"], true);
    assert_eq!(json["grounding"]["sources"][0]["source"], "run");
    let back: RecordedOutput = serde_json::from_value(json).expect("deserializes");
    assert_eq!(back, row);

    // Every verdict member survives the round trip unchanged.
    for verdict in [
        GroundingVerdict::Grounded,
        GroundingVerdict::SummaryOnly,
        GroundingVerdict::SourceUnread,
        GroundingVerdict::SourceUnreadable,
        GroundingVerdict::TraceAccessFailed,
    ] {
        let grounding = graded(verdict, verdict == GroundingVerdict::Grounded);
        let json = serde_json::to_value(&grounding).expect("serializes");
        let back: Grounding = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, grounding);
    }
}

#[test]
fn legacy_row_without_grounding_reads_as_ungraded_not_grounded() {
    // A pre-0.1.2 manifest row has no `grounding` key at all. It must
    // deserialize to the explicit `Ungraded` state — never to a grounded one.
    let legacy = json!({
        "addr": {
            "run_id": "grounding-run",
            "node_path": [{ "node": "annotate" }],
            "iteration": 0,
            "local_seq": 0
        },
        "call": { "call": "actor", "actor": "trace_annotator" },
        "outputs": { "annotation": "from a summary projection" },
        "score": null,
        "signal": null,
        "artifacts": []
    });
    let row: RecordedOutput = serde_json::from_value(legacy).expect("legacy row deserializes");
    assert_eq!(row.grounding, Grounding::Ungraded);
    assert!(
        !row.grounding.trace_body_read(),
        "ungraded is unread, fail closed"
    );

    // And the run-level projection over legacy rows says so explicitly.
    let report = GroundingReport::compute(&manifest(vec![row]), &GroundingPlan::default());
    assert_eq!(report.status, GroundingStatus::Ungraded);
    assert_eq!(report.tally.ungraded, 1);
    assert_eq!(report.tally.grounded, 0);
}

#[test]
fn completion_is_independent_of_grounding() {
    // The observed defect: "completed 8/8" over a mix of grounded,
    // summary-only, uninspected, unreadable, and trace-access-failure rows.
    // The run stays Completed; the grounding projection must NOT.
    let manifest = manifest(vec![
        output(0, graded(GroundingVerdict::Grounded, true)),
        output(1, graded(GroundingVerdict::SummaryOnly, false)),
        output(2, graded(GroundingVerdict::SourceUnread, false)),
        output(3, graded(GroundingVerdict::SourceUnreadable, false)),
        output(4, graded(GroundingVerdict::TraceAccessFailed, false)),
        output(5, Grounding::Ungraded),
    ]);
    assert_eq!(manifest.status, RunStatus::Completed);

    // Trace not required: the run degrades but does not carry the typed
    // required-trace failure.
    let relaxed = GroundingReport::compute(&manifest, &GroundingPlan::default());
    assert_eq!(relaxed.status, GroundingStatus::Degraded);
    assert_eq!(relaxed.tally.total(), 6);
    assert_eq!(relaxed.tally.grounded, 1);
    assert_eq!(relaxed.tally.ungraded, 1);

    // Trace REQUIRED: unread rows yield the typed failure status while the
    // manifest's terminal status is untouched.
    let required = GroundingReport::compute(
        &manifest,
        &GroundingPlan {
            trace_required: true,
        },
    );
    assert_eq!(required.status, GroundingStatus::RequiredTraceUnread);
    assert_eq!(manifest.status, RunStatus::Completed);

    // All rows grounded from read bodies: the required plan is satisfied.
    let clean = self::manifest(vec![
        output(0, graded(GroundingVerdict::Grounded, true)),
        output(1, graded(GroundingVerdict::Grounded, true)),
    ]);
    let report = GroundingReport::compute(
        &clean,
        &GroundingPlan {
            trace_required: true,
        },
    );
    assert_eq!(report.status, GroundingStatus::Grounded);

    // A contradictory row — claims `grounded` without reading the body — is
    // counted as unread and fails a trace-required plan.
    let contradictory = self::manifest(vec![output(0, graded(GroundingVerdict::Grounded, false))]);
    let report = GroundingReport::compute(
        &contradictory,
        &GroundingPlan {
            trace_required: true,
        },
    );
    assert_eq!(report.status, GroundingStatus::RequiredTraceUnread);
}
