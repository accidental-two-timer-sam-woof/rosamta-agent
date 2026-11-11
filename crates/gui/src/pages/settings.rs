//! The settings window: this host's `host.toml`, its remote and the
//! background service.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::config::{AdoptPolicy, HostConfig, OriginalLines, ScriptPolicy, SyncPolicy};
use rosamta_core::daemon::service::Service;

use super::{bordered, dot, icon_button, mono, row, section, segmented};
use crate::actions;
use crate::app::Rosamta;
use crate::text::{t, tf, when};
use crate::theme::{font, palette, space::*};

const ADOPT: &[(&str, &str)] = &[
    ("policy.auto", "auto"),
    ("policy.ask", "ask"),
    ("policy.off", "off"),
];
const ORIGINAL: &[(&str, &str)] = &[("policy.delete", "delete"), ("policy.comment", "comment")];
const SYNC: &[(&str, &str)] = &[("policy.auto", "auto"), ("policy.manual", "manual")];
const SCRIPTS: &[(&str, &str)] = &[("policy.auto", "auto"), ("policy.ask", "ask")];
const INTERVALS: &[(&str, i64)] = &[
    ("settings.every_5", 5),
    ("settings.every_15", 15),
    ("settings.every_30", 30),
    ("settings.hourly", 60),
];

const TITLE_BAR: f32 = 44.;

pub struct Settings {
    app: Entity<Rosamta>,
    tag: Entity<InputState>,
    host_id: Entity<InputState>,
    /// The host ID last put into `host_id`, so a new one replaces what is typed.
    shown_host_id: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// Open the settings window, or bring it forward when it is open.
pub fn open(app: &Entity<Rosamta>, cx: &mut App) {
    if let Some(window) = app.read(cx).settings
        && window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let mut options = TitleBar::window_options();
    options.window_bounds = Some(WindowBounds::centered(size(px(560.), px(660.)), cx));
    options.window_min_size = Some(size(px(480.), px(360.)));
    options.titlebar = Some(TitlebarOptions {
        title: Some(t("nav.settings")),
        appears_transparent: true,
        traffic_light_position: Some(point(px(16.), px(16.))),
    });
    let opened = open_window(options, cx, |window, cx| {
        cx.new(|cx| Settings::new(app.clone(), window, cx))
    });
    if let Ok((window, _)) = opened {
        app.update(cx, |app, _| app.settings = Some(window));
    }
}

impl Settings {
    fn new(app: Entity<Rosamta>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tag =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("settings.tag_placeholder")));
        let host_id = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&tag, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_tag(window, cx);
                }
            }),
            cx.subscribe_in(&host_id, window, |this, _, event, window, cx| match event {
                // Renaming commits to the repo, so not on leaving the field.
                InputEvent::PressEnter { .. } => this.rename(window, cx),
                // The button to rename shows once the name differs.
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
            cx.observe_in(&app, window, |this, _, window, cx| {
                this.show_host_id(window, cx);
                cx.notify();
            }),
        ];
        let mut this = Settings {
            app,
            tag,
            host_id,
            shown_host_id: None,
            _subscriptions: subscriptions,
        };
        this.show_host_id(window, cx);
        this
    }

    fn saved_host_id(&self, cx: &App) -> Option<String> {
        let host = self.app.read(cx).snap.host.as_ref();
        host.map(|host| host.id.clone())
    }

    /// Put the saved host ID into its field when it changed.
    fn show_host_id(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.saved_host_id(cx);
        if self.shown_host_id != id {
            self.shown_host_id = id.clone();
            self.host_id.update(cx, |input, cx| {
                input.set_value(id.unwrap_or_default(), window, cx)
            });
        }
    }

    /// Run a write of the app's from this window.
    fn run(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        name: &'static str,
        action: impl FnOnce(&rosamta_core::apply::Context) -> actions::Done + Send + 'static,
    ) {
        self.app
            .update(cx, |app, cx| app.run(window, cx, name, action));
    }

    fn add_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tag = self.tag.read(cx).value().trim().to_string();
        if tag.is_empty() || self.app.read(cx).busy.is_some() {
            return;
        }
        self.tag
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.run(window, cx, "add-tag", move |core| {
            actions::add_tag(core, tag)
        });
    }

    /// The host ID as typed, when it is not the saved one.
    fn new_host_id(&self, cx: &App) -> Option<String> {
        let id = self.host_id.read(cx).value().trim().to_string();
        (!id.is_empty() && Some(&id) != self.saved_host_id(cx).as_ref()).then_some(id)
    }

    fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.new_host_id(cx) {
            self.run(window, cx, "rename", move |core| actions::rename(core, id));
        }
    }

    /// A row choosing one value of a `host.toml` setting.
    fn choice<T: Clone + PartialEq + Into<toml_edit::Value> + Send + 'static>(
        &self,
        setting: &'static str,
        label: &'static str,
        options: &'static [(&'static str, T)],
        current: T,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let selected = options.iter().position(|(_, value)| *value == current);
        row(cx).child(div().flex_1().child(t(label))).child(
            segmented(
                setting,
                options.iter().map(|(label, _)| (t(label), enabled)),
                selected.unwrap_or(usize::MAX),
                cx.listener(move |this, index: &usize, window, cx| {
                    if let Some((_, value)) =
                        options.get(*index).filter(|_| Some(*index) != selected)
                    {
                        let value = value.clone();
                        this.run(window, cx, setting, move |core| {
                            actions::set_host(core, setting, value)
                        });
                    }
                }),
            )
            .small(),
        )
    }

    fn host(&self, host: &HostConfig, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let tags = host.tags.iter().enumerate().map(|(index, tag)| {
            let removed = tag.clone();
            h_flex()
                .flex_none()
                .h(px(S24))
                .pl(px(10.))
                .gap(px(S2))
                .rounded_full()
                .bg(p.chip)
                .child(tag.clone())
                .child(
                    icon_button(("remove-tag", index), IconName::Close, t("settings.remove"))
                        .xsmall()
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let tag = removed.clone();
                            this.run(window, cx, "remove-tag", move |core| {
                                actions::remove_tag(core, tag)
                            });
                        })),
                )
        });
        let label = |key: &str| div().flex_none().w(px(120.)).child(t(key));
        v_flex()
            .child(
                row(cx)
                    .child(label("settings.host_id"))
                    .child(div().flex_1().child(Input::new(&self.host_id).small()))
                    .when(self.new_host_id(cx).is_some(), |row| {
                        row.child(
                            bordered(Button::new("rename"), cx)
                                .label(t("settings.rename"))
                                .small()
                                .loading(self.app.read(cx).busy == Some("rename"))
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.rename(window, cx)),
                                ),
                        )
                    }),
            )
            .child(
                row(cx).child(label("settings.tags")).child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .flex_wrap()
                        .gap(px(S8))
                        .children(tags)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(120.))
                                .child(Input::new(&self.tag).small()),
                        )
                        .child(
                            bordered(Button::new("add-tag"), cx)
                                .label(t("settings.add"))
                                .small()
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_tag(window, cx)),
                                ),
                        ),
                ),
            )
    }

    fn automation(&self, host: &HostConfig, busy: bool, cx: &mut Context<Self>) -> Div {
        let adopt = match host.adopt {
            AdoptPolicy::Auto => "auto",
            AdoptPolicy::Ask => "ask",
            AdoptPolicy::Off => "off",
        };
        let original = match host.adopt_original {
            OriginalLines::Delete => "delete",
            OriginalLines::Comment => "comment",
        };
        let scripts = match host.scripts {
            ScriptPolicy::Auto => "auto",
            ScriptPolicy::Ask => "ask",
        };
        v_flex()
            .child(self.choice("adopt", "policy.adopt", ADOPT, adopt, !busy, cx))
            .child(self.choice(
                "adopt_original",
                "policy.original",
                ORIGINAL,
                original,
                !busy,
                cx,
            ))
            .child(self.choice("scripts", "policy.scripts", SCRIPTS, scripts, !busy, cx))
            .child(
                row(cx)
                    .child(div().flex_1().child(t("settings.shell_notice")))
                    .child(
                        Switch::new("shell_notice")
                            .checked(host.shell_notice)
                            .disabled(busy)
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                let checked = *checked;
                                this.run(window, cx, "shell_notice", move |core| {
                                    actions::set_host(core, "shell_notice", checked)
                                });
                            })),
                    ),
            )
    }

    fn sync(&self, host: &HostConfig, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let policy = |policy| match policy {
            SyncPolicy::Auto => "auto",
            SyncPolicy::Manual => "manual",
        };
        let remote = match &self.app.read(cx).snap.remote {
            Some(url) => mono(url.clone(), cx).truncate().text_color(p.muted),
            None => div().text_color(p.muted).child(t("status.none")),
        };
        // Nothing pulls on its own while pulling is manual.
        let timed = !busy && host.pull == SyncPolicy::Auto;
        v_flex()
            .child(
                row(cx)
                    .child(div().flex_1().child(t("settings.remote")))
                    .child(remote.min_w_0()),
            )
            .child(self.choice(
                "publish",
                "policy.publish",
                SYNC,
                policy(host.publish),
                !busy,
                cx,
            ))
            .child(self.choice("pull", "policy.pull", SYNC, policy(host.pull), !busy, cx))
            .child(self.choice(
                "pull_interval",
                "settings.pull_interval",
                INTERVALS,
                i64::from(host.pull_interval),
                timed,
                cx,
            ))
    }

    fn daemon(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let snap = &self.app.read(cx).snap;
        let status = match (snap.started_at, snap.service) {
            (Some(at), _) if snap.running => tf("status.running", &[("time", &when(at))]),
            // On, but the system has not let it start yet.
            (_, Service::RequiresApproval) => t("status.approval"),
            _ => t("status.stopped"),
        };
        v_flex()
            .child(
                row(cx)
                    .child(div().flex_1().child(t("settings.login")))
                    .child(
                        Switch::new("service")
                            .checked(snap.service == Service::Enabled)
                            .disabled(busy || snap.service == Service::Unavailable)
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                let checked = *checked;
                                this.run(window, cx, "service", move |core| {
                                    actions::set_service(core, checked)
                                });
                            })),
                    ),
            )
            .child(
                row(cx)
                    .child(div().flex_1().child(t("settings.status")))
                    .child(dot(snap.running, cx))
                    .child(div().text_color(p.muted).child(status)),
            )
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let busy = self.app.read(cx).busy.is_some();
        let host = self.app.read(cx).snap.host.clone();
        let bar = TitleBar::new()
            .h(px(TITLE_BAR))
            .bg(p.well)
            .border_color(p.line)
            .child(
                // Centered in the window: the same room on both sides.
                div()
                    .flex_1()
                    .pr(px(if cfg!(target_os = "macos") { 80. } else { 0. }))
                    .text_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t("nav.settings")),
            );
        let groups = v_flex()
            .gap(px(S20))
            .when_some(host.as_ref(), |groups, host| {
                groups
                    .child(section(t("settings.host"), self.host(host, busy, cx), cx))
                    .child(section(
                        t("settings.automation"),
                        self.automation(host, busy, cx),
                        cx,
                    ))
                    .child(section(t("settings.sync"), self.sync(host, busy, cx), cx))
            })
            .child(section(t("settings.daemon"), self.daemon(busy, cx), cx));
        v_flex()
            .size_full()
            .bg(p.well)
            .text_color(p.text)
            .text_size(px(font::BODY))
            .child(bar)
            .child(
                div()
                    .id("settings")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(S24))
                    .py(px(S16))
                    .child(groups),
            )
    }
}
