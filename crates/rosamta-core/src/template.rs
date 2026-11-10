//! Templates: dotfiles rendered for each host with minijinja, in a sandbox
//! that cannot read files or run anything (ADR 0006).

use std::collections::BTreeMap;
use std::path::Path;

use minijinja::{Environment, UndefinedBehavior, Value, context};

use crate::config::HostConfig;

/// No loader, so `include`, `import` and `extends` find nothing; undefined
/// names are errors rather than empty text.
fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env
}

/// A syntax error with its line, or `Ok`; `name` appears in the message.
pub fn check(name: &str, source: &str) -> Result<(), String> {
    environment()
        .template_from_named_str(name, source)
        .map(drop)
        .map_err(|err| describe(&err))
}

/// `vars.*` names the template uses that `vars` lacks, sorted. Static: a
/// name inside an `if` counts even when that branch never runs.
pub fn missing_vars(
    name: &str,
    source: &str,
    vars: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let env = environment();
    let template = env
        .template_from_named_str(name, source)
        .map_err(|err| describe(&err))?;
    let mut missing: Vec<String> = template
        .undeclared_variables(true)
        .iter()
        .filter_map(|used| used.strip_prefix("vars.")?.split('.').next())
        .filter(|var| !vars.contains_key(*var))
        .map(str::to_string)
        .collect();
    missing.sort();
    missing.dedup();
    Ok(missing)
}

pub fn render(name: &str, source: &str, host: &HostConfig, home: &Path) -> Result<String, String> {
    let env = environment();
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
            render("t", text, &host(&["work"], &[]), home).unwrap(),
            "me@work.com"
        );
        assert_eq!(
            render("t", text, &host(&[], &[]), home).unwrap(),
            "me@home.com"
        );
        let text = "{{ host.id }} {{ host.home }} {{ vars.email }}";
        assert_eq!(
            render("t", text, &host(&[], &[("email", "a@b.c")]), home).unwrap(),
            "mbp /home/me a@b.c"
        );
        let os = render("t", "{{ host.os }}/{{ host.arch }}", &host(&[], &[]), home).unwrap();
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
                render("t", text, &host(&[], &[]), Path::new("/h")).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn syntax_errors_name_the_line() {
        let err = check("home/.npmrc", "ok\n{% if %}\n").unwrap_err();
        assert!(err.contains("home/.npmrc:2"), "{err}");
        assert!(check("t", "{{ vars.x }}").is_ok());
    }

    #[test]
    fn missing_vars_are_the_unset_ones() {
        let text = "{{ vars.email }}{% if vars.signing %}{{ vars.key.id }}{% endif %}{{ host.id }}";
        let vars: BTreeMap<String, String> = [("email".to_string(), String::new())].into();
        assert_eq!(missing_vars("t", text, &vars).unwrap(), ["key", "signing"]);
    }
}
