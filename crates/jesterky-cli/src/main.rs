use async_trait::async_trait;
use clap::{Parser, Subcommand};
use jesterky_actor::{
    viz::{
        adapt_manifest, redraw_lines, render_run_view, render_run_view_lines, render_tree,
        RenderOpts,
    },
    FakeActor, MemArtifactStore, MemEventSink, NdjsonEventSink, ReplayActor, ReplayClock,
    ReplayResource, SharedEventSink, SystemClock, TeeEventSink,
};
use jesterky_contract::{
    manifest_schema_json, workflow_schema_json, Artifact, BudgetEngine, BudgetKind,
    BudgetObservation, BudgetPlan, BudgetSnapshot, CallKind, ContractError, Event, EventKind,
    GoalSnapshot, GoalState, HostConfig, HostRole, LiveBus, LiveStream, NodePath, RunManifest,
    RunStatus, RunStopReason, Severity, ShardProgress, WorkflowSpec,
};
use jesterky_core::ledger::Ledger;
use jesterky_core::{CheckpointStore, Clock, ProgramRegistry, Runner};
use jesterky_model::{AdaptiveLimiter, CodexModel, ModelActor};
use jesterky_quality::{
    host_config as workload_host_config, programs, programs_with_dungeon, DungeonGridActor,
    DungeonGridState,
};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
        /// Read the args JSON from a file instead of the command line. Use this
        /// for large seeded ledgers (`ledger.jobs`) that would blow past the
        /// shell's `ARG_MAX`. Mutually exclusive with `--args`.
        #[arg(long, conflicts_with = "args")]
        args_file: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Write canonical run events as NDJSON. Use `-` for stdout.
        #[arg(long)]
        events_out: Option<PathBuf>,
        /// Export the finished native manifest/events to a Containers Trace V5 bundle.
        #[arg(long, requires = "out")]
        trace_out: Option<PathBuf>,
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
        /// Reasoning effort for native GPT routes. Defaults depend on the workload.
        #[arg(long, value_enum)]
        effort: Option<ReasoningEffort>,
        /// Sandboxed `CODEX_HOME` for `--actor codex` (proxy `config.toml` + auth).
        #[arg(long)]
        codex_home: Option<PathBuf>,
        /// Working dir the codex sandbox may read (the repo under audit).
        #[arg(long)]
        cd: Option<PathBuf>,
        /// Disable the live btop panel (default is on for interactive TTYs).
        #[arg(long)]
        no_follow: bool,
        /// Force the live panel even when stdout is not a TTY.
        #[arg(long)]
        follow: bool,
        /// Redraw interval for the live panel in seconds.
        #[arg(long, default_value_t = 0.2)]
        viz_interval: f64,
        /// Force plain output (also auto-off when not a TTY or NO_COLOR is set).
        #[arg(long)]
        no_color: bool,
        /// Panel width in columns for `--follow`.
        #[arg(long, default_value_t = 76)]
        width: usize,
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
    /// Export an existing native manifest through the generic synth-trace importer.
    TraceExport {
        manifest: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        atif: bool,
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

#[derive(Clone, clap::ValueEnum)]
enum ReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
}

impl ReasoningEffort {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run_cli().await {
        Ok(code) => code,
        Err(err) => {
            print_error_report(err.as_ref());
            ExitCode::FAILURE
        }
    }
}

/// A LOUD, unmissable failure report on stderr: a bordered header, the primary
/// message, the full `source()` cause chain (so a wrapped error shows every
/// layer, not just the outermost line), and a class hint that says what to *do*.
/// Color only when stderr is a terminal. This is the difference between "it
/// failed" and knowing why — a run error must never be a single terse line that
/// scrolls past or gets smeared over the live panel.
fn print_error_report(err: &(dyn Error + 'static)) {
    let tty = std::io::stderr().is_terminal();
    let (red, bold, dim, reset) = if tty {
        ("\x1b[31m", "\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "", "")
    };
    let mut stderr = io::stderr();
    let _ = writeln!(stderr);
    let _ = writeln!(stderr, "{red}{bold}━━━ jesterky run failed ━━━{reset}");
    let _ = writeln!(stderr, "{red}{bold}✗{reset} {bold}{err}{reset}");
    // Walk the cause chain: each wrapped layer on its own `caused by` line.
    let mut source = err.source();
    while let Some(cause) = source {
        let _ = writeln!(stderr, "  {dim}caused by:{reset} {cause}");
        source = cause.source();
    }
    if let Some(hint) = failure_hint(err) {
        let _ = writeln!(stderr, "{bold}hint:{reset} {hint}");
    }
    let _ = writeln!(stderr);
}

/// Map a failure to an actionable hint, keyed off the message so it covers errors
/// from any layer (core, ledger, host). Generic — no workload specifics.
fn failure_hint(err: &(dyn Error + 'static)) -> Option<String> {
    // Search the whole chain's text so a wrapped cause still triggers the hint.
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(" | ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    let lower = text.to_lowercase();
    if lower.contains("unresolved reference") {
        // e.g. `unresolved reference: ledger.docs_json`
        let key = text
            .rsplit("unresolved reference:")
            .next()
            .map(|s| s.split(['|', '(']).next().unwrap_or(s).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "that reference".to_string());
        return Some(format!(
            "a node input references `{key}`, but nothing seeded or produced it. \
             Seed it with `--args '{{\"<key>\": ...}}'` (drop the `ledger.` prefix), \
             or remove the binding from the spec if the program can default it."
        ));
    }
    if lower.contains("min_success gate failed") {
        return Some(
            "map shards failed the success gate. Re-run with `--out <file>` and inspect \
             the manifest, or look at the ✗ rows in `--follow` for each shard's reason."
                .to_string(),
        );
    }
    if lower.contains("program not registered") || lower.contains("unknown node") {
        return Some(
            "the spec references an op/node the host doesn't provide. Check the spec's \
             `op`/`entrypoint` names against the registered programs."
                .to_string(),
        );
    }
    if lower.contains("auth") || lower.contains("unauthorized") || lower.contains("401") {
        return Some(
            "model auth failed. codex uses the ChatGPT bundle (`~/.codex/auth.json`) or the \
             proxy config under `--codex-home` — check that, and any `SYNTH_API_KEY` the route needs."
                .to_string(),
        );
    }
    if lower.contains("quota") || lower.contains("usage limit") || lower.contains("429") {
        return Some(
            "the model route is rate-limited or out of quota. Wait and retry, or switch route."
                .to_string(),
        );
    }
    None
}

async fn run_cli() -> Result<ExitCode, Box<dyn Error>> {
    match Cli::parse().command {
        Command::Run {
            spec,
            args,
            args_file,
            out,
            events_out,
            trace_out,
            run_id,
            actor,
            model,
            effort,
            codex_home,
            cd,
            no_follow,
            follow,
            viz_interval,
            no_color,
            width,
        } => {
            // `--args-file` reads the args JSON from disk (large seeded ledgers
            // that would exceed the shell's ARG_MAX). clap already enforces it is
            // exclusive with `--args`.
            let args = match args_file {
                Some(path) => Some(std::fs::read_to_string(&path).map_err(|e| {
                    format!("failed to read --args-file `{}`: {e}", path.display())
                })?),
                None => args,
            };
            run_spec(
                &spec,
                args.as_deref(),
                out.as_deref(),
                events_out.as_deref(),
                trace_out.as_deref(),
                run_id.as_deref(),
                actor,
                model.as_deref(),
                effort,
                codex_home.as_deref(),
                cd.as_deref(),
                no_follow,
                follow,
                viz_interval,
                no_color,
                width,
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
        Command::TraceExport {
            manifest,
            bundle,
            atif,
        } => export_trace_v5(&manifest, &bundle, atif),
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
    let view = adapt_manifest(&manifest, spec.as_ref(), None, None);
    // Color on only for an interactive TTY, unless forced off or NO_COLOR is set.
    let color =
        !no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
    print!(
        "{}",
        render_run_view(
            &view,
            &RenderOpts {
                width,
                color,
                tick: 0,
                elapsed_secs: None
            }
        )
    );
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
    events_out: Option<&Path>,
    trace_out: Option<&Path>,
    run_id: Option<&str>,
    actor: ActorKind,
    model: Option<&str>,
    effort: Option<ReasoningEffort>,
    codex_home: Option<&Path>,
    cd: Option<&Path>,
    no_follow: bool,
    follow_flag: bool,
    viz_interval: f64,
    no_color: bool,
    width: usize,
) -> Result<ExitCode, Box<dyn Error>> {
    let follow = should_follow(no_follow, follow_flag);
    let spec: WorkflowSpec = read_json(spec_path)?;
    let args = parse_args(args_json)?;
    let is_dungeongrid = spec.name.starts_with("dungeongrid");
    // DungeonGrid is LLM-only: hero turns must go through a real model policy.
    if is_dungeongrid && matches!(actor, ActorKind::Fake) {
        return Err(
            "dungeongrid is an LLM workflow — use `--actor codex` (no scripted/fake policy). \
             Example: jesterky run examples/dungeongrid_4p.json --actor codex \
             --model deepseek/deepseek-v4-pro-direct --codex-home /tmp/jesterky_codex_home \
             --args '{\"max_turns\":12}' --follow"
                .into(),
        );
    }
    // Shared env for DungeonGrid programs + hero actor (no-op for other workloads).
    let dungeon_state = DungeonGridState::new();
    let program_registry = programs_with_dungeon(dungeon_state.clone());
    // Host-side live-progress stream: the codex model *publishes* per-shard
    // tokens / steps / latest action, the follow thread owns the consumer end and
    // folds it each frame. When not following we drop the consumer, so publishes
    // become no-ops (nothing buffers). Harmless for the fake actor (nothing sends).
    let (live, live_stream) = LiveBus::channel();
    // A chat-proxy route (deepseek/*, gemini/*, …) spawns the native jesterky
    // Responses↔chat shim; the guard must outlive the whole run (it aborts the
    // localhost server on drop), so it lives in this scope, not the match arm.
    let mut _proxy_guard: Option<jesterky_proxy::ChatProxy> = None;
    let actor: Arc<dyn jesterky_core::Actor> = match actor {
        ActorKind::Fake => Arc::new(FakeActor),
        // Real model call via codex. The quality-scan roles give the
        // scanner/report actors their system prompts; unknown actors get the
        // generic instruction. DungeonGrid wraps the model as a *policy* and
        // still steps the in-process env (observe → LLM action → step).
        ActorKind::Codex => {
            let model = model.unwrap_or("gpt-5.5");
            assert_model_allowed(model)?;
            preflight_codex_home_route(model, codex_home).await?;
            // ChatGPT models take a reasoning effort; proxy routes generally don't.
            // DungeonGrid turns are short JSON — keep effort low for gpt routes.
            let effort = match (effort, model.starts_with("gpt")) {
                (Some(value), true) => value.as_str(),
                (Some(_), false) => {
                    return Err("--effort is only supported for native GPT routes".into());
                }
                (None, true) if is_dungeongrid => "low",
                (None, true) => "high",
                (None, false) => "",
            };
            // AIMD concurrency gate for this model+provider: start at the map's
            // configured width, drop it on 429s, climb back on clean calls. The
            // map barrier caps concurrency at the width, so that is also the max.
            let ceiling = spec.runplan.map_concurrency.unwrap_or(4).max(1) as usize;
            let limiter = AdaptiveLimiter::new(ceiling, 1, ceiling);
            let mut codex = CodexModel::new(model, effort).with_limiter(limiter);
            if let Some(home) = codex_home {
                // An explicit --codex-home always wins (power users / custom proxies).
                codex = codex.with_codex_home(home);
            } else if let Some(proxy) = jesterky_proxy::ChatProxy::spawn(model).await? {
                // A chat-only route: jesterky spawns its own Responses↔chat proxy
                // (localhost) pointed at the provider's real endpoint, and points
                // codex at it. gpt-* routes resolve to None here (native codex).
                eprintln!(
                    "codex: `{model}` → jesterky Responses↔chat proxy on 127.0.0.1:{} → provider",
                    proxy.port()
                );
                codex = codex.with_codex_home(proxy.codex_home().to_path_buf());
                _proxy_guard = Some(proxy);
            }
            if let Some(cd) = cd {
                codex = codex.with_cwd(cd);
            }
            let mut model_actor = ModelActor::new(codex).with_live(live.clone());
            let spec_dir = spec_path.parent().unwrap_or(spec_path);
            if let Some(host) = resolve_host_config(&spec) {
                model_actor = apply_host_config(model_actor, &host, spec_dir)?;
            }
            if is_dungeongrid {
                Arc::new(DungeonGridActor::with_policy(
                    dungeon_state.clone(),
                    Arc::new(model_actor),
                ))
            } else {
                Arc::new(model_actor)
            }
        }
    };
    let shared_sink = if follow {
        Some(Arc::new(SharedEventSink::new()))
    } else {
        None
    };
    let sink = run_event_sink(shared_sink.clone(), events_out)?;
    let mut run_runner = runner(
        actor,
        None,
        Arc::new(SystemClock),
        Some(Arc::new(ManifestCheckpointStore::default())),
        Some(sink),
    );
    // Pair DungeonGrid programs with the same env state the hero actor holds.
    run_runner.programs = program_registry.clone();
    let run_id = run_id.unwrap_or("jesterky-cli-run").to_string();
    let item_labels = live_item_labels(&spec, &args, &program_registry);
    let render_opts = RenderOpts {
        width,
        color: follow_color_enabled(follow, no_color),
        tick: 0,
        elapsed_secs: None,
    };
    let follow_stop = Arc::new(AtomicBool::new(false));
    // The follow thread OWNS the stream consumer and folds it each frame; on join
    // it returns its final folded per-shard state for the settled frame below.
    // Not following: drop the consumer so the model's publishes no-op.
    let budget_plan = resolve_budget_plan(&spec, &args)?;
    let follow_thread: Option<std::thread::JoinHandle<HashMap<NodePath, ShardProgress>>> = if follow
    {
        let sink = shared_sink.expect("follow enabled implies shared sink");
        let spec_for_follow = spec.clone();
        let model_for_follow = model.map(str::to_string);
        let interval = Duration::from_secs_f64(viz_interval.max(0.05));
        let stop = follow_stop.clone();
        let labels = item_labels.clone();
        let opts = render_opts;
        let budget_plan_follow = budget_plan.clone();
        let run_id_follow = run_id.clone();
        Some(std::thread::spawn(move || {
            follow_viz_loop(
                sink,
                &spec_for_follow,
                labels.as_deref(),
                model_for_follow.as_deref(),
                live_stream,
                stop,
                interval,
                opts,
                budget_plan_follow,
                run_id_follow,
            )
        }))
    } else {
        drop(live_stream);
        None
    };

    // Wall-clock start so the settled frame can report final throughput (tps/tpm).
    let run_started = std::time::Instant::now();
    // Capture the result WITHOUT `?`: a run error must still stop + join the
    // follow thread (which restores the cursor and stops redrawing) before we
    // propagate. Bailing with `?` here would leave the thread drawing over the
    // error message and the terminal cursor hidden — the corrupted-frame bug.
    let result = run_runner.run(&spec, run_id.clone(), args).await;

    follow_stop.store(true, Ordering::Relaxed);
    let final_progress = follow_thread
        .map(|thread| thread.join().expect("follow thread joins"))
        .unwrap_or_default();

    let mut manifest = result?;
    let wall_secs = run_started.elapsed().as_secs_f64();
    // Attach formal budget projection (progress + ETA) when caps were declared.
    if let Some(snap) = project_budgets(
        &run_id,
        &budget_plan,
        &manifest,
        Some(&final_progress),
        wall_secs,
        &[],
    ) {
        // Hard budget exhaust → fail the run when configured.
        if budget_plan.fail_on_hard_exhaust
            && snap.state == jesterky_contract::BudgetState::Exhausted
            && snap
                .items
                .iter()
                .any(|i| i.hard && i.state == jesterky_contract::BudgetState::Exhausted)
        {
            manifest.status = RunStatus::Failed;
            manifest.stop_reason = RunStopReason::BudgetExhausted;
        }
        manifest.budgets = Some(snap);
    }

    if follow {
        let mut view = adapt_manifest(&manifest, Some(&spec), None, Some(&final_progress));
        if let Some(model) = model {
            view.model = Some(model.to_string());
        }
        let final_opts = RenderOpts {
            elapsed_secs: Some(wall_secs),
            ..render_opts
        };
        let mut stdout = io::stdout();
        let lines = render_run_view_lines(&view, &final_opts);
        let mut prev = 0usize;
        redraw_lines(&mut prev, &lines, &mut stdout)?;
        if render_opts.color {
            let _ = write!(stdout, "\x1b[?25h");
            stdout.flush()?;
        }
        print_status_line(&manifest);
        print_usage_line(model, &final_progress, wall_secs);
        if let Some(budgets) = &manifest.budgets {
            println!(
                "budgets={}",
                serde_json::to_string_pretty(budgets).unwrap_or_else(|_| "{}".into())
            );
        }
        if let Some(goals) = &manifest.goals {
            println!("{}", goals_report_line(goals));
            println!(
                "goals={}",
                serde_json::to_string_pretty(goals).unwrap_or_else(|_| "{}".into())
            );
        }
        if is_dungeongrid {
            println!(
                "episode_result={}",
                serde_json::to_string_pretty(&dungeon_state.episode_result())
                    .unwrap_or_else(|_| "{}".into())
            );
        } else if let Some(field) = resolve_host_config(&spec)
            .and_then(|host| host.viz)
            .and_then(|viz| viz.matrix_report_field)
        {
            if let Some(report) = matrix_report_from_manifest(&manifest, &field) {
                println!("{report}");
            }
        }
    } else {
        print_manifest(&manifest);
        if let Some(budgets) = &manifest.budgets {
            println!(
                "budgets={}",
                serde_json::to_string_pretty(budgets).unwrap_or_else(|_| "{}".into())
            );
        }
        if let Some(goals) = &manifest.goals {
            println!("{}", goals_report_line(goals));
            println!(
                "goals={}",
                serde_json::to_string_pretty(goals).unwrap_or_else(|_| "{}".into())
            );
        }
        if is_dungeongrid {
            println!(
                "episode_result={}",
                serde_json::to_string_pretty(&dungeon_state.episode_result())
                    .unwrap_or_else(|_| "{}".into())
            );
        }
    }

    // Persist the manifest ALWAYS — including a failed run, so a scan that blew its
    // min_success gate (or hit an unresolved binding mid-way) still leaves its
    // recorded verdicts on disk to inspect instead of throwing the work away.
    if let Some(out) = out {
        write_json(out, &manifest)?;
        write_json(&spec_sidecar_path(out), &spec)?;
        if let Some(bundle) = trace_out {
            export_trace_v5(out, bundle, true)?;
        }
    }

    if manifest.status == RunStatus::Failed {
        let reason = failure_reason(&manifest)
            .unwrap_or_else(|| "run failed (no reason recorded)".to_string());
        // Same loud report as a hard error, but the manifest is already written.
        print_error_report(&RunFailure(reason));
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

fn export_trace_v5(
    manifest_path: &Path,
    bundle: &Path,
    atif: bool,
) -> Result<ExitCode, Box<dyn Error>> {
    let import = synth_trace_command()?
        .args(["import", "--format", "jesterky", "--input"])
        .arg(manifest_path)
        .arg("--bundle")
        .arg(bundle)
        .status()?;
    if !import.success() {
        return Err(format!(
            "synth-trace jesterky import failed with status {import}; native manifest remains at {}",
            manifest_path.display()
        )
        .into());
    }
    let validation = synth_trace_command()?
        .arg("validate")
        .arg(bundle)
        .status()?;
    if !validation.success() {
        return Err(format!("synth-trace validation failed with status {validation}").into());
    }
    if atif {
        let projection = synth_trace_command()?
            .arg("project")
            .arg(bundle)
            .args(["--format", "atif"])
            .status()?;
        if !projection.success() {
            return Err(
                format!("synth-trace ATIF projection failed with status {projection}").into(),
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn synth_trace_command() -> Result<ProcessCommand, Box<dyn Error>> {
    const VERSION: &str = "0.3.0.20260725";
    const WHEEL_SHA256: &str = "1eafbca64b40c84c8c9d2554e68c8115605eea971444a378ca1d738fc39f61ee";
    let python = std::env::var("SYNTH_TRACE_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let wheel = std::env::var("SYNTH_TRACE_CONTAINERS_WHEEL_PATH")
        .or_else(|_| std::env::var("SYNTH_CONTAINERS_WHEEL"))
        .map_err(|_| "SYNTH_TRACE_CONTAINERS_WHEEL_PATH or SYNTH_CONTAINERS_WHEEL is required")?;
    let preflight = ProcessCommand::new(&python)
        .arg("-c")
        .arg(
            "import hashlib,importlib.metadata,pathlib,sys; \
             assert importlib.metadata.version('synth-containers') == sys.argv[2]; \
             assert hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest() == sys.argv[3]",
        )
        .arg(&wheel)
        .arg(VERSION)
        .arg(WHEEL_SHA256)
        .status()?;
    if !preflight.success() {
        return Err(
            "synth-containers provenance preflight failed for the selected trace interpreter"
                .into(),
        );
    }
    let mut command = ProcessCommand::new(python);
    command.args(["-m", "synth_containers.tracing.cli"]);
    Ok(command)
}

/// The reason a run finished `Failed`, pulled from its `WorkflowFailed` event.
fn failure_reason(manifest: &RunManifest) -> Option<String> {
    manifest
        .events
        .iter()
        .find(|e| matches!(e.kind, EventKind::WorkflowFailed))
        .and_then(|e| e.payload.get("error").and_then(|v| v.as_str()))
        .map(str::to_string)
}

/// A run that executed but finished `Failed` (gate/actor), carrying its reason so
/// the shared loud-error report + hint logic can render it like any other failure.
#[derive(Debug)]
struct RunFailure(String);

impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for RunFailure {}

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
        None,
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
    sink: Option<Arc<dyn jesterky_core::EventSink>>,
) -> Runner {
    Runner {
        programs: demo_programs(),
        actor,
        resource,
        sink: sink.unwrap_or_else(|| Arc::new(MemEventSink::new())),
        clock,
        store: Arc::new(MemArtifactStore::new()),
        checkpoints,
    }
}

fn run_event_sink(
    shared: Option<Arc<SharedEventSink>>,
    events_out: Option<&Path>,
) -> Result<Arc<dyn jesterky_core::EventSink>, Box<dyn Error>> {
    let collector: Arc<dyn jesterky_core::EventSink> = Arc::new(MemEventSink::new());
    let mut sinks = vec![collector];
    if let Some(shared) = shared {
        sinks.push(shared);
    }
    if let Some(path) = events_out {
        let sink: Arc<dyn jesterky_core::EventSink> = if path.as_os_str() == "-" {
            Arc::new(NdjsonEventSink::stdout())
        } else {
            Arc::new(NdjsonEventSink::file(path)?)
        };
        sinks.push(sink);
    }
    if sinks.len() == 1 {
        Ok(sinks.remove(0))
    } else {
        Ok(Arc::new(TeeEventSink::new(sinks)))
    }
}

/// The programs available to CLI runs: the real quality-scan workload
/// (`quality.expand` / `quality.aggregate`). A richer CLI would let specs bring
/// their own; today this is the one built-in workload.
fn demo_programs() -> ProgramRegistry {
    programs()
}

/// Environment escape hatch that lifts the Anthropic-model ban for one run.
const ALLOW_ANTHROPIC_ENV: &str = "JESTERKY_ALLOW_ANTHROPIC";

/// Hard ban on Anthropic model routes. jesterky drives model calls through
/// codex, and the house policy is: **no Anthropic models on these routes.** This
/// refuses any model id that names an Anthropic model before a single call goes
/// out. It is deliberately a manual, per-run override — set
/// `JESTERKY_ALLOW_ANTHROPIC=1` to turn the ban off when you explicitly mean to.
fn assert_model_allowed(model: &str) -> Result<(), Box<dyn Error>> {
    // Manual off-switch. Any value other than "0"/"" lifts the ban for this run.
    match std::env::var(ALLOW_ANTHROPIC_ENV).ok().as_deref() {
        Some(v) if !v.is_empty() && v != "0" => return Ok(()),
        _ => {}
    }
    const BANNED: &[&str] = &["claude", "anthropic", "sonnet", "opus", "haiku"];
    let lower = model.to_ascii_lowercase();
    if let Some(hit) = BANNED.iter().find(|needle| lower.contains(**needle)) {
        return Err(format!(
            "anthropic model route `{model}` is banned (matched `{hit}`). \
             House policy: no Anthropic models on jesterky routes. If you really \
             mean to, lift the ban for this run with `{ALLOW_ANTHROPIC_ENV}=1`."
        )
        .into());
    }
    Ok(())
}

async fn preflight_codex_home_route(
    model: &str,
    codex_home: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let Some(codex_home) = codex_home else {
        return Ok(());
    };
    let config_path = codex_home.join("config.toml");
    let Ok(config) = std::fs::read_to_string(&config_path) else {
        return Ok(());
    };
    let Some(base_url) = config_string_value(&config, "base_url") else {
        return Ok(());
    };
    let env_key =
        config_string_value(&config, "env_key").unwrap_or_else(|| "SYNTH_API_KEY".to_string());
    let api_key = env::var(&env_key).map_err(|_| {
        format!(
            "codex route preflight failed before fan-out: `{}` points at `{}`, \
             but required env var `{}` is not set. Export it before running the scan.",
            config_path.display(),
            base_url,
            env_key
        )
    })?;
    let url = format!("{}/responses", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .post(&url)
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "model": model,
            "input": [{
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": "Return exactly this JSON object and nothing else: {\"ok\":true}",
                }],
            }],
            "max_output_tokens": 32,
        }))
        .send()
        .await
        .map_err(|err| {
            format!("codex route preflight failed before fan-out: could not reach `{url}`: {err}")
        })?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() || route_body_is_error(&body) {
        return Err(format!(
            "codex route preflight failed before fan-out for model `{model}` via `{url}`: \
             HTTP {status}: {}. Fix the provider route/balance or use a working model, \
             for example `--model gpt-5.4-mini`.",
            compact_error_body(&body)
        )
        .into());
    }
    Ok(())
}

fn config_string_value(config: &str, key: &str) -> Option<String> {
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('#') || !line.starts_with(key) {
            continue;
        }
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        if left.trim() != key {
            continue;
        }
        let value = right.trim().trim_matches('"').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn route_body_is_error(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    value.get("error").is_some_and(|error| !error.is_null())
        || value
            .get("detail")
            .and_then(|detail| detail.get("error"))
            .is_some_and(|error| !error.is_null())
        || value
            .get("resource_exhaustion")
            .is_some_and(|error| !error.is_null())
        || value
            .get("detail")
            .and_then(|detail| detail.get("resource_exhaustion"))
            .is_some_and(|error| !error.is_null())
}

fn compact_error_body(body: &str) -> String {
    let one_line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 500 {
        one_line.chars().take(500).collect::<String>() + "…"
    } else {
        one_line
    }
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
    print_status_line(manifest);
}

fn print_status_line(manifest: &RunManifest) {
    println!(
        "status={} events={} recorded={}",
        format!("{:?}", manifest.status).to_lowercase(),
        manifest.events.len(),
        manifest.recorded.len()
    );
}

fn print_usage_line(
    model: Option<&str>,
    progress: &HashMap<NodePath, ShardProgress>,
    wall_secs: f64,
) {
    let (input_tokens, output_tokens) =
        progress
            .values()
            .fold((0u64, 0u64), |(input, output), shard| {
                (
                    input.saturating_add(shard.tokens_in),
                    output.saturating_add(shard.tokens_out),
                )
            });
    let total_tokens = input_tokens.saturating_add(output_tokens);
    if total_tokens == 0 && wall_secs <= 0.0 {
        return;
    }
    let tps = if wall_secs > 0.0 {
        total_tokens as f64 / wall_secs
    } else {
        0.0
    };
    let mut line = format!(
        "usage time={} tokens={} input={} output={} tps={}",
        format_wall(wall_secs),
        fmt_cli_tokens(total_tokens),
        fmt_cli_tokens(input_tokens),
        fmt_cli_tokens(output_tokens),
        fmt_cli_tokens(tps.round() as u64),
    );
    if let Some(model) = model {
        if let Some(cost) = estimated_cost_usd(model, input_tokens, output_tokens) {
            line.push_str(&format!(" cost_est={}", fmt_usd(cost)));
        }
    }
    println!("{line}");
}

fn estimated_cost_usd(model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let lower = model.to_ascii_lowercase();
    let (input_per_mtok, output_per_mtok) = if lower.contains("gemini-3.1-flash-lite") {
        (0.25, 1.50)
    } else {
        return None;
    };
    Some(
        (input_tokens as f64 / 1_000_000.0) * input_per_mtok
            + (output_tokens as f64 / 1_000_000.0) * output_per_mtok,
    )
}

fn fmt_usd(cost: f64) -> String {
    if cost < 0.01 {
        format!("${cost:.4}")
    } else if cost < 1.0 {
        format!("${cost:.3}")
    } else {
        format!("${cost:.2}")
    }
}

fn fmt_cli_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        let k = n as f64 / 1_000.0;
        if k < 10.0 {
            format!("{k:.1}k")
        } else {
            format!("{}k", k.round() as u64)
        }
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

fn format_wall(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    if minutes > 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("0:{seconds:02}")
    }
}

fn resolve_host_config(spec: &WorkflowSpec) -> Option<HostConfig> {
    spec.host
        .clone()
        .or_else(|| workload_host_config(&spec.name))
}

fn apply_host_config(
    mut actor: ModelActor<CodexModel>,
    host: &HostConfig,
    spec_dir: &Path,
) -> Result<ModelActor<CodexModel>, Box<dyn Error>> {
    for (name, role) in &host.roles {
        let prompt = resolve_role_prompt(role, spec_dir)?;
        actor = actor.with_role(name, prompt);
    }
    for (actor_name, schema) in &host.output_schemas {
        actor = actor.with_output_schema(actor_name, spec_dir.join(schema));
    }
    // Seeded execution workspaces (host-only, honored by jesterky-sandbox).
    actor = actor.with_spec_dir(spec_dir);
    for (actor_name, sandbox) in &host.sandboxes {
        actor = actor.with_sandbox(actor_name, sandbox.clone());
    }
    Ok(actor)
}

fn resolve_role_prompt(role: &HostRole, spec_dir: &Path) -> Result<String, Box<dyn Error>> {
    if let Some(prompt) = &role.prompt {
        return Ok(prompt.clone());
    }
    if let Some(file) = &role.prompt_file {
        return Ok(std::fs::read_to_string(spec_dir.join(file))?);
    }
    Err(format!(
        "host role in `{}` needs `prompt` or `prompt_file`",
        spec_dir.display()
    )
    .into())
}

fn matrix_report_from_manifest(manifest: &RunManifest, field: &str) -> Option<String> {
    // Prefer recorded actor outputs, then fall back to ledger-shaped program
    // results that may only appear on the final event payload / latest recorded
    // outputs (DungeonGrid finalize is a pure program — surface episode_result
    // from any recorded map of that name if present, else pretty-print JSON).
    if let Some(text) = manifest.recorded.iter().find_map(|r| {
        r.outputs
            .get(field)
            .and_then(|v| value_as_report(v))
            .or_else(|| {
                r.outputs
                    .get("summary")
                    .and_then(|s| s.get(field))
                    .and_then(value_as_report)
            })
    }) {
        return Some(text);
    }
    // Last-resort: scan all recorded outputs for a nested field (finalize may
    // not be recorded — programs aren't — so also check WorkflowCompleted? no).
    // For program-only finals, the host can re-derive from the last hero scores.
    manifest.recorded.iter().rev().find_map(|r| {
        if field == "episode_result" {
            if let (Some(done), Some(score)) =
                (r.outputs.get("done").and_then(|v| v.as_bool()), r.score)
            {
                if done {
                    return Some(format!(
                        "episode_result (from last turn): score={score:.3} done=true"
                    ));
                }
            }
        }
        None
    })
}

fn value_as_report(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            Some(serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => None,
    }
}

fn should_follow(no_follow: bool, follow_flag: bool) -> bool {
    if no_follow {
        false
    } else if follow_flag {
        true
    } else {
        io::stdout().is_terminal()
    }
}

fn follow_color_enabled(follow: bool, no_color: bool) -> bool {
    follow && !no_color && std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

fn live_item_labels(
    spec: &WorkflowSpec,
    args: &serde_json::Value,
    program_registry: &ProgramRegistry,
) -> Option<Vec<String>> {
    let host = resolve_host_config(spec)?;
    let viz = host.viz?;
    let op = viz.item_labels_op.as_ref()?;
    let expand = program_registry.get(op)?;
    let mut ledger = Ledger::new();
    if let Some(fields) = args.as_object() {
        for (key, value) in fields {
            ledger.set(key, value.clone());
        }
    }
    let out = expand(&ledger, args).ok()?;
    // DungeonGrid: one party lane per hero (not one row per scheduled turn).
    if spec.name.starts_with("dungeongrid") {
        if let Some(ids) = out.get("hero_ids").and_then(|v| v.as_array()) {
            let heroes: Vec<String> = ids
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if !heroes.is_empty() {
                return Some(heroes);
            }
        }
        if let Some(ids) = args.get("hero_ids").and_then(|v| v.as_array()) {
            let heroes: Vec<String> = ids
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if !heroes.is_empty() {
                return Some(heroes);
            }
        }
    }
    let jobs_key = viz.item_jobs_field.as_deref().unwrap_or("jobs");
    let label_key = viz.item_label_field.as_deref().unwrap_or("slug");
    let labels: Vec<String> = out
        .get(jobs_key)?
        .as_array()?
        .iter()
        .filter_map(|job| {
            job.as_str().map(str::to_string).or_else(|| {
                job.get(label_key)
                    .or_else(|| job.get("dimension"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
        })
        .collect();
    if labels.is_empty() {
        None
    } else {
        Some(labels)
    }
}

fn partial_manifest(spec: &WorkflowSpec, run_id: &str, events: Vec<Event>) -> RunManifest {
    RunManifest {
        run_id: run_id.to_string(),
        workflow_name: spec.name.clone(),
        spec_hash: String::new(),
        args: serde_json::Value::Null,
        events,
        recorded: Vec::new(),
        checkpoints: Vec::new(),
        trace: None,
        status: RunStatus::Completed,
        stop_reason: RunStopReason::Completed,
        budgets: None,
        goals: None,
        invariants: None,
        grounding: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn follow_viz_loop(
    sink: Arc<SharedEventSink>,
    spec: &WorkflowSpec,
    item_labels: Option<&[String]>,
    model: Option<&str>,
    mut stream: LiveStream,
    stop: Arc<AtomicBool>,
    interval: Duration,
    mut opts: RenderOpts,
    budget_plan: BudgetPlan,
    run_id: String,
) -> HashMap<NodePath, ShardProgress> {
    let mut stdout = io::stdout();
    let mut previous_lines = 0usize;
    let started = std::time::Instant::now();
    let mut tick = 0u32;
    let mut budget_history: Vec<BudgetObservation> = Vec::new();
    if opts.color {
        let _ = write!(stdout, "\x1b[?25l");
        let _ = stdout.flush();
    }

    loop {
        opts.tick = tick;
        let wall = started.elapsed().as_secs_f64();
        opts.elapsed_secs = Some(wall);
        let events = sink.snapshot();
        let snapshot = stream.fold();
        let mut partial = partial_manifest(spec, "live", events);
        // Mid-run meter: actor calls from live events; tokens from shard fold.
        if let Some(snap) = project_budgets(
            &run_id,
            &budget_plan,
            &partial,
            Some(&snapshot),
            wall,
            &budget_history,
        ) {
            // Keep a short history so ETA has >1 sample.
            append_budget_samples(&mut budget_history, &snap, wall);
            partial.budgets = Some(snap);
        }
        let mut view = adapt_manifest(&partial, Some(spec), item_labels, Some(&snapshot));
        if let Some(model) = model {
            view.model = Some(model.to_string());
        }
        let lines = render_run_view_lines(&view, &opts);
        let _ = redraw_lines(&mut previous_lines, &lines, &mut stdout);

        tick = tick.wrapping_add(1);
        if sink.is_terminal() || stop.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(interval);
    }

    if opts.color {
        let _ = write!(stdout, "\x1b[?25h");
        let _ = stdout.flush();
    }
    // A final drain so the settled frame the caller renders has every last event.
    stream.fold()
}

/// One-line goal summary for the settled panel (dual of the budget progress
/// line): `goals 1/2 required met [early-stop] · quality ✓ · coverage 0.72`.
fn goals_report_line(snap: &GoalSnapshot) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut head = format!(
        "goals {}/{} required met",
        snap.required_met, snap.required_total
    );
    if snap.terminated_early {
        head.push_str(" [early-stop]");
    }
    parts.push(head);
    for item in &snap.items {
        if !item.show_progress {
            continue;
        }
        let mark = match item.state {
            GoalState::Met => "✓".to_string(),
            GoalState::Unmet => format!("{:.2}", item.progress),
            GoalState::Unknown => "?".to_string(),
        };
        let opt = if item.required { "" } else { " (opt)" };
        parts.push(format!("{} {}{}", item.label, mark, opt));
    }
    parts.join(" · ")
}

/// Budgets from the spec's `runplan.budgets`, optionally **deep-merged** with
/// `--args.budgets` (partial overlays allowed — see [`BudgetPlan::overlay_json`]).
fn resolve_budget_plan(
    spec: &WorkflowSpec,
    args: &serde_json::Value,
) -> Result<BudgetPlan, ContractError> {
    let base = spec.runplan.budgets.clone();
    match args.get("budgets") {
        Some(raw) if !raw.is_null() => base.overlay_json(raw),
        _ => Ok(base),
    }
}

/// Project formal budget progress + ETA from events, live shard progress, and
/// optional history samples (for multi-point burn-rate estimates).
fn project_budgets(
    run_id: &str,
    plan: &BudgetPlan,
    manifest: &RunManifest,
    progress: Option<&HashMap<NodePath, ShardProgress>>,
    wall_secs: f64,
    history: &[BudgetObservation],
) -> Option<BudgetSnapshot> {
    if plan.is_empty() {
        return None;
    }
    let actor_calls = manifest
        .events
        .iter()
        .filter(|e| e.kind == EventKind::ActorInvoked)
        .count() as f64;
    // Prefer recorded actor count when available (finished run).
    let actor_calls = if !manifest.recorded.is_empty() {
        manifest
            .recorded
            .iter()
            .filter(|r| matches!(r.call, CallKind::Actor { .. }))
            .count() as f64
    } else {
        actor_calls
    };
    let tokens = progress
        .map(|p| {
            p.values()
                .map(|s| s.tokens_in.saturating_add(s.tokens_out))
                .sum::<u64>() as f64
        })
        .unwrap_or(0.0);
    // In-flight map items (started, not yet completed/failed) are RESERVED actor
    // calls: under a wide parallel map, K calls can be outstanding before any
    // settles, so `committed = spent + reserved` warns/ETAs against the real load
    // rather than lagging behind it. Zero on a settled manifest (all items done).
    let started = manifest
        .events
        .iter()
        .filter(|e| e.kind == EventKind::MapItemStarted)
        .count();
    let settled = manifest
        .events
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                EventKind::MapItemCompleted | EventKind::MapItemFailed
            )
        })
        .count();
    let in_flight_calls = started.saturating_sub(settled) as f64;

    let mut observations = history.to_vec();
    // Seed t=0 zeros once so a single live sample still forms a burn-rate pair.
    if history.is_empty() && wall_secs > 0.05 {
        for cap in &plan.caps {
            observations.push(BudgetObservation {
                kind: cap.kind,
                t_secs: 0.0,
                spent: 0.0,
                reserved: 0.0,
            });
        }
    }
    // Always append the current sample point so ETA has a fresh endpoint.
    for cap in &plan.caps {
        let spent = match cap.kind {
            BudgetKind::ActorCalls => actor_calls,
            BudgetKind::Tokens => tokens,
            BudgetKind::WallSeconds => wall_secs,
        };
        let reserved = match cap.kind {
            BudgetKind::ActorCalls => in_flight_calls,
            _ => 0.0,
        };
        observations.push(BudgetObservation {
            kind: cap.kind,
            t_secs: wall_secs,
            spent,
            reserved,
        });
    }
    Some(BudgetEngine::snapshot(
        run_id,
        plan,
        &observations,
        wall_secs,
    ))
}

fn append_budget_samples(
    history: &mut Vec<BudgetObservation>,
    snap: &BudgetSnapshot,
    wall_secs: f64,
) {
    let interval = snap.plan.eta.sample_interval_secs.max(0.05);
    if let Some(last) = history.last() {
        if wall_secs - last.t_secs < interval {
            return;
        }
    }
    for item in &snap.items {
        history.push(BudgetObservation {
            kind: item.kind,
            t_secs: wall_secs,
            spent: item.spent,
            reserved: item.reserved,
        });
    }
    // Cap history length (keep latest ~80 samples).
    if history.len() > 240 {
        let drain = history.len() - 180;
        history.drain(0..drain);
    }
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
