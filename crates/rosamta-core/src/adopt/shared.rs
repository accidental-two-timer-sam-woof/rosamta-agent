//! Content another host adopted already: the same lines installed on two
//! hosts become one module that lists both.

use crate::config::{HostFacts, Module};

/// The repo module that does exactly what `new` does but only for other
/// hosts, if adding this host to it makes it load here. Its order must come
/// after `after` within the same file, so the file's content keeps running
/// in the order it is written.
pub(super) fn find<'a>(
    repo: &'a [(String, Module)],
    new: &Module,
    facts: &HostFacts,
    after: u32,
) -> Option<&'a (String, Module)> {
    repo.iter().find(|(_, old)| {
        old.enabled
            && old.order > after
            && old.order / 1000 == after / 1000
            && !old.when.hosts.is_empty()
            && !old.when.hosts.contains(&facts.id)
            && loads_with_this_host(old, facts)
            && same(old, new)
    })
}

fn loads_with_this_host(old: &Module, facts: &HostFacts) -> bool {
    let mut when = old.when.clone();
    when.hosts.push(facts.id.clone());
    if !when.os.is_empty() {
        when.os.push(facts.os);
    }
    when.matches(facts)
}

/// Same shells and the same work. Values carry `$HOME` already; map order
/// does not matter, PATH order does.
fn same(a: &Module, b: &Module) -> bool {
    let shells = |module: &Module| {
        let mut shells = module.shells.clone();
        shells.sort();
        shells
    };
    !a.shells.is_empty()
        && shells(a) == shells(b)
        && a.phase == b.phase
        && a.env == b.env
        && a.path == b.path
        && a.aliases == b.aliases
        && crate::config::Shell::ALL
            .into_iter()
            .all(|shell| a.raw.get(shell).map(code) == b.raw.get(shell).map(code))
}

/// Snippet text without comment lines, blank lines or trailing spaces.
pub(super) fn code(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.trim_start().starts_with('#'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Os;

    fn module(text: &str) -> Module {
        toml::from_str(text).unwrap()
    }

    fn facts() -> HostFacts {
        HostFacts {
            os: Os::Linux,
            arch: crate::config::Arch::X86_64,
            id: "box".into(),
            tags: Vec::new(),
        }
    }

    const BUN: &str = "order = 7001\nshells = [\"zsh\"]\n[when]\nhosts = [\"mbp\"]\nos = [\"macos\"]\n[env]\nBUN_INSTALL = \"$HOME/.bun\"\n[path]\nprepend = [\"$BUN_INSTALL/bin\"]\n";

    #[test]
    fn the_same_content_from_another_host_is_found() {
        let repo = vec![("bun".to_string(), module(BUN))];
        let new = module(
            "shells = [\"zsh\"]\n[path]\nprepend = [\"$BUN_INSTALL/bin\"]\n[env]\nBUN_INSTALL = \"$HOME/.bun\"\n",
        );
        assert_eq!(name(find(&repo, &new, &facts(), 7000)), Some("bun"));
    }

    fn name(found: Option<&(String, Module)>) -> Option<&str> {
        found.map(|(name, _)| name.as_str())
    }

    #[test]
    fn different_values_shells_or_limits_are_not() {
        let new = module(
            BUN.replace("[when]\nhosts = [\"mbp\"]\nos = [\"macos\"]\n", "")
                .as_str(),
        );
        for old in [
            BUN.replace(".bun\"", ".bun2\""),
            BUN.replace("[\"zsh\"]", "[\"bash\"]"),
            BUN.replace("hosts = [\"mbp\"]", "hosts = [\"mbp\", \"box\"]"),
            BUN.replace("hosts = [\"mbp\"]\n", ""),
            BUN.replace("os = [\"macos\"]", "os = [\"macos\"]\ntags = [\"work\"]"),
            format!("enabled = false\n{BUN}"),
        ] {
            let repo = vec![("bun".to_string(), module(&old))];
            assert_eq!(name(find(&repo, &new, &facts(), 7000)), None, "{old}");
        }
        // Content this host adopted from later in the file would run first.
        let repo = vec![("bun".to_string(), module(BUN))];
        assert_eq!(name(find(&repo, &new, &facts(), 7001)), None);
        assert_eq!(name(find(&repo, &new, &facts(), 6000)), None);
    }

    #[test]
    fn snippets_compare_without_comments() {
        let old = "order = 7001\nshells = [\"zsh\"]\n[when]\nhosts = [\"mbp\"]\n[raw]\nzsh = '''\n# Added by OrbStack\nsource ~/.orbstack/shell/init.zsh  \n'''\n";
        let new = "shells = [\"zsh\"]\n[raw]\nzsh = '''\n# OrbStack: command-line tools\nsource ~/.orbstack/shell/init.zsh\n'''\n";
        let repo = vec![("orbstack".to_string(), module(old))];
        assert_eq!(
            name(find(&repo, &module(new), &facts(), 7000)),
            Some("orbstack")
        );
    }
}
