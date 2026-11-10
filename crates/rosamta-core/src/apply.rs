//! `rsmt apply`, `hook`, `unhook`, `eject` and `restore --original`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{HostFacts, Module, Shell};
use crate::error::{Error, Result};
use crate::generated::{self, CURRENT};
use crate::hooks::{self, FileChange, HookEnv};
use crate::paths::Paths;
use crate::process::output_with_timeout;
use crate::render::{self, Layout};
use crate::shells::{self, Detected};
use crate::snapshot::Snapshots;
use crate::{host, repo, validate};

const SMOKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything apply needs to know about this host.
pub struct Context {
    pub paths: Paths,
    pub home: PathBuf,
    pub path_var: String,
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
        })
    }

    fn snapshots(&self) -> Snapshots {
        Snapshots::new(self.paths.state(), &self.home)
    }

    fn installed(&self) -> Vec<Detected> {
        shells::detect(&self.path_var, &self.paths.state().join("shells.toml"))
    }

    fn hook_env(&self, installed: &[Detected]) -> HookEnv {
        let exe = |shell| {
            installed
                .iter()
                .find(|found| found.shell == shell)
                .map(|found| found.path.as_path())
        };
        HookEnv::detect(&self.home, exe(Shell::Zsh), exe(Shell::Fish))
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

/// Parse every module; any error sends the user to `rsmt validate`.
pub fn load_modules(repo_dir: &Path) -> Result<Vec<(String, Module)>> {
    let mut modules = Vec::new();
    for path in repo::module_files(repo_dir)? {
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
        modules.push((repo::module_name(&path), module));
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
    let modules = render::select(load_modules(&repo_dir)?, &facts);

    let installed = ctx.installed();
    let skipped = installed.iter().filter_map(Detected::unsupported).collect();
    let shells: Vec<Shell> = installed
        .iter()
        .filter(|found| found.unsupported().is_none())
        .map(|found| found.shell)
        .collect();

    let generated_dir = ctx.paths.generated();
    let current = generated::current_id(&generated_dir);
    // Render against the current version's paths, so unchanged modules compare equal.
    let id = current.clone().unwrap_or_else(generated::new_version_id);
    let layout = generated::layout(&generated_dir, ctx.paths.state(), &id);
    let files = render::render(&modules, &shells, &layout);
    let old = match &current {
        Some(id) => generated::read_version(&generated_dir, id)?,
        None => BTreeMap::new(),
    };
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
    let old = if before.is_some() {
        format!("a/{name}")
    } else {
        "/dev/null".into()
    };
    let new = if after.is_some() {
        format!("b/{name}")
    } else {
        "/dev/null".into()
    };
    diff.unified_diff().header(&old, &new).to_string()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Applied {
    pub version: Option<String>,
    pub up_to_date: bool,
    pub shells: Vec<Shell>,
    pub skipped: Vec<String>,
    pub hooks_changed: Vec<PathBuf>,
}

/// Render, smoke-test, then switch. On any failure the current version stays.
pub fn apply(ctx: &Context) -> Result<Applied> {
    let repo_dir = ctx.paths.repo();
    if repo_dir.join(repo::MANIFEST).is_file() {
        repo::migrate(&repo_dir, crate::config::CURRENT_FORMAT, repo::MIGRATIONS)?;
    }
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
    };
    if applied.up_to_date {
        return Ok(applied);
    }

    let generated_dir = ctx.paths.generated();
    let id = generated::new_version_id();
    let layout = generated::layout(&generated_dir, ctx.paths.state(), &id);
    let facts = host_facts(&ctx.paths)?;
    let modules = render::select(load_modules(&repo_dir)?, &facts);
    let files = render::render(&modules, &plan.shells, &layout);
    generated::write_version(&layout, &files)?;

    let result = check_and_smoke(ctx, &plan, &layout, &files);
    if let Err(err) = result {
        generated::remove_version(&layout);
        return Err(err);
    }
    generated::switch(&generated_dir, &id)?;
    generated::prune(&generated_dir)?;
    applied.version = Some(id);
    Ok(applied)
}

fn check_and_smoke(
    ctx: &Context,
    plan: &Plan,
    layout: &Layout,
    files: &BTreeMap<PathBuf, String>,
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
        let before = smoke(ctx, found, &baseline);
        let after = smoke(ctx, found, &layout.version_dir);
        match after {
            Smoke::TimedOut => failures.push(format!(
                "{}: did not finish starting within {}s",
                found.shell,
                SMOKE_TIMEOUT.as_secs()
            )),
            Smoke::Stderr(lines) => {
                let mut known = match before {
                    Smoke::Stderr(lines) => lines,
                    Smoke::TimedOut => Vec::new(),
                };
                for line in lines {
                    if let Some(index) = known.iter().position(|seen| *seen == line) {
                        known.remove(index);
                        continue;
                    }
                    let module = files
                        .keys()
                        .filter_map(|rel| rel.file_name()?.to_str())
                        .find(|name| line.contains(*name));
                    let who = module
                        .map(|name| format!("module {}", module_of(Path::new(name))))
                        .unwrap_or_else(|| "unknown module".into());
                    failures.push(format!("{} ({who}): {line}", found.shell));
                }
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

fn is_raw(rel: &Path) -> bool {
    rel.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.ends_with("-raw"))
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

enum Smoke {
    Stderr(Vec<String>),
    TimedOut,
}

/// Start the shell the way a terminal would, pointed at `dir`.
fn smoke(ctx: &Context, found: &Detected, dir: &Path) -> Smoke {
    let mut cmd = std::process::Command::new(&found.path);
    match found.shell {
        Shell::Sh => cmd.args(["-l", "-c", "exit"]),
        _ => cmd.args(["-i", "-c", "exit"]),
    };
    cmd.env("ROSAMTA_CURRENT", dir)
        .env("TERM", "dumb")
        .env("HOME", &ctx.home)
        .env_remove("ROSAMTA_ENV")
        .env_remove("ROSAMTA_PROFILE");
    match output_with_timeout(&mut cmd, SMOKE_TIMEOUT) {
        Ok(Some(output)) => Smoke::Stderr(
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .map(str::to_string)
                .filter(|line| !line.is_empty())
                .collect(),
        ),
        Ok(None) => Smoke::TimedOut,
        Err(err) => Smoke::Stderr(vec![format!(
            "could not start {}: {err}",
            found.path.display()
        )]),
    }
}

/// `rsmt hook [shell...]`: install hooks and stop treating the shells as unhooked.
pub fn hook(ctx: &Context, shells: &[Shell]) -> Result<Vec<PathBuf>> {
    let installed = ctx.installed();
    let env = ctx.hook_env(&installed);
    let mut changes = Vec::new();
    for shell in shells {
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
        let inline = |stage: &str| inline(&version, shell, stage);
        for path in hooks::hooked_files(shell, &env)? {
            let before = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let body = match (shell, name) {
                (Shell::Fish, _) => format!(
                    "{}if status is-interactive\n{}end\n",
                    inline("env")?,
                    inline("rc")?
                ),
                (Shell::Zsh, ".zshrc") => inline("rc")?,
                (Shell::Zsh, ".zprofile") => {
                    std::fs::read_to_string(version.join("zsh/profile.zsh")).unwrap_or_default()
                }
                (Shell::Bash, ".bashrc") => format!(
                    "{}if [[ $- == *i* ]]; then\n{}fi\n",
                    inline("env")?,
                    inline("rc")?
                ),
                (Shell::Bash, _) => {
                    let block = hooks::find_block(&before)
                        .map(|(start, end)| &before[start..end])
                        .unwrap_or_default();
                    let keep = block
                        .lines()
                        .filter(|line| line.contains(".bashrc"))
                        .map(|line| format!("{line}\n"))
                        .collect::<String>();
                    format!("{}{keep}", inline("env")?)
                }
                _ => inline("env")?,
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
    // The plain branch lists each fragment as `. '<path>'` (fish: `source '<path>'`).
    for line in loader.lines() {
        let quoted = line
            .trim()
            .strip_prefix(". ")
            .or_else(|| line.trim().strip_prefix("source "));
        let Some(path) = quoted
            .and_then(|q| q.strip_prefix('\''))
            .and_then(|q| q.strip_suffix('\''))
        else {
            continue;
        };
        let path = PathBuf::from(path.replace(r"'\''", "'").replace(r"\'", "'"));
        out.push_str(&std::fs::read_to_string(&path).map_err(Error::read(&path))?);
    }
    Ok(out)
}

pub fn eject(ctx: &Context, changes: &[FileChange]) -> Result<()> {
    hooks::commit(changes, &ctx.snapshots(), "eject")?;
    // The background service does not exist yet; add-daemon stops it here.
    ctx.set_unhooked(&Shell::ALL)
}

/// What `rsmt restore --original` would write.
pub fn plan_restore(ctx: &Context) -> Result<Vec<FileChange>> {
    let mut changes = Vec::new();
    for original in ctx.snapshots().originals()? {
        let before = match std::fs::read(&original.path) {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                return Err(Error::Read {
                    path: original.path,
                    source: err,
                });
            }
        };
        let after = original
            .content
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        changes.push(FileChange {
            path: original.path,
            before,
            after,
        });
    }
    changes.retain(|change| !change.is_noop());
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
    snapshots.save(&paths, "restore original")?;
    for original in originals {
        match original.content {
            Some(bytes) => {
                if let Some(dir) = original.path.parent() {
                    std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
                }
                std::fs::write(&original.path, bytes).map_err(Error::write(&original.path))?;
            }
            None => match std::fs::remove_file(&original.path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                    return Err(Error::Write {
                        path: original.path,
                        source: err,
                    });
                }
                _ => {}
            },
        }
    }
    ctx.set_unhooked(&Shell::ALL)?;
    Ok(paths)
}

/// Today's UTC date as YYYY-MM-DD.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = (secs / 86_400) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
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
    }

    #[test]
    fn today_is_a_date() {
        let date = today();
        assert_eq!(date.len(), 10);
        assert!(date.starts_with("20"), "{date}");
    }
}
