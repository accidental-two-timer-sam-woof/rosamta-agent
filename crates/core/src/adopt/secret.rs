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
        .any(|whole| {
            let token = value(whole);
            // In quotes a token may be a literal value (`X="…"`, fish
            // `set X '…'`): it never passes as a name, and the part before
            // its `=` is judged too, unless it is a known name.
            let start = whole.as_ptr() as usize - text.as_ptr() as usize;
            let quoted = text[..start].ends_with(['"', '\'']);
            let assigned = whole[..whole.len() - token.len()].trim_end_matches(['=', '+']);
            let name = !quoted && assigned.is_empty() && known_name(token);
            high(token, name)
                || (quoted
                    && !assigned.is_empty()
                    && !known_name(assigned)
                    && high(assigned, false))
        })
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
        ] {
            assert!(!looks_secret(text), "{text}");
        }
    }
}
