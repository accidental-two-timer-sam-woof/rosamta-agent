//! What the app shows: one snapshot read from core, replaced as a whole.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use rosamta_core::adopt::{self, QueueItem};
use rosamta_core::apply::{self, Context, ShellStatus};
use rosamta_core::config::{Deploy, HostConfig, HostFacts, Module, Phase};
use rosamta_core::daemon::instance;
use rosamta_core::daemon::service::{self, Service};
use rosamta_core::dotfiles::{self, Choice};
use rosamta_core::host;
use rosamta_core::repo::{self, CommitInfo};
use rosamta_core::sync::{self, SyncState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Structured,
    Raw,
}

#[derive(Clone)]
pub struct ModuleRow {
    pub name: String,
    pub file: PathBuf,
    pub phase: Phase,
    pub kind: Kind,
    pub enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DotfileStatus {
    Deployed,
    Pending,
    Skipped,
    NotDeployed,
}

#[derive(Clone)]
pub struct DotfileRow {
    pub path: PathBuf,
    pub deploy: Deploy,
    pub encrypted: bool,
    pub status: DotfileStatus,
}

#[derive(Clone)]
pub struct Snapshot {
    /// This host has `host.toml` and a repo.
    pub ready: bool,
    pub host: Option<HostConfig>,
    pub host_file: PathBuf,
    pub home: PathBuf,
    pub queue: Vec<QueueItem>,
    pub sync: SyncState,
    pub running: bool,
    pub started_at: Option<u64>,
    pub service: Service,
    pub remote: Option<String>,
    pub shells: Vec<ShellStatus>,
    pub modules: Vec<ModuleRow>,
    pub dotfiles: Vec<DotfileRow>,
    pub commits: Vec<CommitInfo>,
    pub snapshots: Vec<CommitInfo>,
    /// The first thing that could not be read.
    pub problem: Option<String>,
}

impl Snapshot {
    pub fn empty() -> Self {
        Snapshot {
            ready: false,
            host: None,
            host_file: PathBuf::new(),
            home: PathBuf::new(),
            queue: Vec::new(),
            sync: SyncState::default(),
            running: false,
            started_at: None,
            service: Service::Unavailable,
            remote: None,
            shells: Vec::new(),
            modules: Vec::new(),
            dotfiles: Vec::new(),
            commits: Vec::new(),
            snapshots: Vec::new(),
            problem: None,
        }
    }
}

/// Read everything core keeps for this host.
pub fn load() -> Snapshot {
    let mut out = Snapshot::empty();
    let ctx = match Context::from_env() {
        Ok(ctx) => ctx,
        Err(err) => {
            out.problem = Some(err.to_string());
            return out;
        }
    };
    let mut note = |err: rosamta_core::Error| {
        out.problem.get_or_insert(err.to_string());
    };
    let repo_dir = ctx.paths.repo();
    out.home = ctx.home.clone();
    out.host_file = ctx.paths.host_config();
    out.ready = out.host_file.is_file() && repo_dir.join(".git").exists();
    let state = ctx.paths.state();
    out.running = instance::running(state).unwrap_or(false);
    out.started_at = instance::load(state)
        .ok()
        .flatten()
        .filter(|_| out.running)
        .map(|daemon| daemon.started_at);
    out.service = service::status(&ctx.paths);
    if !out.ready {
        return out;
    }
    match host::load(&out.host_file) {
        Ok(host) => out.host = Some(host),
        Err(err) => note(err),
    }
    out.queue = adopt::queue(&ctx).unwrap_or_else(|err| {
        note(err);
        Vec::new()
    });
    out.sync = sync::load_state(state).unwrap_or_default();
    out.remote = repo::remote(&repo_dir);
    out.shells = apply::shells(&ctx).unwrap_or_else(|err| {
        note(err);
        Vec::new()
    });
    out.modules = modules(&ctx).unwrap_or_else(|err| {
        note(err);
        Vec::new()
    });
    if let Some(host) = &out.host {
        out.dotfiles = dotfile_rows(&ctx, host, &out.queue).unwrap_or_else(|err| {
            note(err);
            Vec::new()
        });
    }
    out.commits = repo::recent_commits(&repo_dir, 20);
    out.snapshots = repo::recent_commits(&state.join("snapshots"), 20);
    out
}

fn modules(ctx: &Context) -> rosamta_core::Result<Vec<ModuleRow>> {
    let mut files = repo::module_files(&ctx.paths.repo())?;
    let local = ctx.paths.local();
    if local.join(repo::MODULES).is_dir() {
        files.extend(repo::module_files(&local)?);
    }
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(|source| rosamta_core::Error::Read {
            path: file.clone(),
            source,
        })?;
        // A module that does not parse is for `rsmt validate` to report.
        let Ok(module) = toml::from_str::<Module>(&text) else {
            continue;
        };
        out.push(ModuleRow {
            name: repo::module_name(&file),
            phase: phase_of(&module),
            kind: if raw(&module) {
                Kind::Raw
            } else {
                Kind::Structured
            },
            enabled: module.enabled,
            file,
        });
    }
    out.sort_by(|a, b| (a.phase, &a.name).cmp(&(b.phase, &b.name)));
    Ok(out)
}

fn raw(module: &Module) -> bool {
    let raw = &module.raw;
    raw.sh.is_some() || raw.bash.is_some() || raw.zsh.is_some() || raw.fish.is_some()
}

/// The phase a module is listed under: its own, else where most of it loads.
fn phase_of(module: &Module) -> Phase {
    if let Some(phase) = module.phase {
        return phase;
    }
    if raw(module) {
        Phase::Late
    } else if module.env.is_empty()
        && module.path.prepend.is_empty()
        && module.path.append.is_empty()
        && !module.aliases.is_empty()
    {
        Phase::Aliases
    } else {
        Phase::Env
    }
}

fn dotfile_rows(
    ctx: &Context,
    host: &HostConfig,
    queue: &[QueueItem],
) -> rosamta_core::Result<Vec<DotfileRow>> {
    let (manifest, _) = dotfiles::usable(&ctx.paths.repo())?;
    let deployed = dotfiles::state::load(ctx.paths.state())?;
    let pending = |path: &Path| {
        queue.iter().any(|item| {
            item.dotfile
                .as_ref()
                .is_some_and(|dotfile| dotfile.path == path)
        })
    };
    let mut out = Vec::new();
    for (path, choice) in dotfiles::select(&manifest, &HostFacts::current(host)) {
        let Choice::Use(entry) = choice else {
            continue;
        };
        // An ignored dotfile keeps its record but is no longer deployed.
        let status = if pending(&path) {
            DotfileStatus::Pending
        } else if deployed.ignored.contains_key(&path) {
            DotfileStatus::Skipped
        } else if deployed.entries.contains_key(&path)
            || (entry.deploy == Deploy::Create && ctx.home.join(&path).symlink_metadata().is_ok())
        {
            DotfileStatus::Deployed
        } else {
            DotfileStatus::NotDeployed
        };
        out.push(DotfileRow {
            path,
            deploy: entry.deploy,
            encrypted: entry.encrypted,
            status,
        });
    }
    Ok(out)
}

/// Changes when anything the snapshot shows may have changed: modification
/// times of the files core keeps, and whether the daemon runs.
pub fn fingerprint() -> u64 {
    let mut hasher = DefaultHasher::new();
    let Ok(ctx) = Context::from_env() else {
        return 0;
    };
    let repo_dir = ctx.paths.repo();
    let state = ctx.paths.state();
    let mut stamp = |path: &Path| {
        path.hash(&mut hasher);
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .hash(&mut hasher);
    };
    stamp(&ctx.paths.host_config());
    for dir in [
        state.to_path_buf(),
        state.join("snapshots/.git"),
        repo_dir.clone(),
        repo_dir.join(".git"),
        repo_dir.join(".git/refs/heads"),
    ] {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            stamp(&entry.path());
        }
    }
    let local = ctx.paths.local();
    for dir in [repo_dir, local] {
        for file in repo::module_files(&dir).unwrap_or_default() {
            stamp(&file);
        }
    }
    instance::running(state).unwrap_or(false).hash(&mut hasher);
    hasher.finish()
}
