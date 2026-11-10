//! Modules by load phase, each with its switch.

use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, v_flex};
use gpui_kit::*;
use rosamta_core::config::Phase;

use super::{Tone, badge, empty, page, row, section};
use crate::actions;
use crate::app::Rosamta;
use crate::state::Kind;
use crate::text::t;
use crate::theme::space::*;

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
    let shown: Vec<_> = app
        .snap
        .modules
        .iter()
        .enumerate()
        .filter(|(_, module)| module.name.to_lowercase().contains(&query))
        .collect();
    // Rows come sorted by phase; each run of one phase is a group.
    let mut groups: Vec<(Phase, Vec<_>)> = Vec::new();
    for (index, module) in shown {
        match groups.last_mut() {
            Some((phase, rows)) if *phase == module.phase => rows.push((index, module)),
            _ => groups.push((module.phase, vec![(index, module)])),
        }
    }
    let busy = app.busy;
    let body = if groups.is_empty() {
        empty(t("modules.empty"), cx).into_any_element()
    } else {
        v_flex()
            .gap(px(S24))
            .children(groups.into_iter().map(|(phase, rows)| {
                let rows = rows.into_iter().map(|(index, module)| {
                    let file = module.file.clone();
                    let (key, tone) = match module.kind {
                        Kind::Structured => ("badge.structured", Tone::Plain),
                        Kind::Raw => ("badge.raw", Tone::Plain),
                    };
                    row(cx)
                        .child(
                            Switch::new(("module", index))
                                .checked(module.enabled)
                                .disabled(busy)
                                .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                                    let (file, enabled) = (file.clone(), *checked);
                                    this.run(window, cx, move |core| {
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
                        .child(badge(t(key), tone, cx))
                });
                section(t(phase_key(phase)), v_flex().children(rows), cx)
            }))
            .into_any_element()
    };
    let search = div().w(px(240.)).child(Input::new(&app.search));
    page("modules", t("nav.modules"), search, body, cx)
}
