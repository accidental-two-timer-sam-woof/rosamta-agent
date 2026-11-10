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
    _poll: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Rosamta {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("modules.search")));
        let tag = cx.new(|cx| InputState::new(window, cx).placeholder(t("settings.tags")));
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
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.rename(window, cx);
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
            _poll: poll,
            _subscriptions: subscriptions,
        };
        this.open_urls(cx);
        this.reload(cx);
        this
    }

    /// Read everything again in the background, then show it.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let snap = cx.background_spawn(async { state::load() }).await;
            this.update(cx, |this, cx| this.show(snap, cx)).ok();
        })
        .detach();
    }

    fn show(&mut self, snap: Snapshot, cx: &mut Context<Self>) {
        self.snap = snap;
        let item = |id| {
            self.snap
                .queue
                .iter()
                .find(|item: &&QueueItem| item.id == id)
        };
        let selected = self.selected.and_then(item).map(|item| item.id);
        let selected = selected.or_else(|| self.snap.queue.first().map(|item| item.id));
        if selected != self.selected || self.detail.is_none() {
            self.selected = selected;
            self.detail = None;
            self.load_detail(cx);
        }
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
        cx.spawn(async move |this, cx| {
            let detail = cx
                .background_spawn(async move { pages::review::detail(&item, scope) })
                .await;
            this.update(cx, |this, cx| {
                if this.selected == Some(id) && this.scope == scope {
                    this.detail = Some(detail);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Run a write in the background; show its error, then read everything again.
    pub fn run(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&Core) -> rosamta_core::Result<()> + Send + 'static,
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
                this.busy = false;
                if let Err(err) = result {
                    window.push_notification(Notification::error(err.to_string()), cx);
                }
                this.detail = None;
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn add_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tag = self.tag.read(cx).value().trim().to_string();
        if tag.is_empty() {
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
            .p(px(S12))
            .gap(px(S4))
            .child(
                div()
                    .px(px(S8))
                    .pb(px(S12))
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("rosamta"),
            )
            .children(PAGES.into_iter().map(|(page, key)| {
                let active = self.page == page;
                h_flex()
                    .id(key)
                    .px(px(S8))
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
                    .px(px(S8))
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
