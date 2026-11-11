//! The root view: sidebar, the current page, refreshing and running actions.

use std::sync::Mutex;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::adopt::{QueueItem, Scope};
use rosamta_core::apply::Context as Core;

use crate::actions;
use crate::pages::{self, review::Detail};
use crate::state::{self, Snapshot};
use crate::text::t;
use crate::theme::{palette, space::*};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Review,
    Modules,
    Dotfiles,
    History,
    Settings,
}

const PAGES: [(Page, &str); 6] = [
    (Page::Overview, "nav.overview"),
    (Page::Review, "nav.review"),
    (Page::Modules, "nav.modules"),
    (Page::Dotfiles, "nav.dotfiles"),
    (Page::History, "nav.history"),
    (Page::Settings, "nav.settings"),
];

/// How often the files core keeps are checked for changes.
const POLL: Duration = Duration::from_secs(2);

/// `rosamta://` URLs from the system, handed over to the window.
pub static URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub struct Rosamta {
    pub page: Page,
    pub snap: Snapshot,
    /// An action is running; buttons wait for it.
    pub busy: bool,
    pub selected: Option<u32>,
    pub scope: Scope,
    pub detail: Option<Detail>,
    pub search: Entity<InputState>,
    pub tag: Entity<InputState>,
    pub host_id: Entity<InputState>,
    pub url: Entity<InputState>,
    /// The host ID last put into `host_id`, so a new one replaces what is typed.
    pub shown_host_id: Option<String>,
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
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("modules.search")));
        let tag = cx.new(|cx| InputState::new(window, cx));
        let host_id = cx.new(|cx| InputState::new(window, cx));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder(t("setup.url")));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&tag, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_tag(window, cx);
                }
            }),
            cx.subscribe_in(&host_id, window, |this, _, event, window, cx| {
                // Enter only: renaming commits to the repo, so not on leaving the field.
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.rename(window, cx);
                }
            }),
            // Without the background service, new rc file content shows up
            // when the app is opened or brought back.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
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
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut this = Rosamta {
            page: Page::Overview,
            snap: Snapshot::empty(),
            busy: false,
            selected: None,
            scope: Scope::Host,
            detail: None,
            search,
            tag,
            host_id,
            url,
            shown_host_id: None,
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
                let problem = result.unwrap_or_else(|err| Some(err.to_string()));
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
            self.busy = false;
        }
        let item = |id| {
            self.snap
                .queue
                .iter()
                .find(|item: &&QueueItem| item.id == id)
        };
        let selected = self.selected.and_then(item).map(|item| item.id);
        let selected = selected.or_else(|| self.snap.queue.first().map(|item| item.id));
        if selected != self.selected {
            self.selected = selected;
            self.scope = Scope::Host;
            self.detail = None;
        }
        // The item may have changed underneath, or what accepting it writes.
        self.load_detail(cx);
        cx.notify();
    }

    pub fn select(&mut self, id: u32, cx: &mut Context<Self>) {
        self.page = Page::Review;
        if self.selected != Some(id) {
            self.selected = Some(id);
            self.scope = Scope::Host;
            self.detail = None;
            self.load_detail(cx);
        }
        cx.notify();
    }

    pub fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.detail = None;
            self.load_detail(cx);
            cx.notify();
        }
    }

    fn load_detail(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        let Some(item) = self.snap.queue.iter().find(|item| item.id == id).cloned() else {
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

    /// Run a write in the background; show its error or warning, then read
    /// everything again. Buttons wait until the new snapshot is shown.
    pub fn run(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&Core) -> actions::Done + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { Core::from_env().and_then(|core| action(&core)) })
                .await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(None) => {}
                    Ok(Some(warning)) => {
                        window.push_notification(Notification::warning(warning), cx)
                    }
                    Err(err) => {
                        let text = match &err {
                            // In this app's words, without the terminal advice.
                            rosamta_core::Error::Waiting(waiting) => {
                                crate::pages::review::waiting_text(waiting).to_string()
                            }
                            err => err.to_string(),
                        };
                        window.push_notification(Notification::error(text), cx);
                        // Put back what is saved, such as a host ID that was refused.
                        this.shown_host_id = None;
                    }
                }
                this.settling = true;
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn add_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tag = self.tag.read(cx).value().trim().to_string();
        if tag.is_empty() || self.busy {
            return;
        }
        self.tag
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.run(window, cx, move |core| actions::add_tag(core, tag));
    }

    fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.host_id.read(cx).value().trim().to_string();
        let current = self.snap.host.as_ref().map(|host| host.id.as_str());
        if id.is_empty() || Some(id.as_str()) == current {
            return;
        }
        self.run(window, cx, move |core| actions::rename(core, id));
    }

    /// Open the item a `rosamta://review/<id>` URL names.
    fn open_urls(&mut self, cx: &mut Context<Self>) {
        let urls = std::mem::take(&mut *URLS.lock().unwrap_or_else(|err| err.into_inner()));
        for url in urls {
            if let Some(id) = review_id(&url) {
                self.select(id, cx);
            }
        }
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let count = self.snap.queue.len();
        v_flex()
            .w(px(200.))
            .h_full()
            .flex_none()
            .bg(p.panel)
            .border_r_1()
            .border_color(p.line)
            .px(px(S12))
            .py(px(S16))
            .gap(px(S4))
            .child(
                div()
                    .px(px(S12))
                    .pb(px(S12))
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("rosamta"),
            )
            .children(PAGES.into_iter().map(|(page, key)| {
                let active = self.page == page;
                h_flex()
                    .id(key)
                    .px(px(S12))
                    .py(px(S8))
                    .rounded(px(S8))
                    .cursor_pointer()
                    .text_sm()
                    .text_color(if active { p.accent } else { p.text })
                    .when(active, |row| row.bg(p.accent_soft))
                    .when(!active, |row| row.hover(|style| style.bg(p.line)))
                    .child(div().flex_1().child(t(key)))
                    .when(page == Page::Review && count > 0, |row| {
                        row.child(pages::count_badge(count, cx))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = page;
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                h_flex()
                    .px(px(S12))
                    .gap(px(S8))
                    .text_sm()
                    .text_color(p.muted)
                    .child(pages::dot(self.snap.running, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .children(self.snap.host.as_ref().map(|host| host.id.clone())),
                    ),
            )
    }
}

impl Render for Rosamta {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let page = if !self.snap.ready {
            pages::setup::render(self, cx).into_any_element()
        } else {
            match self.page {
                Page::Overview => pages::overview::render(self, cx).into_any_element(),
                Page::Review => pages::review::render(self, cx).into_any_element(),
                Page::Modules => pages::modules::render(self, cx).into_any_element(),
                Page::Dotfiles => pages::dotfiles::render(self, cx).into_any_element(),
                Page::History => pages::history::render(self, cx).into_any_element(),
                Page::Settings => pages::settings::render(self, window, cx).into_any_element(),
            }
        };
        h_flex()
            .size_full()
            .bg(p.bg)
            .text_color(p.text)
            .when(self.snap.ready, |root| root.child(self.sidebar(cx)))
            .child(div().flex_1().h_full().min_w_0().child(page))
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

    use gpui_kit::TestAppContext;
    use rosamta_core::apply::Context as Core;

    // Not `super::*`: gpui's `test` attribute would replace the standard one.
    use super::{Detail, Page, Rosamta, actions};

    /// Set by the parent test for the child run in a home of its own.
    const OWN_HOME: &str = "ROSAMTA_TEST_OWN_HOME";

    /// The app reads HOME and the XDG paths everywhere, so the views are
    /// tested in a child process with a home of its own: setting the
    /// environment here would race the other tests, and never the real one.
    #[test]
    fn the_review_page_follows_the_queue_and_actions() {
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
            .args(["--exact", "app::view_tests::in_own_home", "--ignored"])
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

    /// Two copies that differ from the repo: two review items.
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
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            crate::theme::init(window, cx);
            Rosamta::new(None, window, cx)
        });
        cx.run_until_parked();
        let ids: Vec<u32> = view.read_with(cx, |view, _| {
            assert!(view.snap.ready);
            view.snap.queue.iter().map(|item| item.id).collect()
        });
        assert_eq!(ids.len(), 2);
        let diff_has = |view: &Rosamta, text: &str| matches!(&view.detail, Some(Detail::Dotfile(diff)) if diff.contains(text));
        view.read_with(cx, |view, _| {
            assert_eq!(view.snap.problem, None);
            assert_eq!(view.selected, Some(ids[0]));
            assert!(diff_has(view, "+repo.a"));
        });

        // Choosing an item opens it on the review page with its own diff.
        view.update(cx, |view, cx| {
            view.page = Page::Overview;
            view.select(ids[1], cx);
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
                view.run(window, cx, move |core| actions::discard(core, second));
                view.run(window, cx, move |core| actions::discard(core, first));
            })
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.busy);
            assert_eq!(view.snap.queue.len(), 1);
            assert_eq!(view.selected, Some(first));
            assert!(diff_has(view, "+repo.a"));
        });
        let read = |name| std::fs::read_to_string(core.home.join(name)).unwrap();
        assert_eq!(
            (read(".a"), read(".b")),
            ("mine\n".into(), "repo.b\n".into())
        );
    }
}
