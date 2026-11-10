//! This host's settings in `host.toml`, its remote and the background service.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Selectable, Sizable, h_flex, v_flex};
use gpui_kit::*;
use rosamta_core::adopt;
use rosamta_core::config::{AdoptPolicy, OriginalLines, SyncPolicy};
use rosamta_core::daemon::service::Service;

use super::{dot, label, mono, page, row, section};
use crate::actions;
use crate::app::Rosamta;
use crate::text::{t, tf, when};
use crate::theme::{palette, space::*};

const ADOPT: &[(&str, &str)] = &[
    ("policy.auto", "auto"),
    ("policy.ask", "ask"),
    ("policy.off", "off"),
];
const ORIGINAL: &[(&str, &str)] = &[("policy.delete", "delete"), ("policy.comment", "comment")];
const SYNC: &[(&str, &str)] = &[("policy.auto", "auto"), ("policy.manual", "manual")];
const INTERVALS: &[(&str, i64)] = &[
    ("settings.every_5", 5),
    ("settings.every_15", 15),
    ("settings.every_30", 30),
    ("settings.hourly", 60),
];

pub fn render(
    app: &mut Rosamta,
    window: &mut Window,
    cx: &mut Context<Rosamta>,
) -> impl IntoElement {
    let id = app.snap.host.as_ref().map(|host| host.id.clone());
    if app.shown_host_id != id {
        app.shown_host_id = id.clone();
        app.host_id.update(cx, |input, cx| {
            input.set_value(id.unwrap_or_default(), window, cx)
        });
    }
    let p = palette(cx);
    let snap = &app.snap;
    let busy = app.busy;
    let mut body = v_flex().gap(px(S24));
    if let Some(host) = &snap.host {
        let tags = h_flex()
            .flex_1()
            .flex_wrap()
            .gap(px(S8))
            .children(host.tags.iter().enumerate().map(|(index, tag)| {
                let removed = tag.clone();
                h_flex()
                    .h(px(S24))
                    .pl(px(S8))
                    .gap(px(S4))
                    .rounded(px(S4))
                    .bg(p.code)
                    .child(tag.clone())
                    .child(
                        Button::new(("remove-tag", index))
                            .label("×")
                            .ghost()
                            .xsmall()
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let tag = removed.clone();
                                this.run(window, cx, move |core| actions::remove_tag(core, tag));
                            })),
                    )
            }))
            .child(div().w(px(160.)).child(Input::new(&app.tag).small()))
            .child(
                Button::new("add-tag")
                    .label(t("settings.add"))
                    .small()
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.add_tag(window, cx))),
            );
        let host_rows = v_flex()
            .child(
                row(cx)
                    .child(label(t("settings.host_id"), cx))
                    .child(div().w(px(240.)).child(Input::new(&app.host_id))),
            )
            .child(row(cx).child(label(t("settings.tags"), cx)).child(tags));
        body = body.child(section(t("settings.host"), host_rows, cx));

        let adopt = match host.adopt {
            AdoptPolicy::Auto => "auto",
            AdoptPolicy::Ask => "ask",
            AdoptPolicy::Off => "off",
        };
        let original = match host.adopt_original {
            OriginalLines::Delete => "delete",
            OriginalLines::Comment => "comment",
        };
        let sync = |policy| match policy {
            SyncPolicy::Auto => "auto",
            SyncPolicy::Manual => "manual",
        };
        let interval = i64::from(host.pull_interval);
        let preferences = v_flex()
            .child(choice("adopt", "policy.adopt", ADOPT, adopt, busy, cx))
            .child(choice(
                "adopt_original",
                "policy.original",
                ORIGINAL,
                original,
                busy,
                cx,
            ))
            .child(choice(
                "publish",
                "policy.publish",
                SYNC,
                sync(host.publish),
                busy,
                cx,
            ))
            .child(choice(
                "pull",
                "policy.pull",
                SYNC,
                sync(host.pull),
                busy,
                cx,
            ))
            .child(
                row(cx).child(label(t("settings.pull_interval"), cx)).child(
                    ButtonGroup::new("pull_interval")
                        .outline()
                        .small()
                        .children(INTERVALS.iter().enumerate().map(|(index, (key, minutes))| {
                            Button::new(("pull_interval", index))
                                .label(t(key))
                                .selected(*minutes == interval)
                                .disabled(busy)
                        }))
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, window, cx| {
                            if let Some(&(_, minutes)) =
                                clicks.first().and_then(|&i| INTERVALS.get(i))
                            {
                                this.run(window, cx, move |core| {
                                    actions::set_host(core, "pull_interval", minutes)
                                });
                            }
                        })),
                ),
            )
            .child(
                row(cx).child(
                    Switch::new("shell_notice")
                        .checked(host.shell_notice)
                        .label(t("settings.shell_notice"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, checked: &bool, window, cx| {
                            let checked = *checked;
                            this.run(window, cx, move |core| {
                                actions::set_host(core, "shell_notice", checked)
                            });
                        })),
                ),
            );
        body = body.child(section(t("settings.preferences"), preferences, cx));
    }
    let remote = match &snap.remote {
        Some(url) => mono(url.clone(), cx).into_any_element(),
        None => div()
            .text_color(p.muted)
            .child(t("status.none"))
            .into_any_element(),
    };
    body = body.child(section(t("settings.remote"), row(cx).child(remote), cx));

    let enabled = matches!(snap.service, Service::Enabled | Service::RequiresApproval);
    let daemon = match snap.started_at {
        Some(at) if snap.running => tf("status.running", &[("time", &when(at))]),
        _ => t("status.stopped"),
    };
    let service = row(cx)
        .child(
            div().flex_1().child(
                Switch::new("service")
                    .checked(enabled)
                    .label(t("settings.login"))
                    .disabled(busy || snap.service == Service::Unavailable)
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        let checked = *checked;
                        this.run(window, cx, move |core| actions::set_service(core, checked));
                    })),
            ),
        )
        .child(
            h_flex()
                .gap(px(S8))
                .text_color(p.muted)
                .child(dot(snap.running, cx))
                .child(daemon),
        );
    body = body.child(section(t("settings.daemon"), service, cx));

    let file = mono(adopt::display(&snap.host_file, &snap.home), cx).text_color(p.muted);
    page("settings", t("nav.settings"), file, body, cx)
}

/// A row choosing one value of a `host.toml` setting.
fn choice(
    setting: &'static str,
    key: &'static str,
    options: &'static [(&'static str, &'static str)],
    current: &str,
    busy: bool,
    cx: &mut Context<Rosamta>,
) -> impl IntoElement {
    row(cx).child(label(t(key), cx)).child(
        ButtonGroup::new(setting)
            .outline()
            .small()
            .children(options.iter().enumerate().map(|(index, (label, value))| {
                Button::new((setting, index))
                    .label(t(label))
                    .selected(*value == current)
                    .disabled(busy)
            }))
            .on_click(cx.listener(move |this, clicks: &Vec<usize>, window, cx| {
                if let Some(&(_, value)) = clicks.first().and_then(|&i| options.get(i)) {
                    this.run(window, cx, move |core| {
                        actions::set_host(core, setting, value)
                    });
                }
            })),
    )
}
