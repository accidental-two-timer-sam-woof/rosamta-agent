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

use crate::adopt::{self, QueueItem, Scope};
use crate::apply::{self, Context};
use crate::config::{HostConfig, SyncPolicy};
use crate::dotfiles::review::{self, Reason, WaitingFor};
use crate::error::Result;
use crate::hooks::HookEnv;
use crate::i18n::{self, Lang};
use crate::problems::{self, Problem};
use crate::sync::{self, Outcome};
use crate::{dotfiles, generated, host, validate};

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
/// How often a template waiting for rbw tries again; unlocking rbw sends
/// no event.
const RBW_RETRY: Duration = Duration::from_secs(60);
/// The wait before pushing again commits that could not reach the remote;
/// after `PUSH_TRIES` misses it doubles each time, up to `PUSH_RETRY_MAX`.
const PUSH_RETRY: Duration = Duration::from_secs(60);
const PUSH_TRIES: u32 = 3;
const PUSH_RETRY_MAX: Duration = Duration::from_secs(15 * 60);
/// How often a problem the notification service did not take is shown again.
const NOTIFY_RETRY: Duration = Duration::from_secs(60);
/// For how long: a desktop's service is up minutes after login, a server's
/// never.
const NOTIFY_GIVE_UP: Duration = Duration::from_secs(10 * 60);

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

/// Deployed dotfile targets, named as file events name them: the directory
/// resolved, not the target, which may be a link into the repo.
fn dotfile_targets(ctx: &Context) -> Vec<PathBuf> {
    let Ok(deployed) = dotfiles::state::load(ctx.paths.state()) else {
        return Vec::new();
    };
    deployed
        .entries
        .keys()
        .map(|rel| {
            let target = ctx.home.join(rel);
            match (target.parent(), target.file_name()) {
                (Some(dir), Some(name)) => real(dir).join(name),
                _ => target,
            }
        })
        .collect()
}

/// Where dotfile changes show: each target's directory, so a file renamed
/// over the target is seen, and a copied directory with everything in it.
/// A link to a directory needs nothing more: edits through it land in the repo.
fn dotfile_watches(targets: &[PathBuf]) -> Vec<(PathBuf, RecursiveMode)> {
    let mut out = Vec::new();
    for target in targets {
        if let Some(dir) = target.parent() {
            out.push((dir.to_path_buf(), RecursiveMode::NonRecursive));
        }
        if std::fs::symlink_metadata(target).is_ok_and(|meta| meta.is_dir()) {
            out.push((target.clone(), RecursiveMode::Recursive));
        }
    }
    out
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
    dotfiles: bool,
}

/// `rc` lists the watched rc files, as [`Daemon::rc_files`] names them;
/// `dotfiles` the deployed targets, as [`dotfile_targets`] names them.
fn route(ctx: &Context, rc: &[PathBuf], dotfiles: &[PathBuf], paths: &[PathBuf]) -> Routes {
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
        dotfiles: paths
            .iter()
            .any(|path| dotfiles.iter().any(|target| path.starts_with(target))),
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
    next_rbw_retry: Instant,
    /// When waiting commits were last pushed again; a pull that fails first
    /// leaves no newer publish attempt to wait from.
    pushed_again: Option<Instant>,
    /// Pushes again in a row that found no network.
    push_misses: u32,
    /// Retries left for a sync after waking that found no network.
    wake_retries: &'static [Duration],
    clock: (Instant, SystemTime),
    exe: Option<(PathBuf, Identity)>,
    /// A problem found by applying the repo: kind and reason.
    repo_problem: Option<(&'static str, String)>,
    /// Problems logged but not shown yet, by key.
    unshown: BTreeSet<String>,
    /// The review note logged but not shown yet.
    review_unshown: Option<String>,
    next_notify_retry: Instant,
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
        ..Daemon::new(
            ctx,
            host,
            Box::new(notify::GiveUp::new(
                notifier,
                NOTIFY_GIVE_UP,
                NOTIFY_RETRY * 3,
            )),
        )
    };
    let mut watching = BTreeSet::new();
    let mut watch = |daemon: &Daemon, list: Vec<(PathBuf, RecursiveMode)>| {
        for (dir, mode) in list {
            if !dir.is_dir() || !watching.insert((dir.clone(), mode == RecursiveMode::Recursive)) {
                continue;
            }
            if let Err(err) = debouncer.watch(&dir, mode) {
                daemon.log(&format!("cannot watch {}: {err}", dir.display()));
            }
        }
    };
    watch(&daemon, watch_list(&daemon.ctx, &daemon.env));
    drop(events);
    daemon.log("started");
    // Catch up on whatever changed while the daemon was not running.
    daemon.repo_changed();
    daemon.rc_changed();
    daemon.dotfiles_changed();
    loop {
        // Dotfiles deployed since, and startup files that moved: their
        // directories are watched from now on.
        watch(&daemon, dotfile_watches(&dotfile_targets(&daemon.ctx)));
        watch(&daemon, watch_list(&daemon.ctx, &daemon.env));
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
            next_rbw_retry: Instant::now() + RBW_RETRY,
            pushed_again: None,
            push_misses: 0,
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
            unshown: BTreeSet::new(),
            review_unshown: None,
            next_notify_retry: Instant::now(),
            ctx,
        }
    }

    fn log(&self, message: &str) {
        log(&self.ctx, message);
    }

    fn changed(&mut self, paths: &[PathBuf]) {
        let mut rc = self.rc_files();
        rc.extend_from_slice(&self.rc_known);
        let routes = route(&self.ctx, &rc, &dotfile_targets(&self.ctx), paths);
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
        // After the repo: its apply may already have handled the change. New
        // [vars] in host.toml change what templates render.
        if routes.dotfiles || routes.host {
            self.dotfiles_changed();
        }
    }

    /// Deploy dotfiles again after a target changed: content written back
    /// to the repo reaches it as a repo change, which publishes it. A target
    /// rosamta wrote itself matches its record and is left alone.
    fn dotfiles_changed(&mut self) {
        // A host that never applied has no deployed dotfiles to look after,
        // and a repo that did not apply is not deployed either.
        if generated::current_id(&self.ctx.paths.generated()).is_none()
            || self.repo_problem.is_some()
        {
            return;
        }
        let result = (|| {
            let _lock = adopt::lock(&self.ctx)?;
            dotfiles::deploy(&self.ctx)
        })();
        match result {
            Ok(steps) => {
                let changed = steps
                    .iter()
                    .filter(|step| step.action != dotfiles::Action::Unchanged)
                    .count();
                if changed > 0 {
                    self.log(&format!("deployed dotfiles: {changed} changed"));
                }
            }
            Err(err) => self.log(&format!("dotfile deploy failed: {err}")),
        }
        self.queue_applied_items();
        self.report_problems();
    }

    /// Batch dotfile and script review items not announced yet; applying
    /// puts them in the queue without a scan to report them.
    fn queue_applied_items(&mut self) {
        let Ok(queue) = adopt::queue(&self.ctx) else {
            return;
        };
        for item in queue.iter().filter(|item| !item.is_rc()) {
            if self.announced.insert(item.id) {
                self.batch.push(item.id);
                self.batch_since.get_or_insert_with(Instant::now);
            }
        }
    }

    /// Watched rc files that exist, named as file events name them: the
    /// directory resolved, not the file, which may be a symlink.
    fn rc_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = adopt::watched_files(&self.env)
            .into_iter()
            .map(
                |watched| match (watched.path.parent(), watched.path.file_name()) {
                    (Some(dir), Some(name)) => real(dir).join(name),
                    _ => watched.path,
                },
            )
            .collect();
        // Nothing is adopted from it, but fish stops loading rosamta without it.
        files.push(real(&self.env.fish_config.join("conf.d")).join("rosamta.fish"));
        files
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
        if self.rc_seen.is_some() {
            // A changed `.zshenv` may move ZDOTDIR; asking the shells is
            // slow, so only once something changed.
            self.env = self.ctx.hook_env(&self.ctx.installed());
            files = self.rc_files();
            seen = fingerprint(&files);
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
        // Edits by hand are committed and pushed only after they apply; an
        // edit saved while applying waits for its own check.
        let unchecked = fingerprint(&self.repo_files()) != seen;
        if self.repo_problem.is_none()
            && !unchecked
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
        self.queue_applied_items();
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
        let applied = apply::apply(&self.ctx).map_err(|err| ("apply_failed", err.to_string()))?;
        match applied.failures().into_iter().next() {
            Some(failure) => Err(("apply_failed", failure)),
            None => Ok(()),
        }
    }

    fn sync_now(&mut self) {
        // A failed apply may have been passing trouble (a download); try again.
        if self.repo_problem.is_some() {
            self.repo_seen = None;
            self.repo_changed();
        }
        if self.host.pull != SyncPolicy::Auto {
            return;
        }
        let started = crate::time::now();
        // Hand edits that did not apply stay unpublished; pulling may fix them.
        match sync::sync_per_policy(&self.ctx, self.repo_problem.is_some()) {
            Ok(_) => {}
            // No remote yet, or no repo: nothing to sync with.
            Err(err) if err.is_usage() => {}
            Err(err) => self.log(&err.to_string()),
        }
        // Pulled settings that did not apply hold back dotfiles, as a repo
        // edit that did not apply does.
        if let Some(pull) = sync::load_state(self.ctx.paths.state())
            .ok()
            .and_then(|state| state.pull)
            .filter(|pull| pull.applying && pull.at >= started)
        {
            self.repo_problem = Some(("apply_failed", pull.reason.unwrap_or_default()));
        }
        // A pull applies dotfiles too.
        self.queue_applied_items();
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
            self.push_misses = 0;
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
        // Pushed some other way: the next time offline starts over.
        if self.push_misses > 0 && self.offline_push().is_none() {
            self.push_misses = 0;
        }
        if self.push_due() {
            self.push_again();
        }
        if !self.unshown.is_empty() && now.0 >= self.next_notify_retry {
            self.next_notify_retry = now.0 + NOTIFY_RETRY;
            self.report_problems();
        }
        if now.0 >= self.next_rbw_retry {
            self.next_rbw_retry = now.0 + RBW_RETRY;
            if self.waits_for_rbw() {
                self.dotfiles_changed();
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

    /// bw's session lives in the shell that unlocked it and a secret command
    /// needs a terminal; only an rbw unlock is something this process sees.
    fn waits_for_rbw(&self) -> bool {
        adopt::queue(&self.ctx).is_ok_and(|queue| {
            queue.iter().any(|item| {
                item.dotfile.as_ref().is_some_and(|dotfile| {
                    matches!(&dotfile.waiting, Some(WaitingFor::Locked { tool }) if tool == "rbw")
                })
            })
        })
    }

    fn push_delay(&self) -> Duration {
        let doublings = (self.push_misses + 1).saturating_sub(PUSH_TRIES).min(8);
        (PUSH_RETRY * 2u32.pow(doublings)).min(PUSH_RETRY_MAX)
    }

    /// When the publish that left commits waiting found no network.
    fn offline_push(&self) -> Option<u64> {
        let state = sync::load_state(self.ctx.paths.state()).ok()?;
        let attempt = state.publish.filter(|_| state.unpushed > 0)?;
        (attempt.outcome == Outcome::Offline).then_some(attempt.at)
    }

    /// Commits wait because the remote could not be reached, and the retry
    /// delay has passed since that try.
    fn push_due(&self) -> bool {
        let delay = self.push_delay();
        self.host.publish == SyncPolicy::Auto
            && self.pushed_again.is_none_or(|at| at.elapsed() >= delay)
            && self
                .offline_push()
                .is_some_and(|at| crate::time::now() >= at + delay.as_secs())
    }

    /// Push the waiting commits; with pulling on, as a whole sync.
    fn push_again(&mut self) {
        self.pushed_again = Some(Instant::now());
        let started = crate::time::now();
        if self.host.pull == SyncPolicy::Auto {
            self.sync_now();
        } else {
            let pushed =
                adopt::lock(&self.ctx).and_then(|lock| sync::publish_if_auto(&self.ctx, &lock));
            if let Err(err) = pushed {
                self.log(&err.to_string());
            }
            self.report_problems();
        }
        self.push_misses = match self.offline_since(started) {
            true => self.push_misses + 1,
            false => 0,
        };
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
        let new: Vec<&QueueItem> = queue.iter().filter(|item| ids.contains(&item.id)).collect();
        // Waiting items need no answer; next to ones that do, the note is
        // about those alone, so its actions never reach a waiting item.
        // Scripts too: running one needs the terminal or the app, not a note.
        let (waits, decisions): (Vec<&QueueItem>, Vec<&QueueItem>) =
            new.into_iter().partition(|item| {
                item.script.is_some()
                    || item
                        .dotfile
                        .as_ref()
                        .is_some_and(|dotfile| dotfile.waiting.is_some())
            });
        let view_only = decisions.is_empty();
        let items = if view_only { waits } else { decisions };
        let body = match items.as_slice() {
            [] => {
                self.save_announced();
                return;
            }
            [item] if item.script.is_some() => self.script_body(item),
            [item] if item.dotfile.is_some() => self.dotfile_body(item),
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
            many => {
                let failed = |item: &&QueueItem| {
                    item.script
                        .as_ref()
                        .is_some_and(|script| script.why == crate::scripts::apply::Why::Failed)
                };
                let key = if !view_only {
                    "review.many"
                } else if many.iter().all(|item| item.script.is_none()) {
                    "review.waiting_many"
                } else if many.iter().all(failed) {
                    "review.scripts_failed"
                } else if many
                    .iter()
                    .all(|item| item.script.is_some() && !failed(item))
                {
                    "review.scripts_waiting"
                } else {
                    "review.items_waiting"
                };
                i18n::text(self.lang, key, &[("count", &many.len().to_string())])
            }
        };
        if self.review_unshown.as_ref() != Some(&body) {
            self.log(&format!("notified: {body}"));
        }
        let note = Note {
            body,
            items: items.iter().map(|item| item.id).collect(),
            view_only,
        };
        if self.notifier.show(&note) {
            self.review_unshown = None;
        } else {
            // Tried again once NOTIFY_RETRY has passed (`elapsed` stays zero
            // until then), and not saved as announced meanwhile.
            self.review_unshown = Some(note.body);
            self.batch = ids;
            self.batch_since = Some(Instant::now() + NOTIFY_RETRY);
        }
        self.save_announced();
    }

    /// Saves the announced items, leaving out a batch not shown yet, so the
    /// daemon announces it after a restart.
    fn save_announced(&self) {
        let state_dir = self.ctx.paths.state();
        if let Ok(Some(mut state)) = instance::load(state_dir) {
            let announced: Vec<u32> = self
                .announced
                .iter()
                .copied()
                .filter(|id| !self.batch.contains(id))
                .collect();
            if state.announced != announced {
                state.announced = announced;
                if let Err(err) = instance::save(state_dir, &state) {
                    self.log(&err.to_string());
                }
            }
        }
    }

    /// What one script item is about, naming the script.
    fn script_body(&self, item: &QueueItem) -> String {
        let Some(script) = &item.script else {
            return String::new();
        };
        let key = if script.why == crate::scripts::apply::Why::Failed {
            "review.script_failed"
        } else {
            "review.script_waiting"
        };
        i18n::text(self.lang, key, &[("script", &script.source)])
    }

    /// What one dotfile item is about, naming the file.
    fn dotfile_body(&self, item: &QueueItem) -> String {
        let file = adopt::display(&item.file, &self.ctx.home);
        let Some(dotfile) = &item.dotfile else {
            return file;
        };
        let text = |key, more: &[(&str, &str)]| {
            let args: Vec<(&str, &str)> = [("file", file.as_str())]
                .into_iter()
                .chain(more.iter().copied())
                .collect();
            i18n::text(self.lang, key, &args)
        };
        match (&dotfile.waiting, dotfile.reason) {
            (Some(WaitingFor::Locked { tool }), _) => {
                text("review.waiting_locked", &[("tool", tool)])
            }
            (Some(WaitingFor::Command { name }), _) => {
                text("review.waiting_command", &[("command", name)])
            }
            (Some(WaitingFor::Unset { vars }), _) => {
                text("review.waiting_unset", &[("vars", &review::var_list(vars))])
            }
            (None, Reason::Changed) => text("review.dotfile_changed", &[]),
            (None, _) => text("review.dotfile_conflict", &[]),
        }
    }

    /// Notify each current problem once; one that cleared may notify again
    /// if it comes back.
    fn report_problems(&mut self) {
        let mut problems: Vec<Problem> = self
            .repo_problem
            .iter()
            .map(|(kind, reason)| Problem::new(kind, reason.clone()))
            .collect();
        let missing = apply::missing_hooks(&self.ctx, &self.env);
        problems.extend(problems::current(&self.ctx, &self.host, &missing));
        let state_dir = self.ctx.paths.state().to_path_buf();
        let Ok(Some(mut state)) = instance::load(&state_dir) else {
            return;
        };
        let mut keys: Vec<String> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for problem in &problems {
            // The daemon's own apply can fail the same way a pull's did.
            let key = problem.key();
            if keys.contains(&key) {
                continue;
            }
            if !state.notified.contains(&key) {
                let body = problem.text(self.lang);
                if self.unshown.insert(key.clone()) {
                    self.log(&format!("notified: {body}"));
                }
                let note = Note {
                    body,
                    items: Vec::new(),
                    view_only: false,
                };
                // Not counted until shown: the notification service may
                // start after the daemon. The tick tries again.
                if !self.notifier.show(&note) {
                    seen.push(key);
                    continue;
                }
                self.unshown.remove(&key);
            }
            seen.push(key.clone());
            keys.push(key);
        }
        self.unshown.retain(|key| seen.contains(key));
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
