//! What importing from chezmoi did: the counts, and what was not imported.

use gpui_kit::component::button::Button;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::chezmoi::{Item, Outcome, Reason};

use super::{label, mono, page, prominent, row, section};
use crate::app::Rosamta;
use crate::text::t;
use crate::theme::{palette, space::*};

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let Some(report) = &app.imported else {
        return div().into_any_element();
    };
    let count = |key: &str, outcome| {
        row(cx)
            .child(label(t(key), cx))
            .child(report.count(outcome).to_string())
    };
    let manual: Vec<&Item> = report
        .items
        .iter()
        .filter(|item| item.outcome == Outcome::Manual)
        .collect();
    let body = v_flex()
        .gap(px(S32))
        .child(section(
            t("import.result"),
            v_flex()
                .child(count("import.imported", Outcome::Imported))
                .child(count("import.skipped", Outcome::Skipped))
                .child(count("import.manual", Outcome::Manual)),
            cx,
        ))
        .when(!manual.is_empty(), |body| {
            body.child(section(t("import.manual"), list(&manual, cx), cx))
        });
    let done = prominent(Button::new("import-done"))
        .label(t("import.done"))
        .on_click(cx.listener(|this, _, _, cx| {
            this.imported = None;
            cx.notify();
        }));
    page("import", t("import.title"), done, body, cx).into_any_element()
}

fn list(items: &[&Item], cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex().children(items.iter().map(|item| {
        row(cx)
            .child(mono(item.source.clone(), cx).flex_1().min_w_0().truncate())
            .child(
                div()
                    .flex_none()
                    .text_color(p.muted)
                    .child(t(why(item.reason))),
            )
    }))
}

fn why(reason: Reason) -> &'static str {
    match reason {
        Reason::Template => "import.why_template",
        Reason::Secret => "import.why_secret",
        Reason::Encrypted => "import.why_encrypted",
        Reason::Scope => "import.why_scope",
        Reason::Differs => "import.why_differs",
        Reason::Modify => "import.why_modify",
        Reason::Remove => "import.why_remove",
        Reason::Symlink => "import.why_symlink",
        Reason::CreateTemplate => "import.why_create_template",
        Reason::ExactDir => "import.why_exact_dir",
        Reason::DirPermissions => "import.why_dir_permissions",
        Reason::EmptyDir => "import.why_empty_dir",
        Reason::ExternalType => "import.why_external_type",
        Reason::ExternalField => "import.why_external_field",
        Reason::ExternalTemplate => "import.why_external_template",
        Reason::Nested => "import.why_nested",
        Reason::Format => "import.why_format",
        Reason::Special => "import.why_special",
        Reason::Link => "import.why_link",
        Reason::MayBeEmpty => "import.why_may_be_empty",
        Reason::Exists => "import.why_exists",
        // Not the reason of an item that was not imported.
        _ => "import.manual",
    }
}
