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

/// A core error in this app's words: without the command it says to run.
// shortcut: cut by text; replace once core errors carry their advice apart.
pub fn plain(error: &str) -> &str {
    match error.rfind("; ") {
        Some(at) if error[at..].contains("`rsmt ") => &error[..at],
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rosamta_core::i18n::{self, Lang};

    use super::plain;

    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// A key that is not in the tables is shown as it is written.
    #[test]
    fn every_text_key_in_the_source_exists() {
        const SECTIONS: [&str; 18] = [
            "action",
            "badge",
            "confirm",
            "dotfiles",
            "history",
            "import",
            "item",
            "library",
            "menu",
            "nav",
            "phase",
            "policy",
            "problem",
            "problem_title",
            "review",
            "settings",
            "setup",
            "status",
        ];
        let mut files = Vec::new();
        sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut found = 0;
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            // Not the tests: they hold file names that look like keys.
            let source = source.split("#[cfg(test)]").next().unwrap();
            // What is between quotes: every second piece.
            for literal in source.split('"').skip(1).step_by(2) {
                let Some((section, name)) = literal.split_once('.') else {
                    continue;
                };
                let named = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                if SECTIONS.contains(&section) && named {
                    found += 1;
                    assert_ne!(
                        i18n::text(Lang::En, literal, &[]),
                        literal,
                        "{}",
                        file.display()
                    );
                }
            }
        }
        assert!(found > 100, "only {found} keys were found");
    }

    #[test]
    fn errors_are_shown_without_the_command_to_run() {
        assert_eq!(
            plain("no review item 3; run `rsmt review`"),
            "no review item 3"
        );
        assert_eq!(
            plain("a.toml: bad; value; run `rsmt validate` for details"),
            "a.toml: bad; value"
        );
        assert_eq!(plain("gpg failed; try again"), "gpg failed; try again");
    }
}
