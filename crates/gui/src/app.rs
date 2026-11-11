//! The root view: sidebar, the current page, refreshing and running actions.

use std::sync::Mutex;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable, Icon, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::adopt::{QueueItem, Scope};
use rosamta_core::apply::Context as Core;
use rosamta_core::config::SyncPolicy;
use rosamta_core::problems::Problem;
use rosamta_core::sync::Outcome;

use crate::actions;
use crate::pages::{self, review::Detail};
use crate::state::{self, Snapshot};
use crate::text::{t, tf, when};
use crate::theme::{self, font, palette, radius, size, space::*};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Review,
    Library,
    History,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LibraryTab {
    Modules,
    Dotfiles,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HistoryTab {
    Changes,
    Snapshots,
}

/// A row of the review list: a queue item by its ID, or a problem by its key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Selection {
    Item(u32),
    Problem(String),
}

const PAGES: [(Page, &str, IconName); 3] = [
    (Page::Review, "nav.review", IconName::Inbox),
    (Page::Library, "nav.library", IconName::Folder),
    (Page::History, "nav.history", IconName::ClockFading),
];

/// How often the files core keeps are checked for changes.
const POLL: Duration = Duration::from_secs(2);

/// `rosamta://` URLs from the system, handed over to the window.
pub static URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub struct Rosamta {
    pub page: Page,
    pub library: LibraryTab,
    pub history: HistoryTab,
    pub snap: Snapshot,
    /// The action that is running, by the name of what started it; every
    /// other action waits for it.
    pub busy: Option<&'static str>,
    pub selected: Option<Selection>,
    pub scope: Scope,
    pub detail: Option<Detail>,
    pub search: Entity<InputState>,
    pub url: Entity<InputState>,
    /// What importing from chezmoi did, shown until dismissed.
    pub imported: Option<rosamta_core::chezmoi::Report>,
    /// The settings window, once it was opened.
    pub settings: Option<AnyWindowHandle>,
    window: AnyWindowHandle,
    fingerprint: u64,
    /// Bumped per request, so only the newest snapshot or preview is shown.
    reloads: u64,
    previews: u64,
    /// An action finished; `busy` ends when the snapshot after it is shown.
    settling: bool,
    /// The last queue refresh problem shown, so it is not shown on every focus.
    refresh_problem: Option<String>,
    /// Reading the login shell's environment failed; shown once.
    environment_error: Option<String>,
    _poll: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Rosamta {
    pub fn new(
        environment_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("library.search")));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder(t("setup.url")));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // Without the background service, new rc file content shows up
            // when the app is opened or brought back.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    // macOS tells of a changed setting only when asked.
                    gpui_kit::base::apply_system_reduce_motion(cx);
                    this.refresh_queue(window, cx);
                }
            }),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let fingerprint = cx.background_spawn(async { state::fingerprint() }).await;
                let alive = this.update(cx, |this, cx| {
                    this.open_urls(cx);
                    if this.fingerprint != fingerprint {
                        this.fingerprint = fingerprint;
                        this.reload(cx);
                    } else if this.busy.is_some() && this.selected_script() {
                        // The log of a script being run grows as it runs.
                        this.load_detail(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut this = Rosamta {
            page: Page::Review,
            library: LibraryTab::Modules,
            history: HistoryTab::Changes,
            snap: Snapshot::empty(),
            busy: None,
            selected: None,
            scope: Scope::Host,
            detail: None,
            search,
            url,
            imported: None,
            settings: None,
            window: window.window_handle(),
            fingerprint: 0,
            reloads: 0,
            previews: 0,
            settling: false,
            refresh_problem: None,
            environment_error,
            _poll: poll,
            _subscriptions: subscriptions,
        };
        this.open_urls(cx);
        this.refresh_queue(window, cx);
        this
    }

    /// Read everything again in the background, then show it.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.reloads += 1;
        let request = self.reloads;
        cx.spawn(async move |this, cx| {
            let snap = cx.background_spawn(async { state::load() }).await;
            this.update(cx, |this, cx| {
                if this.reloads == request {
                    this.show(snap, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Queue what changed in rc files, as `rsmt review` does, then reload.
    fn refresh_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async {
                    Core::from_env().and_then(|core| actions::refresh(&core))
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if let Some(err) = this.environment_error.take() {
                    window.push_notification(Notification::warning(err), cx);
                }
                let problem = result.unwrap_or_else(|err| Some(said(&err)));
                if let Some(text) = problem.as_ref().filter(|_| problem != this.refresh_problem) {
                    window.push_notification(Notification::error(text.clone()), cx);
                }
                this.refresh_problem = problem;
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }

    fn show(&mut self, snap: Snapshot, cx: &mut Context<Self>) {
        self.snap = snap;
        if std::mem::take(&mut self.settling) {
            self.busy = None;
        }
        if !self.selected.as_ref().is_some_and(|row| self.listed(row)) {
            self.selected = self.first_row();
            self.scope = Scope::Host;
            self.detail = None;
        }
        // The item may have changed underneath, or what accepting it writes.
        self.load_detail(cx);
        cx.notify();
    }

    fn listed(&self, row: &Selection) -> bool {
        match row {
            Selection::Item(id) => self.snap.queue.iter().any(|item| item.id == *id),
            Selection::Problem(key) => self.snap.problems.iter().any(|one| one.key() == *key),
        }
    }

    /// The review list's first row: a problem, else a decision, else what waits.
    fn first_row(&self) -> Option<Selection> {
        let queue = &self.snap.queue;
        let item = queue
            .iter()
            .find(|item| !pages::review::waits(item))
            .or(queue.first());
        match self.snap.problems.first() {
            Some(problem) => Some(Selection::Problem(problem.key())),
            None => item.map(|item| Selection::Item(item.id)),
        }
    }

    /// Open a row of the review list.
    pub fn select(&mut self, row: Selection, cx: &mut Context<Self>) {
        self.page = Page::Review;
        if self.selected.as_ref() != Some(&row) {
            self.selected = Some(row);
            self.scope = Scope::Host;
            self.detail = None;
            self.load_detail(cx);
        }
        cx.notify();
    }

    pub fn selected_item(&self) -> Option<&QueueItem> {
        let Some(Selection::Item(id)) = &self.selected else {
            return None;
        };
        self.snap.queue.iter().find(|item| item.id == *id)
    }

    pub fn selected_problem(&self) -> Option<&Problem> {
        let Some(Selection::Problem(key)) = &self.selected else {
            return None;
        };
        self.snap.problems.iter().find(|one| one.key() == *key)
    }

    pub fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.detail = None;
            self.load_detail(cx);
            cx.notify();
        }
    }

    fn selected_script(&self) -> bool {
        self.selected_item()
            .is_some_and(|item| item.script.is_some())
    }

    fn load_detail(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };
        let scope = self.scope;
        self.previews += 1;
        let request = self.previews;
        cx.spawn(async move |this, cx| {
            let detail = cx
                .background_spawn(async move { pages::review::detail(&item, scope) })
                .await;
            this.update(cx, |this, cx| {
                if this.previews == request {
                    this.detail = Some(detail);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Run a write in the background; show its error or warning in the
    /// window that asked, then read everything again. `name` says which
    /// button waits with a spinner; the others are off until the new
    /// snapshot is shown.
    pub fn run(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        name: &'static str,
        action: impl FnOnce(&Core) -> actions::Done + Send + 'static,
    ) {
        self.run_then(window, cx, name, action, |_, warning| warning);
    }

    pub fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(window, cx, "sync", actions::sync_now);
    }

    /// Try again after the problem `kind`.
    pub fn retry(&mut self, kind: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.run(window, cx, "sync", move |core| actions::retry(core, kind));
    }

    /// Create a repo from chezmoi's source, then show what was imported.
    pub fn import_chezmoi(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_then(
            window,
            cx,
            "import",
            actions::import_chezmoi,
            |this, (report, warning)| {
                this.imported = Some(report);
                warning
            },
        );
    }

    /// `run`, with `done` taking what the action returned and giving the
    /// warning to show.
    fn run_then<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        name: &'static str,
        action: impl FnOnce(&Core) -> rosamta_core::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T) -> Option<String> + 'static,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(name);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { Core::from_env().and_then(|core| action(&core)) })
                .await;
            let note = this.update(cx, |this, cx| {
                let note = match result.map(|value| done(this, value)) {
                    Ok(None) => None,
                    Ok(Some(warning)) => Some(Notification::warning(warning)),
                    Err(err) => Some(Notification::error(said(&err))),
                };
                this.settling = true;
                this.reload(cx);
                note
            });
            // The window that asked may be closed by now; the app goes on.
            if let Ok(Some(note)) = note {
                cx.update(|window, cx| window.push_notification(note, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Open the settings window; the app is not being updated then, so the
    /// window can read it.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        let app = cx.entity();
        cx.defer(move |cx| pages::settings::open(&app, cx));
    }

    /// Open the item a `rosamta://review/<id>` URL names.
    fn open_urls(&mut self, cx: &mut Context<Self>) {
        let urls = std::mem::take(&mut *URLS.lock().unwrap_or_else(|err| err.into_inner()));
        for url in urls {
            if let Some(id) = review_id(&url) {
                self.select(Selection::Item(id), cx);
                // The settings window may be in front.
                let window = self.window;
                cx.defer(move |cx| {
                    window
                        .update(cx, |_, window, _| window.activate_window())
                        .ok();
                });
            }
        }
    }

    /// One line on where syncing stands.
    fn sync_status(&self) -> SharedString {
        let snap = &self.snap;
        if self.busy == Some("sync") {
            return t("status.syncing");
        }
        if snap.remote.is_none() {
            return t("status.no_remote");
        }
        let manual = snap
            .host
            .as_ref()
            .is_some_and(|host| host.publish == SyncPolicy::Manual);
        if manual && snap.sync.unpushed > 0 {
            let count = snap.sync.unpushed.to_string();
            return tf("status.unpushed", &[("count", &count)]);
        }
        let synced = [&snap.sync.publish, &snap.sync.pull]
            .into_iter()
            .flatten()
            .filter(|attempt| attempt.outcome == Outcome::Ok)
            .map(|attempt| attempt.at)
            .max();
        match synced {
            Some(at) => tf("status.synced", &[("time", &when(at))]),
            None => t("status.never_synced"),
        }
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let snap = &self.snap;
        let count = snap.problems.len()
            + snap
                .queue
                .iter()
                .filter(|item| !pages::review::waits(item))
                .count();
        let busy = self.busy.is_some();
        let nav = PAGES.into_iter().map(|(page, key, icon)| {
            let active = self.page == page;
            h_flex()
                .id(key)
                .h(px(S32))
                .px(px(S12))
                .gap(px(S8))
                .rounded(px(radius::ROW))
                .cursor_pointer()
                .when(active, |row| row.bg(p.accent_soft))
                .when(!active, |row| {
                    row.hover(|style| style.bg(p.text.opacity(0.06)))
                })
                .child(Icon::new(icon).size(px(S16)).text_color(if active {
                    p.accent
                } else {
                    p.muted
                }))
                .child(div().flex_1().child(t(key)))
                .when(page == Page::Review && count > 0, |row| {
                    row.child(pages::count_badge(count, cx))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = page;
                    cx.notify();
                }))
        });
        let footer = v_flex()
            .gap(px(S4))
            .px(px(S4))
            .pt(px(S8))
            .border_t_1()
            .border_color(p.line)
            .child(
                h_flex()
                    .h(px(28.))
                    .gap(px(S8))
                    .child(pages::dot(snap.running, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .children(snap.host.as_ref().map(|host| host.id.clone())),
                    )
                    .child(
                        pages::icon_button("settings", IconName::Settings, t("nav.settings"))
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                    ),
            )
            .child(
                h_flex()
                    .h(px(28.))
                    .pl(px(S16))
                    .gap(px(S8))
                    .text_size(px(font::CAPTION))
                    .text_color(p.muted)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.sync_status()),
                    )
                    .child(
                        pages::icon_button("sync", IconName::RefreshCw, t("action.sync"))
                            .loading(self.busy == Some("sync"))
                            .disabled(busy || snap.remote.is_none())
                            .on_click(cx.listener(|this, _, window, cx| this.sync(window, cx))),
                    ),
            );
        v_flex()
            .w(px(size::SIDEBAR))
            .h_full()
            .flex_none()
            // Over a blurred window the desktop shows through.
            .bg(if theme::TRANSLUCENT {
                p.panel.opacity(0.7)
            } else {
                p.panel
            })
            .border_r_1()
            .border_color(p.line)
            .px(px(S12))
            .pb(px(S12))
            .child(
                // Beside the toolbar: on macOS the window's buttons are here.
                div()
                    .flex_none()
                    .h(px(if cfg!(target_os = "macos") {
                        size::TOOLBAR
                    } else {
                        S12
                    }))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .child(v_flex().gap(px(S2)).children(nav))
            .child(div().flex_1())
            .child(footer)
    }
}

/// A core error in this app's words.
fn said(err: &rosamta_core::Error) -> String {
    match err {
        rosamta_core::Error::Waiting(waiting) => pages::review::waiting_text(waiting).to_string(),
        // The queue is read again afterwards; the list shows what is there now.
        rosamta_core::Error::NoItem(_) | rosamta_core::Error::Stale(_) => {
            t("item.stale").to_string()
        }
        err => crate::text::plain(&err.to_string()).to_string(),
    }
}

impl Render for Rosamta {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let root = h_flex()
            .size_full()
            .text_color(p.text)
            .text_size(px(font::BODY));
        if !self.snap.loaded {
            // Nothing is known yet; the setup page is for a host without a repo.
            return root.bg(p.bg);
        }
        let sidebar = self.snap.ready && self.imported.is_none();
        let page = if self.imported.is_some() {
            pages::import::render(self, cx).into_any_element()
        } else if !self.snap.ready {
            pages::setup::render(self, cx).into_any_element()
        } else {
            match self.page {
                Page::Review => pages::review::render(self, cx).into_any_element(),
                Page::Library => pages::library::render(self, cx).into_any_element(),
                Page::History => pages::history::render(self, cx).into_any_element(),
            }
        };
        root.when(sidebar, |root| root.child(self.sidebar(cx)))
            .child(v_flex().flex_1().h_full().min_w_0().bg(p.bg).child(page))
    }
}

/// The item ID in `rosamta://review/<id>`.
pub fn review_id(url: &str) -> Option<u32> {
    url.strip_prefix("rosamta://review/")?
        .trim_end_matches('/')
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::review_id;

    #[test]
    fn review_urls_name_an_item() {
        assert_eq!(review_id("rosamta://review/12"), Some(12));
        assert_eq!(review_id("rosamta://review/12/"), Some(12));
        assert_eq!(review_id("rosamta://review/x"), None);
        assert_eq!(review_id("https://review/12"), None);
    }
}

#[cfg(test)]
mod view_tests {
    use std::path::Path;
    use std::process::Command;

    use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use rosamta_core::apply::Context as Core;
    use rosamta_core::chezmoi::Outcome;

    // Not `super::*`: gpui's `test` attribute would replace the standard one.
    use super::{Detail, Page, Rosamta, Selection, actions};

    /// Set by the parent test for the child run in a home of its own.
    const OWN_HOME: &str = "ROSAMTA_TEST_OWN_HOME";

    /// The app reads HOME and the XDG paths everywhere, so the views are
    /// tested in a child process with a home of its own: setting the
    /// environment here would race the other tests, and never the real one.
    #[test]
    fn the_review_page_follows_the_queue_and_actions() {
        in_child("app::view_tests::in_own_home");
    }

    #[test]
    fn the_setup_page_imports_from_chezmoi() {
        in_child("app::view_tests::import_in_own_home");
    }

    fn in_child(test: &str) {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap());
        // Nothing of this process's environment but what running needs: no
        // XDG paths, ZDOTDIR or git index of the developer's.
        child.env_clear();
        for var in ["PATH", "TMPDIR", "RUST_BACKTRACE", "LLVM_PROFILE_FILE"] {
            if let Some(value) = std::env::var_os(var) {
                child.env(var, value);
            }
        }
        child
            .args(["--exact", test, "--ignored"])
            .env("HOME", home.path())
            .env(OWN_HOME, home.path())
            .env("ROSAMTA_NO_KEYCHAIN", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com");
        let out = child.output().unwrap();
        assert!(
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("1 passed"),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// Two copies that differ from the repo and a script to run: three
    /// review items.
    fn set_up(core: &Core) {
        let repo = core.paths.repo();
        write(
            &repo.join("rosamta.toml"),
            "format = 1\nhosts = [\"test\"]\n",
        );
        write(
            &core.paths.host_config(),
            "id = \"test\"\nadopt = \"ask\"\n",
        );
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(git.success());
        let mut manifest = String::new();
        for name in [".a", ".b"] {
            write(&repo.join("home").join(name), &format!("repo{name}\n"));
            write(&core.home.join(name), "mine\n");
            manifest.push_str(&format!(
                "[[dotfile]]\npath = \"{name}\"\ndeploy = \"copy\"\n\n"
            ));
        }
        write(&repo.join("dotfiles.toml"), &manifest);
        rosamta_core::dotfiles::deploy(core).unwrap();
        write(
            &repo.join("scripts.toml"),
            "[[script]]\nsource = \"setup.sh\"\nrun = \"once\"\n",
        );
        write(
            &repo.join("setup.sh"),
            "[ -f \"$HOME/fail\" ] && exit 3\necho ran > \"$HOME/ran\"\n",
        );
        write(&core.home.join("fail"), "");
    }

    /// The app in a window, in a root as `open_window` builds it, which
    /// notifications need.
    fn open(cx: &mut TestAppContext) -> (Entity<Rosamta>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let mut built = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            crate::theme::init(window, cx);
            let view = cx.new(|cx| Rosamta::new(None, window, cx));
            built = Some(view.clone());
            gpui_kit::base::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        (built.unwrap(), cx)
    }

    #[test]
    #[ignore = "run by the_setup_page_imports_from_chezmoi, in a home of its own"]
    fn import_in_own_home() {
        let Some(own) = std::env::var_os(OWN_HOME) else {
            return;
        };
        assert_eq!(std::env::var_os("HOME"), Some(own));
        let core = Core::from_env().unwrap();
        let source = core.home.join(".local/share/chezmoi");
        write(&source.join("dot_gitconfig"), "[user]\n\tname = me\n");
        write(
            &source.join("dot_bad.tmpl"),
            "{{ range .list }}x{{ end }}\n",
        );

        let mut cx = TestAppContext::single();
        let (view, cx) = open(&mut cx);
        view.read_with(cx, |view, _| {
            assert!(!view.snap.ready && view.snap.chezmoi);
        });
        cx.update(|window, cx| view.update(cx, |view, cx| view.import_chezmoi(window, cx)));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.snap.ready && view.busy.is_none());
            let report = view.imported.as_ref().expect("the result is shown");
            assert_eq!(report.count(Outcome::Imported), 1);
            assert_eq!(report.count(Outcome::Manual), 1);
        });
        assert!(core.paths.repo().join("home/.gitconfig").is_file());
    }

    #[test]
    #[ignore = "run by the_review_page_follows_the_queue_and_actions, in a home of its own"]
    fn in_own_home() {
        let Some(own) = std::env::var_os(OWN_HOME) else {
            return;
        };
        assert_eq!(std::env::var_os("HOME"), Some(own));
        let core = Core::from_env().unwrap();
        set_up(&core);

        let mut cx = TestAppContext::single();
        let (view, cx) = open(&mut cx);
        let ids: Vec<u32> = view.read_with(cx, |view, _| {
            assert!(view.snap.ready);
            view.snap.queue.iter().map(|item| item.id).collect()
        });
        assert_eq!(ids.len(), 3);
        let diff_has = |view: &Rosamta, text: &str| matches!(&view.detail, Some(Detail::Dotfile(diff)) if diff.contains(text));
        view.read_with(cx, |view, _| {
            assert!(view.snap.problems.is_empty(), "{:?}", view.snap.problems);
            assert_eq!(view.selected, Some(Selection::Item(ids[0])));
            assert!(diff_has(view, "+repo.a"));
        });

        // Choosing an item opens it on the review page with its own diff.
        view.update(cx, |view, cx| {
            view.page = Page::History;
            view.select(Selection::Item(ids[1]), cx);
        });
        view.read_with(cx, |view, _| {
            assert!(view.page == Page::Review && view.detail.is_none());
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| diff_has(view, "+repo.b")));

        // While an action runs another waits its turn: ignored, not queued.
        // Afterwards the item is gone and the remaining one is selected.
        let (first, second) = (ids[0], ids[1]);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.run(window, cx, "discard", move |core| {
                    actions::discard(core, second)
                });
                view.run(window, cx, "discard", move |core| {
                    actions::discard(core, first)
                });
            })
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.busy.is_none());
            assert_eq!(view.snap.queue.len(), 2);
            assert_eq!(view.selected, Some(Selection::Item(first)));
            assert!(diff_has(view, "+repo.a"));
        });
        let read = |name| std::fs::read_to_string(core.home.join(name)).unwrap();
        assert_eq!(
            (read(".a"), read(".b")),
            ("mine\n".into(), "repo.b\n".into())
        );

        // A script shows what it would run, and after a failed run its log;
        // it stays listed until it succeeds.
        let script = ids[2];
        view.update(cx, |view, cx| view.select(Selection::Item(script), cx));
        cx.run_until_parked();
        let shown = |view: &Rosamta| match &view.detail {
            Some(Detail::Script { text, log, .. }) => (text.clone(), log.clone()),
            _ => panic!("not a script"),
        };
        let (text, log) = view.read_with(cx, |view, _| shown(view));
        assert!(text.contains("exit 3") && log.is_none());
        let run = |cx: &mut gpui_kit::VisualTestContext| {
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    let shown = view.snap.queue.iter().find(|item| item.id == script);
                    let shown = shown.and_then(|item| item.script.clone()).unwrap();
                    view.run(window, cx, "run", move |core| {
                        actions::run_script(core, script, shown)
                    });
                })
            });
            cx.run_until_parked();
        };
        run(cx);
        let (_, log) = view.read_with(cx, |view, _| {
            assert_eq!(view.selected, Some(Selection::Item(script)));
            shown(view)
        });
        assert!(log.unwrap().ends_with("# exited with 3\n"));
        std::fs::remove_file(core.home.join("fail")).unwrap();
        run(cx);
        view.read_with(cx, |view, _| {
            assert_eq!(view.snap.queue.len(), 1);
            assert!(view.snap.queue.iter().all(|item| item.script.is_none()));
        });
        assert_eq!(read("ran"), "ran\n");

        // The settings window opens once; asking again brings it forward.
        cx.update(|_, cx| {
            crate::pages::settings::open(&view, cx);
            crate::pages::settings::open(&view, cx);
            assert_eq!(cx.windows().len(), 2);
        });

        // What cannot be read is a problem in the list, in front of the items.
        write(&core.paths.host_config(), "id = [\n");
        view.update(cx, |view, cx| view.reload(cx));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let kinds: Vec<&str> = view.snap.problems.iter().map(|one| one.kind).collect();
            assert_eq!(kinds, ["read"]);
        });
    }
}
