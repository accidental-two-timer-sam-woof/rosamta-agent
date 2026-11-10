mod actions;
mod app;
mod pages;
#[cfg(feature = "screenshot")]
mod screenshot;
mod state;
mod text;
mod theme;

use gpui_kit::*;

use crate::app::{Rosamta, URLS};

fn take_urls(urls: impl IntoIterator<Item = String>) {
    let mut queued = URLS.lock().unwrap_or_else(|err| err.into_inner());
    queued.extend(urls.into_iter().filter(|url| url.starts_with("rosamta://")));
}

fn main() {
    // Before any thread starts: opened from Finder, the app has launchd's
    // environment, without the PATH and XDG paths the user's shell sets.
    let environment_error = rosamta_core::daemon::adopt_login_environment();
    take_urls(std::env::args().skip(1));
    let application = application().with_assets(assets::Assets);
    application.on_open_urls(take_urls);
    application.run(|cx| {
        init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1080.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("rosamta".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_window, _view) = open_window(options, cx, |window, cx| {
            theme::init(window, cx);
            cx.new(|cx| Rosamta::new(environment_error, window, cx))
        })
        .expect("failed to open window");
        #[cfg(feature = "screenshot")]
        screenshot::start(_window, _view, cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
