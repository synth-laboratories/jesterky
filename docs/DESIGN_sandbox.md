# Design: a consistent sandbox primitive for jesterky workflows

## Problem

An agentic actor (codex) needs to *execute* in a workspace — read a materialized
project, run a build, run a test/oracle, iterate — not just translate text in a
single shot. Today the only exec surface is codex's own `--sandbox read-only` +
an optional global `--cd`, hardcoded in `jesterky-model/src/codex.rs` and wired
through one CLI flag. There is no declarative, per-node way for a workflow to say
"give this actor a **writable workspace seeded with these files**, let it run
tools, then capture what it produced." The dev-port task exposed this: the porter
never ran the Python reference or its own Rust — it ported blind.

## Where it belongs (and where it must NOT)

`crates/jesterky-core/src/traits.rs` is explicit: *"a call or a subprocess is the
host's private concern … never in this contract."* `ActorRequest` is `{addr,
actor, inputs}` — nothing about execution. So:

- **Core contract (`Node`, `ActorRequest`) stays untouched.** A sandbox never
  enters replay identity. Replaying a recorded actor result must not require a
  sandbox at all.
- **Declarative config lives in `HostConfig`** (`topology.rs`), which is already
  "host-only, core ignores, not part of replay identity." We add a per-actor
  `sandboxes` map there — symmetric with the existing `roles` / `output_schemas`
  maps.
- **The runtime lives in a new host crate `jesterky-sandbox`**, consumed by the
  host wiring (CLI / ModelActor), never by core.

This mirrors the existing split: `Model`/`Actor`/`Resource` are host traits; the
core holds trait objects and knows nothing of codex, HTTP, or now docker.

## Contract addition (`jesterky-contract`)

```rust
// HostConfig gains:
pub sandboxes: BTreeMap<String /* actor name */, SandboxConfig>,

pub struct SandboxConfig {
    pub backend: SandboxBackend,   // Local | Docker { image, .. }
    pub mode: SandboxMode,         // ReadOnly | WorkspaceWrite
    #[serde(default)] pub network: bool,
    #[serde(default)] pub seed: SandboxSeed,
    #[serde(default)] pub capture: Option<SandboxCapture>,
}

pub enum SandboxBackend {
    Local,
    Docker { image: String, #[serde(default)] setup: Vec<String> },
}

pub enum SandboxMode { ReadOnly, WorkspaceWrite }

pub struct SandboxSeed {
    /// Host dir(s) copied into the workspace before the actor runs (relative to
    /// the spec dir). E.g. the source_task's `gold_python/` + scenarios.
    #[serde(default)] pub copy_from: Vec<String>,
    /// Ledger input field holding `{files:[{path,content}]}` to write to disk —
    /// lets an upstream node produce the workspace contents.
    #[serde(default)] pub files_input: Option<String>,
    /// Commands run once after seeding (e.g. `uv sync`, `cargo fetch`).
    #[serde(default)] pub setup: Vec<String>,
}

pub struct SandboxCapture {
    /// Globs (relative to workspace) collected back into the actor's outputs
    /// after it finishes — the produced crate, a results file, etc.
    pub globs: Vec<String>,
    /// Output field to hold the captured `{files:[{path,content}]}`.
    pub into: String,
}
```

All of these are host-only; the core `spec_hash` / replay path never sees them.

## Runtime trait (`jesterky-sandbox`)

```rust
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Host-visible workspace path (codex `--cd`). For docker this is the mount
    /// source; the container sees it at a fixed in-container path.
    fn workdir(&self) -> &Path;
    /// Permission level the actor should self-apply (codex `--sandbox`).
    fn mode(&self) -> SandboxMode;
    /// Wrap a command so it runs INSIDE the sandbox. Local = run on host with
    /// cwd=workdir. Docker = `docker exec -w <cwd> <cid> <argv…>`.
    fn wrap(&self, program: &str, args: &[String]) -> Command;
    /// Read matching files back out after the actor runs (capture).
    async fn collect(&self, globs: &[String]) -> Result<Vec<FileBlob>, SandboxError>;
}

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    /// Materialize a FRESH, isolated sandbox for ONE actor invocation (one map
    /// shard), seeded per `seed`. RAII cleanup on the returned handle's Drop.
    async fn create(&self, cfg: &SandboxConfig, seed_ctx: &SeedCtx)
        -> Result<Box<dyn Sandbox>, SandboxError>;
}
```

Two providers to start: `LocalSandboxProvider`, `DockerSandboxProvider`. Both
seed identically (write `files_input`, copy `copy_from`, run `setup`); they differ
only in `wrap` (host vs `docker exec`) and lifecycle (temp dir vs container).

Isolation is per-invocation: a `map`/`session_group` fan-out gets one sandbox per
shard, keyed by `addr`, so parallel shards never share a workspace — the same
guarantee `isolation: "worktree"` gives elsewhere.

## CodexModel change

Replace the hardcoded `--sandbox read-only` + `cwd: Option<PathBuf>` with an
optional `Arc<dyn Sandbox>`:

- `--cd` ← `sandbox.workdir()`
- `--sandbox` ← `sandbox.mode()` (`read-only` | `workspace-write`)
- **Local backend:** spawn `codex` on the host as today — the workspace is a host
  temp dir, host toolchain. This is the achievable first slice and already lets
  the porter run `uv run python …` and `cargo` against a real, seeded workspace.
- **Docker backend:** spawn codex via `sandbox.wrap("codex", …)` so codex itself
  runs INSIDE the container (image toolchain, real isolation). Requirements,
  resolved below.

## Docker execution model (the load-bearing fork)

Goal: the agent truly runs the env with isolation. That rules out
"codex-on-host, mount a dir" (its shell tools would run on the host toolchain).
**Codex runs inside the container** (`docker exec`). Requirements:

1. **Image** must carry the toolchains the task needs (rust + python + uv) and the
   `codex` binary. Native `linux/arm64` only — OrbStack, **no QEMU** (house rule).
2. **Auth + proxy config**: bind-mount the proxy-materialized `CODEX_HOME`
   (`~/.cache/jesterky/…`, already built by `jesterky-proxy`) into the container.
3. **Proxy reachability**: the chat proxy listens on host `127.0.0.1:<port>`; from
   the container that loopback is the container's own. The codex config's proxy
   base URL must be `host.docker.internal:<port>` for the docker backend (OrbStack
   maps it). So `jesterky-proxy` must emit a backend-aware base URL, or the docker
   provider rewrites it at container start.
4. **Lifecycle**: `docker run -d --rm` a long-lived container per invocation
   (seeded workspace mounted or `docker cp`'d in), `docker exec` codex + tools,
   `docker cp` / mounted-dir readback for capture, `docker rm -f` on Drop.

Local backend has none of these wrinkles — hence local first, docker second.

## Host glue (CLI / runner)

Before driving an actor that has a `SandboxConfig`: build a `SeedCtx` from the
resolved ledger inputs + spec dir, `provider.create(...)`, inject the `Arc<dyn
Sandbox>` into the codex actor for that call, run, then `collect(capture.globs)`
and merge the captured files into the actor's outputs under `capture.into` so
downstream nodes / scoring read them from the ledger. Sandbox is dropped
(cleaned up) when the call returns.

## Phasing / status

1. ✅ **Contract**: `SandboxConfig` + `HostConfig.sandboxes` (schema regenerated).
2. ✅ **`jesterky-sandbox` crate**: `Sandbox`/`SandboxProvider` traits, shared
   `stage_seed_files`/`run_setup`/`collect_globs` + minimal glob, RAII cleanup.
3. ✅ **`LocalSandboxProvider`**: host temp-dir workspace, host toolchain.
   Integration test: seed→run→capture→cleanup.
4. ✅ **`DockerSandboxProvider`**: host mount ↔ `/workspace`, `docker exec` for
   setup + commands, capture off the shared mount, `docker rm -f` on Drop.
   `--network none` unless `network: true`. Integration test against a real
   `alpine` container (skips cleanly with no daemon). OrbStack, arm64-native.
5. ✅ **CodexModel**: honors `req.sandbox` — mode-aware `--sandbox`, `--cd`
   workspace, runs via `sandbox.command` (so docker runs codex in-container).
6. ✅ **Host wiring** in `jesterky-cli`: `HostConfig.sandboxes` → `ModelActor`
   (`with_sandbox`/`with_spec_dir`); create/seed/capture per actor call.

### Proven live (07-09) — an agentic port end-to-end, both backends

Task: `gamebench/tasks/dev-port-singleplayer/` — `dev_port_to_rust.sandboxed.json`
(local) / `dev_port_to_rust.docker.json` (docker); porter seeded with
`gold_python/` + `scenarios.json` + a TRAIN-subset oracle + `check.py`,
workspace-write, `capture: {globs:[Cargo.toml, src/**/*.rs], into: "files"}`.
Must use a codex-NATIVE model (gpt-5.5) — the chat proxy doesn't translate tool
calls, so agentic tool use needs the real Responses API.

- **Local**: codex gpt-5.5 in a `~/.cache/jesterky/workspaces` workdir (not
  `$TMPDIR` — codex refuses), network via `-c
  sandbox_workspace_write.network_access=true`. Scored **1.0 (20/20)** on
  tictactoe (16 held-out) — vs blind ports deepseek 0.0 / gemini 0.8.
- **Docker** (`jesterky-devport:latest`, arm64: rust+python+uv+node+codex): host
  `~/.codex/auth.json` bind-mounted as in-container `CODEX_HOME`; codex runs
  `danger-full-access` (`actor_self_sandbox=false` — the container isolates); crate
  captured off the shared mount; container `docker rm -f` on Drop. Scored **0.8
  (16/20)** (misses are held-out policy/RNG scenarios not in the train subset —
  agent variance, not an infra defect).

Docker-specific fixes made: `SandboxBackend::Docker` gained `mounts` + `env`;
`Sandbox::env()` (in-container CODEX_HOME override) + `actor_self_sandbox()`
(container → codex `danger-full-access`); `--output-schema` (a host path) is
skipped in-container since `ModelActor` validates the schema host-side anyway.
```
