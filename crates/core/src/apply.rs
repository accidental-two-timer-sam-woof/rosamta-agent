//! `rsmt apply`, `hook`, `unhook`, `eject` and `restore --original`.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::{HostConfig, HostFacts, Module, Shell};
use crate::error::{Error, Result};
use crate::generated::{self, CURRENT};
use crate::hooks::{self, FileChange, HookEnv};
use crate::paths::Paths;
use crate::process::output_with_timeout;
use crate::render::{self, Layout};
use crate::secrets::Session;
use crate::shells::{self, Detected};
use crate::snapshot::Snapshots;
use crate::{host, repo, validate};

const SMOKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything apply needs to know about this host.
pub struct Context {
    pub paths: Paths,
    pub home: PathBuf,
    pub path_var: String,
    /// `rsmt apply` in a terminal: the only run that unlocks a secret
    /// source or runs a secret command.
    pub interactive: bool,
    /// The session an interactive run shares, so a tool unlocks and a
    /// command runs once however many deploys the run makes.
    pub session: Mutex<Option<Arc<Session>>>,
}

impl Context {
    pub fn from_env() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        Ok(Self {
            paths: Paths::from_env()?,
            home,
            path_var: std::env::var("PATH").unwrap_or_default(),
            interactive: false,
            session: Mutex::default(),
        })
    }

    /// Secret sources for one deploy; outside a terminal each deploy asks
    /// afresh, since a tool may have been unlocked in between.
    pub(crate) fn secrets(&self, host: &HostConfig) -> Result<Arc<Session>> {
        if !self.interactive {
            return Session::new(self, host, false).map(Arc::new);
        }
        let mut shared = self.session.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(session) = &*shared {
            return Ok(Arc::clone(session));
        }
        let session = Arc::new(Session::new(self, host, true)?);
        *shared = Some(Arc::clone(&session));
        Ok(session)
    }

    fn snapshots(&self) -> Snapshots {
        Snapshots::new(self.paths.state(), &self.home)
    }

    pub fn installed(&self) -> Vec<Detected> {
        shells::detect(&self.path_var, &self.paths.state().join("shells.toml"))
    }

    pub fn hook_env(&self, installed: &[Detected]) -> HookEnv {
        let exe = |shell| {
            installed
                .iter()
                .find(|found| found.shell == shell)
                .map(|found| found.path.as_path())
        };
        HookEnv::detect(
            &self.home,
            &self.paths.generated().join(CURRENT),
            exe(Shell::Zsh),
            exe(Shell::Fish),
        )
    }

    fn unhooked_file(&self) -> PathBuf {
        self.paths.state().join("unhooked")
    }

    /// Shells the user removed hooks from; apply leaves them alone.
    pub fn unhooked(&self) -> Vec<Shell> {
        std::fs::read_to_string(self.unhooked_file())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }

    fn set_unhooked(&self, shells: &[Shell]) -> Result<()> {
        let mut shells = shells.to_vec();
        shells.sort();
        shells.dedup();
        let text: String = shells.iter().map(|shell| format!("{shell}\n")).collect();
        let path = self.unhooked_file();
        std::fs::create_dir_all(self.paths.state()).map_err(Error::write(self.paths.state()))?;
        std::fs::write(&path, text).map_err(Error::write(&path))
    }
}

/// Parse every module in the repo; any error sends the user to `rsmt validate`.
pub fn load_modules(repo_dir: &Path) -> Result<Vec<(String, Module)>> {
    parse_modules(repo::module_files(repo_dir)?)
}

/// Modules that stay on this host (secrets); they never enter the repo.
pub fn load_local_modules(paths: &Paths) -> Result<Vec<(String, Module)>> {
    parse_modules(repo::module_files(&paths.local())?)
}

/// Repo and local modules together; a name may be used only once.
fn all_modules(paths: &Paths) -> Result<Vec<(String, Module)>> {
    let mut files = repo::module_files(&paths.repo())?;
    files.extend(repo::module_files(&paths.local())?);
    parse_modules(files)
}

fn parse_modules(files: Vec<PathBuf>) -> Result<Vec<(String, Module)>> {
    let mut modules = Vec::new();
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for path in files {
        let name = repo::module_name(&path);
        if let Some(first) = seen.get(&name) {
            return Err(Error::Usage(format!(
                "module name `{name}` is used by more than one file: {}, {}; rename one",
                first.display(),
                path.display()
            )));
        }
        let text = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
        let module: Module = validate::parse(&text).map_err(|issue| {
            Error::Usage(format!(
                "{}: {}; run `rsmt validate` for details",
                path.display(),
                issue.message
            ))
        })?;
        if let Some(problem) = render::check(&module).into_iter().next() {
            return Err(Error::Usage(format!("{}: {problem}", path.display())));
        }
        seen.insert(name.clone(), path);
        modules.push((name, module));
    }
    Ok(modules)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Plan {
    #[serde(skip)]
    pub layout: Layout,
    #[serde(skip)]
    pub files: BTreeMap<PathBuf, String>,
    /// Generated files that would change: relative path, before, after.
    pub file_changes: Vec<(PathBuf, Option<String>, Option<String>)>,
    pub hook_changes: Vec<FileChange>,
    /// Shells that get scripts.
    pub shells: Vec<Shell>,
    /// Shells that keep the scripts they had, since they could not be checked.
    #[serde(skip)]
    pub kept: Vec<Shell>,
    /// Shells that are installed but too old, with the reason.
    pub skipped: Vec<String>,
    pub current: Option<String>,
    #[serde(skip)]
    pub installed: Vec<Detected>,
    #[serde(skip)]
    pub hook_env: Option<HookEnv>,
}

impl Plan {
    pub fn is_up_to_date(&self) -> bool {
        self.current.is_some()
            && self.file_changes.is_empty()
            && self.hook_changes.iter().all(FileChange::is_noop)
    }
}

fn host_facts(paths: &Paths) -> Result<HostFacts> {
    let path = paths.host_config();
    if !path.exists() {
        return Err(Error::Usage(
            "this host is not set up; run `rsmt init` first".into(),
        ));
    }
    Ok(HostFacts::current(&host::load(&path)?))
}

/// Work out what apply would do, without writing anything.
pub fn plan(ctx: &Context) -> Result<Plan> {
    let repo_dir = ctx.paths.repo();
    if !repo_dir.join(repo::MANIFEST).is_file() {
        return Err(Error::Usage(format!(
            "no repo at {}; run `rsmt init` first",
            repo_dir.display()
        )));
    }
    repo::ensure_supported(repo::read_manifest(&repo_dir)?.format)?;
    let facts = host_facts(&ctx.paths)?;
    let modules = render::select(all_modules(&ctx.paths)?, &facts);

    let installed = ctx.installed();
    let skipped = installed.iter().filter_map(Detected::unsupported).collect();
    let shells: Vec<Shell> = installed
        .iter()
        .filter(|found| found.unsupported().is_none())
        .map(|found| found.shell)
        .collect();

    let generated_dir = ctx.paths.generated();
    let current = generated::current_id(&generated_dir);
    let old = match &current {
        Some(id) => generated::read_version(&generated_dir, id)?,
        None => BTreeMap::new(),
    };
    // A shell not found this time, or whose version did not come back (another
    // PATH, a slow start), keeps its last checked scripts until it can be checked.
    let known = |shell: &Shell| {
        installed
            .iter()
            .any(|found| found.shell == *shell && found.version.is_some())
    };
    let kept: Vec<Shell> = Shell::ALL
        .into_iter()
        .filter(|shell| !shells.contains(shell) && !known(shell))
        .filter(|shell| old.keys().any(|rel| rel.starts_with(shell.name())))
        .collect();
    // Render against the current version's paths, so unchanged modules compare equal.
    let id = current.clone().unwrap_or_else(generated::new_version_id);
    let layout = generated::layout(&generated_dir, ctx.paths.state(), &id);
    let mut files = render::render(&modules, &shells, &layout);
    files.extend(carried(&old, &kept));
    let mut file_changes = Vec::new();
    for path in old
        .keys()
        .chain(files.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let before = old.get(path).cloned();
        let after = files.get(path).cloned();
        if before != after {
            file_changes.push((path.clone(), before, after));
        }
    }

    let hook_env = ctx.hook_env(&installed);
    let unhooked = ctx.unhooked();
    let mut hook_changes = Vec::new();
    for shell in shells.iter().filter(|shell| !unhooked.contains(shell)) {
        hook_changes.extend(hooks::plan_install(*shell, &hook_env)?);
    }
    hook_changes.retain(|change| !change.is_noop());
    Ok(Plan {
        layout,
        files,
        file_changes,
        hook_changes,
        shells,
        kept,
        skipped,
        current,
        installed,
        hook_env: Some(hook_env),
    })
}

/// A unified diff of every change in the plan.
pub fn diff(plan: &Plan, generated_dir: &Path) -> String {
    let mut out = String::new();
    for (rel, before, after) in &plan.file_changes {
        let name = generated_dir.join(CURRENT).join(rel).display().to_string();
        out.push_str(&unified(&name, before.as_deref(), after.as_deref()));
    }
    for change in &plan.hook_changes {
        let name = change.path.display().to_string();
        out.push_str(&unified(
            &name,
            change.before.as_deref(),
            change.after.as_deref(),
        ));
    }
    out
}

pub fn unified(name: &str, before: Option<&str>, after: Option<&str>) -> String {
    let diff = similar::TextDiff::from_lines(before.unwrap_or(""), after.unwrap_or(""));
    // Paths are absolute, so no a/ and b/ prefixes.
    let old = if before.is_some() { name } else { "/dev/null" };
    let new = if after.is_some() { name } else { "/dev/null" };
    diff.unified_diff().header(old, new).to_string()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Applied {
    pub version: Option<String>,
    pub up_to_date: bool,
    pub shells: Vec<Shell>,
    pub skipped: Vec<String>,
    pub hooks_changed: Vec<PathBuf>,
    pub externals: Vec<crate::externals::Step>,
    pub dotfiles: Vec<crate::dotfiles::Step>,
    pub scripts: Vec<crate::scripts::apply::Step>,
}

impl Applied {
    /// `what: why` for each dotfile, external or script that failed.
    pub fn failures(&self) -> Vec<String> {
        let line = |what: String, reason: &Option<String>| {
            format!("{what}: {}", reason.as_deref().unwrap_or("failed"))
        };
        let dotfiles = self
            .dotfiles
            .iter()
            .filter(|step| step.action == crate::dotfiles::Action::Failed)
            .map(|step| line(step.path.display().to_string(), &step.reason));
        let externals = self
            .externals
            .iter()
            .filter(|step| step.action == crate::externals::Action::Failed)
            .map(|step| line(step.path.display().to_string(), &step.reason));
        let scripts = self
            .scripts
            .iter()
            .filter(|step| step.action == crate::scripts::apply::Action::Failed)
            .map(|step| line(step.source.clone(), &step.reason));
        dotfiles.chain(externals).chain(scripts).collect()
    }
}

/// Download externals and run `before` scripts; render, smoke-test, then
/// switch; then deploy dotfiles and run `after` scripts. If the shells fail
/// their check, the current version stays and no dotfile is touched. The
/// caller holds the adopt lock.
pub fn apply(ctx: &Context) -> Result<Applied> {
    migrate(ctx)?;
    around_shells(ctx, || finish(ctx, &[]))
}

/// What shells may load comes first: on a new host the module loading
/// oh-my-zsh needs it downloaded, and a tool a script installs needs to be
/// there, before the smoke test.
pub fn around_shells(ctx: &Context, shells: impl FnOnce() -> Result<Applied>) -> Result<Applied> {
    use crate::scripts::{Timing, apply as scripts, review};
    let externals = crate::externals::apply(ctx)?;
    let mut ran = scripts::apply(ctx, Timing::Before)?;
    review::sync_queue(ctx)?;
    let applied = Applied {
        externals,
        ..shells()?
    };
    let mut applied = with_dotfiles(ctx, applied)?;
    ran.extend(scripts::apply(ctx, Timing::After)?);
    review::sync_queue(ctx)?;
    applied.scripts = ran;
    Ok(applied)
}

/// Whether externals to download or `before` scripts wait to run ahead of
/// the dotfiles: either may install or sign in to a secret source the
/// templates use.
pub fn setup_pending(ctx: &Context) -> bool {
    use crate::externals::Action;
    use crate::scripts::{Timing, apply as scripts};
    let externals = crate::externals::plan(ctx).map_or(true, |steps| {
        steps
            .iter()
            .any(|step| matches!(step.action, Action::Downloaded | Action::Updated))
    });
    let scripts = crate::adopt::host_config(ctx)
        .and_then(|host| scripts::pending(ctx, &host))
        .map_or(true, |pending| {
            pending
                .iter()
                .any(|pending| pending.script.timing == Timing::Before)
        });
    externals || scripts
}

/// Deploy dotfiles after the shells switched.
pub fn with_dotfiles(ctx: &Context, mut applied: Applied) -> Result<Applied> {
    applied.dotfiles = crate::dotfiles::deploy::deploy_all(ctx)?;
    Ok(applied)
}

/// Apply after `change` edits rc files or modules, judging shell startup
/// against how shells started before the change. A change for `shells`
/// that cannot be checked now (not found, no version) is refused.
pub fn apply_after(
    ctx: &Context,
    shells: &[Shell],
    change: impl FnOnce() -> Result<()>,
) -> Result<Applied> {
    migrate(ctx)?;
    let unchecked: Vec<&str> = plan(ctx)?
        .kept
        .iter()
        .filter(|shell| shells.contains(shell))
        .map(|shell| shell.name())
        .collect();
    if !unchecked.is_empty() {
        return Err(Error::Apply(format!(
            "cannot start {} to check it, so nothing changed; try again",
            unchecked.join(", ")
        )));
    }
    let installed = ctx.installed();
    let env = ctx.hook_env(&installed);
    let generated_dir = ctx.paths.generated();
    let baseline_dir = generated::current_id(&generated_dir).map_or_else(
        || PathBuf::from("/nonexistent"),
        |id| generated_dir.join(id),
    );
    let mut baselines = Vec::new();
    for found in installed
        .iter()
        .filter(|found| found.unsupported().is_none())
    {
        if !hooks::hooked_files(found.shell, &env)?.is_empty() {
            baselines.push((found.shell, smoke(ctx, found, &baseline_dir)));
        }
    }
    change()?;
    finish(ctx, &baselines)
}

fn migrate(ctx: &Context) -> Result<()> {
    let repo_dir = ctx.paths.repo();
    if repo_dir.join(repo::MANIFEST).is_file() {
        repo::migrate(&repo_dir, crate::config::CURRENT_FORMAT, repo::MIGRATIONS)?;
    }
    Ok(())
}

fn finish(ctx: &Context, baselines: &[(Shell, Vec<Smoke>)]) -> Result<Applied> {
    let plan = plan(ctx)?;
    let hooks_changed: Vec<PathBuf> = plan
        .hook_changes
        .iter()
        .map(|change| change.path.clone())
        .collect();
    let mut applied = Applied {
        version: plan.current.clone(),
        up_to_date: plan.is_up_to_date(),
        shells: plan.shells.clone(),
        skipped: plan.skipped.clone(),
        hooks_changed,
        externals: Vec::new(),
        dotfiles: Vec::new(),
        scripts: Vec::new(),
    };
    if applied.up_to_date {
        return Ok(applied);
    }

    let generated_dir = ctx.paths.generated();
    let id = generated::new_version_id();
    let layout = generated::layout(&generated_dir, ctx.paths.state(), &id);
    let facts = host_facts(&ctx.paths)?;
    let modules = render::select(all_modules(&ctx.paths)?, &facts);
    let mut files = render::render(&modules, &plan.shells, &layout);
    if let Some(old_id) = &plan.current {
        let old = generated::read_version(&generated_dir, old_id)?;
        // They name their version's folder, which is now this one.
        files.extend(
            carried(&old, &plan.kept)
                .into_iter()
                .map(|(rel, text)| (rel, text.replace(old_id.as_str(), &id))),
        );
    }
    generated::write_version(&layout, &files)?;

    let result = check_and_smoke(ctx, &plan, &layout, &files, baselines);
    if let Err(err) = result {
        generated::remove_version(&layout);
        return Err(err);
    }
    generated::switch(&generated_dir, &id)?;
    generated::prune(&generated_dir)?;
    applied.version = Some(id);
    Ok(applied)
}

/// The files of `kept` shells in a version.
fn carried(version: &BTreeMap<PathBuf, String>, kept: &[Shell]) -> BTreeMap<PathBuf, String> {
    version
        .iter()
        .filter(|(rel, _)| kept.iter().any(|shell| rel.starts_with(shell.name())))
        .map(|(rel, text)| (rel.clone(), text.clone()))
        .collect()
}

fn check_and_smoke(
    ctx: &Context,
    plan: &Plan,
    layout: &Layout,
    files: &BTreeMap<PathBuf, String>,
    baselines: &[(Shell, Vec<Smoke>)],
) -> Result<()> {
    // dash and BusyBox ash exit on a parse error in a sourced file, so broken
    // snippets must never reach a shell.
    let mut broken = Vec::new();
    for found in plan
        .installed
        .iter()
        .filter(|found| plan.shells.contains(&found.shell))
    {
        for rel in files
            .keys()
            .filter(|rel| rel.starts_with(found.shell.name()) && is_raw(rel))
        {
            if let Some(message) =
                generated::syntax_error(&found.path, found.shell, &layout.version_dir.join(rel))
            {
                broken.push(format!(
                    "{} ({}): {}",
                    module_of(rel),
                    found.shell,
                    first_line(&message)
                ));
            }
        }
    }
    if !broken.is_empty() {
        return Err(Error::Apply(format!(
            "syntax errors, nothing applied:\n  {}",
            broken.join("\n  ")
        )));
    }

    hooks::commit(&plan.hook_changes, &ctx.snapshots(), "install hooks")?;

    let hook_env = plan.hook_env.as_ref().expect("plan has hook env");
    let mut failures = Vec::new();
    for found in plan
        .installed
        .iter()
        .filter(|found| plan.shells.contains(&found.shell))
    {
        if hooks::hooked_files(found.shell, hook_env)?.is_empty() {
            continue;
        }
        let baseline = match &plan.current {
            Some(id) => ctx.paths.generated().join(id),
            None => PathBuf::from("/nonexistent"),
        };
        let before = baselines
            .iter()
            .find(|(shell, _)| *shell == found.shell)
            .map_or_else(|| smoke(ctx, found, &baseline), |(_, smoke)| smoke.clone());
        let after = smoke(ctx, found, &layout.version_dir);
        // A line without a file name could come from any module this apply changed.
        let mut changed: Vec<String> = plan
            .file_changes
            .iter()
            .map(|(rel, _, _)| rel)
            .filter(|rel| rel.starts_with(found.shell.name()) && is_fragment(rel))
            .map(|rel| module_of(rel))
            .collect();
        changed.dedup();
        let suspects = match changed.as_slice() {
            [] => "unknown module".to_string(),
            [one] => format!("module {one}"),
            many => format!("one of modules {}", many.join(", ")),
        };
        for (before, after) in before.into_iter().zip(after) {
            let lines = match after {
                Smoke::Stderr(lines) => lines,
                Smoke::Stopped(lines) => {
                    // A shell that already stopped early (`exec tmux`) is not this apply's doing.
                    if !matches!(before, Smoke::Stopped(_)) {
                        failures.push(format!(
                            "{} ({suspects}): exited before startup finished",
                            found.shell
                        ));
                    }
                    lines
                }
                // Slow before this apply too, so not this apply's doing.
                Smoke::TimedOut if matches!(before, Smoke::TimedOut) => continue,
                Smoke::TimedOut => {
                    failures.push(format!(
                        "{}: did not finish starting within {}s",
                        found.shell,
                        SMOKE_TIMEOUT.as_secs()
                    ));
                    continue;
                }
            };
            let mut known = match before {
                Smoke::Stderr(lines) | Smoke::Stopped(lines) => lines,
                Smoke::TimedOut => Vec::new(),
            };
            for line in lines {
                let message = without_location(&line);
                if let Some(index) = known
                    .iter()
                    .position(|seen| without_location(seen) == message)
                {
                    known.remove(index);
                    continue;
                }
                let module = files
                    .keys()
                    .filter_map(|rel| rel.file_name()?.to_str())
                    .find(|name| line.contains(*name));
                let who = module
                    .map(|name| format!("module {}", module_of(Path::new(name))))
                    .unwrap_or_else(|| suspects.clone());
                failures.push(format!("{} ({who}): {line}", found.shell));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Error::Apply(format!(
            "new errors when starting shells, nothing applied:\n  {}",
            failures.join("\n  ")
        )))
    }
}

fn kind_of(rel: &Path) -> Option<&str> {
    let stem = rel.file_stem()?.to_str()?;
    stem.rsplit_once('-').map(|(_, kind)| kind)
}

/// An error line without the file and line it came from, so an error that
/// only moved (rc file to generated script) is not new:
/// `f: line 3: msg` (bash), `f: 3: msg` (dash), `f:3: msg`, `f:.:3: msg` (zsh).
fn without_location(line: &str) -> &str {
    let Some((file, rest)) = line.split_once(':') else {
        return line;
    };
    if file.contains(' ') || !(file.starts_with('/') || file.starts_with('~')) {
        return line;
    }
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("line ").unwrap_or(rest);
    let rest = match rest.split_once(':') {
        // zsh names the builtin between file and line: `f:.:3: msg`.
        Some((word, after))
            if !word.is_empty()
                && !word.contains(' ')
                && !word.chars().all(|c| c.is_ascii_digit()) =>
        {
            after
        }
        _ => rest,
    };
    match rest.split_once(':') {
        Some((digits, msg)) if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) => {
            msg.trim_start()
        }
        _ => line,
    }
}

fn is_raw(rel: &Path) -> bool {
    kind_of(rel) == Some("raw")
}

/// A module's fragment, as opposed to a loader such as `env.zsh`.
fn is_fragment(rel: &Path) -> bool {
    matches!(kind_of(rel), Some("env" | "alias" | "raw"))
}

/// `6-0050-ngrok-raw.zsh` -> `ngrok`.
fn module_of(rel: &Path) -> String {
    let stem = rel
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let without_kind = stem.rsplit_once('-').map_or(stem, |(rest, _)| rest);
    without_kind
        .splitn(3, '-')
        .nth(2)
        .unwrap_or(without_kind)
        .to_string()
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

/// What a smoke test runs once startup is done, printing `SMOKE_MARK`.
const SMOKE_COMMAND: &str = "echo rosamta-smoke-reached";
const SMOKE_MARK: &str = "rosamta-smoke-reached";

#[derive(Clone)]
enum Smoke {
    /// Started and ran the command; the error lines.
    Stderr(Vec<String>),
    /// Ended before the command, as after an `exit` in a module; the error lines.
    Stopped(Vec<String>),
    TimedOut,
}

/// Start the shell the way terminals do, pointed at `dir`: one result per
/// way, as bash and zsh read other files as login shells.
fn smoke(ctx: &Context, found: &Detected, dir: &Path) -> Vec<Smoke> {
    let ways: &[&[&str]] = match found.shell {
        Shell::Sh => &[&["-l", "-c", SMOKE_COMMAND]],
        Shell::Bash | Shell::Zsh => &[
            &["-i", "-c", SMOKE_COMMAND],
            &["-l", "-i", "-c", SMOKE_COMMAND],
        ],
        Shell::Fish => &[&["-i", "-c", SMOKE_COMMAND]],
    };
    ways.iter()
        .map(|args| {
            let mut cmd = std::process::Command::new(&found.path);
            cmd.args(*args)
                .env("ROSAMTA_CURRENT", dir)
                .env("TERM", "dumb")
                .env("HOME", &ctx.home)
                .env_remove("ROSAMTA_ENV")
                .env_remove("ROSAMTA_PROFILE");
            match output_with_timeout(&mut cmd, SMOKE_TIMEOUT) {
                Ok(Some(output)) => {
                    let lines = String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .map(str::to_string)
                        .filter(|line| !line.is_empty())
                        .collect();
                    // Startup output need not end in a newline (`printf` of a title).
                    let reached = String::from_utf8_lossy(&output.stdout).contains(SMOKE_MARK);
                    if reached {
                        Smoke::Stderr(lines)
                    } else {
                        Smoke::Stopped(lines)
                    }
                }
                Ok(None) => Smoke::TimedOut,
                Err(err) => Smoke::Stderr(vec![format!(
                    "could not start {}: {err}",
                    found.path.display()
                )]),
            }
        })
        .collect()
}

/// `rsmt hook [shell...]`: install hooks and stop treating the shells as
/// unhooked. No shells means every installed shell rosamta supports.
pub fn hook(ctx: &Context, shells: &[Shell]) -> Result<Vec<PathBuf>> {
    let installed = ctx.installed();
    let env = ctx.hook_env(&installed);
    let shells: Vec<Shell> = if shells.is_empty() {
        installed
            .iter()
            .filter(|found| found.unsupported().is_none())
            .map(|found| found.shell)
            .collect()
    } else {
        shells.to_vec()
    };
    let mut changes = Vec::new();
    for shell in &shells {
        let found = installed.iter().find(|found| found.shell == *shell);
        match found.map(Detected::unsupported) {
            None => return Err(Error::Usage(format!("{shell} is not installed"))),
            Some(Some(reason)) => return Err(Error::Usage(reason)),
            Some(None) => changes.extend(hooks::plan_install(*shell, &env)?),
        }
    }
    changes.retain(|change| !change.is_noop());
    hooks::commit(&changes, &ctx.snapshots(), "install hooks")?;
    let unhooked: Vec<Shell> = ctx
        .unhooked()
        .into_iter()
        .filter(|shell| !shells.contains(shell))
        .collect();
    ctx.set_unhooked(&unhooked)?;
    Ok(changes.into_iter().map(|change| change.path).collect())
}

/// `rsmt unhook <shell>`: remove only that shell's block, and remember it.
pub fn unhook(ctx: &Context, shell: Shell) -> Result<Vec<PathBuf>> {
    let env = ctx.hook_env(&ctx.installed());
    let changes = hooks::plan_remove(shell, &env)?;
    hooks::commit(&changes, &ctx.snapshots(), &format!("remove {shell} hook"))?;
    let mut unhooked = ctx.unhooked();
    unhooked.push(shell);
    ctx.set_unhooked(&unhooked)?;
    Ok(changes.into_iter().map(|change| change.path).collect())
}

/// What `rsmt eject` would write: each hook replaced by the code it loads.
pub fn plan_eject(ctx: &Context) -> Result<Vec<FileChange>> {
    let generated_dir = ctx.paths.generated();
    let Some(id) = generated::current_id(&generated_dir) else {
        return Err(Error::Usage(
            "nothing has been applied; there is nothing to eject".into(),
        ));
    };
    let version = generated_dir.join(&id);
    let env = ctx.hook_env(&ctx.installed());
    let stamp = format!("# Ejected from rosamta on {}.\n", today());
    let mut changes = Vec::new();
    for shell in Shell::ALL {
        if !version.join(shell.name()).is_dir() {
            // Hooked once, but no scripts now (uninstalled): the hook just goes.
            changes.extend(hooks::plan_remove(shell, &env)?);
            continue;
        }
        let inline = |stage: &str| inline(&version, shell, stage);
        // Once per process tree, as with the hooks.
        let env_once = || -> Result<String> {
            let text = inline("env")?;
            if text.is_empty() {
                return Ok(text);
            }
            let (guard, end_guard) = render::env_guard(shell, &id);
            Ok(format!("{guard}{text}{end_guard}"))
        };
        let interactive = |text: String| match shell {
            _ if text.is_empty() => text,
            Shell::Fish => format!("if status is-interactive\n{text}end\n"),
            _ => format!("if [[ $- == *i* ]]; then\n{text}fi\n"),
        };
        for path in hooks::hooked_files(shell, &env)? {
            let before = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let body = match (shell, name) {
                (Shell::Fish, _) | (Shell::Bash, ".bashrc") => {
                    format!("{}{}", env_once()?, interactive(inline("rc")?))
                }
                (Shell::Zsh, ".zshrc") => inline("rc")?,
                (Shell::Zsh, ".zprofile") => login(&version, shell)?,
                (Shell::Bash, _) => {
                    let block = hooks::find_block(&before)
                        .map(|(start, end)| &before[start..end])
                        .unwrap_or_default();
                    let keep = block
                        .lines()
                        .filter(|line| line.contains(".bashrc"))
                        .map(|line| format!("{line}\n"))
                        .collect::<String>();
                    format!("{}{}{keep}", env_once()?, login(&version, shell)?)
                }
                (Shell::Sh, _) => format!("{}{}", env_once()?, login(&version, shell)?),
                _ => env_once()?,
            };
            let content = format!("{stamp}{body}");
            let after = match (shell, hooks::find_block(&before)) {
                (Shell::Fish, _) => content,
                (_, Some((start, end))) => {
                    format!("{}{content}{}", &before[..start], &before[end..])
                }
                (_, None) => before.clone(),
            };
            changes.push(FileChange {
                path,
                before: Some(before),
                after: Some(after),
            });
        }
    }
    Ok(changes)
}

/// The fragments a loader sources, concatenated in its order.
fn inline(version: &Path, shell: Shell, stage: &str) -> Result<String> {
    let loader_path = version
        .join(shell.name())
        .join(format!("{stage}.{}", shell.ext()));
    let loader = std::fs::read_to_string(&loader_path).map_err(Error::read(&loader_path))?;
    let mut out = String::new();
    for path in loader.lines().filter_map(sourced) {
        out.push_str(&pasted(shell, &path)?);
    }
    Ok(out)
}

/// The login loader with its login snippets inline; the PATH lines stay as
/// they are.
fn login(version: &Path, shell: Shell) -> Result<String> {
    let path = version
        .join(shell.name())
        .join(format!("profile.{}", shell.ext()));
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = String::new();
    for line in without_header(&text).lines() {
        match sourced(line) {
            Some(path) => out.push_str(&pasted(shell, &path)?),
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    Ok(out)
}

/// A fragment as pasted into an rc file. One that may `return` goes in a
/// function, so `return` ends the fragment and not the whole rc file; the
/// rest stay as they are, as `typeset` in a function makes locals.
fn pasted(shell: Shell, path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path).map_err(Error::read(path))?;
    let text = without_header(&text);
    let returns = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .flat_map(|line| line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
        .any(|word| word == "return");
    Ok(if shell == Shell::Fish || !returns {
        text.to_string()
    } else {
        format!("_rosamta_fragment() {{\n{text}}}\n_rosamta_fragment\nunset -f _rosamta_fragment\n")
    })
}

/// The fragment a loader line sources: loaders write `. '<path>'`
/// (fish: `source '<path>'`).
fn sourced(line: &str) -> Option<PathBuf> {
    let line = line.trim();
    let quoted = line
        .strip_prefix(". ")
        .or_else(|| line.strip_prefix("source "))?;
    let path = quoted.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(PathBuf::from(
        path.replace(r"'\''", "'").replace(r"\'", "'"),
    ))
}

/// Ejected code is the user's to edit, so the "generated" notice goes.
fn without_header(text: &str) -> &str {
    text.strip_prefix(render::HEADER).unwrap_or(text)
}

/// Write the plan and stop hooking every shell. The caller disables the
/// background service, which on macOS needs the app bundle.
/// Returns the dotfiles that were links and are now regular files.
pub fn eject(ctx: &Context, changes: &[FileChange]) -> Result<Vec<PathBuf>> {
    // Links first, so nothing below writes through one into the repo.
    let released = crate::dotfiles::manage::eject(ctx)?;
    hooks::commit(changes, &ctx.snapshots(), "eject")?;
    ctx.set_unhooked(&Shell::ALL)?;
    Ok(released)
}

/// What `rsmt restore --original` would write.
pub fn plan_restore(ctx: &Context) -> Result<Vec<FileChange>> {
    let mut changes = Vec::new();
    for original in ctx.snapshots().originals()? {
        let before = match std::fs::read(&original.path) {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            // A dotfile directory that did not exist before rosamta, or a dangling link.
            Err(_) if original.path.is_dir() || original.path.is_symlink() => Some(String::new()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                return Err(Error::Read {
                    path: original.path,
                    source: err,
                });
            }
        };
        // A link or mode that differs is a change even when the bytes match.
        let kind_differs = (!legacy(&original)
            && std::fs::read_link(&original.path).ok() != original.link)
            || original.mode.is_some_and(|mode| {
                std::fs::metadata(&original.path)
                    .is_ok_and(|meta| meta.permissions().mode() & 0o7777 != mode)
            });
        let after = match (original.content, &original.link) {
            (Some(bytes), _) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            (None, Some(_)) => Some(String::new()),
            (None, None) => None,
        };
        let change = FileChange {
            path: original.path,
            before,
            after,
        };
        if kind_differs || !change.is_noop() {
            changes.push(change);
        }
    }
    Ok(changes)
}

/// Put every file back byte for byte, and stop hooking every shell.
pub fn restore_original(ctx: &Context) -> Result<Vec<PathBuf>> {
    let snapshots = ctx.snapshots();
    let originals = snapshots.originals()?;
    let paths: Vec<PathBuf> = originals
        .iter()
        .map(|original| original.path.clone())
        .collect();
    // rosamta's links go first: the repo keeps what they show, and putting
    // files back must not write through them.
    crate::dotfiles::manage::unlink(ctx)?;
    let mut current = Vec::new();
    for path in &paths {
        current.extend(crate::dotfiles::tree::files(path)?);
    }
    snapshots.save(&current, "restore original")?;
    for original in originals {
        put_back(&original)?;
    }
    ctx.set_unhooked(&Shell::ALL)?;
    Ok(paths)
}

/// Taken before snapshots kept links and modes: its bytes are written
/// through whatever link is there, as they were then.
fn legacy(original: &crate::snapshot::Original) -> bool {
    original.content.is_some() && original.mode.is_none() && original.link.is_none()
}

/// One original: its link, then its bytes, written through the link only
/// when the link was there before rosamta.
fn put_back(original: &crate::snapshot::Original) -> Result<()> {
    let path = &original.path;
    let now_link = std::fs::read_link(path).ok();
    if original.link.is_some() && now_link != original.link {
        crate::dotfiles::tree::remove(path)?;
    } else if original.link.is_none() && now_link.is_some() && !legacy(original) {
        std::fs::remove_file(path).map_err(Error::write(path))?;
    }
    if (original.link.is_some() || original.content.is_some())
        && let Some(dir) = path.parent()
    {
        std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
    }
    if let Some(link) = &original.link
        && std::fs::symlink_metadata(path).is_err()
    {
        std::os::unix::fs::symlink(link, path).map_err(Error::write(path))?;
    }
    let Some(bytes) = &original.content else {
        return match original.link {
            Some(_) => Ok(()),
            None => crate::dotfiles::tree::remove(path),
        };
    };
    if std::fs::read(path).ok().as_ref() == Some(bytes) {
        return match original.mode {
            Some(mode) => crate::dotfiles::tree::set_mode(path, mode),
            None => Ok(()),
        };
    }
    // Through a link the bytes belong to the file at the end of its chain.
    let target = match original.link.is_some() || legacy(original) {
        true => chain_end(path)?,
        false => path.clone(),
    };
    let mode = original
        .mode
        .or_else(|| crate::dotfiles::tree::mode_of(&target))
        .unwrap_or(0o644);
    // A fresh owner-only file swapped in: the bytes never sit in a file others
    // can read, and a link put there meanwhile is replaced, not followed.
    crate::dotfiles::tree::put(crate::dotfiles::tree::Content::Bytes(bytes), &target)?;
    crate::dotfiles::tree::set_mode(&target, mode)
}

/// The path a link chain from `path` ends at, even when no file is there.
fn chain_end(path: &Path) -> Result<PathBuf> {
    let mut at = path.to_path_buf();
    // As many hops as the kernel follows.
    for _ in 0..40 {
        let Ok(link) = std::fs::read_link(&at) else {
            return Ok(at);
        };
        at = at.parent().expect("a link has a parent").join(link);
    }
    Err(Error::Usage(format!("{} links in a loop", path.display())))
}

/// One shell on this host, for status screens.
#[derive(Debug, Clone)]
pub struct ShellStatus {
    pub shell: Shell,
    pub installed: bool,
    /// Major and minor version, when the shell reports one.
    pub version: Option<(u32, u32)>,
    /// Startup files holding rosamta's block.
    pub hooked: Vec<PathBuf>,
}

/// Every supported shell: whether it is installed and where it is hooked.
pub fn shells(ctx: &Context) -> Result<Vec<ShellStatus>> {
    let installed = ctx.installed();
    let env = ctx.hook_env(&installed);
    Shell::ALL
        .iter()
        .map(|&shell| {
            let found = installed.iter().find(|found| found.shell == shell);
            Ok(ShellStatus {
                shell,
                installed: found.is_some(),
                version: found.and_then(|found| found.version),
                hooked: hooks::hooked_files(shell, &env)?,
            })
        })
        .collect()
}

/// Startup files that lost the block of a shell the current version serves:
/// those shells no longer load rosamta.
pub fn missing_hooks(ctx: &Context, env: &HookEnv) -> Result<Vec<PathBuf>> {
    let generated_dir = ctx.paths.generated();
    let Some(id) = generated::current_id(&generated_dir) else {
        return Ok(Vec::new());
    };
    let unhooked = ctx.unhooked();
    let mut out = Vec::new();
    for found in ctx.installed() {
        if found.unsupported().is_none()
            && !unhooked.contains(&found.shell)
            && generated_dir.join(&id).join(found.shell.name()).is_dir()
        {
            out.extend(hooks::missing(found.shell, env)?);
        }
    }
    Ok(out)
}

/// Today's UTC date as YYYY-MM-DD.
fn today() -> String {
    crate::time::date(crate::time::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_names_come_back_from_file_names() {
        assert_eq!(module_of(Path::new("zsh/6-0050-ngrok-raw.zsh")), "ngrok");
        assert_eq!(
            module_of(Path::new("6-0010-zsh-syntax-highlighting-raw.zsh")),
            "zsh-syntax-highlighting"
        );
        assert!(is_raw(Path::new("zsh/6-0050-ngrok-raw.zsh")));
        assert!(!is_raw(Path::new("zsh/0-0050-path-env.zsh")));
        assert!(is_fragment(Path::new("zsh/0-0050-path-env.zsh")));
        assert!(!is_fragment(Path::new("zsh/env.zsh")));
    }

    #[test]
    fn errors_that_only_moved_compare_equal() {
        let msg = "/h/.cargo/env: No such file or directory";
        for line in [
            "/h/.bashrc: line 3: /h/.cargo/env: No such file or directory",
            "/g/v-1/bash/0-1-cargo-raw.bash: line 5: /h/.cargo/env: No such file or directory",
            "/h/.profile: 3: /h/.cargo/env: No such file or directory",
        ] {
            assert_eq!(without_location(line), msg, "{line}");
        }
        assert_eq!(
            without_location("/h/.zshenv:.:1: no such file or directory: /x"),
            "no such file or directory: /x"
        );
        assert_eq!(
            without_location("/h/.zshrc:12: command not found: x"),
            "command not found: x"
        );
        assert_eq!(
            without_location("bash: x: command not found"),
            "bash: x: command not found"
        );
        assert_eq!(without_location("plain message"), "plain message");
    }

    #[test]
    fn today_is_a_date() {
        let date = today();
        assert_eq!(date.len(), 10);
        assert!(date.starts_with("20"), "{date}");
    }
}
