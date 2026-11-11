//! Changing where a module or dotfile applies, in place, keeping comments.

use serde::Serialize;
use toml_edit::{Item, TableLike};

use crate::config::{Arch, Os, Shell};

/// What to add so an item loads on this host: to its `when`, and to a
/// module's `shells`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Widening {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<Os>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<Arch>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shells: Vec<Shell>,
}

impl Widening {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `"macos"` for `Os::Macos`: the name the files use.
pub fn name(value: impl Serialize) -> String {
    match toml::Value::try_from(value) {
        Ok(toml::Value::String(name)) => name,
        _ => unreachable!("only unit enums are named"),
    }
}

/// Add `widening` to `item`, a module's document root or one dotfile entry.
/// Only lists that are there grow: a missing list places no limit, and
/// starting one would narrow the item instead.
pub(crate) fn widen(item: &mut dyn TableLike, widening: &Widening) {
    for shell in &widening.shells {
        add(item, "shells", &name(shell));
    }
    let Some(when) = item.get_mut("when").and_then(Item::as_table_like_mut) else {
        return;
    };
    let wanted = [
        ("hosts", widening.host.clone()),
        ("os", widening.os.map(name)),
        ("arch", widening.arch.map(name)),
    ];
    for (key, value) in wanted {
        if let Some(value) = value {
            add(when, key, &value);
        }
    }
}

/// Take `host` out of `hosts`, if another host is listed; `false` when
/// none is, so no other host uses the module.
pub(crate) fn remove_host(module: &mut dyn TableLike, host: &str) -> bool {
    let Some(hosts) = module
        .get_mut("when")
        .and_then(Item::as_table_like_mut)
        .and_then(|when| when.get_mut("hosts"))
        .and_then(Item::as_array_mut)
    else {
        return false;
    };
    if hosts.iter().all(|value| value.as_str() == Some(host)) {
        return false;
    }
    hosts.retain(|value| value.as_str() != Some(host));
    // The new first value keeps the space it had after a comma.
    if let Some(first) = hosts.get_mut(0) {
        first.decor_mut().set_prefix("");
    }
    true
}

/// Put back the host and OS limits `every_host` dropped.
pub(crate) fn limit(item: &mut dyn TableLike, hosts: &[String], os: &[Os]) {
    let Some(when) = item
        .entry("when")
        .or_insert(toml_edit::table())
        .as_table_like_mut()
    else {
        return;
    };
    let os: Vec<String> = os.iter().map(name).collect();
    for (key, values) in [("hosts", hosts), ("os", os.as_slice())] {
        if !values.is_empty() {
            let array: toml_edit::Array = values.iter().map(String::as_str).collect();
            when.insert(key, toml_edit::value(array));
        }
    }
}

/// Drop the host and OS limits, for content every host should load.
pub(crate) fn every_host(item: &mut dyn TableLike) {
    let Some(when) = item.get_mut("when").and_then(Item::as_table_like_mut) else {
        return;
    };
    when.remove("hosts");
    when.remove("os");
    if when.is_empty() {
        item.remove("when");
    }
}

fn add(table: &mut dyn TableLike, key: &str, value: &str) {
    if let Some(array) = table.get_mut(key).and_then(Item::as_array_mut)
        && !array.iter().any(|old| old.as_str() == Some(value))
    {
        array.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::DocumentMut;

    fn edit(text: &str, change: impl FnOnce(&mut dyn TableLike)) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        change(doc.as_table_mut());
        doc.to_string()
    }

    #[test]
    fn widening_keeps_comments_and_skips_values_already_there() {
        let text =
            "# bun\nshells = [\"zsh\"]\n\n[when]\nhosts = [\"mbp\"] # first\nos = [\"macos\"]\n";
        let widening = Widening {
            host: Some("box".into()),
            os: Some(Os::Linux),
            arch: None,
            shells: vec![Shell::Zsh],
        };
        assert_eq!(
            edit(text, |doc| widen(doc, &widening)),
            "# bun\nshells = [\"zsh\"]\n\n[when]\nhosts = [\"mbp\", \"box\"] # first\nos = [\"macos\", \"linux\"]\n"
        );
    }

    #[test]
    fn widening_never_starts_a_limit() {
        let widening = Widening {
            host: Some("box".into()),
            os: Some(Os::Macos),
            arch: None,
            shells: vec![Shell::Bash],
        };
        let open = "[env]\nA = \"1\"\n";
        assert_eq!(edit(open, |doc| widen(doc, &widening)), open);
        let hosts = "[when]\nhosts = [\"mbp\"]\n";
        assert_eq!(
            edit(hosts, |doc| widen(doc, &widening)),
            "[when]\nhosts = [\"mbp\", \"box\"]\n"
        );
    }

    #[test]
    fn removing_a_host_leaves_the_others() {
        let two = "[when]\nhosts = [\"mbp\", \"box\"]\n";
        assert_eq!(
            edit(two, |doc| assert!(remove_host(doc, "box"))),
            "[when]\nhosts = [\"mbp\"]\n"
        );
        assert_eq!(
            edit(two, |doc| assert!(remove_host(doc, "mbp"))),
            "[when]\nhosts = [\"box\"]\n"
        );
        for alone in ["[when]\nhosts = [\"box\"]\n", "[env]\nA = \"1\"\n"] {
            assert_eq!(edit(alone, |doc| assert!(!remove_host(doc, "box"))), alone);
        }
    }

    #[test]
    fn limits_come_back_after_every_host() {
        let text = "[when]\nhosts = [\"mbp\"]\nos = [\"macos\"]\n";
        let open = edit(text, every_host);
        assert_eq!(
            edit(&open, |doc| limit(doc, &["mbp".into()], &[Os::Macos])),
            text
        );
    }

    #[test]
    fn every_host_drops_hosts_and_os_only() {
        let text = "[when]\nhosts = [\"mbp\"]\nos = [\"macos\"]\ntags = [\"work\"]\n";
        assert_eq!(edit(text, every_host), "[when]\ntags = [\"work\"]\n");
        assert_eq!(edit("[when]\nhosts = [\"mbp\"]\n", every_host), "");
    }
}
