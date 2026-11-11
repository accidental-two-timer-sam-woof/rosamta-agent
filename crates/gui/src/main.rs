mod actions;
mod app;
mod pages;
#[cfg(feature = "screenshot")]
mod screenshot;
mod state;
mod text;
mod theme;

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::app::{Rosamta, URLS};
use crate::text::t;

actions!(rosamta, [OpenSettings, CloseWindow, Quit]);

// The icons the app uses that gpui-kit's own set leaves out.
icon_assets!(
    OwnIcons,
    [
        CircleQuestionMark,
        Clock,
        Code,
        FileCode,
        FilePlus,
        ClockFading,
        Link,
        List,
        Lock
    ]
);

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match OwnIcons.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Assets.list(path)
    }
}

fn take_urls(urls: impl IntoIterator<Item = String>) {
    let mut queued = URLS.lock().unwrap_or_else(|err| err.into_inner());
    queued.extend(urls.into_iter().filter(|url| url.starts_with("rosamta://")));
}

/// The app menu, and the keys that go with it on a system without one.
fn menus(view: Entity<Rosamta>, cx: &mut App) {
    let [settings, close, quit] = if cfg!(target_os = "macos") {
        ["cmd-,", "cmd-w", "cmd-q"]
    } else {
        ["ctrl-,", "ctrl-w", "ctrl-q"]
    };
    cx.bind_keys([
        KeyBinding::new(settings, OpenSettings, None),
        KeyBinding::new(close, CloseWindow, None),
        KeyBinding::new(quit, Quit, None),
    ]);
    cx.on_action(move |_: &OpenSettings, cx| pages::settings::open(&view, cx));
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    });
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus(vec![Menu {
        name: "rosamta".into(),
        items: vec![
            MenuItem::action(t("menu.settings"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(t("menu.close"), CloseWindow),
            MenuItem::action(t("menu.quit"), Quit),
        ],
        disabled: false,
    }]);
}

fn main() {
    // Before any thread starts: opened from Finder, the app has launchd's
    // environment, without the PATH and XDG paths the user's shell sets.
    let environment_error = rosamta_core::daemon::adopt_login_environment();
    take_urls(std::env::args().skip(1));
    let application = application().with_assets(AppAssets);
    application.on_open_urls(take_urls);
    application.run(|cx| {
        init(cx);
        // The library's own text, such as an input's menu, in the same language.
        if text::lang() == rosamta_core::i18n::Lang::ZhTw {
            component::set_locale("zh-TW");
        }
        let mut options = TitleBar::window_options();
        options.window_bounds = Some(WindowBounds::centered(size(px(1080.), px(720.)), cx));
        options.window_min_size = Some(size(px(760.), px(520.)));
        options.titlebar = Some(TitlebarOptions {
            title: Some("rosamta".into()),
            appears_transparent: true,
            // In the middle of the toolbar's height.
            traffic_light_position: Some(point(px(20.), px(20.))),
        });
        if theme::TRANSLUCENT {
            options.window_background = WindowBackgroundAppearance::Blurred;
        }
        let mut built = None;
        let window = cx
            .open_window(options, |window, cx| {
                theme::init(window, cx);
                let view = cx.new(|cx| Rosamta::new(environment_error, window, cx));
                built = Some(view.clone());
                // Each part paints its own background, so the sidebar can
                // leave its part of the window see-through.
                cx.new(|cx| {
                    let root = base::Root::new(view, window, cx);
                    if theme::TRANSLUCENT {
                        root.bg(transparent_black())
                    } else {
                        root
                    }
                })
            })
            .expect("failed to open window");
        let view = built.expect("the window was built");
        menus(view.clone(), cx);
        #[cfg(feature = "screenshot")]
        screenshot::start(window.into(), view, cx);
        // The settings window does not outlive the main one.
        let main: AnyWindowHandle = window.into();
        cx.on_window_closed(move |cx, _| {
            if !cx.windows().contains(&main) {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
