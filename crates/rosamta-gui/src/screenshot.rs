//! Development only (`--features screenshot`): save every page as a PNG and
//! quit, for checking layouts where the screen cannot be captured.
//! `ROSAMTA_SCREENSHOTS=<dir>` turns it on; `ROSAMTA_SCREENSHOTS_THEME` is
//! `light` or `dark`, else the system's.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::app::{Page, Rosamta};

const PAGES: [(Page, &str); 6] = [
    (Page::Overview, "overview"),
    (Page::Review, "review"),
    (Page::Modules, "modules"),
    (Page::Dotfiles, "dotfiles"),
    (Page::History, "history"),
    (Page::Settings, "settings"),
];

pub fn start(window: AnyWindowHandle, view: Entity<Rosamta>, cx: &mut App) {
    let Some(dir) = std::env::var_os("ROSAMTA_SCREENSHOTS").map(PathBuf::from) else {
        return;
    };
    let theme = std::env::var("ROSAMTA_SCREENSHOTS_THEME").unwrap_or_default();
    match theme.as_str() {
        "light" => Theme::change(ThemeMode::Light, None, cx),
        "dark" => Theme::change(ThemeMode::Dark, None, cx),
        _ => {}
    }
    let suffix = if theme.is_empty() {
        String::new()
    } else {
        format!("-{theme}")
    };
    cx.spawn(async move |cx| {
        let executor = cx.background_executor().clone();
        let wait = |ms| executor.timer(Duration::from_millis(ms));
        // The first snapshot loads in the background.
        for _ in 0..40 {
            wait(500).await;
            if view.read_with(cx, |view, _| view.snap.ready) {
                break;
            }
        }
        wait(1000).await;
        let save = |name: &str, cx: &mut AsyncApp| {
            let path = dir.join(format!("{name}{suffix}.png"));
            let saved = cx.update_window(window, |_, window, cx| {
                // Draw here: a locked screen stops the display link.
                window.draw(cx).clear(cx);
                window.render_to_image().map(|image| image.save(&path))
            });
            eprintln!("{}: {saved:?}", path.display());
        };
        // A home without a repo: the setup page, and importing from chezmoi.
        if !view.read_with(cx, |view, _| view.snap.ready) {
            save("setup", cx);
            if view.read_with(cx, |view, _| view.snap.chezmoi) {
                cx.update_window(window, |_, window, cx| {
                    view.update(cx, |view, cx| view.import_chezmoi(window, cx));
                })
                .ok();
                for _ in 0..40 {
                    wait(500).await;
                    if view.read_with(cx, |view, _| view.imported.is_some() && !view.busy) {
                        break;
                    }
                }
                wait(800).await;
                save("import", cx);
                view.update(cx, |view, cx| {
                    view.imported = None;
                    cx.notify();
                });
            }
        }
        for (page, name) in PAGES {
            view.update(cx, |view, cx| {
                view.page = page;
                cx.notify();
            });
            wait(800).await;
            save(name, cx);
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}
