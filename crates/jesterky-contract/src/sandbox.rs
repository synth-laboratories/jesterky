//! Declarative **sandbox** config: how the host materializes an execution
//! workspace for an agentic actor (codex) — a seeded workdir the actor may run
//! tools in, then capture from. Host-only, exactly like [`crate::HostConfig`]
//! prompts/schemas: the core runner and replay identity never read any of this
//! (a call or subprocess "is the host's private concern"). The runtime that
//! honors these lives in `jesterky-sandbox`.

use serde::{Deserialize, Serialize};

/// Per-actor sandbox declaration (keyed by actor name in `HostConfig.sandboxes`).
#[derive(Clone, Debug, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub backend: SandboxBackend,
    #[serde(default)]
    pub mode: SandboxMode,
    /// Whether the actor's tools may reach the network. Off by default.
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub seed: SandboxSeed,
    #[serde(default)]
    pub capture: Option<SandboxCapture>,
}

/// Where the actor executes. `Local` = a host temp dir with the host toolchain;
/// `Docker` = codex runs INSIDE the named container (image toolchain, isolated).
#[derive(Clone, Debug, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SandboxBackend {
    Local,
    Docker {
        image: String,
        /// Commands baked/run at container start before the actor (e.g. warm caches).
        #[serde(default)]
        setup: Vec<String>,
        /// Extra bind mounts as docker `-v` specs, `host:container[:ro]`. `${HOME}`
        /// and `$VAR` are expanded. This is how codex auth reaches the container:
        /// mount host `~/.codex` to an in-container path, then point `CODEX_HOME`
        /// there via [`Self::Docker::env`].
        #[serde(default)]
        mounts: Vec<String>,
        /// Env exported to the actor's process inside the container (e.g.
        /// `CODEX_HOME=/codex-home`). Overrides the actor's own env for that key.
        #[serde(default)]
        env: Vec<(String, String)>,
    },
}

/// The permission level the actor should self-apply (maps to codex `--sandbox`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    /// Read the workspace, run nothing that writes it. Default — least privilege.
    #[default]
    ReadOnly,
    /// Create/modify files and run builds/tools in the workspace.
    WorkspaceWrite,
}

impl SandboxMode {
    /// The codex `--sandbox` value.
    pub fn codex_flag(self) -> &'static str {
        match self {
            SandboxMode::ReadOnly => "read-only",
            SandboxMode::WorkspaceWrite => "workspace-write",
        }
    }
}

/// How the workspace is populated before the actor runs. All three compose, in
/// order: write `files_input`, copy `copy_from`, run `setup`.
#[derive(Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct SandboxSeed {
    /// Host dirs copied into the workspace root (paths relative to the spec dir).
    #[serde(default)]
    pub copy_from: Vec<String>,
    /// Ledger input field holding `{files:[{path,content}]}` written to the
    /// workspace — lets an upstream node produce the workspace contents.
    #[serde(default)]
    pub files_input: Option<String>,
    /// Commands run once after seeding (e.g. `uv sync`).
    #[serde(default)]
    pub setup: Vec<String>,
}

/// What to read back out of the workspace after the actor finishes, and where to
/// put it in the actor's outputs so downstream nodes can consume it.
#[derive(Clone, Debug, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct SandboxCapture {
    /// Globs relative to the workspace root (e.g. `["Cargo.toml", "src/**/*.rs"]`).
    pub globs: Vec<String>,
    /// Output field to hold the captured `{files:[{path,content}]}`.
    pub into: String,
}
