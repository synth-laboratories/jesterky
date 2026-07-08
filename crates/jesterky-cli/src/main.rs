use async_trait::async_trait;
use clap::{Parser, Subcommand};
use jesterky_actor::{
    viz::{adapt_manifest, render_run_view, render_tree, RenderOpts},
    FakeActor, MemArtifactStore, MemEventSink, ReplayActor, ReplayClock, ReplayResource,
    SystemClock,
};
use jesterky_contract::{
    manifest_schema_json, workflow_schema_json, Artifact, Event, RunManifest, Severity,
    WorkflowSpec,
};
use jesterky_core::{CheckpointStore, Clock, ProgramRegistry, Runner};
use jesterky_model::{CodexModel, ModelActor};
use jesterky_quality::{SCANNER_ACTOR, SUMMARY_ACTOR};
use std::collections::HashMap;
use std::error::Error;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

#[derive(Parser)]
#[command(name = "jesterky")]
#[command(about = "Run and replay jesterky workflow specs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Run {
        spec: PathBuf,
        #[arg(long)]
        args: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        run_id: Option<String>,
        /// Which host actor drives `actor` nodes. `fake` echoes inputs (default,
        /// no network); `codex` calls the real model via `codex exec`.
        #[arg(long, value_enum, default_value_t = ActorKind::Fake)]
        actor: ActorKind,
        /// Model id for `--actor codex` (default `gpt-5.5`; a proxy route id like
        /// `deepseek/deepseek-v4-pro-direct` omits the reasoning-effort flag).
        #[arg(long)]
        model: Option<String>,
        /// Sandboxed `CODEX_HOME` for `--actor codex` (proxy `config.toml` + auth).
        #[arg(long)]
        codex_home: Option<PathBuf>,
        /// Working dir the codex sandbox may read (the repo under audit).
        #[arg(long)]
        cd: Option<PathBuf>,
    },
    Replay {
        manifest: PathBuf,
        #[arg(long)]
        spec: Option<PathBuf>,
    },
    Validate {
        spec: PathBuf,
    },
    /// Render a finished run as a btop-style phase/item panel.
    Visualize {
        manifest: PathBuf,
        /// Spec for the run (adds concurrency to the header).
        #[arg(long)]
        spec: Option<PathBuf>,
        /// Force plain output (also auto-off when not a TTY or NO_COLOR is set).
        #[arg(long)]
        no_color: bool,
        /// Panel width in columns.
        #[arg(long, default_value_t = 76)]
        width: usize,
    },
    Schema {
        artifact: SchemaArtifact,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum SchemaArtifact {
    Workflow,
    Manifest,
}

#[derive(Clone, clap::ValueEnum)]
enum ActorKind {
    /// Echoes inputs as outputs — deterministic, no network (the default).
    Fake,
    /// Drives the real model via `codex exec` (ChatGPT-bundle auth, no API key).
    Codex,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run_cli().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run_cli() -> Result<ExitCode, Box<dyn Error>> {
    match Cli::parse().command {
        Command::Run {
            spec,
            args,
            out,
            run_id,
            actor,
            model,
            codex_home,
            cd,
        } => {
            run_spec(
                &spec,
                args.as_deref(),
                out.as_deref(),
                run_id.as_deref(),
                actor,
                model.as_deref(),
                codex_home.as_deref(),
                cd.as_deref(),
            )
            .await
        }
        Command::Replay { manifest, spec } => replay_manifest(&manifest, spec.as_deref()).await,
        Command::Validate { spec } => validate_spec(&spec),
        Command::Visualize {
            manifest,
            spec,
            no_color,
            width,
        } => visualize(&manifest, spec.as_deref(), no_color, width),
        Command::Schema { artifact } => {
            print_schema(artifact);
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn validate_spec(spec_path: &Path) -> Result<ExitCode, Box<dyn Error>> {
    let spec: WorkflowSpec = read_json(spec_path)?;
    for diagnostic in spec.validate() {
        println!(
            "{} {}: {}",
            severity_label(diagnostic.severity),
            diagnostic.path,
            diagnostic.message
        );
    }

    match spec.validate_and_hash() {
        Ok(spec_hash) => {
            println!("spec_hash {spec_hash}");
            Ok(ExitCode::SUCCESS)
        }
        Err(_) => Ok(ExitCode::FAILURE),
    }
}

fn visualize(
    manifest_path: &Path,
    spec_path: Option<&Path>,
    no_color: bool,
    width: usize,
) -> Result<ExitCode, Box<dyn Error>> {
    let manifest: RunManifest = read_json(manifest_path)?;
    let spec: Option<WorkflowSpec> = match spec_path {
        Some(path) => Some(read_json(path)?),
        None => None,
    };
    let view = adapt_manifest(&manifest, spec.as_ref());
    // Color on only for an interactive TTY, unless forced off or NO_COLOR is set.
    let color = !no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
    print!("{}", render_run_view(&view, &RenderOpts { width, color }));
    Ok(ExitCode::SUCCESS)
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_spec(
    spec_path: &Path,
    args_json: Option<&str>,
    out: Option<&Path>,
    run_id: Option<&str>,
    actor: ActorKind,
    model: Option<&str>,
    codex_home: Option<&Path>,
    cd: Option<&Path>,
) -> Result<ExitCode, Box<dyn Error>> {
    let spec: WorkflowSpec = read_json(spec_path)?;
    let args = parse_args(args_json)?;
    let actor: Arc<dyn jesterky_core::Actor> = match actor {
        ActorKind::Fake => Arc::new(FakeActor),
        // Real model call via codex. The quality-scan roles give the
        // scanner/report actors their system prompts; unknown actors get the
        // generic instruction.
        ActorKind::Codex => {
            let model = model.unwrap_or("gpt-5.5");
            // ChatGPT models take a reasoning effort; proxy routes generally don't.
            let effort = if model.starts_with("gpt") { "high" } else { "" };
            let mut codex = CodexModel::new(model, effort);
            if let Some(home) = codex_home {
                codex = codex.with_codex_home(home);
            }
            if let Some(cd) = cd {
                codex = codex.with_cwd(cd);
            }
            let mut model_actor = ModelActor::new(codex);
            for (name, prompt) in jesterky_quality::roles() {
                model_actor = model_actor.with_role(name, prompt);
            }
            let examples = spec_path.parent().unwrap_or(spec_path);
            model_actor = model_actor
                .with_output_schema(
                    SCANNER_ACTOR,
                    examples.join("quality_verdict.schema.json"),
                )
                .with_output_schema(
                    SUMMARY_ACTOR,
                    examples.join("quality_summary.schema.json"),
                );
            Arc::new(model_actor)
        }
    };
    let runner = runner(
        actor,
        None,
        Arc::new(SystemClock),
        Some(Arc::new(ManifestCheckpointStore::default())),
    );
    let run_id = run_id.unwrap_or("jesterky-cli-run").to_string();
    let manifest = runner.run(&spec, run_id, args).await?;

    print_manifest(&manifest);
    if let Some(out) = out {
        write_json(out, &manifest)?;
        write_json(&spec_sidecar_path(out), &spec)?;
    }

    Ok(ExitCode::SUCCESS)
}

async fn replay_manifest(
    manifest_path: &Path,
    spec_override: Option<&Path>,
) -> Result<ExitCode, Box<dyn Error>> {
    let manifest: RunManifest = read_json(manifest_path)?;
    let spec_path = spec_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| spec_sidecar_path(manifest_path));
    let spec: WorkflowSpec = read_json(&spec_path).map_err(|err| {
        format!(
            "failed to read replay spec sidecar `{}`: {err}",
            spec_path.display()
        )
    })?;
    // Make `spec_hash` load-bearing: the replay spec MUST be the exact topology
    // the manifest was recorded from. Otherwise we'd silently re-drive a
    // different spec and report a confusing event diff instead of "wrong spec".
    let spec_hash = spec.validate_and_hash()?;
    if spec_hash != manifest.spec_hash {
        return Err(format!(
            "replay spec `{}` (spec_hash {spec_hash}) does not match the manifest, \
             which was recorded from spec_hash {} — wrong or stale spec",
            spec_path.display(),
            manifest.spec_hash
        )
        .into());
    }
    // wall_ms is not part of replay fidelity (see fidelity_events), so a plain
    // deterministic clock is all replay needs.
    let replay_clock = Arc::new(ReplayClock::default());
    let checkpoints = Arc::new(ManifestCheckpointStore::from_manifest(&manifest)?);
    let runner = runner(
        Arc::new(ReplayActor::from_manifest(&manifest)),
        Some(Arc::new(ReplayResource::from_manifest(&manifest))),
        replay_clock,
        Some(checkpoints),
    );
    let replayed = runner
        .run(&spec, manifest.run_id.clone(), manifest.args.clone())
        .await?;

    if sorted_events_json(&manifest.events) == sorted_events_json(&replayed.events) {
        println!(
            "replay ok: events={} recorded={}",
            manifest.events.len(),
            manifest.recorded.len()
        );
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("{}", diff_summary(&manifest.events, &replayed.events));
        Ok(ExitCode::FAILURE)
    }
}

fn print_schema(artifact: SchemaArtifact) {
    match artifact {
        SchemaArtifact::Workflow => println!("{}", workflow_schema_json()),
        SchemaArtifact::Manifest => println!("{}", manifest_schema_json()),
    }
}

fn runner(
    actor: Arc<dyn jesterky_core::Actor>,
    resource: Option<Arc<dyn jesterky_core::Resource>>,
    clock: Arc<dyn Clock>,
    checkpoints: Option<Arc<dyn CheckpointStore>>,
) -> Runner {
    Runner {
        programs: demo_programs(),
        actor,
        resource,
        sink: Arc::new(MemEventSink::new()),
        clock,
        store: Arc::new(MemArtifactStore::new()),
        checkpoints,
    }
}

/// The programs available to CLI runs: the real quality-scan workload
/// (`quality.expand` / `quality.aggregate`). A richer CLI would let specs bring
/// their own; today this is the one built-in workload.
fn demo_programs() -> ProgramRegistry {
    jesterky_quality::programs()
}

fn parse_args(args_json: Option<&str>) -> Result<serde_json::Value, serde_json::Error> {
    match args_json {
        Some(raw) => serde_json::from_str(raw),
        None => Ok(serde_json::json!({})),
    }
}

fn print_manifest(manifest: &RunManifest) {
    if let Some(trace) = &manifest.trace {
        print!("{}", render_tree(trace));
    } else {
        println!("trace: <empty>");
    }
    println!(
        "status={} events={} recorded={}",
        format!("{:?}", manifest.status).to_lowercase(),
        manifest.events.len(),
        manifest.recorded.len()
    );
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), Box<dyn Error>> {
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn spec_sidecar_path(manifest_path: &Path) -> PathBuf {
    let mut path = manifest_path.to_path_buf();
    path.set_extension("spec.json");
    path
}

/// Sorted, wall-clock-free projection of an event stream for replay fidelity.
/// Fidelity is over event IDENTITY (`addr`) + kind + payload; `wall_ms` is
/// metadata (ADR #5) that a replay need NOT reproduce. Under a parallel map with
/// real async actors the original emission order is nondeterministic, so a
/// recorded timestamp cannot be re-attached to the same `addr` on replay — so we
/// zero `wall_ms` before comparing.
fn fidelity_events(events: &[Event]) -> Vec<Event> {
    let mut events = events.to_vec();
    for event in &mut events {
        event.wall_ms = 0;
    }
    events.sort_by(|a, b| a.addr.cmp(&b.addr));
    events
}

fn sorted_events_json(events: &[Event]) -> String {
    serde_json::to_string(&fidelity_events(events)).expect("events serialize")
}

fn diff_summary(expected: &[Event], actual: &[Event]) -> String {
    let expected = fidelity_events(expected);
    let actual = fidelity_events(actual);

    let first_mismatch = expected
        .iter()
        .zip(&actual)
        .position(|(left, right)| left != right);
    match first_mismatch {
        Some(idx) => format!(
            "replay mismatch at sorted event {idx}: expected {:?}, got {:?}",
            expected[idx], actual[idx]
        ),
        None if expected.len() != actual.len() => format!(
            "replay mismatch: expected {} events, got {}",
            expected.len(),
            actual.len()
        ),
        None => "replay mismatch".to_string(),
    }
}


#[derive(Default)]
struct ManifestCheckpointStore {
    latest: Mutex<HashMap<String, serde_json::Value>>,
}

impl ManifestCheckpointStore {
    fn from_manifest(manifest: &RunManifest) -> Result<Self, Box<dyn Error>> {
        let mut latest = HashMap::new();
        for checkpoint in &manifest.checkpoints {
            match &checkpoint.state {
                Artifact::Inline(value) => {
                    latest.insert(checkpoint.session.clone(), value.clone());
                }
                Artifact::Ref(reference) => {
                    return Err(format!(
                        "checkpoint `{}` is offloaded to `{}`; replay requires inline checkpoint state",
                        checkpoint.session, reference.key
                    )
                    .into());
                }
            }
        }
        Ok(Self {
            latest: Mutex::new(latest),
        })
    }
}

#[async_trait]
impl CheckpointStore for ManifestCheckpointStore {
    async fn save(
        &self,
        session: &str,
        state: serde_json::Value,
    ) -> Result<jesterky_contract::ArtifactRef, jesterky_core::HostError> {
        self.latest
            .lock()
            .unwrap()
            .insert(session.to_string(), state.clone());
        Ok(jesterky_contract::ArtifactRef {
            key: format!("ckpt/{session}"),
            size_bytes: state.to_string().len() as u64,
            content_type: "application/json".to_string(),
        })
    }

    async fn load(
        &self,
        session: &str,
    ) -> Result<Option<serde_json::Value>, jesterky_core::HostError> {
        Ok(self.latest.lock().unwrap().get(session).cloned())
    }
}
