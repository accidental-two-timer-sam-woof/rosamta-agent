//! Turn a segment into a structured module, line by line. Anything a rule
//! does not fully understand makes the whole segment a snippet: a snippet
//! still works, while a wrong guess would change the shell.

use indexmap::IndexMap;

use crate::config::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Posix,
    Zsh,
    Fish,
}

/// One step of recognition. Providers (`add-providers-and-doctor`) register
/// their own ahead of the structured one.
pub trait Recognizer {
    /// A module equivalent to `lines`, or `None` to let the next one try.
    fn recognize(&self, lines: &[&str], dialect: Dialect) -> Option<Module>;
}

pub fn recognizers() -> Vec<Box<dyn Recognizer>> {
    vec![Box::new(Structured)]
}

/// The first recognizer's answer; `None` means keep a snippet.
pub fn recognize(
    lines: &[&str],
    dialect: Dialect,
    recognizers: &[Box<dyn Recognizer>],
) -> Option<Module> {
    recognizers
        .iter()
        .find_map(|recognizer| recognizer.recognize(lines, dialect))
}

/// Environment variables, PATH entries and aliases.
pub struct Structured;

impl Recognizer for Structured {
    fn recognize(&self, lines: &[&str], dialect: Dialect) -> Option<Module> {
        let lines: Vec<&str> = lines.iter().map(|line| line.trim()).collect();
        let mut found = Found::default();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            index += 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if dialect != Dialect::Fish && line.starts_with("case ") {
                let end = (index..lines.len()).find(|&end| lines[end] == "esac")?;
                path_case(&lines[index - 1..=end], &mut found)?;
                index = end + 1;
                continue;
            }
            let commands = match dialect {
                Dialect::Fish => fish_words(line)?,
                Dialect::Zsh if zsh_path(line, &mut found)? => continue,
                _ => posix_words(line)?,
            };
            for words in commands {
                match dialect {
                    Dialect::Fish => fish_command(&words, &mut found)?,
                    _ => posix_command(&words, &mut found)?,
                }
            }
        }
        found.finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    /// Literal text, and whether it was quoted or escaped.
    Lit(String, bool),
    Var(String),
    /// A leading `~` that the shell expands.
    Tilde,
}

type Word = Vec<Part>;

#[derive(Default)]
struct Found {
    env: IndexMap<String, String>,
    /// Assigned but not (yet) exported.
    shell_vars: IndexMap<String, String>,
    /// Exported before being assigned.
    exported: Vec<String>,
    prepend: Vec<String>,
    append: Vec<String>,
    aliases: IndexMap<String, String>,
}

impl Found {
    fn assign(&mut self, name: &str, value: String, export: bool) {
        if export || self.exported.iter().any(|seen| seen == name) {
            self.env.insert(name.to_string(), value);
        } else {
            self.shell_vars.insert(name.to_string(), value);
        }
    }

    fn export(&mut self, name: &str) {
        match self.shell_vars.shift_remove(name) {
            Some(value) => {
                self.env.insert(name.to_string(), value);
            }
            None => self.exported.push(name.to_string()),
        }
    }

    fn finish(self) -> Option<Module> {
        let exported_unset = self
            .exported
            .iter()
            .any(|name| !self.env.contains_key(name));
        let empty = self.env.is_empty()
            && self.prepend.is_empty()
            && self.append.is_empty()
            && self.aliases.is_empty();
        if !self.shell_vars.is_empty() || exported_unset || empty {
            return None;
        }
        let mut module: Module = toml::from_str("").expect("an empty module parses");
        module.env = self.env;
        module.path.prepend = self.prepend;
        module.path.append = self.append;
        module.aliases = self.aliases;
        Some(module)
    }
}

fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `$NAME` or `${NAME}` at the start of `rest`, with the length it spans.
fn variable(rest: &str, braces: bool) -> Option<(String, usize)> {
    let body = rest.strip_prefix('$')?;
    if braces && let Some(inner) = body.strip_prefix('{') {
        let name = &inner[..inner.find('}')?];
        return is_name(name).then(|| (name.to_string(), name.len() + 3));
    }
    let len = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(body.len());
    let name = &body[..len];
    is_name(name).then(|| (name.to_string(), len + 1))
}

fn push_lit(word: &mut Word, c: char, quoted: bool) {
    match word.last_mut() {
        Some(Part::Lit(text, was)) if *was == quoted => text.push(c),
        _ => word.push(Part::Lit(c.to_string(), quoted)),
    }
}

/// Commands of plain words, split on unquoted blanks and `;`. `None` for
/// anything else: operators, substitutions, globs, line continuations.
fn posix_words(line: &str) -> Option<Vec<Vec<Word>>> {
    let mut commands = vec![Vec::new()];
    let mut word: Option<Word> = None;
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let mut step = c.len_utf8();
        let at_start = word.as_ref().is_none_or(Vec::is_empty);
        match c {
            ' ' | '\t' => {
                commands.last_mut()?.extend(word.take());
            }
            ';' => {
                commands.last_mut()?.extend(word.take());
                commands.push(Vec::new());
            }
            '#' if at_start => break,
            '\'' => {
                let end = rest[1..].find('\'')? + 1;
                let current = word.get_or_insert_with(Vec::new);
                current.push(Part::Lit(rest[1..end].to_string(), true));
                step = end + 1;
            }
            '"' => {
                let current = word.get_or_insert_with(Vec::new);
                let mut inner = &rest[1..];
                loop {
                    let d = inner.chars().next()?;
                    match d {
                        '"' => break,
                        '\\' => {
                            let next = inner[1..].chars().next()?;
                            if "$`\"\\".contains(next) {
                                push_lit(current, next, true);
                            } else {
                                push_lit(current, '\\', true);
                                push_lit(current, next, true);
                            }
                            inner = &inner[1 + next.len_utf8()..];
                        }
                        '$' => {
                            let (name, len) = variable(inner, true)?;
                            current.push(Part::Var(name));
                            inner = &inner[len..];
                        }
                        '`' => return None,
                        _ => {
                            push_lit(current, d, true);
                            inner = &inner[d.len_utf8()..];
                        }
                    }
                }
                step = rest.len() - inner.len() + 1;
            }
            '\\' => {
                let next = rest[1..].chars().next()?;
                push_lit(word.get_or_insert_with(Vec::new), next, true);
                step = 1 + next.len_utf8();
            }
            '$' => {
                let (name, len) = variable(rest, true)?;
                word.get_or_insert_with(Vec::new).push(Part::Var(name));
                step = len;
            }
            '~' if at_start || after_assignment(word.as_deref()) => {
                let next = rest[1..].chars().next();
                if !next.is_none_or(|next| next == '/' || next == ' ') {
                    return None;
                }
                word.get_or_insert_with(Vec::new).push(Part::Tilde);
            }
            '`' | '&' | '|' | '<' | '>' | '(' | ')' | '*' | '?' | '[' | '{' | '}' => {
                return None;
            }
            _ => push_lit(word.get_or_insert_with(Vec::new), c, false),
        }
        rest = &rest[step..];
    }
    commands.last_mut()?.extend(word);
    commands.retain(|words| !words.is_empty());
    Some(commands)
}

/// True right after the `=` of an unquoted `NAME=`.
fn after_assignment(word: Option<&[Part]>) -> bool {
    matches!(word, Some([Part::Lit(text, false)]) if text.ends_with('=') && is_name(&text[..text.len() - 1]))
}

/// Split `NAME=value` into the name and the value's parts.
fn assignment(word: &Word) -> Option<(String, Word)> {
    let Some(Part::Lit(first, false)) = word.first() else {
        return None;
    };
    let (name, rest) = first.split_once('=')?;
    if !is_name(name) {
        return None;
    }
    let mut value = Vec::new();
    if !rest.is_empty() {
        value.push(Part::Lit(rest.to_string(), false));
    }
    value.extend(word[1..].iter().cloned());
    Some((name.to_string(), value))
}

fn plain(word: &Word) -> Option<&str> {
    match word.as_slice() {
        [Part::Lit(text, false)] => Some(text),
        _ => None,
    }
}

/// The value as a module writes it: literal text, `$NAME`, a leading `~`.
fn module_value(parts: &[Part]) -> Option<String> {
    let mut out = String::new();
    for (index, part) in parts.iter().enumerate() {
        match part {
            Part::Tilde if index == 0 => out.push('~'),
            Part::Tilde => return None,
            Part::Lit(text, _) => {
                // A literal `$NAME` or leading `~` would expand once rendered.
                let expands = text.match_indices('$').any(|(at, _)| {
                    text[at + 1..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '{')
                });
                if expands || (out.is_empty() && text.starts_with('~')) {
                    return None;
                }
                out.push_str(text);
            }
            Part::Var(name) => {
                let next_is_name = parts.get(index + 1).is_some_and(|next| {
                    matches!(next, Part::Lit(text, _) if text.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
                });
                if next_is_name {
                    out.push_str(&format!("${{{name}}}"));
                } else {
                    out.push_str(&format!("${name}"));
                }
            }
        }
    }
    Some(out)
}

/// An alias body every shell runs the same way: words, flags, paths and
/// pipes. `$?`, quoting and substitutions differ between POSIX shells and
/// fish, so those aliases stay in a snippet for their own shell.
fn alias_body(parts: &[Part]) -> Option<String> {
    let body = literal(parts)?;
    let portable = body
        .chars()
        .all(|c| c.is_alphanumeric() || " -_./=,:@%+~|".contains(c));
    (portable && !body.trim().is_empty()).then_some(body)
}

/// An alias body is literal text; a variable would expand at definition time.
fn literal(parts: &[Part]) -> Option<String> {
    let mut out = String::new();
    for (index, part) in parts.iter().enumerate() {
        match part {
            Part::Lit(text, _) => out.push_str(text),
            Part::Tilde if index == 0 => out.push('~'),
            _ => return None,
        }
    }
    Some(out)
}

/// PATH elements around `$PATH`: before it are prepended, after it appended.
fn path_change(elements: Vec<Word>, found: &mut Found) -> Option<()> {
    let at = elements
        .iter()
        .position(|element| matches!(element.as_slice(), [Part::Var(name)] if name == "PATH"))?;
    let mut values = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        if index != at {
            let value = module_value(element)?;
            if value.is_empty() || value.contains("$PATH") || value.contains("${PATH}") {
                return None;
            }
            values.push(value);
        }
    }
    let append = values.split_off(at);
    found.prepend.extend(values);
    found.append.extend(append);
    Some(())
}

/// Split a value on literal `:`, the PATH separator, quoted or not.
fn split_colons(value: &[Part]) -> Vec<Word> {
    let mut elements = vec![Vec::new()];
    for part in value {
        match part {
            Part::Lit(text, quoted) => {
                for (index, piece) in text.split(':').enumerate() {
                    if index > 0 {
                        elements.push(Vec::new());
                    }
                    if !piece.is_empty() {
                        elements
                            .last_mut()
                            .unwrap()
                            .push(Part::Lit(piece.into(), *quoted));
                    }
                }
            }
            other => elements.last_mut().unwrap().push(other.clone()),
        }
    }
    elements
}

fn assign(name: &str, value: &[Part], export: bool, found: &mut Found) -> Option<()> {
    if name == "PATH" {
        return path_change(split_colons(value), found);
    }
    found.assign(name, module_value(value)?, export);
    Some(())
}

fn posix_command(words: &[Word], found: &mut Found) -> Option<()> {
    match plain(&words[0]) {
        Some("export") if words.len() > 1 => {
            for word in &words[1..] {
                match assignment(word) {
                    Some((name, value)) => assign(&name, &value, true, found)?,
                    None if plain(word).is_some_and(is_name) => found.export(plain(word)?),
                    None => return None,
                }
            }
            Some(())
        }
        Some("alias") if words.len() == 2 => {
            let Some(Part::Lit(first, false)) = words[1].first() else {
                return None;
            };
            let (name, rest) = first.split_once('=')?;
            let mut body = Vec::new();
            if !rest.is_empty() {
                body.push(Part::Lit(rest.to_string(), false));
            }
            body.extend(words[1][1..].iter().cloned());
            found.aliases.insert(name.to_string(), alias_body(&body)?);
            Some(())
        }
        _ if words.len() == 1 => {
            let (name, value) = assignment(&words[0])?;
            assign(&name, &value, false, found)
        }
        _ => None,
    }
}

/// zsh's `path=(dir $path)`, `path=($path dir)` and `path+=(dir)`.
/// `Some(false)` when the line is not about `path`.
fn zsh_path(line: &str, found: &mut Found) -> Option<bool> {
    let Some(rest) = line.strip_prefix("path") else {
        return Some(false);
    };
    let (append, rest) = match rest.strip_prefix("+=") {
        Some(rest) => (true, rest),
        None => match rest.strip_prefix('=') {
            Some(rest) => (false, rest),
            None => return Some(false),
        },
    };
    let inner = match rest.trim().strip_prefix('(') {
        Some(inner) => inner.strip_suffix(')')?,
        None if append => rest.trim(),
        None => return None,
    };
    let mut commands = posix_words(inner)?;
    if commands.len() > 1 {
        return None;
    }
    let mut words = commands.pop().unwrap_or_default();
    for word in &mut words {
        if matches!(word.as_slice(), [Part::Var(name)] if name == "path") {
            *word = vec![Part::Var("PATH".into())];
        }
    }
    if append {
        words.insert(0, vec![Part::Var("PATH".into())]);
    }
    path_change(words, found)?;
    Some(true)
}

/// `case ":$PATH:" in *:"dir":*) ;; *) export PATH="dir:$PATH" ;; esac`:
/// add a directory unless it is already there.
fn path_case(lines: &[&str], found: &mut Found) -> Option<()> {
    let code: Vec<&str> = lines
        .iter()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .copied()
        .collect();
    let joined = code.join(" ");
    let body = joined
        .strip_prefix("case ")?
        .trim_start()
        .strip_suffix("esac")?
        .trim_end();
    let body = ["\":$PATH:\" in", "\":${PATH}:\" in"]
        .iter()
        .find_map(|head| body.strip_prefix(head))?
        .trim();
    let (check, rest) = body.split_once(')')?;
    let rest = rest.trim_start().strip_prefix(";;")?.trim_start();
    let assignment = rest.strip_prefix("*)")?.trim().strip_suffix(";;")?.trim();
    let dir = check
        .trim_matches('*')
        .trim_matches(|c| c == ':' || c == '"');
    let dir = module_value(posix_words(dir)?.first()?.first()?)?;
    let mut added = Found::default();
    for words in posix_words(assignment)? {
        posix_command(&words, &mut added)?;
    }
    let only = |entries: &[String]| entries.len() == 1 && entries[0] == dir;
    let fits = added.env.is_empty() && added.shell_vars.is_empty() && added.aliases.is_empty();
    match (only(&added.prepend), only(&added.append)) {
        (true, false) if fits && added.append.is_empty() => found.prepend.push(dir),
        (false, true) if fits && added.prepend.is_empty() => found.append.push(dir),
        _ => return None,
    }
    Some(())
}

/// fish words, split on unquoted blanks and `;`. `None` for command
/// substitutions, globs, brace expansion, pipes and redirections.
fn fish_words(line: &str) -> Option<Vec<Vec<Word>>> {
    let mut commands = vec![Vec::new()];
    let mut word: Option<Word> = None;
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let mut step = c.len_utf8();
        let at_start = word.as_ref().is_none_or(Vec::is_empty);
        match c {
            ' ' | '\t' => commands.last_mut()?.extend(word.take()),
            ';' => {
                commands.last_mut()?.extend(word.take());
                commands.push(Vec::new());
            }
            '#' if at_start => break,
            '\'' => {
                let current = word.get_or_insert_with(Vec::new);
                let mut inner = &rest[1..];
                loop {
                    let d = inner.chars().next()?;
                    match d {
                        '\'' => break,
                        '\\' if inner[1..].starts_with(['\'', '\\']) => {
                            push_lit(current, inner[1..].chars().next()?, true);
                            inner = &inner[2..];
                        }
                        _ => {
                            push_lit(current, d, true);
                            inner = &inner[d.len_utf8()..];
                        }
                    }
                }
                step = rest.len() - inner.len() + 1;
            }
            '"' => {
                let current = word.get_or_insert_with(Vec::new);
                let mut inner = &rest[1..];
                loop {
                    let d = inner.chars().next()?;
                    match d {
                        '"' => break,
                        '\\' if inner[1..].starts_with(['"', '\\', '$']) => {
                            push_lit(current, inner[1..].chars().next()?, true);
                            inner = &inner[2..];
                        }
                        '$' => {
                            let (name, len) = variable(inner, false)?;
                            if inner[len..].starts_with('[') {
                                return None;
                            }
                            current.push(Part::Var(name));
                            inner = &inner[len..];
                        }
                        _ => {
                            push_lit(current, d, true);
                            inner = &inner[d.len_utf8()..];
                        }
                    }
                }
                step = rest.len() - inner.len() + 1;
            }
            '\\' => {
                let next = rest[1..].chars().next()?;
                if !" $\\'\"*?~#(){}[]<>&|;".contains(next) {
                    return None;
                }
                push_lit(word.get_or_insert_with(Vec::new), next, true);
                step = 1 + next.len_utf8();
            }
            '$' => {
                let (name, len) = variable(rest, false)?;
                if rest[len..].starts_with('[') {
                    return None;
                }
                word.get_or_insert_with(Vec::new).push(Part::Var(name));
                step = len;
            }
            '~' if at_start => {
                if !rest[1..].chars().next().is_none_or(|next| next == '/') {
                    return None;
                }
                word.get_or_insert_with(Vec::new).push(Part::Tilde);
            }
            '&' | '|' | '<' | '>' | '(' | ')' | '*' | '?' | '[' | ']' | '{' | '}' => {
                return None;
            }
            _ => push_lit(word.get_or_insert_with(Vec::new), c, false),
        }
        rest = &rest[step..];
    }
    commands.last_mut()?.extend(word);
    commands.retain(|words| !words.is_empty());
    Some(commands)
}

fn fish_command(words: &[Word], found: &mut Found) -> Option<()> {
    match plain(&words[0])? {
        "set" => {
            let mut export = false;
            let mut index = 1;
            while let Some(flag) = words
                .get(index)
                .and_then(plain)
                .filter(|w| w.starts_with('-'))
            {
                match flag {
                    "--global" => {}
                    "--export" => export = true,
                    _ if flag.starts_with("--") => return None,
                    _ => {
                        if !flag[1..].chars().all(|c| c == 'g' || c == 'x') {
                            return None;
                        }
                        export |= flag.contains('x');
                    }
                }
                index += 1;
            }
            let name = words
                .get(index)
                .and_then(plain)
                .filter(|name| is_name(name))?;
            let values = &words[index + 1..];
            if !export {
                return None;
            }
            if name == "PATH" {
                if values.iter().any(|value| literal_has(value, ':')) {
                    return None;
                }
                return path_change(values.to_vec(), found);
            }
            let [value] = values else {
                return None;
            };
            found.assign(name, module_value(value)?, true);
            Some(())
        }
        "fish_add_path" => {
            let mut append = false;
            let mut dirs = Vec::new();
            for word in &words[1..] {
                match plain(word) {
                    Some("-a" | "--append") => append = true,
                    Some("-p" | "--prepend" | "-g" | "--global" | "-P" | "--path") => {}
                    Some(flag) if flag.starts_with('-') => return None,
                    _ => dirs.push(module_value(word)?),
                }
            }
            if dirs.is_empty() {
                return None;
            }
            if append {
                found.append.extend(dirs);
            } else {
                found.prepend.extend(dirs);
            }
            Some(())
        }
        "alias" => {
            let (name, body) = match &words[1..] {
                [name, body] => (plain(name)?.to_string(), body.clone()),
                [one] => {
                    let Some(Part::Lit(first, false)) = one.first() else {
                        return None;
                    };
                    let (name, rest) = first.split_once('=')?;
                    let mut body = Vec::new();
                    if !rest.is_empty() {
                        body.push(Part::Lit(rest.to_string(), false));
                    }
                    body.extend(one[1..].iter().cloned());
                    (name.to_string(), body)
                }
                _ => return None,
            };
            if name.starts_with('-') {
                return None;
            }
            found.aliases.insert(name, alias_body(&body)?);
            Some(())
        }
        "export" if words.len() == 2 => {
            let (name, value) = assignment(&words[1])?;
            if name == "PATH" {
                return path_change(split_colons(&value), found);
            }
            found.assign(&name, module_value(&value)?, true);
            Some(())
        }
        _ => None,
    }
}

fn literal_has(word: &Word, c: char) -> bool {
    word.iter()
        .any(|part| matches!(part, Part::Lit(text, _) if text.contains(c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structured(text: &str, dialect: Dialect) -> Option<Module> {
        let lines: Vec<&str> = text.lines().collect();
        Structured.recognize(&lines, dialect)
    }

    type Parts<'a> = (
        Vec<(&'a str, &'a str)>,
        &'a [String],
        &'a [String],
        Vec<(&'a str, &'a str)>,
    );

    fn parts(module: &Module) -> Parts<'_> {
        (
            module
                .env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            &module.path.prepend,
            &module.path.append,
            module
                .aliases
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
        )
    }

    #[test]
    fn posix_env_path_and_alias() {
        let module = structured(
            "# bun\nexport BUN_INSTALL=\"$HOME/.bun\"\nexport PATH=\"$BUN_INSTALL/bin:$PATH\"\nalias cc=\"claude\"\nalias ll='ls -l'  # long\n",
            Dialect::Posix,
        )
        .unwrap();
        assert_eq!(
            parts(&module),
            (
                vec![("BUN_INSTALL", "$HOME/.bun")],
                &["$BUN_INSTALL/bin".to_string()][..],
                &[][..],
                vec![("cc", "claude"), ("ll", "ls -l")]
            )
        );
    }

    #[test]
    fn assignment_then_export() {
        let module = structured("A=1; export A\nB=~/x\nexport B", Dialect::Posix).unwrap();
        assert_eq!(parts(&module).0, [("A", "1"), ("B", "~/x")]);
        assert!(structured("A=1", Dialect::Posix).is_none());
        assert!(structured("export A", Dialect::Posix).is_none());
    }

    #[test]
    fn path_positions() {
        let module = structured(
            "export PATH=/a:$PATH:/b\nPATH=\"$PATH:@HOME@/x y\"",
            Dialect::Posix,
        )
        .unwrap();
        assert_eq!(module.path.prepend, ["/a"]);
        assert_eq!(module.path.append, ["/b", "@HOME@/x y"]);
        assert!(structured("export PATH=/only", Dialect::Posix).is_none());
    }

    #[test]
    fn anything_unclear_is_a_snippet() {
        for text in [
            "export A=\"$(brew --prefix)\"",
            "export A=`date`",
            "export A=${B:-c}",
            "source ~/.orbstack/shell/init.zsh",
            "export FOO=1\nsource ~/.x",
            "[ -s \"$NVM_DIR/nvm.sh\" ] && \\. \"$NVM_DIR/nvm.sh\"",
            "export A='$HOME'",
            "export A=\"~/x\"",
            "alias x=\"cd $HOME\"",
            "export PATH=\"/a:$PATH\" \\",
            "typeset -U path",
            "alias alert='notify-send -i \"$([ $? = 0 ] && echo terminal)\"'",
            "alias x='a; b'",
            "eval \"$(/home/linuxbrew/.linuxbrew/bin/brew shellenv bash)\"",
        ] {
            assert!(structured(text, Dialect::Zsh).is_none(), "{text}");
        }
    }

    #[test]
    fn zsh_path_arrays() {
        let module = structured(
            "path=(\"$HOME/bin\" $path)\npath+=(/opt/x)\npath=($path /y)",
            Dialect::Zsh,
        )
        .unwrap();
        assert_eq!(module.path.prepend, ["$HOME/bin"]);
        assert_eq!(module.path.append, ["/opt/x", "/y"]);
        assert!(structured("path=(/only)", Dialect::Zsh).is_none());
    }

    #[test]
    fn deduplicating_case() {
        let text = "case \":${PATH}:\" in\n    *:\"$HOME/.cargo/bin\":*)\n        ;;\n    *)\n        # keep first\n        export PATH=\"$HOME/.cargo/bin:$PATH\"\n        ;;\nesac";
        let module = structured(text, Dialect::Posix).unwrap();
        assert_eq!(module.path.prepend, ["$HOME/.cargo/bin"]);
        let mismatch = text.replace("export PATH=\"$HOME/.cargo/bin:", "export PATH=\"/other:");
        assert!(structured(&mismatch, Dialect::Posix).is_none());
    }

    #[test]
    fn fish_rules() {
        let module = structured(
            "set -gx EDITOR nvim\nset -x GOPATH \"$HOME/go\"\nset -gx PATH /a $PATH\nfish_add_path -a ~/b\nalias ll 'ls -l'\nalias gs='git status'",
            Dialect::Fish,
        )
        .unwrap();
        assert_eq!(
            parts(&module),
            (
                vec![("EDITOR", "nvim"), ("GOPATH", "$HOME/go")],
                &["/a".to_string()][..],
                &["~/b".to_string()][..],
                vec![("ll", "ls -l"), ("gs", "git status")]
            )
        );
        for text in [
            "set -Ux A 1",
            "set -g A 1",
            "set -gx A 1 2",
            "eval (conda shell.fish hook) | source",
            "source \"$HOME/.cargo/env.fish\"",
        ] {
            assert!(structured(text, Dialect::Fish).is_none(), "{text}");
        }
    }

    #[test]
    fn variables_next_to_name_characters_get_braces() {
        let module = structured("export A=\"${B}_x\"", Dialect::Posix).unwrap();
        assert_eq!(module.env["A"], "${B}_x");
    }

    #[test]
    fn a_recognizer_ahead_of_structured_wins() {
        struct Everything;
        impl Recognizer for Everything {
            fn recognize(&self, _: &[&str], _: Dialect) -> Option<Module> {
                let mut module: Module = toml::from_str("").unwrap();
                module.env.insert("PROVIDER".into(), "1".into());
                Some(module)
            }
        }
        let chain: Vec<Box<dyn Recognizer>> = vec![Box::new(Everything), Box::new(Structured)];
        let module = recognize(&["eval \"$(zoxide init zsh)\""], Dialect::Zsh, &chain).unwrap();
        assert_eq!(module.env["PROVIDER"], "1");
        assert!(recognize(&["eval x"], Dialect::Zsh, &recognizers()).is_none());
    }
}
