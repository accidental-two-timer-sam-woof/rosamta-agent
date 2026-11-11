//! What needs the user: problems, items to decide on, and items that wait.
//! The list, and for the selected row what it is about.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable, Icon, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::adopt::{self, QueueItem, Scope};
use rosamta_core::apply::{self, Context as Core};
use rosamta_core::dotfiles::{
    self,
    review::{Reason, WaitingFor},
};
use rosamta_core::problems::Problem;
use rosamta_core::repo;
use rosamta_core::scripts::{Run, apply::Why, review::ScriptItem, run::log_path};

use super::{
    bordered, borderless, card, code, diff, empty, frame, icon_button, mono, prominent, segmented,
    toolbar,
};
use crate::actions;
use crate::app::{Rosamta, Selection};
use crate::text::{lang, t, tf, when};
use crate::theme::{FADE, font, palette, radius, size, space::*};

/// What the selected item would change, read in the background.
pub enum Detail {
    Rc {
        /// The rc files and their unified diffs.
        files: Vec<(PathBuf, String)>,
        /// New module files: where, and their content.
        modules: Vec<(String, String)>,
    },
    Dotfile(String),
    Script {
        path: PathBuf,
        text: String,
        /// The end of the last run's log, if it ran.
        log: Option<String>,
    },
    Error(String),
}

/// Enough of a script to read, and of a log to see how the last run ended.
const SHOWN: usize = 64 * 1024;

fn script_detail(core: &Core, script: &ScriptItem) -> rosamta_core::Result<Detail> {
    let path = core.paths.repo().join(&script.source);
    let text = std::fs::read(&path).map_err(|source| rosamta_core::Error::Read {
        path: path.clone(),
        source,
    })?;
    let log = std::fs::read(log_path(core, &script.source))
        .ok()
        .map(|log| {
            let mut start = log.len().saturating_sub(SHOWN);
            if start > 0 {
                // From the first whole line on.
                start += log[start..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(0, |at| at + 1);
            }
            String::from_utf8_lossy(&log[start..]).into_owned()
        });
    Ok(Detail::Script {
        path,
        text: String::from_utf8_lossy(&text[..text.len().min(SHOWN)]).into_owned(),
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
                let text = apply::unified(&name, change.before.as_deref(), change.after.as_deref());
                (change.path.clone(), text)
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
    read().unwrap_or_else(|err| Detail::Error(crate::text::plain(&err.to_string()).to_string()))
}

/// Not a decision: the item cannot be made until something else happens.
pub fn waits(item: &QueueItem) -> bool {
    item.dotfile
        .as_ref()
        .is_some_and(|dotfile| dotfile.waiting.is_some())
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

/// What happened, in one line.
fn item_what(item: &QueueItem, home: &Path) -> SharedString {
    if let Some(script) = &item.script {
        let run = t(match script.run {
            Run::Once => "item.run_once",
            Run::Onchange => "item.run_onchange",
        });
        let why = t(match script.why {
            Why::New => "item.why_new",
            Why::Changed => "item.why_changed",
            Why::Failed => "item.why_failed",
        });
        return format!("{run} · {why}").into();
    }
    let Some(dotfile) = &item.dotfile else {
        let file = adopt::display(&item.file, home);
        return tf("item.new_content", &[("file", &file)]);
    };
    match (&dotfile.waiting, dotfile.reason) {
        (Some(waiting), _) => waiting_text(waiting),
        (None, Reason::Conflict) => t("item.both_changed"),
        (None, _) => t("item.local_changed"),
    }
}

fn time(at: u64) -> Option<String> {
    (at > 0).then(|| when(at))
}

/// A problem's short name; its full text is what the notification says.
fn problem_title(problem: &Problem) -> SharedString {
    t(&format!("problem_title.{}", problem.kind))
}

/// What a problem is about, in one line.
fn problem_what(problem: &Problem, unpushed: u32) -> SharedString {
    if problem.kind == "offline" {
        return tf("status.unpushed", &[("count", &unpushed.to_string())]);
    }
    if problem.arg.is_empty() {
        return problem.text(lang()).into();
    }
    problem.arg.clone().into()
}

/// Trying again can clear it: the next sync publishes and pulls again.
fn retryable(problem: &Problem) -> bool {
    problem.attempt.is_some() && problem.kind != "upgrade"
}

/// One row of the list: a name and when, then what happened. A selected
/// row differs by its background alone.
fn list_row(
    id: impl Into<ElementId>,
    selected: bool,
    title: String,
    at: Option<String>,
    what: SharedString,
    cx: &App,
) -> Stateful<Div> {
    let p = palette(cx);
    v_flex()
        .id(id)
        .px(px(14.))
        .py(px(10.))
        .gap(px(S2))
        .border_b_1()
        .border_color(p.line)
        .cursor_pointer()
        .when(selected, |row| row.bg(p.accent_soft))
        .when(!selected, |row| row.hover(|style| style.bg(p.well)))
        .child(
            h_flex()
                .gap(px(S8))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .children(at.map(|at| {
                    div()
                        .flex_none()
                        .text_size(px(font::CAPTION))
                        .text_color(p.muted)
                        .child(at)
                })),
        )
        .child(
            div()
                .truncate()
                .text_size(px(font::MONO))
                .text_color(p.muted)
                .child(what),
        )
}

/// A block of the list: its icon, name and count over its rows, in a frame
/// of its own. What only waits is framed with dashes.
fn group(
    icon: IconName,
    tint: Hsla,
    title: &str,
    rows: Vec<Stateful<Div>>,
    waiting: bool,
    cx: &App,
) -> Option<impl IntoElement> {
    if rows.is_empty() {
        return None;
    }
    let p = palette(cx);
    let count = rows.len().to_string();
    // The last row's rule hides under the frame.
    let rows = div().mb(px(-1.)).children(rows);
    let rows = if waiting {
        v_flex()
            .rounded(px(radius::BOX))
            .border_1()
            .border_dashed()
            .border_color(p.line_strong)
            .overflow_hidden()
            .child(rows)
    } else {
        frame(cx).child(rows)
    };
    Some(
        v_flex()
            .gap(px(S8))
            .child(
                h_flex()
                    .px(px(S4))
                    .gap(px(6.))
                    .child(Icon::new(icon).size(px(14.)).text_color(tint))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t(title)),
                    )
                    .child(div().text_color(p.muted).child(count)),
            )
            .child(rows),
    )
}

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let p = palette(cx);
    let snap = &app.snap;
    if snap.queue.is_empty() && snap.problems.is_empty() {
        return v_flex()
            .size_full()
            .child(toolbar(t("nav.review"), div(), div(), true, cx))
            .child(empty(t("item.empty"), cx));
    }
    let problems = snap
        .problems
        .iter()
        .enumerate()
        .map(|(index, problem)| {
            let row = Selection::Problem(problem.key());
            let at = problem
                .attempt
                .as_ref()
                .and_then(|attempt| time(attempt.at));
            list_row(
                ("problem", index),
                app.selected.as_ref() == Some(&row),
                problem_title(problem).to_string(),
                at,
                problem_what(problem, snap.sync.unpushed),
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select(row.clone(), cx)))
        })
        .collect();
    let item_row = |item: &QueueItem, cx: &mut Context<Rosamta>| {
        let id = item.id;
        list_row(
            ("item", id),
            app.selected == Some(Selection::Item(id)),
            item_title(item, &snap.home),
            time(item.detected_at),
            item_what(item, &snap.home),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.select(Selection::Item(id), cx)))
    };
    let (waiting, decide): (Vec<&QueueItem>, Vec<&QueueItem>) =
        snap.queue.iter().partition(|item| waits(item));
    let decide = decide.into_iter().map(|item| item_row(item, cx)).collect();
    let waiting = waiting.into_iter().map(|item| item_row(item, cx)).collect();
    let list = v_flex()
        .id("review-list")
        .w(px(size::LIST))
        .flex_none()
        .h_full()
        .overflow_y_scroll()
        .bg(p.well)
        .border_r_1()
        .border_color(p.line)
        .pl(px(S24))
        .pr(px(S16))
        .py(px(S20))
        .gap(px(S24))
        .children(group(
            IconName::TriangleAlert,
            p.warn,
            "item.problems",
            problems,
            false,
            cx,
        ))
        .children(group(
            IconName::CircleQuestionMark,
            p.accent,
            "item.decide",
            decide,
            false,
            cx,
        ))
        .children(group(
            IconName::Clock,
            p.muted,
            "item.waiting",
            waiting,
            true,
            cx,
        ));
    let (actions, shown) = match (app.selected_item(), app.selected_problem()) {
        (Some(item), _) => (
            item_actions(app, item, cx).into_any_element(),
            item_detail(app, item, cx).into_any_element(),
        ),
        (None, Some(problem)) => (
            problem_actions(app, problem, cx).into_any_element(),
            problem_detail(app, problem, cx).into_any_element(),
        ),
        (None, None) => (div().into_any_element(), div().into_any_element()),
    };
    // Fades in when another row is chosen and when its content arrives, not
    // when the same content is read again.
    let mut hasher = DefaultHasher::new();
    (&app.selected, app.detail.is_some()).hash(&mut hasher);
    let shown = div()
        .id("review-detail")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .px(px(S40))
        .py(px(S24))
        .child(shown)
        .with_animation(
            ElementId::NamedInteger("review-detail".into(), hasher.finish()),
            Animation::new(FADE).with_easing(ease_in_out),
            |detail, delta| detail.opacity(delta),
        );
    v_flex()
        .size_full()
        .child(toolbar(t("nav.review"), div(), actions, true, cx))
        .child(
            h_flex()
                .flex_1()
                .min_h_0()
                .items_start()
                .child(list)
                .child(shown),
        )
}

/// The selected item's choices, the main one last.
fn item_actions(app: &Rosamta, item: &QueueItem, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let id = item.id;
    let busy = app.busy.is_some();
    let row = h_flex().gap(px(S8));
    if waits(item) {
        return row;
    }
    let ignore = borderless(Button::new("ignore"))
        .label(t("action.ignore"))
        .disabled(busy)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.run(window, cx, "ignore", move |core| actions::ignore(core, id));
        }));
    if let Some(shown) = item.script.clone() {
        // Marking it as run is what ignoring a script does.
        return row
            .child(
                borderless(Button::new("mark-run"))
                    .label(t("item.mark_run"))
                    .disabled(busy)
                    .on_click(cx.listener({
                        let shown = shown.clone();
                        move |this, _, window, cx| {
                            let shown = shown.clone();
                            this.run(window, cx, "mark-run", move |core| {
                                actions::mark_run(core, id, shown)
                            });
                        }
                    })),
            )
            .child(
                prominent(Button::new("run"))
                    .label(t("action.run"))
                    .loading(app.busy == Some("run"))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let shown = shown.clone();
                        this.run(window, cx, "run", move |core| {
                            actions::run_script(core, id, shown)
                        });
                    })),
            );
    }
    if item.dotfile.is_some() {
        let file = item_title(item, &app.snap.home);
        return row
            .child(ignore)
            .child(
                bordered(Button::new("discard"), cx)
                    .label(t("item.use_repo"))
                    .loading(app.busy == Some("discard"))
                    .disabled(busy)
                    // It replaces a file this app cannot bring back.
                    .on_click(cx.listener(move |_, _, window, cx| {
                        let (view, file) = (cx.weak_entity(), file.clone());
                        window.open_alert_dialog(cx, move |alert, _, _| {
                            let view = view.clone();
                            alert
                                .confirm()
                                .title(tf("confirm.use_repo", &[("file", &file)]))
                                .ok_text(t("confirm.replace"))
                                .ok_variant(ButtonVariant::Danger)
                                .cancel_text(t("confirm.cancel"))
                                .on_ok(move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.run(window, cx, "discard", move |core| {
                                            actions::discard(core, id)
                                        });
                                    })
                                    .ok();
                                    true
                                })
                        });
                    })),
            )
            .child(
                prominent(Button::new("keep"))
                    .label(t("item.keep_local"))
                    .loading(app.busy == Some("keep"))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(window, cx, "keep", move |core| {
                            actions::accept(core, id, Scope::Host)
                        });
                    })),
            );
    }
    // Lines holding a secret stay on this host.
    let local = item.local;
    let scopes = [
        (t("item.this_host"), !busy),
        (t("item.all_hosts"), !busy && !local),
    ];
    row.child(segmented(
        "scope",
        scopes,
        usize::from(app.scope == Scope::All),
        cx.listener(move |this, index: &usize, _, cx| {
            let scope = if *index == 1 && !local {
                Scope::All
            } else {
                Scope::Host
            };
            this.set_scope(scope, cx);
        }),
    ))
    .child(ignore)
    .child(
        prominent(Button::new("adopt"))
            .label(t("action.adopt"))
            .loading(app.busy == Some("adopt"))
            .disabled(busy)
            .on_click(cx.listener(move |this, _, window, cx| {
                let scope = this.scope;
                this.run(window, cx, "adopt", move |core| {
                    actions::accept(core, id, scope)
                });
            })),
    )
}

/// The button that opens a file the detail shows.
/// Opens `path`, or with `reveal` shows it in its folder: a script waits
/// for a decision, and whatever the system opens it with may run it.
fn open_button(index: usize, path: PathBuf, reveal: bool, cx: &mut Context<Rosamta>) -> AnyElement {
    icon_button(
        ("open-file", index),
        IconName::ExternalLink,
        t("item.open_file"),
    )
    .on_click(cx.listener(move |_, _, window, cx| {
        if reveal {
            return cx.reveal_path(&path);
        }
        match actions::to_open(&path) {
            Ok(target) => cx.open_with_system(&target),
            Err(err) => window.push_notification(Notification::error(err.to_string()), cx),
        }
    }))
    .into_any_element()
}

/// A detail's title, and under it what it is about.
fn headline(title: String, about: Vec<String>, cx: &App) -> impl IntoElement {
    v_flex()
        .gap(px(S2))
        .child(
            div()
                .text_size(px(font::TITLE))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(font::CAPTION))
                .text_color(palette(cx).muted)
                .child(about.join(" · ")),
        )
}

fn item_detail(app: &Rosamta, item: &QueueItem, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let p = palette(cx);
    let home = &app.snap.home;
    let about = [
        Some(item_what(item, home).to_string()),
        time(item.detected_at),
    ];
    let page = v_flex().gap(px(S20)).child(headline(
        item_title(item, home),
        about.into_iter().flatten().collect(),
        cx,
    ));
    if waits(item) {
        return page;
    }
    match &app.detail {
        None => page,
        Some(Detail::Error(err)) => page.child(div().text_color(p.warn).child(err.clone())),
        Some(Detail::Dotfile(text)) => {
            let path = item
                .dotfile
                .as_ref()
                .map(|dotfile| home.join(&dotfile.path));
            let path = path.unwrap_or_default();
            page.child(card(
                t("item.diff"),
                Some(adopt::display(&path, home).into()),
                Some(open_button(0, path, false, cx)),
                diff(text, cx),
                cx,
            ))
        }
        Some(Detail::Script { path, text, log }) => page
            .child(card(
                t("item.script"),
                None,
                Some(open_button(0, path.clone(), true, cx)),
                code(text, cx),
                cx,
            ))
            .when_some(log.as_ref(), |page, log| {
                page.child(card(t("item.last_run"), None, None, code(log, cx), cx))
            }),
        Some(Detail::Rc { files, modules }) => page
            .children(files.iter().enumerate().map(|(index, (path, text))| {
                card(
                    t("item.rc"),
                    Some(adopt::display(path, home).into()),
                    Some(open_button(index, path.clone(), false, cx)),
                    diff(text, cx),
                    cx,
                )
            }))
            .children(modules.iter().map(|(name, text)| {
                card(
                    t("item.module"),
                    Some(name.clone().into()),
                    None,
                    code(text, cx),
                    cx,
                )
            })),
    }
}

fn problem_actions(
    app: &Rosamta,
    problem: &Problem,
    cx: &mut Context<Rosamta>,
) -> impl IntoElement {
    let kind = problem.kind;
    h_flex().when(retryable(problem), |row| {
        row.child(
            prominent(Button::new("retry"))
                .label(t("action.retry"))
                .loading(app.busy == Some("sync"))
                .disabled(app.busy.is_some() || app.snap.remote.is_none())
                .on_click(cx.listener(move |this, _, window, cx| this.retry(kind, window, cx))),
        )
    })
}

fn problem_detail(app: &Rosamta, problem: &Problem, cx: &App) -> impl IntoElement {
    let attempt = problem.attempt.as_ref();
    let unpushed = app.snap.sync.unpushed;
    let about = [
        attempt.and_then(|attempt| time(attempt.at)),
        (unpushed > 0).then(|| tf("status.unpushed", &[("count", &unpushed.to_string())]).into()),
    ];
    let reason = attempt
        .and_then(|attempt| attempt.reason.as_deref())
        .filter(|reason| !reason.is_empty());
    let files = attempt
        .map(|attempt| attempt.files.as_slice())
        .unwrap_or_default();
    v_flex()
        .gap(px(S20))
        .child(headline(
            problem.text(lang()),
            about.into_iter().flatten().collect(),
            cx,
        ))
        .when_some(reason, |page, reason| {
            page.child(card(
                t("item.error"),
                None,
                None,
                mono(crate::text::plain(reason).to_string(), cx)
                    .px(px(S16))
                    .py(px(S12)),
                cx,
            ))
        })
        .when(!files.is_empty(), |page| {
            page.child(card(
                t("item.files"),
                None,
                None,
                v_flex()
                    .px(px(S16))
                    .py(px(S12))
                    .gap(px(S4))
                    .children(files.iter().map(|file| mono(file.clone(), cx))),
                cx,
            ))
        })
}
