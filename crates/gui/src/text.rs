//! Interface text, from the same tables as the daemon's notifications.

use std::sync::OnceLock;

use gpui_kit::SharedString;
use rosamta_core::i18n::{self, Lang};

pub fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(Lang::system)
}

pub fn t(key: &str) -> SharedString {
    i18n::text(lang(), key, &[]).into()
}

pub fn tf(key: &str, args: &[(&str, &str)]) -> SharedString {
    i18n::text(lang(), key, args).into()
}

/// `HH:MM` today, `MM-DD HH:MM` on other days, in local time.
pub fn when(secs: u64) -> String {
    let (date, time) = rosamta_core::time::local(secs);
    let (today, _) = rosamta_core::time::local(rosamta_core::time::now());
    if date == today {
        time
    } else {
        format!("{} {time}", &date[5..])
    }
}
