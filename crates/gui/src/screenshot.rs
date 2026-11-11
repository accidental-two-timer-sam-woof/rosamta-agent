//! Development only (`--features screenshot`): save every page as a PNG and
//! quit, for checking layouts where the screen cannot be captured.
//! `ROSAMTA_SCREENSHOTS=<dir>` turns it on; `ROSAMTA_SCREENSHOTS_THEME` is
//! `light` or `dark`, else the system's.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::app::{HistoryTab, LibraryTab, Page, Rosamta, Selection};

const PAGES: [(Page, LibraryTab, HistoryTab, &str); 5] = [
    (
        Page::Review,
        LibraryTab::Modules,
        HistoryTab::Changes,
        "review",
    ),
    (
        Page::Library,
        LibraryTab::Modules,
        HistoryTab::Changes,
        "modules",
    ),
    (
        Page::Library,
        LibraryTab::Dotfiles,
        HistoryTab::Changes,
        "dotfiles",
    ),
    (
        Page::History,
        LibraryTab::Modules,
        HistoryTab::Changes,
        "history",
    ),
    (
        Page::History,
        LibraryTab::Modules,
        HistoryTab::Snapshots,
        "snapshots",
    ),
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
        let save_window = |window: AnyWindowHandle, name: &str, cx: &mut AsyncApp| {
            let path = dir.join(format!("{name}{suffix}.png"));
            let saved = cx.update_window(window, |_, window, cx| {
                // Draw here: a locked screen stops the display link.
                window.draw(cx).clear(cx);
                window.render_to_image().map(|image| image.save(&path))
            });
            eprintln!("{}: {saved:?}", path.display());
        };
        let save = |name: &str, cx: &mut AsyncApp| save_window(window, name, cx);
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
                    if view.read_with(cx, |view, _| view.imported.is_some() && view.busy.is_none())
                    {
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
        // The first item, where a problem would be shown first.
        view.update(cx, |view, cx| {
            if let Some(item) = view.snap.queue.first() {
                view.select(Selection::Item(item.id), cx);
            }
        });
        wait(800).await;
        for (page, library, history, name) in PAGES {
            view.update(cx, |view, cx| {
                (view.page, view.library, view.history) = (page, library, history);
                cx.notify();
            });
            wait(800).await;
            save(name, cx);
        }
        // A problem, when there is one, beside the item shown above.
        let problem = view.read_with(cx, |view, _| {
            view.snap.problems.first().map(|one| one.key())
        });
        if let Some(key) = problem {
            view.update(cx, |view, cx| view.select(Selection::Problem(key), cx));
            wait(800).await;
            save("problem", cx);
        }
        cx.update(|cx| crate::pages::settings::open(&view, cx));
        wait(800).await;
        if let Some(settings) = view.read_with(cx, |view, _| view.settings) {
            save_window(settings, "settings", cx);
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}
