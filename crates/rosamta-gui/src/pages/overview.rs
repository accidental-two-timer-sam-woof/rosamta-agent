//! What needs attention, how this host is set up, and what happened lately.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::config::{AdoptPolicy, SyncPolicy};
use rosamta_core::sync::{Attempt, Outcome};

use super::{Tone, badge, dot, label, mono, page, review, row, section};
use crate::actions;
use crate::app::Rosamta;
use crate::text::{t, tf, when};
use crate::theme::{palette, space::*};

/// Items listed here before the rest wait on the review page.
const ATTENTION: usize = 5;
const ACTIVITY: usize = 8;

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let snap = &app.snap;
    let p = palette(cx);
    let synced = [&snap.sync.publish, &snap.sync.pull]
        .into_iter()
        .flatten()
        .filter(|attempt| attempt.outcome == Outcome::Ok)
        .map(|attempt| attempt.at)
        .max();
    let actions = h_flex()
        .gap(px(S12))
        .children(synced.map(|at| {
            div()
                .text_sm()
                .text_color(p.muted)
                .child(tf("overview.synced", &[("time", &when(at))]))
        }))
        .child(
            Button::new("sync")
                .label(t("overview.sync"))
                .loading(app.busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.run(window, cx, actions::sync_now);
                })),
        );
    let body = v_flex()
        .gap(px(S32))
        .children(attention(app, cx))
        .child(section(t("overview.status"), status(app, cx), cx))
        .child(section(t("overview.shells"), shells(app, cx), cx))
        .when(!snap.commits.is_empty(), |body| {
            body.child(section(t("overview.activity"), activity(app, cx), cx))
        });
    page("overview", t("nav.overview"), actions, body, cx)
}

fn attention(app: &Rosamta, cx: &mut Context<Rosamta>) -> Option<impl IntoElement> {
    let snap = &app.snap;
    let problems: Vec<String> = snap
        .problem
        .iter()
        .cloned()
        .chain(problem("publish", snap.sync.publish.as_ref()))
        .chain(problem("pull", snap.sync.pull.as_ref()))
        .collect();
    if snap.queue.is_empty() && problems.is_empty() {
        return None;
    }
    let p = palette(cx);
    let items = snap.queue.iter().take(ATTENTION).map(|item| {
        let id = item.id;
        let (key, tone) = review::item_badge(item);
        row(cx)
            .child(badge(t(key), tone, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(review::item_title(item, &snap.home)))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(p.muted)
                            .child(review::item_meta(item, &snap.home)),
                    ),
            )
            .child(
                Button::new(("view", id))
                    .label(t("overview.view"))
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx))),
            )
    });
    let problems = problems.into_iter().map(|text| {
        row(cx)
            .child(badge(t("badge.warning"), Tone::Warn, cx))
            .child(div().flex_1().min_w_0().child(text))
    });
    let body = v_flex().children(items).children(problems);
    Some(section(t("overview.attention"), body, cx))
}

/// The text for a publish or pull that needs the user.
fn problem(kind: &str, attempt: Option<&Attempt>) -> Option<String> {
    let attempt = attempt?;
    let reason = attempt.reason.clone().unwrap_or_default();
    let key = match attempt.outcome {
        Outcome::Ok | Outcome::Offline => return None,
        Outcome::Blocked => format!("problem.{kind}_blocked"),
        Outcome::Failed => format!("problem.{kind}_failed"),
        Outcome::Conflict => {
            let files = attempt.files.join(", ");
            return Some(t_owned("problem.conflict", &[("files", &files)]));
        }
    };
    Some(t_owned(&key, &[("reason", &reason)]))
}

fn t_owned(key: &str, args: &[(&str, &str)]) -> String {
    tf(key, args).to_string()
}

fn status(app: &Rosamta, cx: &App) -> impl IntoElement {
    let snap = &app.snap;
    let daemon = match snap.started_at {
        Some(at) if snap.running => tf("status.running", &[("time", &when(at))]),
        _ => t("status.stopped"),
    };
    let mut rows = vec![(
        t("status.daemon"),
        h_flex()
            .gap(px(S8))
            .child(dot(snap.running, cx))
            .child(daemon),
    )];
    if let Some(host) = &snap.host {
        let adopt = match host.adopt {
            AdoptPolicy::Auto => t("policy.auto"),
            AdoptPolicy::Ask => t("policy.ask"),
            AdoptPolicy::Off => t("policy.off"),
        };
        let mut publish = vec![policy(host.publish)];
        publish.extend(last(snap.sync.publish.as_ref()));
        if snap.sync.unpushed > 0 {
            let count = snap.sync.unpushed.to_string();
            publish.push(tf("status.unpushed", &[("count", &count)]));
        }
        let mut pull = vec![policy(host.pull)];
        if host.pull == SyncPolicy::Auto {
            let minutes = host.pull_interval.to_string();
            pull.push(tf("status.every", &[("minutes", &minutes)]));
        }
        pull.extend(last(snap.sync.pull.as_ref()));
        rows.push((t("policy.adopt"), h_flex().child(adopt)));
        rows.push((t("policy.publish"), h_flex().child(join(publish))));
        rows.push((t("policy.pull"), h_flex().child(join(pull))));
    }
    let remote = match &snap.remote {
        Some(url) => h_flex().child(mono(url.clone(), cx)),
        None => h_flex().child(t("status.none")),
    };
    rows.push((t("status.remote"), remote));
    v_flex().children(rows.into_iter().map(|(name, value)| {
        row(cx)
            .child(label(name, cx))
            .child(value.flex_1().min_w_0())
    }))
}

fn policy(policy: SyncPolicy) -> SharedString {
    match policy {
        SyncPolicy::Auto => t("policy.auto"),
        SyncPolicy::Manual => t("policy.manual"),
    }
}

fn last(attempt: Option<&Attempt>) -> Option<SharedString> {
    attempt.map(|attempt| tf("status.last", &[("time", &when(attempt.at))]))
}

fn join(parts: Vec<SharedString>) -> SharedString {
    parts
        .iter()
        .map(SharedString::as_ref)
        .collect::<Vec<&str>>()
        .join(" · ")
        .into()
}

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
            .map(|file| rosamta_core::adopt::display(file, home))
            .collect();
        let (key, tone) = if !shell.installed {
            ("status.missing", Tone::Plain)
        } else if shell.hooked.is_empty() {
            ("status.unhooked", Tone::Warn)
        } else {
            ("status.hooked", Tone::Ok)
        };
        row(cx)
            .child(div().flex_none().w(px(S48)).child(shell.shell.name()))
            .child(
                div()
                    .flex_none()
                    .w(px(S48))
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

fn activity(app: &Rosamta, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex().children(app.snap.commits.iter().take(ACTIVITY).map(|commit| {
        row(cx)
            .child(
                div()
                    .flex_none()
                    .w(px(96.))
                    .text_color(p.muted)
                    .child(when(commit.at)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(commit.subject.clone()),
            )
            .child(mono(commit.hash.clone(), cx).text_color(p.muted))
    }))
}
