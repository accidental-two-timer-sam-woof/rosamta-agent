//! What rosamta manages on this host: modules by load phase, each with its
//! switch, and dotfiles with how each is deployed and where it stands.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Icon, Sizable, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::adopt;
use rosamta_core::config::{Deploy, Phase};

use super::{Tone, badge, body, empty, frame, hint, mono, row, section, segmented, toolbar};
use crate::actions;
use crate::app::{LibraryTab, Rosamta, Selection};
use crate::state::{DotfileStatus, Kind};
use crate::text::t;
use crate::theme::{palette, space::*};

pub fn phase_key(phase: Phase) -> &'static str {
    match phase {
        Phase::Env => "phase.env",
        Phase::Early => "phase.early",
        Phase::Framework => "phase.framework",
        Phase::Tools => "phase.tools",
        Phase::Prompt => "phase.prompt",
        Phase::Aliases => "phase.aliases",
        Phase::Late => "phase.late",
        Phase::Login => "phase.login",
    }
}

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let query = app.search.read(cx).value().trim().to_lowercase();
    let tabs = segmented(
        "library-tabs",
        [(t("nav.modules"), true), (t("nav.dotfiles"), true)],
        usize::from(app.library == LibraryTab::Dotfiles),
        cx.listener(|this, index: &usize, _, cx| {
            this.library = if *index == 1 {
                LibraryTab::Dotfiles
            } else {
                LibraryTab::Modules
            };
            cx.notify();
        }),
    );
    let search = div().w(px(200.)).child(
        Input::new(&app.search)
            .small()
            .prefix(Icon::new(IconName::Search).small()),
    );
    let content = match app.library {
        LibraryTab::Modules => modules(app, &query, cx).into_any_element(),
        LibraryTab::Dotfiles => dotfiles(app, &query, cx).into_any_element(),
    };
    v_flex()
        .size_full()
        .child(toolbar(t("nav.library"), tabs, search, true, cx))
        .child(body("library", content))
}

/// What to say where a list has no rows: nothing matches, or there is nothing.
fn nothing(query: &str, none: &str, cx: &App) -> AnyElement {
    let key = if query.is_empty() {
        none
    } else {
        "library.no_match"
    };
    empty(t(key), cx).into_any_element()
}

fn modules(app: &Rosamta, query: &str, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let shown: Vec<_> = app
        .snap
        .modules
        .iter()
        .enumerate()
        .filter(|(_, module)| module.name.to_lowercase().contains(query))
        .collect();
    // Rows come sorted by phase; each run of one phase is a group.
    let mut groups: Vec<(Phase, Vec<_>)> = Vec::new();
    for (index, module) in shown {
        match groups.last_mut() {
            Some((phase, rows)) if *phase == module.phase => rows.push((index, module)),
            _ => groups.push((module.phase, vec![(index, module)])),
        }
    }
    let busy = app.busy.is_some();
    // Where the modules load from; not something a search looks for.
    let hooks = query.is_empty() && !app.snap.shells.is_empty();
    if groups.is_empty() && !hooks {
        return nothing(query, "library.no_modules", cx);
    }
    v_flex()
        .gap(px(S32))
        .when(hooks, |page| {
            page.child(section(t("library.hooks"), shells(app, cx), cx))
        })
        .children(groups.into_iter().map(|(phase, rows)| {
            let rows = rows.into_iter().map(|(index, module)| {
                let file = module.file.clone();
                let (icon, key) = match module.kind {
                    Kind::Structured => (IconName::List, "badge.structured"),
                    Kind::Raw => (IconName::Code, "badge.raw"),
                };
                row(cx)
                    .child(
                        Switch::new(("module", index))
                            .checked(module.enabled)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                                let (file, enabled) = (file.clone(), *checked);
                                this.run(window, cx, "module", move |core| {
                                    actions::set_module_enabled(core, file, enabled)
                                });
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(module.name.clone()),
                    )
                    .child(hint(("module-kind", index), icon, t(key), cx))
            });
            section(t(phase_key(phase)), v_flex().children(rows), cx)
        }))
        .into_any_element()
}

/// Each shell: its version, the startup files that load rosamta, its state.
fn shells(app: &Rosamta, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let home = &app.snap.home;
    v_flex().children(app.snap.shells.iter().map(|shell| {
        let version = shell
            .version
            .map(|(major, minor)| format!("{major}.{minor}"))
            .unwrap_or_else(|| "—".into());
        let files: Vec<String> = shell
            .hooked
            .iter()
            .map(|file| adopt::display(file, home))
            .collect();
        let (key, tone) = if !shell.installed {
            ("status.missing", Tone::Plain)
        } else if shell.hooked.is_empty() {
            ("status.unhooked", Tone::Warn)
        } else {
            ("status.hooked", Tone::Ok)
        };
        row(cx)
            .child(div().flex_none().w(px(S40)).child(shell.shell.name()))
            .child(
                div()
                    .flex_none()
                    .w(px(S32))
                    .text_color(p.muted)
                    .child(version),
            )
            .child(
                mono(files.join("  "), cx)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(p.muted),
            )
            .child(badge(t(key), tone, cx))
    }))
}

fn dotfiles(app: &Rosamta, query: &str, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let snap = &app.snap;
    let home = &snap.home;
    let shown: Vec<_> = snap
        .dotfiles
        .iter()
        .enumerate()
        .map(|(index, dotfile)| {
            (
                index,
                dotfile,
                adopt::display(&home.join(&dotfile.path), home),
            )
        })
        .filter(|(_, _, path)| path.to_lowercase().contains(query))
        .collect();
    if shown.is_empty() {
        return nothing(query, "library.no_dotfiles", cx);
    }
    let rows = shown.into_iter().map(|(index, dotfile, path)| {
        let (icon, deploy) = match dotfile.deploy {
            Deploy::Link => (IconName::Link, "dotfiles.link"),
            Deploy::Copy => (IconName::Copy, "dotfiles.copy"),
            Deploy::Template => (IconName::FileCode, "dotfiles.template"),
            Deploy::Create => (IconName::FilePlus, "dotfiles.create"),
        };
        let (status, tone) = match dotfile.status {
            DotfileStatus::Deployed => ("dotfiles.deployed", Tone::Ok),
            DotfileStatus::Pending => ("dotfiles.pending", Tone::Warn),
            DotfileStatus::Skipped => ("dotfiles.skipped", Tone::Plain),
            DotfileStatus::NotDeployed => ("dotfiles.not_deployed", Tone::Plain),
        };
        // One that waits for a decision leads to it.
        let item = snap.queue.iter().find(|item| {
            item.dotfile
                .as_ref()
                .is_some_and(|queued| queued.path == dotfile.path)
        });
        let p = palette(cx);
        row(cx)
            .id(("dotfile", index))
            .child(mono(path, cx).flex_1().min_w_0().truncate())
            .when(dotfile.encrypted, |row| {
                row.child(hint(
                    ("dotfile-locked", index),
                    IconName::Lock,
                    t("badge.encrypted"),
                    cx,
                ))
            })
            .child(hint(("dotfile-deploy", index), icon, t(deploy), cx))
            .child(badge(t(status), tone, cx))
            .when_some(item.map(|item| item.id), |row, id| {
                row.cursor_pointer()
                    .hover(|style| style.bg(p.well))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select(Selection::Item(id), cx)),
                    )
            })
    });
    frame(cx)
        .child(div().mb(px(-1.)).children(rows))
        .into_any_element()
}
