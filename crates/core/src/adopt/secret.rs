//! Spot content that looks like a secret, so it never reaches the repo.

use std::sync::OnceLock;

use regex::Regex;

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

/// `NAME=literal` (POSIX), `NAME = "literal"` (TOML, key maybe quoted) or
/// `set ... NAME literal` (fish): a value with no expansion in it (an
/// escaped `\$` is none), which a
/// secret-looking name makes suspicious. With spaces around `=` only a
/// quoted value counts, so INI lines such as `signingkey = ABCD` do not.
fn literal_assignments() -> &'static [Regex; 2] {
    static RULES: OnceLock<[Regex; 2]> = OnceLock::new();
    RULES.get_or_init(|| {
        let quoted = r#"(?:'[^']+'|"(?:[^"$`\\]|\\.)+")"#;
        let value = format!(r#"(?:{quoted}|(?:[^\s'"$`\\;()]|\\.)+)"#);
        let start = r"(?m)^\s*";
        let end = r"\s*(?:;|#|$)";
        [
            Regex::new(&format!(
                r#"{start}(?:export\s+)?["']?([A-Za-z_][A-Za-z0-9_]*)["']?(?:={value}|\s*=\s*{quoted}){end}"#
            ))
            .unwrap(),
            Regex::new(&format!(
                r"{start}set\s+(?:-{{1,2}}[A-Za-z]+\s+)*([A-Za-z_][A-Za-z0-9_]*)\s+{value}{end}"
            ))
            .unwrap(),
        ]
    })
}

const SECRET_NAMES: [&str; 4] = ["KEY", "TOKEN", "SECRET", "PASSWORD"];
/// Shannon entropy above this, over at least `MIN_LEN` characters, is suspicious.
const MIN_ENTROPY: f64 = 4.0;
const MIN_LEN: usize = 20;
/// Paths hold `/` too, so a token with one must look more random: base64
/// keys do, words joined by `/` stay below.
const MIN_SLASHED_ENTROPY: f64 = 4.5;
const MIN_SLASHED_LEN: usize = 32;

/// Whether `text` holds what looks like a secret.
pub fn looks_secret(text: &str) -> bool {
    secret_rule(text).is_some()
}

/// The rule `text` matches: a gitleaks rule ID, `secret-name` or `high-entropy`.
pub fn secret_rule(text: &str) -> Option<&'static str> {
    known_format(text)
        .or_else(|| secret_name(text).then_some("secret-name"))
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
        literal_assignments().iter().any(|rule| {
            rule.captures_iter(text).any(|caps| {
                let name = caps[1].to_ascii_uppercase();
                SECRET_NAMES.iter().any(|word| name.contains(word))
            })
        })
    })
}

fn high_entropy(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "+/=_-.".contains(c)))
        .filter(|token| token.len() >= MIN_LEN && mixed(token))
        .any(|token| match token.contains('/') {
            false => entropy(token) > MIN_ENTROPY,
            true => {
                // `NAME=value` is judged by its value.
                let value = match token.split_once('=') {
                    Some((name, value))
                        if !name.is_empty()
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
                    {
                        value
                    }
                    _ => token,
                };
                !path_like(value)
                    && value.len() >= MIN_SLASHED_LEN
                    && entropy(value) > MIN_SLASHED_ENTROPY
            }
        })
}

/// Keys mix digits or letter cases; words and host names such as
/// `my-macbook-pro` use one case and no digits.
fn mixed(token: &str) -> bool {
    let has = |test: fn(&char) -> bool| token.chars().any(|c| test(&c));
    has(char::is_ascii_digit) || (has(char::is_ascii_uppercase) && has(char::is_ascii_lowercase))
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
        ] {
            assert!(!looks_secret(text), "{text}");
        }
    }
}
