//! The background service (`rsmt daemon run`): adopt, apply and sync when
//! rc files, the repo or `host.toml` change, and on a schedule. It talks to
//! the CLI and GUI only through files and the adopt lock.

pub mod env;
pub mod instance;
pub mod notify;
pub mod service;

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};

use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, new_debouncer};

use crate::adopt::{self, Scope};
use crate::apply::{self, Context};
use crate::config::{HostConfig, SyncPolicy};
use crate::error::Result;
use crate::hooks::HookEnv;
use crate::i18n::{self, Lang};
use crate::sync::{self, Outcome};
use crate::{generated, host, validate};

pub use notify::{Action, Note, Notifier};

pub enum Event {
    /// Paths changed under a watched directory, after the debounce.
    Changed(Vec<PathBuf>),
    /// A click on a review notification.
    Action(Action, Vec<u32>),
}

/// Why [`run`] returned.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// The executable at this path was replaced; start the new one.
    Restart(PathBuf),
}

/// Quiet time before a burst of file events counts as one change.
const DEBOUNCE: Duration = Duration::from_secs(2);
/// New review items within this window share one notification.
const BATCH: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_secs(5);
/// Wall time running this far ahead of monotonic time means the host slept.
const SLEPT: Duration = Duration::from_secs(60);
/// After waking, the network often takes a while to come back.
const WAKE_RETRIES: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(120)];
const LOG_LIMIT: u64 = 1 << 20;

/// Directories to watch: the parents of watched files, without recursion,
/// so an installer writing a new file and renaming it over the old one is
/// seen; the repo and the config directory, recursively.
fn watch_list(ctx: &Context, env: &HookEnv) -> Vec<(PathBuf, RecursiveMode)> {
    let mut out: Vec<(PathBuf, RecursiveMode)> = Vec::new();
    for dir in [
        env.home.clone(),
        env.zshenv_dir.clone(),
        env.zdotdir.clone(),
        env.fish_config.clone(),
        env.fish_config.join("conf.d"),
    ] {
        let dir = real(&dir);
        if dir.is_dir() && !out.iter().any(|(seen, _)| *seen == dir) {
            out.push((dir, RecursiveMode::NonRecursive));
        }
    }
    let config = config_dir(ctx);
    for dir in [ctx.paths.repo(), config] {
        if dir.is_dir() {
            out.push((real(&dir), RecursiveMode::Recursive));
        }
    }
    out
}

/// The path with symlinks resolved, as file events report it (macOS's
/// `/var` is `/private/var`); the path itself when it does not exist.
fn real(path: &Path) -> PathBuf {
    if let Ok(path) = std::fs::canonicalize(path) {
        return path;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => real(parent).join(name),
        _ => path.to_path_buf(),
    }
}

fn config_dir(ctx: &Context) -> PathBuf {
    let host = ctx.paths.host_config();
    host.parent().expect("host.toml has a parent").to_path_buf()
}

/// Content hashes of `paths`, for telling a real change from a touch or
/// from rosamta's own write.
fn fingerprint<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for path in paths {
        path.hash(&mut hasher);
        std::fs::read(path).ok().hash(&mut hasher);
    }
    hasher.finish()
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

type Identity = (u64, u64, SystemTime);

/// The file at `path`, to notice when an upgrade replaces it.
fn identity(path: &Path) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.dev(), meta.ino(), meta.modified().ok()?))
}

/// The executable's path as started. Asked again after an upgrade, Linux
/// would answer with the old file, now `... (deleted)`.
fn exe_identity() -> Option<(PathBuf, Identity)> {
    let path = std::env::current_exe().ok()?;
    let identity = identity(&path)?;
    Some((path, identity))
}

/// Which handlers a batch of changed paths reaches.
#[derive(Debug, Default, PartialEq, Eq)]
struct Routes {
    host: bool,
    repo: bool,
    rc: bool,
}

/// `rc` lists the watched rc files, as [`Daemon::rc_files`] names them.
fn route(ctx: &Context, rc: &[PathBuf], paths: &[PathBuf]) -> Routes {
    let repo = real(&ctx.paths.repo());
    let local = real(&ctx.paths.local());
    let host = real(&ctx.paths.host_config());
    let in_repo = |path: &PathBuf| {
        (path.starts_with(&repo) && !path.starts_with(repo.join(".git")))
            || path.starts_with(&local)
    };
    Routes {
        host: paths.contains(&host),
        repo: paths.iter().any(in_repo),
        // Not everything in HOME: shell history changes after each command.
        rc: paths.iter().any(|path| rc.contains(path)),
    }
}

/// The path of a new executable that replaced the running one.
fn replaced(exe: &Option<(PathBuf, Identity)>) -> Option<Stop> {
    let (path, started) = exe.as_ref()?;
    identity(path)
        .is_some_and(|now| now != *started)
        .then(|| Stop::Restart(path.clone()))
}

fn set_up(ctx: &Context) -> bool {
    ctx.paths.host_config().is_file() && ctx.paths.repo().join(".git").exists()
}

/// Sleep until `ready`; `Some` when the executable was replaced meanwhile.
fn wait_until(exe: &Option<(PathBuf, Identity)>, ready: impl Fn() -> bool) -> Option<Stop> {
    while !ready() {
        std::thread::sleep(TICK);
        if let Some(stop) = replaced(exe) {
            return Some(stop);
        }
    }
    None
}

struct Daemon {
    ctx: Context,
    host: HostConfig,
    lang: Lang,
    notifier: Box<dyn Notifier>,
    /// Where the shells read rc files; asking them is slow.
    env: HookEnv,
    /// Watched rc files as of the last scan.
    rc_seen: Option<u64>,
    /// Which files those were, so a deleted one is noticed.
    rc_known: Vec<PathBuf>,
    /// Repo and local modules as of the last apply.
    repo_seen: Option<u64>,
    /// Review items already announced, also before a restart.
    announced: BTreeSet<u32>,
    batch: Vec<u32>,
    batch_since: Option<Instant>,
    next_sync: Instant,
    /// Retries left for a sync after waking that found no network.
    wake_retries: &'static [Duration],
    clock: (Instant, SystemTime),
    exe: Option<(PathBuf, Identity)>,
    /// A problem found by applying the repo: kind and reason.
    repo_problem: Option<(&'static str, String)>,
}

/// Watch and react until the executable is replaced or the event channel
/// closes. `events` must be the sender paired with `receiver`.
pub fn run(
    ctx: Context,
    notifier: Box<dyn Notifier>,
    events: Sender<Event>,
    receiver: Receiver<Event>,
) -> Result<Stop> {
    let exe = exe_identity();
    // Started at login before `rsmt init`, or with a broken host.toml: wait
    // instead of failing over and over under launchd's KeepAlive.
    if !set_up(&ctx) {
        log(&ctx, "waiting for `rsmt init`");
        if let Some(stop) = wait_until(&exe, || set_up(&ctx)) {
            return Ok(stop);
        }
    }
    let host_config = [ctx.paths.host_config()];
    let host = loop {
        let seen = fingerprint(&host_config);
        match host::load(&host_config[0]) {
            Ok(host) => break host,
            Err(err) => {
                log(&ctx, &format!("waiting for host.toml to be fixed: {err}"));
                if let Some(stop) = wait_until(&exe, || fingerprint(&host_config) != seen) {
                    return Ok(stop);
                }
            }
        }
    };
    let watcher_events = events.clone();
    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        if let Ok(batch) = result {
            let paths = batch
                .into_iter()
                .flat_map(|event| event.event.paths)
                .collect();
            let _ = watcher_events.send(Event::Changed(paths));
        }
    })
    .map_err(|err| crate::Error::Daemon(format!("cannot watch files: {err}")))?;
    let mut daemon = Daemon {
        exe,
        ..Daemon::new(ctx, host, notifier)
    };
    for (dir, mode) in watch_list(&daemon.ctx, &daemon.env) {
        if let Err(err) = debouncer.watch(&dir, mode) {
            daemon.log(&format!("cannot watch {}: {err}", dir.display()));
        }
    }
    drop(events);
    daemon.log("started");
    // Catch up on whatever changed while the daemon was not running.
    daemon.repo_changed();
    daemon.rc_changed();
    loop {
        match receiver.recv_timeout(TICK) {
            Ok(Event::Changed(paths)) => daemon.changed(&paths),
            Ok(Event::Action(action, ids)) => daemon.act(action, &ids),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if let Some(stop) = daemon.tick() {
            log(&daemon.ctx, "restarting for the new version");
            return Ok(stop);
        }
    }
    drop(debouncer);
    Err(crate::Error::Daemon("the file watcher stopped".into()))
}

fn log(ctx: &Context, message: &str) {
    log_to(ctx.paths.state(), message);
}

/// Append to `daemon.log` in the state directory, starting over past 1 MiB.
pub fn log_to(state: &Path, message: &str) {
    let path = state.join("daemon.log");
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_LIMIT) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(
            file,
            "{} {message}",
            crate::time::date_time(crate::time::now())
        );
    }
}

impl Daemon {
    fn new(ctx: Context, host: HostConfig, notifier: Box<dyn Notifier>) -> Self {
        let previous = instance::load(ctx.paths.state()).ok().flatten();
        Self {
            next_sync: Instant::now(),
            wake_retries: &[],
            host,
            lang: Lang::system(),
            notifier,
            env: ctx.hook_env(&ctx.installed()),
            rc_seen: None,
            rc_known: Vec::new(),
            repo_seen: None,
            announced: previous
                .map_or_else(BTreeSet::new, |state| state.announced.into_iter().collect()),
            batch: Vec::new(),
            batch_since: None,
            clock: (Instant::now(), SystemTime::now()),
            exe: None,
            repo_problem: None,
            ctx,
        }
    }

    fn log(&self, message: &str) {
        log(&self.ctx, message);
    }

    fn changed(&mut self, paths: &[PathBuf]) {
        let mut rc = self.rc_files();
        rc.extend_from_slice(&self.rc_known);
        let routes = route(&self.ctx, &rc, paths);
        if routes.host {
            let host = self.ctx.paths.host_config();
            match host::load(&host) {
                Ok(config) => {
                    // A new interval or pull policy takes effect now.
                    self.next_sync = Instant::now();
                    self.host = config;
                }
                Err(err) => self.log(&format!("keeping the previous settings: {err}")),
            }
        }
        if routes.repo {
            self.repo_changed();
        }
        if routes.rc {
            self.rc_changed();
        }
    }

    /// Watched rc files that exist, named as file events name them: the
    /// directory resolved, not the file, which may be a symlink.
    fn rc_files(&self) -> Vec<PathBuf> {
        adopt::watched_files(&self.env)
            .into_iter()
            .map(
                |watched| match (watched.path.parent(), watched.path.file_name()) {
                    (Some(dir), Some(name)) => real(dir).join(name),
                    _ => watched.path,
                },
            )
            .collect()
    }

    /// Scan rc files and act on the adopt policy; announce new review items.
    fn rc_changed(&mut self) {
        // Taken before the scan: a write during the scan then still differs
        // from it when its own event arrives.
        let mut files = self.rc_files();
        let mut seen = fingerprint(&files);
        if self.rc_seen == Some(seen) {
            return;
        }
        let result = (|| {
            let lock = adopt::lock(&self.ctx)?;
            let reviewed = adopt::refresh(&self.ctx)?;
            if !reviewed.adopted.is_empty() {
                // Adopting rewrote the files; that write is rosamta's own,
                // an edit made while the push runs is not.
                files = self.rc_files();
                seen = fingerprint(&files);
                if let Err(err) = sync::publish_if_auto(&self.ctx, &lock) {
                    self.log(&err.to_string());
                }
            }
            Ok::<_, crate::Error>(reviewed)
        })();
        match result {
            Ok(reviewed) => {
                self.rc_seen = Some(seen);
                self.rc_known = files;
                self.log(&format!(
                    "scanned rc files: {} adopted, {} to review",
                    reviewed.adopted.len(),
                    reviewed.items.len()
                ));
                for item in reviewed.items {
                    if self.announced.insert(item.id) {
                        self.batch.push(item.id);
                        self.batch_since.get_or_insert_with(Instant::now);
                    }
                }
            }
            // Not counted as seen: the next change or repo apply scans again.
            Err(err) => self.log(&format!("scan failed: {err}")),
        }
        self.report_problems();
    }

    fn repo_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        files_under(&self.ctx.paths.repo(), &mut files);
        files_under(&self.ctx.paths.local(), &mut files);
        files.sort();
        files
    }

    /// Validate and apply the repo, then publish per the publish policy.
    fn repo_changed(&mut self) {
        let seen = fingerprint(&self.repo_files());
        if self.repo_seen == Some(seen) {
            return;
        }
        self.repo_seen = Some(seen);
        self.repo_problem = self.apply_repo().err();
        // Edits by hand are committed and pushed only after they apply.
        if self.repo_problem.is_none()
            && self.host.publish == SyncPolicy::Auto
            && let Err(err) = sync::publish(&self.ctx)
            && !err.is_usage()
        {
            self.log(&err.to_string());
        }
        self.report_problems();
        // Applying may have hooked another shell, and a scan that failed on
        // the previous repo can work now.
        if self.repo_problem.is_none() {
            self.env = self.ctx.hook_env(&self.ctx.installed());
            self.rc_changed();
        }
    }

    fn apply_repo(&self) -> std::result::Result<(), (&'static str, String)> {
        let repo = self.ctx.paths.repo();
        let report = validate::validate(&repo).map_err(|err| ("invalid", err.to_string()))?;
        if let Some((file, issue)) = report
            .files
            .iter()
            .find_map(|file| Some((file, file.errors.first()?)))
        {
            let line = issue
                .line
                .map(|line| format!(":{line}"))
                .unwrap_or_default();
            return Err((
                "invalid",
                format!("{}{line}: {}", file.path.display(), issue.message),
            ));
        }
        // A host that never applied keeps its shells as they are.
        if generated::current_id(&self.ctx.paths.generated()).is_none() {
            return Ok(());
        }
        let _lock = adopt::lock(&self.ctx).map_err(|err| ("apply_failed", err.to_string()))?;
        apply::apply(&self.ctx)
            .map(drop)
            .map_err(|err| ("apply_failed", err.to_string()))
    }

    fn sync_now(&mut self) {
        if self.host.pull != SyncPolicy::Auto {
            return;
        }
        match sync::sync(&self.ctx) {
            Ok(_) => {}
            // No remote yet, or no repo: nothing to sync with.
            Err(err) if err.is_usage() => {}
            Err(err) => self.log(&err.to_string()),
        }
        self.report_problems();
    }

    fn act(&mut self, action: Action, ids: &[u32]) {
        self.log(&format!("clicked {} for {ids:?}", action.id()));
        let mut seen = None;
        let result = (|| {
            let lock = adopt::lock(&self.ctx)?;
            for &id in ids {
                let done = match action {
                    Action::Adopt => adopt::accept(&self.ctx, id, Scope::Host).map(drop),
                    Action::Ignore => adopt::ignore(&self.ctx, id).map(drop),
                    Action::Review => Ok(()),
                };
                // Already handled from the CLI or GUI.
                if let Err(err) = done {
                    self.log(&format!("{} {id}: {err}", action.id()));
                }
            }
            if action == Action::Adopt {
                // Adopting rewrote the files; that write is rosamta's own.
                seen = Some(fingerprint(&self.rc_files()));
                sync::publish_if_auto(&self.ctx, &lock)?;
            }
            Ok::<_, crate::Error>(())
        })();
        if let Err(err) = result {
            self.log(&err.to_string());
        }
        if action == Action::Review
            && let Some(id) = ids.first()
        {
            open(&format!("rosamta://review/{id}"));
        }
        if seen.is_some() {
            self.rc_seen = seen;
        }
        self.report_problems();
    }

    /// Scheduled work: sync on the interval or after sleep, announce the
    /// batch, and notice an upgrade.
    fn tick(&mut self) -> Option<Stop> {
        let (mono, wall) = self.clock;
        let now = (Instant::now(), SystemTime::now());
        let wall_elapsed = now.1.duration_since(wall).unwrap_or_default();
        let woke = wall_elapsed > now.0.duration_since(mono) + SLEPT;
        self.clock = now;
        if woke {
            self.wake_retries = &WAKE_RETRIES;
        }
        if woke || now.0 >= self.next_sync {
            self.next_sync = now.0 + Duration::from_secs(u64::from(self.host.pull_interval) * 60);
            let started = crate::time::now();
            self.sync_now();
            match self.wake_retries.split_first() {
                Some((delay, rest)) if self.offline_since(started) => {
                    self.next_sync = now.0 + *delay;
                    self.wake_retries = rest;
                }
                _ => self.wake_retries = &[],
            }
        }
        if self
            .batch_since
            .is_some_and(|since| since.elapsed() >= BATCH)
        {
            self.announce();
        }
        replaced(&self.exe)
    }

    /// Whether a sync from `started` on could not reach the remote.
    fn offline_since(&self, started: u64) -> bool {
        sync::load_state(self.ctx.paths.state()).is_ok_and(|state| {
            [state.publish, state.pull]
                .into_iter()
                .flatten()
                .any(|attempt| attempt.at >= started && attempt.outcome == Outcome::Offline)
        })
    }

    fn announce(&mut self) {
        let ids = std::mem::take(&mut self.batch);
        self.batch_since = None;
        let queue = adopt::queue(&self.ctx).unwrap_or_default();
        // Queue IDs are never reused, so ones that left can be forgotten.
        self.announced
            .retain(|id| queue.iter().any(|item| item.id == *id));
        let state_dir = self.ctx.paths.state();
        if let Ok(Some(mut state)) = instance::load(state_dir) {
            let announced: Vec<u32> = self.announced.iter().copied().collect();
            if state.announced != announced {
                state.announced = announced;
                if let Err(err) = instance::save(state_dir, &state) {
                    self.log(&err.to_string());
                }
            }
        }
        let waiting: Vec<_> = queue.iter().filter(|item| ids.contains(&item.id)).collect();
        let body = match waiting.as_slice() {
            [] => return,
            [item] => {
                let lines = item.end + 1 - item.start;
                i18n::text(
                    self.lang,
                    if lines == 1 {
                        "review.line"
                    } else {
                        "review.lines"
                    },
                    &[
                        ("file", &adopt::display(&item.file, &self.ctx.home)),
                        ("lines", &lines.to_string()),
                    ],
                )
            }
            many => i18n::text(
                self.lang,
                "review.many",
                &[("count", &many.len().to_string())],
            ),
        };
        self.log(&format!("notified: {body}"));
        self.notifier.show(&Note {
            body,
            items: waiting.iter().map(|item| item.id).collect(),
        });
    }

    /// Notify each current problem once; one that cleared may notify again
    /// if it comes back.
    fn report_problems(&mut self) {
        let mut problems: Vec<(&'static str, String)> = self.repo_problem.iter().cloned().collect();
        if let Ok(state) = sync::load_state(self.ctx.paths.state()) {
            for (attempt, pull) in [(&state.publish, false), (&state.pull, true)] {
                let Some(attempt) = attempt else {
                    continue;
                };
                let reason = attempt.reason.clone().unwrap_or_default();
                let first = reason.lines().next().unwrap_or_default().to_string();
                problems.push(match (attempt.outcome, pull) {
                    (Outcome::Ok | Outcome::Offline, _) => continue,
                    (Outcome::Conflict, _) => ("conflict", attempt.files.join(", ")),
                    (Outcome::Failed, _) if reason.contains("upgrade rosamta") => {
                        ("upgrade", String::new())
                    }
                    (Outcome::Failed, true) if attempt.applying => ("apply_failed", first),
                    (Outcome::Failed, true) => ("pull_failed", first),
                    (Outcome::Failed, false) => ("publish_failed", first),
                    (Outcome::Blocked, true) => ("pull_blocked", first),
                    (Outcome::Blocked, false) => ("publish_blocked", first),
                });
            }
        }
        let state_dir = self.ctx.paths.state().to_path_buf();
        let Ok(Some(mut state)) = instance::load(&state_dir) else {
            return;
        };
        let mut keys: Vec<String> = Vec::new();
        for (kind, reason) in &problems {
            // The daemon's own apply can fail the same way a pull's did.
            let key = format!("{kind}:{reason}");
            if keys.contains(&key) {
                continue;
            }
            if !state.notified.contains(&key) {
                let arg = if *kind == "conflict" {
                    "files"
                } else {
                    "reason"
                };
                let body = i18n::text(self.lang, &format!("problem.{kind}"), &[(arg, reason)]);
                self.log(&format!("notified: {body}"));
                self.notifier.show(&Note {
                    body,
                    items: Vec::new(),
                });
            }
            keys.push(key);
        }
        if state.notified != keys {
            state.notified = keys;
            if let Err(err) = instance::save(&state_dir, &state) {
                self.log(&err.to_string());
            }
        }
    }
}

/// Open a URL with the system handler; the GUI registers `rosamta://`.
fn open(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Read the login shell's environment and adopt it for this process; on
/// failure say why and keep the inherited one. Call before starting threads.
pub fn adopt_login_environment() -> Option<String> {
    let shell = env::user_shell()?;
    match env::login_shell_env(&shell) {
        Ok(vars) => {
            for (name, value) in vars {
                // SAFETY: called at startup, before the daemon starts any thread.
                unsafe { std::env::set_var(name, value) };
            }
            None
        }
        Err(err) => Some(err),
    }
}

#[cfg(test)]
mod tests;
