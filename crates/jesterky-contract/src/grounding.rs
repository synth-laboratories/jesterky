//! **Annotation grounding** — evidence quality for a run's outputs, kept
//! deliberately SEPARATE from workflow lifecycle.
//!
//! `RunStatus::Completed` says the graph finished; it says nothing about
//! whether an annotation actually read the trace it claims to describe. Reusing
//! `completed` for both produced runs that reported "completed 8/8" while the
//! rows mixed genuinely trace-grounded annotations with summary-only,
//! uninspected, and trace-access-failure rows. This module gives each output a
//! typed [`GroundingVerdict`] and the run a [`GroundingReport`] projection, so
//! a workflow may complete while some outputs carry non-grounded verdicts —
//! and a trace-REQUIRED workflow degrades to a typed
//! [`GroundingStatus::RequiredTraceUnread`] independently of terminality.
//!
//! # Semantics (important)
//!
//! | Surface | Meaning |
//! |---|---|
//! | [`Grounding`] | Per-output record on [`crate::artifact::RecordedOutput`]. Missing on legacy rows ⇒ `Ungraded`, NEVER `Grounded`. |
//! | [`GroundingVerdict`] | Evidence quality of one graded output. |
//! | [`GroundingVerdictRecord::trace_body_read`] | TRUE only when the annotator read the trace BODY, not a summary/count projection. |
//! | [`GroundingReport`] | Pure projection over the manifest's recorded rows; on [`crate::artifact::RunManifest::grounding`]. Does not change `status`. |
//! | [`GroundingPlan::trace_required`] | Declares that every output must be graded from a read trace body (`runplan.grounding`). |
//!
//! Shape aligned with [`crate::budget`] / [`crate::goal`]: a plan on
//! [`crate::topology::RunPlan`], a versioned snapshot on the manifest, and a
//! pure compute function. Grading itself is host/annotator work — the core
//! never judges actor quality (house rule); it only projects the verdicts it
//! was handed, and fails closed when a required one is absent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::artifact::{ArtifactRef, RunManifest};

/// Version stamped into every [`GroundingVerdictRecord`] and
/// [`GroundingReport`] (mirrors [`crate::goal::GOAL_ENGINE_VERSION`]).
pub const GROUNDING_SCHEMA_VERSION: &str = "grounding.v1";

// ──────────────────────────── per-output verdict ───────────────────────────

/// Evidence quality of one graded output. Closed set; adding a member is a
/// contract change (mirrors the [`crate::budget::BudgetKind`] discipline).
///
/// Note there is deliberately no `Ungraded` member here: "nobody graded this
/// row" is the [`Grounding::Ungraded`] state, not a verdict an annotator may
/// assert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingVerdict {
    /// The annotation was produced from the READ trace body and cites it.
    Grounded,
    /// The annotation was produced from a summary/count projection only.
    SummaryOnly,
    /// A source exists and is readable, but the annotator never opened it.
    SourceUnread,
    /// The source body exists but could not be parsed/decoded.
    SourceUnreadable,
    /// Fetching the trace failed (GC'd snapshot, auth, transport).
    TraceAccessFailed,
}

/// Reviewer state of a graded verdict (human/second-pass audit trail).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    #[default]
    Unreviewed,
    Reviewed,
    Disputed,
}

/// Immutable identity of source material a verdict was graded against.
/// Internally tagged (`"source": "..."`) so refs stay flat JSON objects.
#[derive(Clone, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum GroundingSourceRef {
    /// A jesterky/SMR run identity.
    Run { run_id: String },
    /// An immutable artifact in the host store.
    Artifact { artifact: ArtifactRef },
    /// A foreign system's record (e.g. a GameBench capture): system + stable id.
    External { system: String, id: String },
}

/// A cited half-open byte range `[start, end)` inside a named source body.
#[derive(Clone, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
pub struct CitedSpan {
    /// Which source body the span indexes (an [`ArtifactRef::key`], event
    /// stream name, or foreign trace id).
    pub source: String,
    pub start: u64,
    pub end: u64,
}

/// One graded output's full grounding evidence.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
pub struct GroundingVerdictRecord {
    pub verdict: GroundingVerdict,
    /// TRUE only when the annotator read the trace BODY. A `grounded` verdict
    /// with `trace_body_read: false` is contradictory and is counted as unread
    /// by [`GroundingReport::compute`].
    pub trace_body_read: bool,
    /// Event ids in the source material the annotation cites.
    #[serde(default)]
    pub cited_event_ids: Vec<String>,
    /// Byte spans in the source material the annotation cites.
    #[serde(default)]
    pub cited_spans: Vec<CitedSpan>,
    /// Annotator identity — a model route (e.g. `openai/gpt-5.5`) or a human
    /// handle.
    pub annotator: String,
    /// [`GROUNDING_SCHEMA_VERSION`] at grading time.
    pub schema_version: String,
    /// Annotator confidence in `[0, 1]`.
    pub confidence: f64,
    #[serde(default)]
    pub review_state: ReviewState,
    /// Immutable source refs (run/artifact identity). Empty when the corpus
    /// carried no identity — which itself is evidence the row cannot be
    /// re-verified.
    #[serde(default)]
    pub sources: Vec<GroundingSourceRef>,
}

/// Per-output grounding state. Internally tagged (`"state": "..."`).
///
/// The `Default` — and therefore the serde default for legacy rows that
/// predate this field — is `Ungraded`: absent grounding data NEVER reads as
/// grounded.
#[derive(Clone, Debug, Default, PartialEq, JsonSchema, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Grounding {
    /// No grounding audit graded this output (the explicit legacy reading).
    #[default]
    Ungraded,
    /// A grounding audit graded this output; the record carries the evidence.
    Graded(GroundingVerdictRecord),
}

impl Grounding {
    /// Whether the trace body behind this output was actually read.
    /// `Ungraded` is unread by definition (fail closed).
    pub fn trace_body_read(&self) -> bool {
        match self {
            Grounding::Ungraded => false,
            Grounding::Graded(record) => record.trace_body_read,
        }
    }
}

// ─────────────────────────────── plan & report ─────────────────────────────

/// Declared grounding requirements for a run (`runplan.grounding`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
pub struct GroundingPlan {
    /// When TRUE, every recorded output must be graded from a READ trace body;
    /// any ungraded or unread row degrades the run's [`GroundingReport`] to
    /// [`GroundingStatus::RequiredTraceUnread`] — independently of
    /// [`crate::artifact::RunStatus`].
    #[serde(default)]
    pub trace_required: bool,
}

impl GroundingPlan {
    /// True when nothing was declared. Used to keep an undeclared plan out of
    /// the serialized spec, so existing workflows' `spec_hash` (and therefore
    /// replay identity) is unchanged by this contract addition.
    pub fn is_empty(&self) -> bool {
        !self.trace_required
    }
}

/// Run-level grounding status. SEPARATE from — and never derived from —
/// [`crate::artifact::RunStatus`]: a `Completed` run may carry any of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingStatus {
    /// Every recorded output is graded `grounded` from a read trace body.
    Grounded,
    /// At least one graded output carries a non-grounded verdict (or a row is
    /// still ungraded) in a workflow that does not require traces.
    Degraded,
    /// The typed failure for trace-REQUIRED workflows: at least one output's
    /// trace body was never read (ungraded, summary-only, unread, unreadable,
    /// or access-failed).
    RequiredTraceUnread,
    /// No outputs are graded (legacy manifests, or grading has not run).
    Ungraded,
}

/// Per-verdict row counts over a manifest's recorded outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
pub struct GroundingTally {
    pub grounded: u64,
    pub summary_only: u64,
    pub source_unread: u64,
    pub source_unreadable: u64,
    pub trace_access_failed: u64,
    pub ungraded: u64,
}

impl GroundingTally {
    pub fn total(&self) -> u64 {
        self.grounded
            + self.summary_only
            + self.source_unread
            + self.source_unreadable
            + self.trace_access_failed
            + self.ungraded
    }
}

/// The run-level grounding projection, attached to
/// [`crate::artifact::RunManifest::grounding`]. A PURE function of the
/// recorded rows + the declared plan; computing it never changes the run's
/// terminal status.
#[derive(Clone, Debug, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
pub struct GroundingReport {
    /// [`GROUNDING_SCHEMA_VERSION`] at compute time.
    pub schema_version: String,
    pub status: GroundingStatus,
    /// Echo of [`GroundingPlan::trace_required`] the report was computed under.
    pub trace_required: bool,
    pub tally: GroundingTally,
}

impl GroundingReport {
    /// Project the grounding status from a manifest's recorded rows. Fail
    /// closed: an ungraded row — and a `grounded` claim whose
    /// `trace_body_read` is false — counts as unread.
    pub fn compute(manifest: &RunManifest, plan: &GroundingPlan) -> GroundingReport {
        let mut tally = GroundingTally::default();
        let mut unread = 0u64;
        for output in &manifest.recorded {
            match &output.grounding {
                Grounding::Ungraded => tally.ungraded += 1,
                Grounding::Graded(record) => match record.verdict {
                    GroundingVerdict::Grounded => tally.grounded += 1,
                    GroundingVerdict::SummaryOnly => tally.summary_only += 1,
                    GroundingVerdict::SourceUnread => tally.source_unread += 1,
                    GroundingVerdict::SourceUnreadable => tally.source_unreadable += 1,
                    GroundingVerdict::TraceAccessFailed => tally.trace_access_failed += 1,
                },
            }
            if !output.grounding.trace_body_read() {
                unread += 1;
            }
        }
        let total = tally.total();
        let status = if plan.trace_required && unread > 0 {
            GroundingStatus::RequiredTraceUnread
        } else if total == 0 || tally.ungraded == total {
            GroundingStatus::Ungraded
        } else if tally.grounded == total && unread == 0 {
            GroundingStatus::Grounded
        } else {
            GroundingStatus::Degraded
        };
        GroundingReport {
            schema_version: GROUNDING_SCHEMA_VERSION.into(),
            status,
            trace_required: plan.trace_required,
            tally,
        }
    }
}
