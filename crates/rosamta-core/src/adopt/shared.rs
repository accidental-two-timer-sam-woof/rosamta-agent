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

/// Snippet text without comment lines, blank lines or trailing spaces;
/// lines inside a quoted string or a here-document are data and stay whole.
pub(super) fn code(text: &str) -> Vec<&str> {
    use super::segment::{heredoc_end, heredocs};
    let mut out = Vec::new();
    let mut rest = text;
    let mut quote = None;
    while !rest.is_empty() {
        let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
        rest = after;
        let line = match quote {
            Some(_) => line,
            None => line.trim_end(),
        };
        if quote.is_none() && (line.is_empty() || line.trim_start().starts_with('#')) {
            continue;
        }
        out.push(line);
        quote = open_quote(line, quote);
        if quote.is_none() {
            let body = heredoc_end(rest, &heredocs(line)).unwrap_or(rest.len());
            out.extend(rest[..body].lines());
            rest = &rest[body..];
        }
    }
    out
}

/// The quote still open after `line`, which starts inside `quote`.
fn open_quote(line: &str, mut quote: Option<char>) -> Option<char> {
    let (mut escaped, mut prev) = (false, ' ');
    for c in line.chars() {
        match quote {
            _ if escaped => escaped = false,
            Some('\'') if c == '\'' => quote = None,
            Some('\'') => {}
            Some(_) if c == '\\' => escaped = true,
            Some(_) if c == '"' => quote = None,
            Some(_) => {}
            None if c == '#' && prev.is_whitespace() => break,
            None if c == '\\' => escaped = true,
            None if c == '\'' || c == '"' => quote = Some(c),
            None => {}
        }
        prev = c;
    }
    quote
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Os;

    #[test]
    fn lines_inside_a_string_are_code() {
        let printf = |line: &str| format!("# say it\nprintf '%s\\n' 'a\n{line}\nb'\n# done\n");
        assert_ne!(code(&printf("# x=1")), code(&printf("# x=2")));
        assert_eq!(code(&printf("# x=1")), ["printf '%s\\n' 'a", "# x=1", "b'"]);
        assert_eq!(
            code("echo \"it's\" # note\n# gone\n"),
            ["echo \"it's\" # note"]
        );
    }

    #[test]
    fn here_document_lines_are_code() {
        let config =
            |line: &str| format!("# write it\ncat > ~/.x <<'EOF'\n{line}\nEOF\n\nexport X=1\n");
        assert_eq!(
            code(&config("# a=1")),
            ["cat > ~/.x <<'EOF'", "# a=1", "EOF", "export X=1"]
        );
        assert_ne!(code(&config("# a=1")), code(&config("# a=2")));
        assert_eq!(
            code(&config("a=1")),
            code(&format!(
                "\n{}",
                config("a=1").replace("# write it", "# from mbp")
            ))
        );
    }

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
