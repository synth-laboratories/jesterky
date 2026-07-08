use async_trait::async_trait;
use clap::{Parser, Subcommand};
use jesterky_actor::{
    viz::render_tree, FakeActor, MemArtifactStore, MemEventSink, ReplayActor, ReplayResource,
    SystemClock,
};
use jesterky_contract::{
    manifest_schema_json, workflow_schema_json, Artifact, Event, RunManifest, WorkflowSpec,
};
use jesterky_core::{CheckpointStore, Clock, ProgramRegistry, Runner};
use std::collections::{HashMap, VecDeque};
use std::error::Error;
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
    },
    Replay {
        manifest: PathBuf,
        #[arg(long)]
        spec: Option<PathBuf>,
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
        Command::Run { spec, args, out } => run_spec(&spec, args.as_deref(), out.as_deref()).await,
        Command::Replay { manifest, spec } => replay_manifest(&manifest, spec.as_deref()).await,
        Command::Schema { artifact } => {
            print_schema(artifact);
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn run_spec(
    spec_path: &Path,
    args_json: Option<&str>,
    out: Option<&Path>,
) -> Result<ExitCode, Box<dyn Error>> {
    let spec: WorkflowSpec = read_json(spec_path)?;
    let args = parse_args(args_json)?;
    let runner = runner(
        Arc::new(FakeActor),
        None,
        Arc::new(SystemClock),
        Some(Arc::new(ManifestCheckpointStore::default())),
    );
    let manifest = runner
        .run(&spec, "jesterky-cli-run".to_string(), args)
        .await?;

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
    let replay_clock = Arc::new(ManifestClock::from_manifest(&manifest));
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

fn demo_programs() -> ProgramRegistry {
    let mut programs = ProgramRegistry::new();
    programs.register(
        "quality.expand",
        Arc::new(|_, _| {
            Ok(serde_json::json!({
                "jobs": [
                    { "id": 0, "target": "alpha" },
                    { "id": 1, "target": "beta" },
                    { "id": 2, "target": "gamma" }
                ]
            }))
        }),
    );
    programs.register(
        "quality.aggregate",
        Arc::new(|_, inputs| {
            let scans = inputs
                .get("scans")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            Ok(serde_json::json!({
                "summary": {
                    "count": scans.len(),
                    "first": scans.first().cloned()
                }
            }))
        }),
    );
    programs
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

fn sorted_events_json(events: &[Event]) -> String {
    let mut events = events.to_vec();
    events.sort_by(|a, b| a.addr.cmp(&b.addr));
    serde_json::to_string(&events).expect("events serialize")
}

fn diff_summary(expected: &[Event], actual: &[Event]) -> String {
    let mut expected = expected.to_vec();
    let mut actual = actual.to_vec();
    expected.sort_by(|a, b| a.addr.cmp(&b.addr));
    actual.sort_by(|a, b| a.addr.cmp(&b.addr));

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

struct ManifestClock {
    wall_ms: Mutex<VecDeque<u64>>,
}

impl ManifestClock {
    fn from_manifest(manifest: &RunManifest) -> Self {
        Self {
            wall_ms: Mutex::new(manifest.events.iter().map(|event| event.wall_ms).collect()),
        }
    }
}

impl Clock for ManifestClock {
    fn now_ms(&self) -> u64 {
        self.wall_ms.lock().unwrap().pop_front().unwrap_or(0)
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
