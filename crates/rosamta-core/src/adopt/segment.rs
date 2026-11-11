//! Watched files and how their content splits into segments.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Phase, Shell};
use crate::hooks::{self, HookEnv};

/// The kind of file content came from: which shells read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// `~/.profile`, read by sh and by bash and zsh through it.
    Profile,
    Bash,
    Zsh,
    Fish,
}

impl Source {
    /// The shell whose hook must be installed before content is moved out.
    pub fn owner(self) -> Shell {
        match self {
            Source::Profile => Shell::Sh,
            Source::Bash => Shell::Bash,
            Source::Zsh => Shell::Zsh,
            Source::Fish => Shell::Fish,
        }
    }

    /// Shells a snippet from this kind of file is rendered for.
    pub fn shells(self) -> &'static [Shell] {
        match self {
            Source::Profile => &[Shell::Sh, Shell::Bash, Shell::Zsh],
            Source::Bash => &[Shell::Bash],
            Source::Zsh => &[Shell::Zsh],
            Source::Fish => &[Shell::Fish],
        }
    }
}

/// A file rosamta watches for content outside its hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub path: PathBuf,
    pub source: Source,
    /// Where snippets from this file load.
    pub phase: Phase,
    /// Startup order of the file, so snippets keep their relative order.
    pub rank: u32,
    /// fish `conf.d` files are adopted whole.
    pub whole_file: bool,
}

/// Every watched file that exists, in startup order.
pub fn watched_files(env: &HookEnv) -> Vec<Watched> {
    let file = |path: PathBuf, source, phase, rank| Watched {
        path,
        source,
        phase,
        rank,
        whole_file: false,
    };
    let mut out = vec![
        file(env.home.join(".profile"), Source::Profile, Phase::Env, 1),
        file(env.home.join(".bash_profile"), Source::Bash, Phase::Env, 2),
        file(env.home.join(".bash_login"), Source::Bash, Phase::Env, 3),
        file(env.zshenv_dir.join(".zshenv"), Source::Zsh, Phase::Env, 4),
        file(env.zdotdir.join(".zprofile"), Source::Zsh, Phase::Login, 5),
        file(env.home.join(".bashrc"), Source::Bash, Phase::Late, 6),
        file(env.zdotdir.join(".zshrc"), Source::Zsh, Phase::Late, 7),
        file(env.zdotdir.join(".zlogin"), Source::Zsh, Phase::Late, 8),
    ];
    // fish reads conf.d (by name) before config.fish.
    let conf_d = env.fish_config.join("conf.d");
    let mut snippets: Vec<PathBuf> = std::fs::read_dir(&conf_d)
        .into_iter()
        .flatten()
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "fish")
                && path.file_name().is_some_and(|name| name != "rosamta.fish")
        })
        .collect();
    snippets.sort();
    out.extend(snippets.into_iter().map(|path| Watched {
        path,
        source: Source::Fish,
        phase: Phase::Late,
        rank: 9,
        whole_file: true,
    }));
    out.push(file(
        env.fish_config.join("config.fish"),
        Source::Fish,
        Phase::Late,
        10,
    ));
    out.retain(|watched| watched.path.is_file());
    out
}

/// A run of lines that is adopted, ignored or kept as one piece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// First and last line, 1-based and inclusive.
    pub start: usize,
    pub end: usize,
    /// The lines as written, each with its line ending.
    pub lines: Vec<String>,
    /// Name inside `# >>> name >>>`, for marker blocks.
    pub marker: Option<String>,
}

impl Segment {
    pub fn text(&self) -> String {
        let mut text: String = self.lines.concat();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    pub fn comment_only(&self) -> bool {
        self.lines
            .iter()
            .map(|line| line.trim())
            .all(|line| line.is_empty() || line.starts_with('#'))
    }
}

/// The `config.fish` fish writes on first start. It is fish's, not the
/// user's, so it is not adopted.
pub fn fish_default(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines
        == [
            "if status is-interactive",
            "# Commands to run in interactive sessions can go here",
            "end",
        ]
}

/// Written above lines kept as comments, so scans skip them.
pub const COMMENTED: &str = "# rosamta: adopted";

fn marker(line: &str, arrows: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('#')?.trim();
    let name = rest.strip_prefix(arrows)?.strip_suffix(arrows)?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Split everything outside the hook into segments: runs of non-blank lines,
/// with whole `# >>> name >>>` ... `# <<< name <<<` blocks kept together.
/// A blank line ends a segment only where `complete` says the code so far
/// parses, so an `if` with a blank line inside stays whole.
pub fn segments(text: &str, complete: &dyn Fn(&str) -> bool) -> Vec<Segment> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let hook = hooks::find_block(text).map(|(start, end)| {
        let line_of = |offset: usize| text[..offset].matches('\n').count();
        // `end` is past the END line, which may be the last and lack a newline.
        (line_of(start), line_of(end - 1) + 1)
    });
    let mut out = Vec::new();
    let mut current: Option<Segment> = None;
    let flush = |current: &mut Option<Segment>, out: &mut Vec<Segment>| {
        if let Some(segment) = current.take() {
            out.push(segment);
        }
    };
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if hook.is_some_and(|(start, end)| index >= start && index < end) {
            flush(&mut current, &mut out);
            index += 1;
            continue;
        }
        if line.trim_start().starts_with(COMMENTED) {
            flush(&mut current, &mut out);
            index += 1;
            while index < lines.len() && lines[index].trim_start().starts_with('#') {
                index += 1;
            }
            continue;
        }
        if let Some(name) = marker(line, ">>>") {
            let close = (index + 1..lines.len())
                .find(|&end| marker(lines[end], "<<<").as_deref() == Some(name.as_str()));
            if let Some(close) = close {
                flush(&mut current, &mut out);
                out.push(Segment {
                    start: index + 1,
                    end: close + 1,
                    lines: lines[index..=close].iter().map(|s| s.to_string()).collect(),
                    marker: Some(name),
                });
                index = close + 1;
                continue;
            }
        }
        if line.trim().is_empty() {
            match &mut current {
                // `sh -n` accepts an unterminated heredoc, so check that first.
                Some(segment) if heredoc_open(&segment.text()) || !complete(&segment.text()) => {
                    segment.end = index + 1;
                    segment.lines.push(line.to_string());
                }
                _ => flush(&mut current, &mut out),
            }
        } else {
            // A line right after a finished block (an installer appending
            // without a blank line) starts its own segment.
            let after_block = current.as_ref().is_some_and(|segment| {
                let last = segment.lines.last().map_or("", |line| line.trim());
                let last = last.trim_end_matches(';').trim();
                ["fi", "done", "esac", "}", "end"].contains(&last)
                    && !heredoc_open(&segment.text())
                    && complete(&segment.text())
            });
            if after_block {
                flush(&mut current, &mut out);
            }
            let segment = current.get_or_insert_with(|| Segment {
                start: index + 1,
                end: index + 1,
                lines: Vec::new(),
                marker: None,
            });
            segment.end = index + 1;
            segment.lines.push(line.to_string());
        }
        index += 1;
    }
    flush(&mut current, &mut out);
    out
}

/// The whole file as one segment (fish `conf.d`), or `None` when it is blank.
pub fn whole(text: &str) -> Option<Segment> {
    let lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    // Comment mode already adopted it.
    if lines.iter().all(|line| line.trim().is_empty()) || text.trim_start().starts_with(COMMENTED) {
        return None;
    }
    Some(Segment {
        start: 1,
        end: lines.len(),
        lines,
        marker: None,
    })
}

/// The heredocs `line` opens, in order: the delimiter, and whether `<<-`
/// strips leading tabs. Not `<<<`, nor `<<` in quotes, comments or `((…))`.
pub(super) fn heredocs(line: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let (mut single, mut double, mut escaped) = (false, false, false);
    let mut arithmetic = 0;
    let mut prev = ' ';
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if !single && !double && !escaped {
            if c == '#' && prev.is_whitespace() {
                break;
            }
            if let Some(after) = rest.strip_prefix("((") {
                arithmetic += 1;
                (prev, rest) = ('(', after);
                continue;
            }
            if arithmetic > 0
                && let Some(after) = rest.strip_prefix("))")
            {
                arithmetic -= 1;
                (prev, rest) = (')', after);
                continue;
            }
            if let Some(after) = rest.strip_prefix("<<") {
                let after = match after.strip_prefix('<') {
                    Some(after) => after,
                    None if arithmetic == 0 => {
                        let spec = after.strip_prefix('-').unwrap_or(after).trim_start();
                        let len = spec
                            .find(|c: char| c.is_whitespace() || ";&|<>()".contains(c))
                            .unwrap_or(spec.len());
                        let word: String = spec[..len]
                            .chars()
                            .filter(|c| !"'\"\\".contains(*c))
                            .collect();
                        if !word.is_empty() {
                            out.push((word, after.starts_with('-')));
                        }
                        after
                    }
                    None => after,
                };
                (prev, rest) = ('<', after);
                continue;
            }
        }
        match c {
            _ if escaped => escaped = false,
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            _ => {}
        }
        prev = c;
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// How far into `rest` the bodies of `pending` heredocs run, through their
/// closing lines; `None` while one is still open.
pub(super) fn heredoc_end(rest: &str, pending: &[(String, bool)]) -> Option<usize> {
    let mut pending = pending.iter();
    let Some(mut want) = pending.next() else {
        return Some(0);
    };
    let mut len = 0;
    for line in rest.split_inclusive('\n') {
        len += line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let body = if want.1 {
            body.trim_start_matches('\t')
        } else {
            body
        };
        if body == want.0 {
            match pending.next() {
                Some(next) => want = next,
                None => return Some(len),
            }
        }
    }
    None
}

/// Whether `text` ends inside a heredoc body.
fn heredoc_open(text: &str) -> bool {
    let mut rest = text;
    while !rest.is_empty() {
        let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
        match heredoc_end(after, &heredocs(line)) {
            Some(len) => rest = &after[len..],
            None => return true,
        }
    }
    false
}

/// Replace absolute paths under `home` with `$HOME` wherever that keeps the
/// meaning: inside single quotes nothing expands, so those stay as written.
pub fn rewrite_home(text: &str, home: &Path, fish: bool) -> String {
    let home = home.to_string_lossy();
    if home.len() < 2 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let (mut single, mut double, mut escaped) = (false, false, false);
    // `$(` starts fresh quoting; the outer state comes back at its `)`.
    let mut outer: Vec<(bool, bool)> = Vec::new();
    let mut rest = text;
    let mut line_start = 0;
    while let Some(c) = rest.chars().next() {
        if c == '\n' && !fish && !single && !double && !escaped {
            // Heredoc bodies are data, and `<<'EOF'` keeps `$HOME` literal.
            let at = text.len() - rest.len();
            let body = &rest[1..];
            let len = heredoc_end(body, &heredocs(&text[line_start..at])).unwrap_or(body.len());
            out.push_str(&rest[..=len]);
            rest = &body[len..];
            line_start = text.len() - rest.len();
            continue;
        }
        if !single && !escaped {
            let opens = if fish { "(" } else { "$(" };
            if rest.starts_with(opens) && (!fish || !double) {
                outer.push((single, double));
                (single, double) = (false, false);
                out.push_str(opens);
                rest = &rest[opens.len()..];
                continue;
            }
            if c == ')'
                && !double
                && let Some(state) = outer.pop()
            {
                (single, double) = state;
                out.push(c);
                rest = &rest[1..];
                continue;
            }
        }
        if !single && !escaped && rest.starts_with(home.as_ref()) {
            let after = &rest[home.len()..];
            let boundary = after
                .chars()
                .next()
                .is_none_or(|next| !(next.is_alphanumeric() || "-_.".contains(next)));
            if boundary {
                // Unquoted POSIX words split on spaces in $HOME; fish never splits.
                out.push_str(if double || fish { "$HOME" } else { "\"$HOME\"" });
                rest = after;
                continue;
            }
        }
        let word_start = out.chars().last().is_none_or(char::is_whitespace);
        if c == '#' && word_start && !single && !double && !escaped {
            // A comment: an apostrophe in it must not open a quote.
            let len = rest.find('\n').unwrap_or(rest.len());
            out.push_str(&rest[..len]);
            rest = &rest[len..];
            continue;
        }
        match c {
            _ if escaped => escaped = false,
            '\\' if !single || fish => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            _ => {}
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Text compared across scans: trailing spaces and the home path do not count.
pub fn normalize(text: &str, home: &Path) -> String {
    let home = home.to_string_lossy();
    text.lines()
        .map(|line| line.trim_end().replace(home.as_ref(), "$HOME"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

pub fn hash(normalized: &str) -> String {
    let mut hasher = crate::fnv::Fnv::new();
    hasher.write(normalized.as_bytes());
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str) -> Vec<(usize, usize)> {
        segments(text, &|_| true)
            .iter()
            .map(|s| (s.start, s.end))
            .collect()
    }

    #[test]
    fn comments_join_the_code_below_them() {
        let text = "\n# Added by OrbStack\nsource ~/.orbstack/shell/init.zsh\n\nexport A=1\n";
        assert_eq!(spans(text), [(2, 3), (5, 5)]);
    }

    #[test]
    fn marker_blocks_stay_whole() {
        let text = "a\n# >>> conda initialize >>>\nx\n\ny\n# <<< conda initialize <<<\nb\n";
        let found = segments(text, &|_| true);
        assert_eq!(spans(text), [(1, 1), (2, 6), (7, 7)]);
        assert_eq!(found[1].marker.as_deref(), Some("conda initialize"));
    }

    #[test]
    fn unclosed_marker_is_an_ordinary_line() {
        assert_eq!(spans("# >>> x >>>\ny\n"), [(1, 2)]);
    }

    #[test]
    fn hook_and_commented_lines_are_skipped() {
        let text = format!(
            "a\n{}\nx\n{}\nb\n\n{COMMENTED} → modules/adopted/a.toml\n# a\n# b\nc\n",
            hooks::BEGIN,
            hooks::END
        );
        assert_eq!(spans(&text), [(1, 1), (5, 5), (10, 10)]);
    }

    #[test]
    fn hook_end_on_a_last_line_without_a_newline_is_skipped() {
        let text = format!("a\n{}\nx\n{}", hooks::BEGIN, hooks::END);
        assert_eq!(spans(&text), [(1, 1)]);
    }

    #[test]
    fn blank_lines_inside_a_heredoc_do_not_split() {
        assert_eq!(spans("cat <<EOF\na\n\nb\nEOF\n\nc\n"), [(1, 5), (7, 7)]);
        assert_eq!(
            spans("cat <<-'END' >x\n\ta\n\n\t}\n\tEND\n\nc\n"),
            [(1, 5), (7, 7)]
        );
        // Neither a here-string, a shift nor a comment opens one.
        assert_eq!(
            spans("cat <<<x\n\ny=$((1 << 2))\n\nz # <<EOF\n\nw\n"),
            [(1, 1), (3, 3), (5, 5), (7, 7)]
        );
    }

    #[test]
    fn a_commented_out_file_is_not_a_segment() {
        assert!(whole(&format!("{COMMENTED} → x.toml\n# a\n")).is_none());
        assert!(whole("# a\n").is_some());
    }

    #[test]
    fn blank_lines_inside_unfinished_code_do_not_split() {
        let text = "if x; then\n  a\n\n  b\nfi\n\nc\n";
        let complete = |code: &str| code.matches("if ").count() == code.matches("fi").count();
        let found = segments(text, &complete);
        assert_eq!(
            found.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
            [(1, 5), (7, 7)]
        );
        assert_eq!(found[0].text(), "if x; then\n  a\n\n  b\nfi\n");
    }

    /// Each installer's addition to each file is the expected number of
    /// segments, and none of them reaches into the file's earlier content.
    #[test]
    fn installer_additions_segment_as_expected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/installers");
        let expected: &[(&str, &str, usize)] = &[
            ("bun", ".bashrc", 1),
            ("conda", ".bashrc", 1),
            ("conda", ".zshrc", 1),
            ("conda", ".config/fish/config.fish", 1),
            ("gcloud", ".bashrc", 2),
            ("homebrew", ".bashrc", 1),
            ("jetbrains-toolbox", ".zprofile", 1),
            ("nvm", ".bashrc", 1),
            ("orbstack", ".zprofile", 1),
            ("rustup", ".bashrc", 1),
            ("rustup", ".profile", 1),
            ("rustup", ".zshenv", 1),
            ("uv", ".bashrc", 1),
            ("uv", ".profile", 1),
            ("uv", ".zshrc", 1),
        ];
        for (fixture, file, count) in expected {
            let after =
                std::fs::read_to_string(root.join(fixture).join("after").join(file)).unwrap();
            let before = std::fs::read_to_string(root.join(fixture).join("before").join(file))
                .unwrap_or_default();
            assert!(
                after.starts_with(&before),
                "{fixture} {file}: installers append"
            );
            let first_new = before.lines().count() + 1;
            let found = segments(&after, &|_| true);
            let new: Vec<&Segment> = found.iter().filter(|s| s.end >= first_new).collect();
            assert_eq!(new.len(), *count, "{fixture} {file}: {new:?}");
            assert!(new.iter().all(|s| s.start >= first_new), "{fixture} {file}");
        }
    }

    #[test]
    fn a_line_after_a_finished_block_starts_a_segment() {
        assert_eq!(spans("if a; then\n  b\nfi\n. x\n"), [(1, 3), (4, 4)]);
        assert_eq!(spans("f() {\n  a\n}\nalias x=y\n"), [(1, 3), (4, 4)]);
    }

    #[test]
    fn segment_text_keeps_line_endings() {
        let found = segments("a\r\nb", &|_| true);
        assert_eq!(found[0].text(), "a\r\nb\n");
    }

    #[test]
    fn home_paths_become_home_where_safe() {
        let home = Path::new("/Users/me");
        let rewrite = |text| rewrite_home(text, home, false);
        assert_eq!(rewrite("/Users/me/x"), "\"$HOME\"/x");
        assert_eq!(rewrite("\"/Users/me/x\""), "\"$HOME/x\"");
        assert_eq!(rewrite("'/Users/me/x'"), "'/Users/me/x'");
        assert_eq!(rewrite("/Users/meow"), "/Users/meow");
        assert_eq!(rewrite("a\\'/Users/me"), "a\\'\"$HOME\"");
        assert_eq!(rewrite("# won't\n. /Users/me/x"), "# won't\n. \"$HOME\"/x");
        assert_eq!(
            rewrite("a=\"$('/Users/me/c' x)\" /Users/me/d"),
            "a=\"$('/Users/me/c' x)\" \"$HOME\"/d"
        );
        assert_eq!(rewrite("a=\"$(/Users/me/c)\""), "a=\"$(\"$HOME\"/c)\"");
        assert_eq!(rewrite_home("/Users/me/x", home, true), "$HOME/x");
        // Heredoc bodies stay as written.
        assert_eq!(
            rewrite("cat <<'EOF'\n/Users/me/x it's\nEOF\n. /Users/me/y\n"),
            "cat <<'EOF'\n/Users/me/x it's\nEOF\n. \"$HOME\"/y\n"
        );
    }

    #[test]
    fn normalized_hash_ignores_home_and_trailing_space() {
        let home = Path::new("/Users/me");
        let a = normalize("export A=/Users/me/x  \n", home);
        let b = normalize("\nexport A=$HOME/x\n", home);
        assert_eq!(hash(&a), hash(&b));
        assert_ne!(hash(&a), hash("export A=1"));
    }
}
