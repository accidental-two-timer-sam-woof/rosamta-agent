//! Templates: dotfiles rendered for each host with minijinja, in a sandbox
//! that cannot read files or run anything but the secret sources rosamta
//! provides (ADR 0006).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use minijinja::{Environment, ErrorKind, UndefinedBehavior, Value, context};
use regex::Regex;

use crate::config::HostConfig;
use crate::secrets::{self, Failure, Session, Waiting};

/// Instructions one render may run: far more than a dotfile needs, and a
/// bound on a template that would loop for hours.
const FUEL: u64 = 1_000_000;

/// No loader, so `include`, `import` and `extends` find nothing; undefined
/// names are errors rather than empty text.
fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_fuel(Some(FUEL));
    // A dotfile ends with a newline like any text file.
    env.set_syntax(
        minijinja::syntax::SyntaxConfig::builder()
            .keep_trailing_newline(true)
            .build()
            .expect("the default delimiters are valid"),
    );
    env
}

/// A syntax error with its line, or `Ok`; `name` appears in the message.
pub fn check(name: &str, source: &str) -> Result<(), String> {
    environment()
        .template_from_named_str(name, source)
        .map(drop)
        .map_err(|err| describe(&err))
}

/// The names the template reads, as `vars.email`, or `vars` alone when it
/// reads `vars` as a whole; functions rosamta provides appear by name.
/// Static: a name inside an `if` counts even when that branch never runs.
pub fn names(name: &str, source: &str) -> Result<HashSet<String>, String> {
    let env = environment();
    let template = env
        .template_from_named_str(name, source)
        .map_err(|err| describe(&err))?;
    Ok(template.undeclared_variables(true))
}

/// Whether the template asks a secret source.
pub fn uses_secrets(names: &HashSet<String>) -> bool {
    names.contains("secret") || names.contains("bitwarden")
}

/// The command names `secret()` is called with; `None` when `secret`
/// appears other than as a call with a quoted name, so any command may run.
pub fn secret_names(source: &str) -> Option<BTreeSet<String>> {
    static CALL: OnceLock<Regex> = OnceLock::new();
    let call = CALL.get_or_init(|| {
        Regex::new(r#"\bsecret\b(?:\s*\(\s*(?:"([^"\\]*)"|'([^'\\]*)'))?"#).unwrap()
    });
    call.captures_iter(source)
        .map(|caps| Some(caps.get(1).or(caps.get(2))?.as_str().to_string()))
        .collect()
}

/// `vars.*` names the template uses, sorted.
pub fn used_vars(name: &str, source: &str) -> Result<Vec<String>, String> {
    let mut used: Vec<String> = names(name, source)?
        .iter()
        .filter_map(|used| used.strip_prefix("vars.")?.split('.').next())
        .map(str::to_string)
        .collect();
    used.sort();
    used.dedup();
    Ok(used)
}

/// The used `vars.*` names that `vars` lacks, sorted.
pub fn missing_vars(
    name: &str,
    source: &str,
    vars: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let mut used = used_vars(name, source)?;
    used.retain(|var| !vars.contains_key(var));
    Ok(used)
}

/// A secret that is waiting fails the render too; `secrets.take_waiting()`
/// tells it apart from an error.
pub fn render(
    name: &str,
    source: &str,
    host: &HostConfig,
    home: &Path,
    secrets: &Arc<Session>,
) -> Result<String, String> {
    let mut env = environment();
    add_functions(&mut env, secrets, home);
    let template = env
        .template_from_named_str(name, source)
        .map_err(|err| describe(&err))?;
    let host_vars: BTreeMap<&str, Value> = BTreeMap::from([
        ("id", Value::from(host.id.as_str())),
        ("os", Value::from(std::env::consts::OS)),
        ("arch", Value::from(std::env::consts::ARCH)),
        ("tags", Value::from(host.tags.clone())),
        ("home", Value::from(home.display().to_string())),
    ]);
    template
        .render(context! { host => host_vars, vars => host.vars.clone() })
        .map_err(|err| describe(&err))
}

fn add_functions(env: &mut Environment<'static>, secrets: &Arc<Session>, home: &Path) {
    let session = Arc::clone(secrets);
    env.add_function("bitwarden", move |item: String, field: Option<String>| {
        session.bitwarden(&item, field.as_deref()).map_err(error)
    });
    let session = Arc::clone(secrets);
    env.add_function("secret", move |name: String, arg: String| {
        session.secret(&name, &arg).map_err(error)
    });
    let socket = secrets::bitwarden_ssh_socket(home)
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    env.add_function("bitwarden_ssh_socket", move || socket.clone());
}

fn error(failure: Failure) -> minijinja::Error {
    let text = match failure {
        Failure::Error(text) => text,
        Failure::Waiting(Waiting::Locked(tool)) => format!("{} is locked", tool.name()),
        Failure::Waiting(Waiting::Command(name)) => {
            format!("secret command `{name}` runs only in `rsmt apply` in a terminal")
        }
    };
    minijinja::Error::new(ErrorKind::InvalidOperation, text)
}

/// minijinja's message with the line it found, and the cause behind it.
fn describe(err: &minijinja::Error) -> String {
    let mut text = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Arc<Session> {
        Arc::new(Session::with(
            false,
            &[],
            Default::default(),
            String::new(),
            None,
        ))
    }

    fn host(tags: &[&str], vars: &[(&str, &str)]) -> HostConfig {
        let mut host = HostConfig::new("mbp".into());
        host.tags = tags.iter().map(|tag| tag.to_string()).collect();
        host.vars = vars
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        host
    }

    #[test]
    fn host_values_choose_the_content() {
        let text = r#"{% if "work" in host.tags %}me@work.com{% else %}me@home.com{% endif %}"#;
        let home = Path::new("/home/me");
        assert_eq!(
            render("t", text, &host(&["work"], &[]), home, &none()).unwrap(),
            "me@work.com"
        );
        assert_eq!(
            render("t", text, &host(&[], &[]), home, &none()).unwrap(),
            "me@home.com"
        );
        let text = "{{ host.id }} {{ host.home }} {{ vars.email }}";
        assert_eq!(
            render("t", text, &host(&[], &[("email", "a@b.c")]), home, &none()).unwrap(),
            "mbp /home/me a@b.c"
        );
        let os = render(
            "t",
            "{{ host.os }}/{{ host.arch }}",
            &host(&[], &[]),
            home,
            &none(),
        )
        .unwrap();
        assert!(
            [
                "macos/aarch64",
                "macos/x86_64",
                "linux/aarch64",
                "linux/x86_64"
            ]
            .contains(&os.as_str())
        );
    }

    #[test]
    fn undefined_names_fail_with_their_line() {
        let err = render(
            "home/.gitconfig",
            "a\n{{ host.ids }}\n",
            &host(&[], &[]),
            Path::new("/h"),
            &none(),
        )
        .unwrap_err();
        assert!(err.contains("home/.gitconfig:2"), "{err}");
    }

    #[test]
    fn templates_cannot_reach_files() {
        for text in [
            r#"{% include "other" %}"#,
            r#"{% extends "base" %}"#,
            r#"{% import "m" as m %}"#,
        ] {
            assert!(
                render("t", text, &host(&[], &[]), Path::new("/h"), &none()).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn endless_templates_stop() {
        let text =
            "{% for a in range(100000) %}{% for b in range(100000) %}{% endfor %}{% endfor %}";
        assert!(render("t", text, &host(&[], &[]), Path::new("/h"), &none()).is_err());
    }

    #[test]
    fn syntax_errors_name_the_line() {
        let err = check("home/.npmrc", "ok\n{% if %}\n").unwrap_err();
        assert!(err.contains("home/.npmrc:2"), "{err}");
        assert!(check("t", "{{ vars.x }}").is_ok());
    }

    #[test]
    fn names_show_secrets_and_whole_vars() {
        let names = names("t", r#"{{ secret ("op", "x") }}{{ vars["a"] }}"#).unwrap();
        assert!(uses_secrets(&names), "{names:?}");
        assert!(names.contains("vars"), "{names:?}");
        let names = names_of(r#"{{ bitwarden_ssh_socket() }}{{ vars.a }}"#);
        assert!(!uses_secrets(&names), "{names:?}");
        assert!(!names.contains("vars"), "{names:?}");
        assert!(uses_secrets(&names_of(r#"{{ bitwarden("x") }}"#)));
    }

    fn names_of(text: &str) -> HashSet<String> {
        names("t", text).unwrap()
    }

    #[test]
    fn missing_vars_are_the_unset_ones() {
        let text = "{{ vars.email }}{% if vars.signing %}{{ vars.key.id }}{% endif %}{{ host.id }}";
        let vars: BTreeMap<String, String> = [("email".to_string(), String::new())].into();
        assert_eq!(missing_vars("t", text, &vars).unwrap(), ["key", "signing"]);
    }

    #[test]
    fn secrets_come_only_from_rosamta_functions() {
        let host = host(&[], &[]);
        let home = Path::new("/h");
        let err = render("t", r#"{{ secret("vault", "x") }}"#, &host, home, &none()).unwrap_err();
        assert!(err.contains("`vault` is not declared"), "{err}");
        let err = render("t", r#"{{ bitwarden("GitHub") }}"#, &host, home, &none()).unwrap_err();
        assert!(err.contains("no Bitwarden tool"), "{err}");
        let text = r#"{% if bitwarden_ssh_socket() %}set{% endif %}"#;
        assert!(render("t", text, &host, home, &none()).is_ok());
    }

    #[test]
    fn a_command_outside_a_terminal_waits() {
        let commands: secrets::SecretCommands =
            toml::from_str("[commands.op]\nrun = [\"op\", \"read\", \"{arg}\"]\n").unwrap();
        let session = Arc::new(Session::with(false, &[], commands, String::new(), None));
        let text = r#"{{ secret("op", "op://a/b") }}"#;
        let err = render("t", text, &host(&[], &[]), Path::new("/h"), &session).unwrap_err();
        assert!(err.contains("runs only in `rsmt apply`"), "{err}");
        assert_eq!(session.take_waiting(), [Waiting::Command("op".into())]);
        assert!(session.take_waiting().is_empty());
    }

    #[test]
    fn secret_names_are_the_quoted_first_arguments() {
        let names =
            |source: &str| secret_names(source).map(|names| names.into_iter().collect::<Vec<_>>());
        assert_eq!(
            names("a={{ secret(\"op\", \"x\") }}\nb={{ secret( 'vault' ,'y') }}\n"),
            Some(vec!["op".to_string(), "vault".to_string()])
        );
        assert_eq!(names("{{ bitwarden_ssh_socket() }}"), Some(vec![]));
        assert_eq!(names("{{ secret(name, \"x\") }}"), None);
        assert_eq!(
            names("{% set get = secret %}{{ get(\"op\", \"x\") }}"),
            None
        );
    }
}
