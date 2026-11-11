//! Spot content that looks like a secret, so it never reaches the repo.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

include!(concat!(env!("OUT_DIR"), "/gitleaks.rs"));

/// Known token formats, from gitleaks (MIT; see THIRD_PARTY_NOTICES.md).
/// Each compiles on first use, and only runs when one of its keywords
/// appears: compiling all of them takes seconds. Returns the rule ID.
fn known_format(text: &str) -> Option<&'static str> {
    static RULES: OnceLock<Vec<OnceLock<Regex>>> = OnceLock::new();
    let rules = RULES.get_or_init(|| GITLEAKS.iter().map(|_| OnceLock::new()).collect());
    let lower = text.to_lowercase();
    GITLEAKS
        .iter()
        .zip(rules)
        .filter(|((_, _, keywords), _)| {
            keywords.is_empty() || keywords.iter().any(|keyword| lower.contains(keyword))
        })
        .find(|((_, pattern, _), rule)| {
            rule.get_or_init(|| {
                Regex::new(pattern).expect("build.rs kept only patterns that compile")
            })
            .is_match(text)
        })
        .map(|((id, _, _), _)| *id)
}

/// `NAME=literal` (POSIX), `NAME = "literal"` (TOML, key maybe quoted),
/// `set ... NAME literal` (fish) or `setenv NAME literal` (csh, launchctl):
/// a value with no expansion in it (an escaped `\$` is none), which a
/// secret-looking name makes suspicious. With spaces around `=` only a
/// quoted value counts, so INI lines such as `signingkey = ABCD` do not.
/// Each captures the name and the value.
fn literal_assignments() -> &'static [Regex; 2] {
    static RULES: OnceLock<[Regex; 2]> = OnceLock::new();
    RULES.get_or_init(|| {
        let (quoted, value) = (QUOTED, VALUE);
        let start = r"(?m)^\s*";
        let end = r"\s*(?:;|#|$)";
        [
            Regex::new(&format!(
                r#"{start}(?:export\s+)?["']?([A-Za-z_][A-Za-z0-9_]*)["']?(?:=({value})|\s*=\s*({quoted})){end}"#
            ))
            .unwrap(),
            Regex::new(&format!(
                r"{start}(?:set\s+(?:-{{1,2}}[A-Za-z]+\s+)*|(?:launchctl\s+)?setenv\s+)([A-Za-z_][A-Za-z0-9_]*)\s+({value}){end}"
            ))
            .unwrap(),
        ]
    })
}

/// A quoted literal (`$'...'` too), and a quoted or bare one: no expansion
/// inside. In double quotes a `$` expands only before a name, `{`, `(` or a
/// special parameter; `"Summer2024$"` is a literal.
const QUOTED: &str = r#"(?:\$?'[^']+'|"(?:[^"$`\\]|\\.|\$[^A-Za-z0-9_{(?!#@*$"\\`=^~+-])+\$?")"#;
const VALUE: &str = r#"(?:\$?'[^']+'|"(?:[^"$`\\]|\\.|\$[^A-Za-z0-9_{(?!#@*$"\\`=^~+-])+\$?"|(?:[^\s'"$`\\;()]|\\.)+)"#;

/// Every `NAME=literal` word in `command` with its value, as in
/// `export A=1 TOKEN=x` or `local -r KEY=x`.
fn assigned_words(command: &str) -> impl Iterator<Item = (&str, &str)> {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let word = WORD.get_or_init(|| {
        Regex::new(&format!(r"(?:^|\s)([A-Za-z_][A-Za-z0-9_]*)=({VALUE})")).unwrap()
    });
    word.captures_iter(command).filter_map(move |caps| {
        // A value that goes on, as in `KEY=a$b` or `KEY="a"$b`, expands.
        let rest = &command[caps.get(0)?.end()..];
        rest.chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == '#')
            .then(|| Some((caps.get(1)?.as_str(), caps.get(2)?.as_str())))
            .flatten()
    })
}

/// Words that make a name a secret's wherever they sit in it, and ones too
/// short or too common for that (`PAT` is in `PATH`, `OS_AUTH_URL` is no
/// secret), which count as the last `_` part.
const SECRET_NAMES: [&str; 8] = [
    "KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PASSPHRASE",
    "CREDENTIAL",
    "PWD",
];
const SECRET_ENDINGS: [&str; 6] = ["AUTH", "OAUTH", "PAT", "PASS", "DSN", "CREDS"];
/// Shannon entropy above this, over at least `MIN_LEN` characters, is suspicious.
const MIN_ENTROPY: f64 = 4.0;
const MIN_LEN: usize = 20;
/// Paths hold `/` too, so a token with one must look more random: base64
/// keys do, words joined by `/` stay below.
const MIN_SLASHED_ENTROPY: f64 = 4.5;
const MIN_SLASHED_LEN: usize = 32;
/// A literal this long that is not made of words could be a key no rule
/// knows; see [`unsure`].
const MIN_UNSURE_LEN: usize = 16;

/// `allow.toml` in the repo: what the secret check lets through, for a rule
/// that takes something harmless for a secret.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowed {
    /// Files or folders in the repo that are not checked at all.
    #[serde(default)]
    files: Vec<String>,
    /// [`Allowed::hash`] of single lines that are not secrets.
    #[serde(default)]
    lines: Vec<String>,
}

impl Allowed {
    pub const MANIFEST: &'static str = "allow.toml";

    /// The repo's list; empty when the file does not exist.
    pub fn load(repo_dir: &Path) -> Result<Self> {
        let path = repo_dir.join(Self::MANIFEST);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(Error::Read { path, source: err }),
        };
        crate::validate::parse(&text)
            .map_err(|issue| Error::Usage(format!("{}: {}", path.display(), issue.message)))
    }

    /// What names a line in `lines`: the start of the SHA-256 of the line
    /// without the white space around it.
    pub fn hash(line: &str) -> String {
        let digest = Sha256::digest(line.trim().as_bytes());
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Whether `file`, relative to the repo, is listed or inside a listed folder.
    pub fn file(&self, file: &str) -> bool {
        self.files.iter().any(|entry| {
            let entry = entry.trim_end_matches('/');
            file.strip_prefix(entry)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    }

    pub(crate) fn checked<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        if self.lines.is_empty() {
            return text.into();
        }
        let kept: Vec<&str> = text
            .lines()
            .filter(|line| !self.lines.contains(&Self::hash(line)))
            .collect();
        kept.join("\n").into()
    }

    /// [`secret_rule`] for the lines of `text` that are not listed.
    pub fn rule(&self, text: &str) -> Option<&'static str> {
        secret_rule(&self.checked(text))
    }

    /// [`unsure`] for the lines of `text` that are not listed.
    pub fn unsure(&self, text: &str) -> bool {
        unsure(&self.checked(text))
    }
}

/// Whether `text` holds what looks like a secret.
pub fn looks_secret(text: &str) -> bool {
    secret_rule(text).is_some()
}

/// The rule `text` matches: a gitleaks rule ID, `secret-name`,
/// `url-credentials` or `high-entropy`.
pub fn secret_rule(text: &str) -> Option<&'static str> {
    known_format(text)
        .or_else(|| secret_name(text).then_some("secret-name"))
        .or_else(|| url_credentials(text).then_some("url-credentials"))
        .or_else(|| high_entropy(text).then_some("high-entropy"))
}

fn secret_name(text: &str) -> bool {
    // A command after `;`, `&&` or `||` is checked as a line of its own too.
    let commands = text
        .lines()
        .flat_map(|line| line.split(';'))
        .flat_map(|part| part.split("&&"))
        .flat_map(|part| part.split("||"));
    std::iter::once(text).chain(commands).any(|text| {
        let leading = literal_assignments()
            .iter()
            .flat_map(|rule| rule.captures_iter(text))
            .filter_map(|caps| Some((caps.get(1)?, caps.get(2).or(caps.get(3))?)))
            .any(|(name, value)| {
                let upper = name.as_str().to_ascii_uppercase();
                !plain(name.as_str(), value.as_str())
                    && (SECRET_NAMES.iter().any(|word| upper.contains(word))
                        || ends_secret(name.as_str()))
            });
        // Later words are often shell locals such as `key_bindings` or
        // `VI_KEYMAP`: only an upper-case name with the word as a part counts.
        leading
            || !text.trim_start().starts_with('#')
                && assigned_words(text).any(|(name, value)| {
                    name.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                        && !plain(name, value)
                        && (name.split('_').any(|part| SECRET_NAMES.contains(&part))
                            || ends_secret(name))
                })
    })
}

/// Whether the last `_` part of `name` is a short secret word, in capitals:
/// `prev_pass` is a shell local.
fn ends_secret(name: &str) -> bool {
    name.rsplit('_')
        .next()
        .is_some_and(|part| SECRET_ENDINGS.contains(&part))
}

/// A value no secret has: a switch or a path whatever the name
/// (`TOKENIZERS_PARALLELISM=false`, `PASSWORD_STORE_DIR=~/.password-store`),
/// and a short number (`KEYTIMEOUT=1`), of four digits or more only when
/// the name does not end in the secret's word (`DB_PASSWORD=482913`).
fn plain(name: &str, value: &str) -> bool {
    let value = value.strip_prefix('$').unwrap_or(value);
    let value = value
        .strip_prefix(['\'', '"'])
        .and_then(|rest| rest.strip_suffix(['\'', '"']))
        .unwrap_or(value);
    let upper = name.to_ascii_uppercase();
    let last = upper.rsplit('_').next().unwrap_or_default();
    let names_it = SECRET_NAMES
        .iter()
        .chain(&SECRET_ENDINGS)
        .any(|word| last.ends_with(word));
    let number =
        value.len() <= if names_it { 3 } else { 6 } && value.bytes().all(|b| b.is_ascii_digit());
    let switch =
        ["true", "false", "yes", "no", "on", "off"].contains(&value.to_ascii_lowercase().as_str());
    // Base64 starts with `/` now and then.
    let path = value == "~"
        || ["~/", "./", "../"]
            .iter()
            .any(|start| value.starts_with(start))
        || value.starts_with('/') && !keyish(value);
    number || switch || path
}

/// `scheme://user:password@host` with a literal password: no expansion,
/// no pattern, and not the word a manual puts there.
fn url_credentials(text: &str) -> bool {
    const PLACEHOLDERS: [&str; 6] = ["pass", "password", "passwd", "pwd", "secret", "token"];
    static RULE: OnceLock<Regex> = OnceLock::new();
    RULE.get_or_init(|| {
        Regex::new(r"[A-Za-z][A-Za-z0-9+.-]*://[^\s/:@]*:([^\s/@$`{}<>()\[\]*|\\]{3,})@").unwrap()
    })
    .captures_iter(text)
    .any(|caps| !PLACEHOLDERS.contains(&caps[1].to_ascii_lowercase().as_str()))
}

/// Whether `text` holds a literal that no rule calls a secret but that
/// could be one: long and not made of words, such as a hex key under a
/// plain name. Adopting on its own would publish it, so the background
/// service asks instead.
pub fn unsure(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "+/=_-.".contains(c)))
        .map(value)
        .any(|token| {
            token.len() >= MIN_UNSURE_LEN
                && !known_name(token)
                && match token.contains('/') {
                    false => !token.split(['-', '_', '.', '=', '+']).all(wordish),
                    true => !path_like(token) && keyish(token),
                }
        })
}

/// Whether a token with a `/` could be base64: it has no `-`, `_` or `.`,
/// which a file or a repo would, and is not made of names.
fn keyish(token: &str) -> bool {
    token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
        && !token.split(['/', '=', '+']).all(named)
}

/// [`wordish`], or a word with a short number inside it (`powerlevel10k`,
/// `k8s`): what the parts of `user/repo` look like.
fn named(part: &str) -> bool {
    let digit = |c: char| c.is_ascii_digit();
    let (head, tail) = part.split_at(part.find(digit).unwrap_or(part.len()));
    let word = tail.trim_start_matches(digit);
    wordish(part) || (tail.len() - word.len() <= 4 && wordish(&format!("{head}{word}")))
}

/// A word, a number, or a word with a short number after it (`aarch64`,
/// `sha256`); capitals only as in `JetBrainsMono`.
fn wordish(part: &str) -> bool {
    let letters = part.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = part.len() - letters.len();
    let capitals = letters.bytes().filter(u8::is_ascii_uppercase).count();
    (letters.is_empty() || digits <= 4)
        && letters.bytes().all(|b| b.is_ascii_alphabetic())
        && (capitals == letters.len() || capitals * 3 <= letters.len())
}

fn high_entropy(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "+/=_-.".contains(c)))
        .any(|whole| {
            let token = value(whole);
            // In quotes a token may be a literal value (`X="…"`, fish
            // `set X '…'`): it never passes as a name, and the part before its
            // `=` is judged too unless it reads as a variable name, with `_`.
            let start = whole.as_ptr() as usize - text.as_ptr() as usize;
            let quoted = text[..start].ends_with(['"', '\'']);
            let assigned = whole[..whole.len() - token.len()].trim_end_matches(['=', '+']);
            let name =
                !quoted && assigned.is_empty() && known_name(token) && !fish_value(&text[..start]);
            high(token, name) || (quoted && !assigned.contains('_') && high(assigned, false))
        })
}

/// Whether a token after `before` is a value of fish `set -gx X …`, so
/// never a name.
fn fish_value(before: &str) -> bool {
    static RULE: OnceLock<Regex> = OnceLock::new();
    let command = before
        .rsplit(['\n', ';', '&', '|'])
        .next()
        .unwrap_or_default();
    RULE.get_or_init(|| {
        Regex::new(r"^\s*set\s+(?:-{1,2}[A-Za-z]+\s+)*[A-Za-z_][A-Za-z0-9_]*\s").unwrap()
    })
    .is_match(command)
}

fn high(token: &str, name: bool) -> bool {
    match token.contains('/') {
        false => token.len() >= MIN_LEN && !words(token) && !name && entropy(token) > MIN_ENTROPY,
        true => {
            !path_like(token)
                && token.len() >= MIN_SLASHED_LEN
                && entropy(token) > MIN_SLASHED_ENTROPY
        }
    }
}

/// `NAME=value` is judged by the value, and so is `NAME+=value` for a known
/// name; elsewhere `+=` may sit inside a key. Base64 padding (`abc==`) is
/// not an assignment.
fn value(token: &str) -> &str {
    let Some((name, value)) = token.split_once('=') else {
        return token;
    };
    let name = match name.strip_suffix('+') {
        Some(name) if known_name(name) => name,
        Some(_) => return token,
        None => name,
    };
    let assignment = !name.is_empty()
        && !value.is_empty()
        && !value.starts_with('=')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if assignment { value } else { token }
}

/// Words joined by `-` or `.` in one case, such as `my-macbook-pro` or
/// `internal-tools.example`: a name, not a key.
fn words(token: &str) -> bool {
    token.contains(['-', '.'])
        && token
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '.')
        && !(token.chars().any(|c| c.is_ascii_uppercase())
            && token.chars().any(|c| c.is_ascii_lowercase()))
}

/// Variable names of tools whose settings files are long lists of them, such
/// as Powerlevel10k's `~/.p10k.zsh`.
const KNOWN_PREFIXES: [&str; 2] = ["POWERLEVEL9K_", "P9K_"];

/// A known tool's variable, such as `POWERLEVEL9K_DIR_FOREGROUND`, also as
/// `NAME=`, `NAME+=` or at the end of a sentence.
fn known_name(token: &str) -> bool {
    let token = token.trim_end_matches(['=', '+', '.']);
    KNOWN_PREFIXES
        .iter()
        .any(|prefix| token.starts_with(prefix))
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// `/usr/bin`, `~/.cargo`, `HOME/.local/share` (what `$HOME/...` leaves):
/// a path, whatever its letters.
fn path_like(token: &str) -> bool {
    let first = token.split('/').next().unwrap_or_default();
    token.starts_with(['/', '~', '.'])
        || first
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        || token.split('/').any(|part| part.starts_with('.'))
}

fn entropy(token: &str) -> f64 {
    let mut counts = [0usize; 256];
    for byte in token.bytes() {
        counts[usize::from(byte)] += 1;
    }
    let len = token.len() as f64;
    counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_token_formats() {
        assert!(looks_secret(concat!(
            "export OPENAI_API_KEY=sk-proj-abcdefghijABCDEFGHIJ1234567890abcdefghij",
            "T3Blbk",
            "FJabcdefghijABCDEFGHIJ1234567890"
        )));
        assert!(looks_secret(
            "export GH=ghp_0123456789abcdefghijABCDEFGHIJ012345"
        ));
    }

    #[test]
    fn listed_lines_and_files_pass() {
        let key = "export GH=ghp_0123456789abcdefghijABCDEFGHIJ012345";
        let text = format!("export A=1\n  {key}\n");
        let allowed: Allowed = toml::from_str(&format!(
            "lines = [{:?}]\nfiles = [\"home/.p10k.zsh\", \"home/.config/nvim/\"]\n",
            Allowed::hash(key)
        ))
        .unwrap();
        assert_eq!(Allowed::default().rule(&text), Some("github-pat"));
        assert_eq!(allowed.rule(&text), None);
        // Another line with the same key is still found.
        assert_eq!(
            allowed.rule(&format!("{text}{key} # copy\n")),
            Some("github-pat")
        );
        assert!(allowed.file("home/.p10k.zsh") && allowed.file("home/.config/nvim/init.lua"));
        assert!(!allowed.file("home/.p10k.zsh.bak") && !allowed.file("home/.config/nvimrc"));
        assert!(toml::from_str::<Allowed>("line = []").is_err());
    }

    /// Lines are listed by content: adopting moves a line into a module.
    #[test]
    fn a_listed_line_passes_wherever_it_appears() {
        let key = "export GH=ghp_0123456789abcdefghijABCDEFGHIJ012345";
        let allowed: Allowed =
            toml::from_str(&format!("lines = [{:?}]\n", Allowed::hash(key))).unwrap();
        let moved =
            format!("# Adopted from ~/.zshrc on mbp.\n[raw]\nzsh = '''\n{key}\nexport B=2\n'''\n");
        assert_eq!(Allowed::default().rule(&moved), Some("github-pat"));
        assert_eq!(allowed.rule(&moved), None);
    }

    #[test]
    fn rules_are_named() {
        assert_eq!(
            secret_rule("GH = \"ghp_0123456789abcdefghijABCDEFGHIJ012345\""),
            Some("github-pat")
        );
        assert_eq!(secret_rule("export MY_API_KEY=abc123"), Some("secret-name"));
        assert_eq!(secret_rule("ll = \"ls -lh\""), None);
    }

    #[test]
    fn names_with_literal_values() {
        assert!(looks_secret("export MY_API_KEY=abc123"));
        assert!(looks_secret("export DB_PASSWORD='hunter2'"));
        assert!(looks_secret("set -gx SERVICE_TOKEN abc"));
        assert!(looks_secret("cd ~; export API_TOKEN=abc123"));
        assert!(looks_secret("export A=1; export API_TOKEN=abc123"));
        assert!(looks_secret("true && set -gx SERVICE_TOKEN abc"));
        assert!(looks_secret(r#"export DB_PASSWORD="S3cure\$Pass!2024""#));
        assert!(!looks_secret(
            "export GITHUB_TOKEN=\"$(cat ~/.config/gh/token)\""
        ));
        assert!(!looks_secret("export SSH_KEY_PATH=$HOME/.ssh/id"));
        assert!(!looks_secret("export TOKEN_FILE=`cat x`"));
        assert!(looks_secret("export A=1 API_TOKEN=abc123"));
        assert!(looks_secret("declare -x DB_PASSWORD=hunter2"));
        assert!(looks_secret("local -r SIGNING_KEY='abc'"));
        assert!(looks_secret("env DEPLOY_TOKEN=abc ./deploy"));
        assert!(looks_secret("export GPG_PASSPHRASE=correct-horse"));
        assert!(!looks_secret("export A=1 SSH_KEY_PATH=$HOME/.ssh/id"));
        assert!(!looks_secret("export A=1 API_TOKEN=abc$SUFFIX"));
        assert!(!looks_secret("export A=1 API_TOKEN=\"abc\"$(cat x)"));
        assert!(!looks_secret("typeset -g VI_KEYMAP=main"));
        assert!(!looks_secret(
            "local key_bindings=\"/usr/share/fzf/key-bindings.zsh\""
        ));
        assert!(!looks_secret("# parse valid KEY=VALUE pairs"));
    }

    #[test]
    fn more_names_and_ways_to_assign() {
        let hex = "0123456789abcdef0123456789abcdef01234567";
        for text in [
            "export MYSQL_PWD=hunter2".to_string(),
            format!("export GITHUB_PAT={hex}"),
            format!("export NPM_AUTH={hex}"),
            "export SERVICE_CREDENTIALS=correct-horse-battery-staple".to_string(),
            format!("export SENTRY_DSN=https://{hex}@o1.ingest.sentry.io/1"),
            "export API_KEY=$'abc123'".to_string(),
            "launchctl setenv API_KEY abc123".to_string(),
            "setenv API_KEY abc123".to_string(),
        ] {
            assert_eq!(secret_rule(&text), Some("secret-name"), "{text}");
        }
    }

    #[test]
    fn plain_values_and_look_alike_names_are_not_secrets() {
        for text in [
            "export KEYTIMEOUT=1",
            "export KEY_REPEAT=2",
            "export TOKENIZERS_PARALLELISM=false",
            "export PASSWORD_STORE_DIR=~/.password-store",
            "export SSH_AUTH_SOCK=/Users/me/.ssh/agent.sock",
            "export GIT_ASKPASS=/usr/bin/ksshaskpass",
            "export GIT_AUTHOR_NAME=\"Me\"",
            "export PATH=/usr/local/bin",
            "export OS_AUTH_URL=https://keystone.example.com:5000/v3",
            "export FOO=1 KEY_REPEAT=2",
            "prev_pass=cur_pass",
        ] {
            assert!(!looks_secret(text), "{text}");
        }
    }

    /// What the name rule lets pass by its value is still held when the value
    /// is the secret: a PIN, base64 that starts with `/`, a `$` that stays.
    #[test]
    fn values_that_only_look_plain() {
        for text in [
            "export DB_PASSWORD=482913",
            "export MYSQL_PWD=0042",
            "export AWS_SECRET_ACCESS_KEY=/k7FAKEfake+bPxRfiCYFAKEKEYwJalrXUtnFAKE",
            "export DB_PASSWORD=\"Summer2024$\"",
            "export DB_PASSWORD=\"pa$%word1\"",
        ] {
            assert_eq!(secret_rule(text), Some("secret-name"), "{text}");
        }
        for text in [
            "export KEYTIMEOUT=1000",
            "export PASSWORD_STORE_CLIP_TIME=45",
            "export DB_PASSWORD=1",
            "export GIT_ASKPASS=/usr/bin/ksshaskpass",
            "export SSH_KEY=/Users/me/Keys/MacBookPro",
            "export API_TOKEN=\"$OTHER_TOKEN\"",
            "export API_TOKEN=\"prefix-${USER}\"",
            "export API_TOKEN=\"$(pass show api)\"",
        ] {
            assert!(!looks_secret(text), "{text}");
        }
    }

    #[test]
    fn passwords_inside_urls() {
        for text in [
            "export DATABASE_URL=postgres://admin:S3cretPass@db.internal:5432/prod",
            "export REDIS_URL=redis://:p4ssw0rd@redis.example.com:6379/0",
            "\turl = https://me:hunter2@git.example.com/org/repo",
        ] {
            assert_eq!(secret_rule(text), Some("url-credentials"), "{text}");
        }
        for text in [
            "export DATABASE_URL=\"postgres://admin:$DB_PASS@db.internal/prod\"",
            "url = https://me:${TOKEN}@git.example.com/org/repo",
            "git clone ssh://git@github.com/org/repo",
            "curl http://localhost:8080/a@b",
            "proxy = http://<user>:<password>@proxy.example:3128",
            "# proxy = http://user:password@proxy.example:3128",
            "[[ \"${URL}\" =~ https://([[:alnum:]_:]+)@github.com/(.*)$ ]]",
        ] {
            assert!(!looks_secret(text), "{text}");
        }
    }

    /// The rate each rule set finds random tokens under a plain name, so a
    /// change to the rules shows here. Hex and short tokens are what the
    /// entropy rule cannot see; `unsure` is what keeps them from being
    /// published without a look.
    #[test]
    fn random_tokens_are_found_or_asked_about() {
        struct Rng(u64);
        impl Rng {
            fn pick(&mut self, alphabet: &[u8], n: usize) -> String {
                (0..n)
                    .map(|_| {
                        self.0 ^= self.0 << 13;
                        self.0 ^= self.0 >> 7;
                        self.0 ^= self.0 << 17;
                        alphabet[(self.0 % alphabet.len() as u64) as usize] as char
                    })
                    .collect()
            }
        }
        const HEX: &[u8] = b"0123456789abcdef";
        const LOWER36: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        const B62: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        // (alphabet, length, least share the rules call a secret, least share
        // that is a secret or asked about), in percent.
        for (alphabet, len, secret, held) in [
            (HEX, 32, 0, 100),
            (HEX, 40, 0, 100),
            (LOWER36, 20, 15, 99),
            (B62, 20, 60, 99),
            (B62, 24, 90, 100),
            (B62, 32, 100, 100),
        ] {
            let mut rng = Rng(0x9E3779B97F4A7C15 ^ (len as u64) << 8 ^ alphabet.len() as u64);
            let (mut found, mut kept) = (0, 0);
            const N: usize = 1000;
            for _ in 0..N {
                let line = format!("export SERVICE_ID={}", rng.pick(alphabet, len));
                let is_secret = looks_secret(&line);
                found += usize::from(is_secret);
                kept += usize::from(is_secret || unsure(&line));
            }
            let size = alphabet.len();
            assert!(found * 100 >= secret * N, "{size}/{len}: found {found}");
            assert!(kept * 100 >= held * N, "{size}/{len}: kept {kept}");
        }
    }

    #[test]
    fn long_literals_that_are_not_words_are_asked_about() {
        for text in [
            "export SERVICE_ID=0123456789abcdef0123456789abcdef",
            "export GIT_COMMIT=0123456789abcdef0123456789abcdef01234567",
            "export UUID=123e4567-e89b-12d3-a456-426614174000",
            "tool --id a1b2c3a1b2c3a1b2c3a1",
        ] {
            assert!(unsure(text) && !looks_secret(text), "{text}");
        }
        for text in [
            "export EDITOR=nvim",
            "export TERM=xterm-256color",
            "export LANG=en_US.UTF-8",
            "export CARGO_BUILD_TARGET=aarch64-apple-darwin",
            "export RUSTUP_TARGET=x86_64-unknown-linux-musl",
            "export BAT_THEME=\"Catppuccin-mocha\"",
            "export HOSTNAME=Xinshous-MacBook-Pro",
            "font-family = JetBrainsMonoNerdFontMono-Regular",
            "theme = \"catppuccin-mocha-lavender-compact\"",
            "plugins=(git zsh-autosuggestions zsh-syntax-highlighting)",
            "export HOMEBREW_NO_INSTALL_CLEANUP_FORMULAE=1",
            "unset ZSH_AUTOSUGGEST_HIGHLIGHT_STYLE",
            "typeset -g POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_BACKGROUND=",
            "  - \"NODE_OPTIONS=--max-old-space-size=4096\"",
            "export FZF_DEFAULT_OPTS='--height 40% --layout=reverse --border'",
            "eval \"$(/home/linuxbrew/.linuxbrew/bin/brew shellenv bash)\"",
            "source ~/.orbstack/shell/init.zsh 2>/dev/null || :",
            "zinit light romkatv/powerlevel10k/powerlevel10k.zsh-theme",
            "ssh-add ~/.ssh/id_ed25519_github_personal_2024",
            "export GOPRIVATE=github.com/OrgName/RepoName",
            "docker run -e \"DATABASE_HOST=postgres.internal.example.com\" app",
            "# Adopted from ~/.zshrc on xinshoutw-macbook-pro.",
        ] {
            assert!(!unsure(text), "{text}");
        }
    }

    /// Base64 may hold `/`. Below the length the entropy rule wants, such a
    /// value was neither found nor asked about.
    #[test]
    fn base64_with_a_slash_is_asked_about() {
        for text in [
            "export BLOB=FAKEfake0123FAKE/fake4567FAKEfake",
            "export PIN=sha256/FAKEfake0123FAKEfake4567=",
        ] {
            assert!(unsure(text) && !looks_secret(text), "{text}");
        }
        for text in [
            "ZSH_THEME=\"powerlevel10k/powerlevel10k\"",
            "zinit light romkatv/powerlevel10k",
            "zinit snippet OMZP::kubectl/k8s/completions",
            "export PATH=/Library/TeX/texbin:/Applications/Ghostty.app/Contents/MacOS:$PATH",
            "source ~/.oh-my-zsh/custom/plugins/zsh-autosuggestions/zsh-autosuggestions.zsh",
            "export TZ=America/Argentina/Buenos_Aires",
            "antigen bundle MichaelAquilina/zsh-you-should-use",
            "export ACCEPT=application/octet/stream",
        ] {
            assert!(!unsure(text), "{text}");
        }
        const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut state = 0x9E3779B97F4A7C15u64;
        let (mut slashed, mut kept) = (0, 0);
        while slashed < 1000 {
            let token: String = (0..24)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    B64[(state % 64) as usize] as char
                })
                .collect();
            if token.contains('/') {
                slashed += 1;
                kept += usize::from(unsure(&format!("export SERVICE_ID={token}")));
            }
        }
        // The rest starts like a path (`/…`, `AB/…`).
        assert!(kept >= 800, "kept {kept}");
    }

    /// The pre-push scan reads module TOML, where `=` has spaces around it.
    #[test]
    fn toml_assignments() {
        for text in [
            "MY_API_KEY = \"abc123\"",
            "DB_PASSWORD = 'hunter2'",
            "\"SERVICE_SECRET\" = \"abc123\"",
            // Hex never passes the entropy rule, so the name has to catch it.
            "DEPLOY_KEY = \"0123456789abcdef0123456789abcdef\"",
        ] {
            assert_eq!(secret_rule(text), Some("secret-name"), "{text}");
        }
        for text in [
            "SSH_KEY_PATH = \"$HOME/.ssh/id\"",
            "API_KEY_REQUIRED = true",
            "\tsigningkey = ABCDEF0123456789",
            "keybind = ctrl+a=new_tab",
        ] {
            assert!(!looks_secret(text), "{text}");
        }
    }

    #[test]
    fn high_entropy_literals() {
        assert!(looks_secret("export X=Zk3q9LmP2vR8tY6wB1nC4xJ7"));
        assert!(looks_secret("export credential=qwertyuiopasdfghjklzxcvbnm"));
        assert!(looks_secret("Zk3q9LmP2vR8tY6wB1nC4xJ7=="));
        assert!(looks_secret("export X=K7Q2M9X4P8R3T6W1Z5N0"));
        assert!(looks_secret("export X=\"Zk3q9LmP2vR8tY6wB1A=\""));
        assert!(looks_secret("export X=qwertyuiopasdfghjkl_zxcvbnm"));
        assert!(looks_secret("export X=\"qwertyuiopasdfghjkl_zxcvbnm\""));
        assert!(looks_secret(
            "kubeadm join 10.0.0.5:6443 --token q8xkzm.vjwpbhrtcfnlsydg"
        ));
        assert!(looks_secret(
            "export NR_USER=NRAK-QW8ZRTYUPLKJHGFDSAXCVBN4M"
        ));
        assert!(looks_secret("export LICENSE=QW8ZRTYUPLKJ_HGFDSAXCVBNMKJH"));
        assert!(looks_secret("export X=K7Q2_M9X4_P8R3_T6W1_Z5N0"));
        assert!(looks_secret(
            "export X=POWERLEVEL9K_QWERTYUIOPASDFGHJZXCVBNM"
        ));
        assert!(looks_secret("X+=Zk3q9LmP2vR8tY6wB1nC4xJ7"));
        assert!(looks_secret("X+=347f23b364db67195b295cfa0e7abc28"));
        assert!(looks_secret("1DLDn0L8D1aQzEqb61EKBo3+=x"));
        assert!(looks_secret("export X=\"K7Q2M9X4P8R3T6W1Z5N0+=abc\""));
        assert!(looks_secret("export X='K7Q2M9X4P8R3T6W1Z5N0=abc'"));
        assert!(looks_secret(
            "export X=\"POWERLEVEL9K_QWERTYUIOPASDFGHJZXCVBNM\""
        ));
        assert!(looks_secret("X = \"P9K_QWERTYUIOPASDFGHJZXCVBNM\""));
        assert!(looks_secret("set -gx X 'K7Q2M9X4P8R3T6W1Z5N0=abc'"));
        assert!(looks_secret("set -gx X P9K_K7Q2_M9X4_P8R3_T6W1_Z5N0"));
        assert!(looks_secret(
            "cd ~; set -gx X a P9K_K7Q2_M9X4_P8R3_T6W1_Z5N0"
        ));
        assert!(looks_secret(&format!(
            "export X=\"{}=Zk3q9LmP2vR8tY6wB1nC4xJ7\"",
            "A".repeat(64)
        )));
        assert!(looks_secret(
            "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        ));
        assert!(!looks_secret(
            "export PATH=\"$HOME/.local/share/pnpm:$PATH\""
        ));
        assert!(!looks_secret(
            "__conda_setup=\"$('/home/me/miniforge3/bin/conda' 'shell.bash' 'hook' 2> /dev/null)\""
        ));
    }

    #[test]
    fn ordinary_installer_lines() {
        for text in [
            "export BUN_INSTALL=\"$HOME/.bun\"",
            "eval \"$(/home/linuxbrew/.linuxbrew/bin/brew shellenv bash)\"",
            "export NVM_DIR=\"$HOME/.nvm\"",
            "source ~/.orbstack/shell/init.zsh 2>/dev/null || :",
            "export PROJECT=~/Code/TheProjectName/SubModule/Thing",
            "zinit light romkatv/powerlevel10k/powerlevel10k.zsh-theme",
            "export GOPRIVATE=github.com/OrgName/RepoName",
            "export ANDROID_HOME=$HOME/Library/Android/sdk",
            "# Adopted from ~/.zshrc on xinshoutw-macbook-pro.",
            "Host my-company-internal-tools.example",
            "export HOST=my-company-internal-tools.example",
            "  # as POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_FOREGROUND below.",
            "  typeset -g POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_BACKGROUND=",
            "  typeset -g POWERLEVEL9K_VCS_BRANCH_ICON='\\uF126 '",
            "  # typeset -g POWERLEVEL9K_CPU_ARCH_X86_64_CONTENT_EXPANSION=",
            "  # POWERLEVEL9K_ASDF_PROMPT_ALWAYS_SHOW.",
            "  typeset -g POWERLEVEL9K_XPLR_VISUAL_IDENTIFIER_EXPANSION=",
            "  POWERLEVEL9K_CONTEXT_TEMPLATE+=foo",
            "  POWERLEVEL9K_KUBECONTEXT_DEFAULT_CONTENT_EXPANSION+='${P9K_KUBECONTEXT_NAME}'",
            "env 'POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_BACKGROUND=red' zsh",
            "alias -g RED='RAILS_ENV=development'",
            "echo \"RUSTFLAGS=-Zfmt-debug=none\" >> $GITHUB_ENV",
            "docker run -e \"PYENV_VIRTUALENV_DISABLE_PROMPT=1\" app",
            "export \"DOCKER_DEFAULT_PLATFORM=linux/amd64\"",
            "Environment=\"KUBECONFIG=/etc/kubernetes/admin.conf\"",
            "  - \"NODE_OPTIONS=--max-old-space-size=4096\"",
            "docker run -e \"DATABASE_HOST=postgres.internal.example.com\" app",
            "export \"QTWEBENGINE_CHROMIUM_FLAGS=--disable-gpu\"",
        ] {
            assert!(!looks_secret(text), "{text}");
        }
    }
}
