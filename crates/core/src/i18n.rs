//! Text in the user's language, English or Traditional Chinese, from
//! `locales/*.toml`.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhTw,
}

impl Lang {
    /// From a locale or language name such as `zh_TW.UTF-8` or `zh-Hant-TW`.
    pub fn from_locale(name: &str) -> Self {
        let name = name.to_ascii_lowercase().replace('_', "-");
        let traditional = ["zh-tw", "zh-hant", "zh-hk", "zh-mo"];
        if traditional.iter().any(|prefix| name.starts_with(prefix)) {
            Lang::ZhTw
        } else {
            Lang::En
        }
    }

    /// The language the user chose for the system.
    pub fn system() -> Self {
        #[cfg(target_os = "macos")]
        {
            // launchd sets no LANG; the preference lives with the user.
            if let Some(first) = objc2_foundation::NSLocale::preferredLanguages().firstObject() {
                return Lang::from_locale(&first.to_string());
            }
        }
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|var| std::env::var(var).ok())
            .find(|value| !value.is_empty())
            .map_or(Lang::En, |value| Lang::from_locale(&value))
    }
}

fn table(lang: Lang) -> &'static toml::Table {
    static TABLES: OnceLock<[toml::Table; 2]> = OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        [
            include_str!("../locales/en.toml"),
            include_str!("../locales/zh-TW.toml"),
        ]
        .map(|text| text.parse().expect("locale files are valid TOML"))
    });
    &tables[lang as usize]
}

/// The text for `key` (`section.name`) with each `{name}` in `args` filled in.
/// Missing keys fall back to English, then to the key itself.
pub fn text(lang: Lang, key: &str, args: &[(&str, &str)]) -> String {
    let lookup = |lang| {
        let (section, name) = key.split_once('.')?;
        table(lang).get(section)?.get(name)?.as_str()
    };
    let mut out = lookup(lang)
        .or_else(|| lookup(Lang::En))
        .unwrap_or(key)
        .to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_choose_the_language() {
        for name in ["zh_TW.UTF-8", "zh-Hant-TW", "zh-Hant", "zh_HK"] {
            assert_eq!(Lang::from_locale(name), Lang::ZhTw, "{name}");
        }
        for name in ["en_US.UTF-8", "C", "zh_CN.UTF-8", "zh-Hans"] {
            assert_eq!(Lang::from_locale(name), Lang::En, "{name}");
        }
    }

    #[test]
    fn both_tables_have_the_same_keys() {
        let keys = |lang| {
            let mut keys: Vec<String> = table(lang)
                .iter()
                .flat_map(|(section, names)| {
                    names
                        .as_table()
                        .unwrap()
                        .keys()
                        .map(move |name| format!("{section}.{name}"))
                })
                .collect();
            keys.sort();
            keys
        };
        assert_eq!(keys(Lang::En), keys(Lang::ZhTw));
    }

    #[test]
    fn arguments_are_filled_in() {
        assert_eq!(
            text(Lang::ZhTw, "review.many", &[("count", "3")]),
            "3 項變動待處理"
        );
        assert_eq!(text(Lang::En, "missing.key", &[]), "missing.key");
    }
}
