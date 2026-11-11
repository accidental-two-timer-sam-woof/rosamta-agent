//! Items to review: the list, and for the selected one its diff and what
//! accepting it would write.

use std::path::Path;

use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::adopt::{self, QueueItem, Scope};
use rosamta_core::apply::{self, Context as Core};
use rosamta_core::dotfiles::{
    self,
    review::{Reason, WaitingFor},
};
use rosamta_core::repo;
use rosamta_core::scripts::{Run, apply::Why, review::ScriptItem, run::log_path};

use super::{Tone, badge, code, diff, empty, header, mono};
use crate::actions;
use crate::app::Rosamta;
use crate::text::{t, tf, when};
use crate::theme::{palette, space::*};

/// What the selected item would change, read in the background.
pub enum Detail {
    Rc {
        /// Unified diffs of the rc files.
        files: Vec<String>,
        /// New module files: where, and their content.
        modules: Vec<(String, String)>,
    },
    Dotfile(String),
    Script {
        text: String,
        /// The end of the last run's log, if it ran.
        log: Option<String>,
    },
    Error(String),
}

/// Enough of a log to see how the last run ended.
const LOG_TAIL: usize = 64 * 1024;

fn script_detail(core: &Core, script: &ScriptItem) -> rosamta_core::Result<Detail> {
    let path = core.paths.repo().join(&script.source);
    let text = std::fs::read(&path).map_err(|source| rosamta_core::Error::Read { path, source })?;
    let log = std::fs::read(log_path(core, &script.source))
        .ok()
        .map(|log| {
            let start = log.len().saturating_sub(LOG_TAIL);
            String::from_utf8_lossy(&log[start..]).into_owned()
        });
    Ok(Detail::Script {
        text: String::from_utf8_lossy(&text).into_owned(),
        log,
    })
}

pub fn detail(item: &QueueItem, scope: Scope) -> Detail {
    let read = || -> rosamta_core::Result<Detail> {
        let core = Core::from_env()?;
        if let Some(script) = &item.script {
            return script_detail(&core, script);
        }
        if let Some(dotfile) = &item.dotfile {
            if dotfile.waiting.is_some() {
                // Nothing rendered yet to compare; `selected` shows what it waits for.
                return Ok(Detail::Dotfile(String::new()));
            }
            return Ok(Detail::Dotfile(dotfiles::review::diff(&core, dotfile)?));
        }
        let plan = adopt::preview(&core, item.id, scope)?;
        let files = plan
            .files
            .iter()
            .map(|change| {
                let name = adopt::display(&change.path, &core.home);
                apply::unified(&name, change.before.as_deref(), change.after.as_deref())
            })
            .collect();
        let (repo_dir, local) = (core.paths.repo(), core.paths.local());
        let modules = plan
            .modules
            .iter()
            .map(|(path, text)| {
                let name = path
                    .strip_prefix(&repo_dir)
                    .or_else(|_| path.strip_prefix(&local))
                    .map(|name| name.display().to_string())
                    .unwrap_or_else(|_| adopt::display(path, &core.home));
                (name, text.clone())
            })
            .collect();
        Ok(Detail::Rc { files, modules })
    };
    read().unwrap_or_else(|err| Detail::Error(err.to_string()))
}

pub fn item_badge(item: &QueueItem) -> (&'static str, Tone) {
    if let Some(script) = &item.script {
        return match script.why {
            Why::Failed => ("badge.failed", Tone::Warn),
            Why::New | Why::Changed => ("badge.script", Tone::Accent),
        };
    }
    match item.dotfile.as_ref().map(|dotfile| dotfile.reason) {
        Some(Reason::Conflict) => ("badge.conflict", Tone::Warn),
        Some(Reason::Changed) => ("badge.changed", Tone::Accent),
        Some(Reason::Waiting) => ("badge.waiting", Tone::Plain),
        None => ("badge.adopt", Tone::Accent),
    }
}

pub fn waiting_text(waiting: &WaitingFor) -> SharedString {
    match waiting {
        WaitingFor::Locked { tool } => tf("item.waiting_locked", &[("tool", tool)]),
        WaitingFor::Command { name } => tf("item.waiting_command", &[("command", name)]),
        WaitingFor::Unset { vars } => tf(
            "item.waiting_unset",
            &[("vars", &dotfiles::review::var_list(vars))],
        ),
    }
}

/// The module it would become, or the dotfile it is.
pub fn item_title(item: &QueueItem, home: &Path) -> String {
    if let Some(script) = &item.script {
        return script.source.clone();
    }
    match (&item.dotfile, &item.module) {
        (Some(dotfile), _) => adopt::display(&home.join(&dotfile.path), home),
        (None, Some(module)) => repo::module_name(module),
        (None, None) => adopt::display(&item.file, home),
    }
}

pub fn item_meta(item: &QueueItem, home: &Path) -> String {
    let mut parts = Vec::new();
    if let Some(script) = &item.script {
        parts.push(t(match script.run {
            Run::Once => "item.run_once",
            Run::Onchange => "item.run_onchange",
        }));
        parts.push(t(match script.why {
            Why::New => "item.why_new",
            Why::Changed => "item.why_changed",
            Why::Failed => "item.why_failed",
        }));
    } else if item.dotfile.is_none() {
        parts.push(adopt::display(&item.file, home).into());
        let (start, end) = (item.start.to_string(), item.end.to_string());
        let lines = if item.start == item.end {
            tf("item.line", &[("start", &start)])
        } else {
            tf("item.lines", &[("start", &start), ("end", &end)])
        };
        parts.push(lines);
    }
    if item.detected_at > 0 {
        parts.push(when(item.detected_at).into());
    }
    parts.join(" · ")
}

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let p = palette(cx);
    let snap = &app.snap;
    let body = if snap.queue.is_empty() {
        div()
            .px(px(S32))
            .child(empty(t("item.empty"), cx))
            .into_any_element()
    } else {
        let list = v_flex()
            .id("review-list")
            .w(px(320.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_t_1()
            .border_r_1()
            .border_color(p.line)
            .children(snap.queue.iter().map(|item| {
                let id = item.id;
                let selected = app.selected == Some(id);
                let (key, tone) = item_badge(item);
                v_flex()
                    .id(("item", id))
                    .px(px(S16))
                    .py(px(S12))
                    .gap(px(S4))
                    .border_b_1()
                    .border_color(p.line)
                    .cursor_pointer()
                    .relative()
                    .when(selected, |card| {
                        card.bg(p.panel).child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(S4))
                                .bg(p.accent),
                        )
                    })
                    .when(!selected, |card| card.hover(|style| style.bg(p.panel)))
                    .child(
                        h_flex()
                            .gap(px(S8))
                            .text_sm()
                            .child(badge(t(key), tone, cx))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(item_title(item, &snap.home)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted)
                            .truncate()
                            .child(item_meta(item, &snap.home)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
            }));
        h_flex()
            .size_full()
            .items_start()
            .child(list)
            .children(selected(app, cx))
            .into_any_element()
    };
    v_flex()
        .size_full()
        .child(header(t("nav.review"), div(), cx))
        .child(div().flex_1().min_h_0().child(body))
}

fn selected(app: &Rosamta, cx: &mut Context<Rosamta>) -> Option<impl IntoElement> {
    let p = palette(cx);
    let item = app
        .snap
        .queue
        .iter()
        .find(|item| Some(item.id) == app.selected)?;
    let id = item.id;
    let busy = app.busy;
    let waiting = item
        .dotfile
        .as_ref()
        .and_then(|dotfile| dotfile.waiting.as_ref());
    let ignore = super::ghost(Button::new("ignore"))
        .label(t("action.ignore"))
        .disabled(busy)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.run(window, cx, move |core| actions::ignore(core, id));
        }));
    let buttons = if item.script.is_some() {
        // Marking it as run is what ignoring a script does.
        h_flex()
            .gap(px(S8))
            .child(
                super::glass(Button::new("mark-run"), cx)
                    .label(t("item.mark_run"))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(window, cx, move |core| actions::ignore(core, id));
                    })),
            )
            .child(
                super::prominent(Button::new("run"))
                    .label(t("action.run"))
                    .loading(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(window, cx, move |core| actions::run_script(core, id));
                    })),
            )
    } else if item.dotfile.is_some() {
        h_flex()
            .gap(px(S8))
            .child(
                super::glass(Button::new("discard"), cx)
                    .label(t("item.use_repo"))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(window, cx, move |core| actions::discard(core, id));
                    })),
            )
            .child(
                super::prominent(Button::new("keep"))
                    .label(t("item.keep_local"))
                    .loading(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(window, cx, move |core| {
                            actions::accept(core, id, Scope::Host)
                        });
                    })),
            )
            .child(ignore)
    } else {
        let scope = app.scope;
        h_flex()
            .gap(px(S8))
            .child(
                // `children`, unlike `child`, keeps each button's own `disabled`.
                super::segments(ButtonGroup::new("scope"), cx)
                    .children([
                        super::segment(Button::new("this-host"), scope == Scope::Host, cx)
                            .label(t("item.this_host")),
                        super::segment(Button::new("all-hosts"), scope == Scope::All, cx)
                            .label(t("item.all_hosts"))
                            // Lines holding a secret stay on this host.
                            .disabled(item.local),
                    ])
                    .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                        let scope = if clicks.contains(&1) {
                            Scope::All
                        } else {
                            Scope::Host
                        };
                        this.set_scope(scope, cx);
                    })),
            )
            .child(
                super::prominent(Button::new("adopt"))
                    .label(t("action.adopt"))
                    .loading(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let scope = this.scope;
                        this.run(window, cx, move |core| actions::accept(core, id, scope));
                    })),
            )
            .child(ignore)
    };
    let content = match (&app.detail, waiting) {
        // Not a decision: no diff and no buttons until it can render.
        (_, Some(waiting)) => div()
            .text_sm()
            .text_color(p.muted)
            .child(waiting_text(waiting))
            .into_any_element(),
        (None, _) => div().into_any_element(),
        (Some(Detail::Error(err)), _) => div()
            .text_sm()
            .text_color(p.warn)
            .child(err.clone())
            .into_any_element(),
        (Some(Detail::Dotfile(text)), _) => diff(text, cx).into_any_element(),
        (Some(Detail::Script { text, log }), _) => v_flex()
            .gap(px(S16))
            .child(heading(t("item.script"), cx))
            .child(code(text, cx))
            .when_some(log.as_ref(), |content, log| {
                content
                    .child(heading(t("item.last_run"), cx))
                    .child(code(log, cx))
            })
            .into_any_element(),
        (Some(Detail::Rc { files, modules }), _) => v_flex()
            .gap(px(S16))
            .child(heading(t("item.rc"), cx))
            .children(files.iter().map(|text| diff(text, cx)))
            .when(!modules.is_empty(), |content| {
                content
                    .child(heading(t("item.module"), cx))
                    .children(modules.iter().map(|(name, text)| {
                        v_flex()
                            .gap(px(S8))
                            .child(mono(name.clone(), cx).text_color(p.muted))
                            .child(code(text, cx))
                    }))
            })
            .into_any_element(),
    };
    Some(
        v_flex()
            .id("review-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .border_t_1()
            .border_color(p.line)
            .p(px(S24))
            .gap(px(S16))
            .child(
                h_flex()
                    .gap(px(S12))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(item_title(item, &app.snap.home)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted)
                                    .truncate()
                                    .child(item_meta(item, &app.snap.home)),
                            ),
                    )
                    .when(waiting.is_none(), |row| row.child(buttons)),
            )
            .child(content),
    )
}

fn heading(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette(cx).muted)
        .child(text)
}
