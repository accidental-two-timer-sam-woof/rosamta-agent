use serde_json::json;

use super::*;
use crate::config::{Arch, Os};

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

struct Fixture {
    shared: Map<String, Value>,
    data: Map<String, Value>,
}

impl Fixture {
    fn new(shared: Value, data: Value) -> Self {
        Self {
            shared: map(shared),
            data: map(data),
        }
    }

    fn machine(&self) -> Machine<'_> {
        Machine {
            shared: &self.shared,
            data: &self.data,
            hostname: Some("mbp-m5"),
            host_id: "xinshoutw-macbook-pro",
        }
    }
}

fn plain() -> Fixture {
    Fixture::new(json!({}), json!({}))
}

fn strings(vars: &Value) -> std::collections::BTreeMap<String, String> {
    map(vars.clone())
        .into_iter()
        .map(|(name, value)| (name, value.as_str().unwrap_or_default().to_string()))
        .collect()
}

/// Render as rosamta does on a host with `os`, `arch`, `id` and `vars`.
fn render(text: &str, os: &str, arch: &str, id: &str, vars: Value) -> String {
    crate::template::check("t", text).unwrap();
    let mut env = minijinja::Environment::new();
    env.set_syntax(
        minijinja::syntax::SyntaxConfig::builder()
            .keep_trailing_newline(true)
            .build()
            .unwrap(),
    );
    env.add_function("bitwarden", |item: String, field: String| {
        format!("<{item}:{field}>")
    });
    env.add_function("secret", |name: String, arg: String| {
        format!("<{name}:{arg}>")
    });
    env.render_str(
        text,
        minijinja::context! {
            host => minijinja::context! { os, arch, id, home => "/home/me" },
            vars => minijinja::Value::from(strings(&vars)),
        },
    )
    .unwrap()
}

#[test]
fn platforms_become_when() {
    let scope = |platforms| Scope {
        platforms,
        hosts: None,
    };
    assert_eq!(scope(ALL).when(), Some(None));
    assert_eq!(
        scope(MACOS).when(),
        Some(Some(When {
            os: vec![Os::Macos],
            ..Default::default()
        }))
    );
    assert_eq!(
        scope(LINUX & AARCH64).when(),
        Some(Some(When {
            os: vec![Os::Linux],
            arch: vec![Arch::Aarch64],
            ..Default::default()
        }))
    );
    // macOS on Apple silicon or any Linux is not one product.
    assert_eq!(scope((MACOS & AARCH64) | LINUX).when(), None);
}

#[test]
fn a_whole_file_condition_becomes_when() {
    let fixture = plain();
    let converted = file(
        "{{ if eq .chezmoi.os \"darwin\" -}}\nyabai -m config layout bsp\n{{ end -}}\n",
        &fixture.machine(),
    )
    .unwrap();
    assert_eq!(converted.text, "yabai -m config layout bsp\n");
    assert!(!converted.template);
    assert_eq!(converted.scope.platforms, MACOS);

    let converted = file(
        "{{ if and (eq .chezmoi.os \"linux\") (eq .chezmoi.hostname \"mbp-m5\" \"nas\") }}x{{ end }}",
        &fixture.machine(),
    )
    .unwrap();
    assert_eq!(converted.scope.platforms, LINUX);
    assert_eq!(
        converted.scope.hosts,
        Some(["nas".to_string(), "xinshoutw-macbook-pro".to_string()].into())
    );
}

#[test]
fn conditions_and_values_render_as_chezmoi_would() {
    let fixture = Fixture::new(
        json!({ "font": "JetBrains Mono", "size": 13 }),
        json!({ "email": "me@example.com", "work": false }),
    );
    let source = "\
font = {{ .font }} {{ .size }}
email = {{ .email }}
os = {{ .chezmoi.os }}/{{ .chezmoi.arch }}
{{- if eq .chezmoi.hostname \"mbp-m5\" }}
here
{{- else if eq .chezmoi.hostname \"Work.Laptop\" }}
work laptop
{{- end }}
{{ if .work }}work{{ else }}home{{ end }}
{{ if not (eq .chezmoi.os \"linux\") }}not linux{{ end }}
array: ${#items[@]} {{ \"{{\" }}
";
    let converted = file(source, &fixture.machine()).unwrap();
    assert!(converted.template);
    assert_eq!(
        converted.uses.vars,
        ["email".to_string(), "work".to_string()].into()
    );
    assert_eq!(
        converted.uses.hostnames,
        [
            ("Work.Laptop".to_string(), "work".to_string()),
            ("mbp-m5".to_string(), "xinshoutw-macbook-pro".to_string()),
        ]
        .into()
    );
    let vars = json!({ "email": "me@example.com", "work": "false" });
    assert_eq!(
        render(
            &converted.text,
            "macos",
            "aarch64",
            "xinshoutw-macbook-pro",
            vars.clone()
        ),
        "font = JetBrains Mono 13\nemail = me@example.com\nos = darwin/arm64\nhere\nhome\nnot linux\narray: ${#items[@]} {{\n"
    );
    assert_eq!(
        render(&converted.text, "linux", "x86_64", "work", vars),
        "font = JetBrains Mono 13\nemail = me@example.com\nos = linux/amd64\nwork laptop\nhome\n\narray: ${#items[@]} {{\n"
    );
}

#[test]
fn secrets_become_rosamta_functions() {
    let fixture = plain();
    let converted = file(
        "{{ (bitwarden \"item\" \"GitHub\").login.password }} {{ (bitwardenFields \"item\" \"AWS\").key.value }} {{ onepasswordRead \"op://v/i/f\" }}",
        &fixture.machine(),
    )
    .unwrap();
    assert!(converted.uses.secrets && converted.uses.op);
    assert_eq!(
        render(&converted.text, "macos", "aarch64", "x", json!({})),
        "<GitHub:password> <AWS:key> <op:op://v/i/f>"
    );
}

#[test]
fn anything_else_is_reported_with_its_line() {
    let fixture = Fixture::new(json!({ "list": [1, 2] }), json!({ "count": 3 }));
    for (source, line, what) in [
        ("a\n{{ .chezmoi.username }}", 2, "`.chezmoi.username`"),
        ("{{ .list }}", 1, "`.list` is not a single value"),
        ("a\nb\n{{ .count }}", 3, "`.count`"),
        ("{{ lookPath \"git\" }}", 1, "`lookPath`"),
        ("{{ .chezmoi.hostname }}", 1, "printing `.chezmoi.hostname`"),
        (
            "{{ (bitwarden \"item\" \"x\").login.totp }}",
            1,
            "`bitwarden` field `.login.totp`",
        ),
    ] {
        let err = file(source, &fixture.machine()).unwrap_err();
        assert_eq!((err.line, err.what.as_str()), (line, what), "{source}");
    }
}

#[test]
fn scripts_keep_text_and_watch_included_files() {
    let fixture = plain();
    let converted = script(
        "{{ if eq .chezmoi.os \"darwin\" -}}\n#!/bin/sh\n# Brewfile {{ include \"dot_Brewfile\" | sha256sum }}\nbrew bundle\n{{ end -}}\n",
        &fixture.machine(),
    )
    .unwrap();
    assert_eq!(converted.text, "#!/bin/sh\n# Brewfile \nbrew bundle\n");
    assert_eq!(converted.uses.watch, ["dot_Brewfile"]);
    assert_eq!(converted.scope.platforms, MACOS);
    let err = script("#!/bin/sh\necho {{ .email }}\n", &fixture.machine()).unwrap_err();
    assert_eq!(err.line, 2);
    let err = script(
        "#!/bin/sh\ncd \"$CHEZMOI_SOURCE_DIR\"\n",
        &fixture.machine(),
    )
    .unwrap_err();
    assert_eq!(err.line, 2);
}

#[test]
fn ignore_rules_follow_their_blocks() {
    let fixture = plain();
    let source = "\
README.md
{{ if ne .chezmoi.os \"darwin\" }}
Library/**
{{ else }}
.config/i3
{{ end }}
{{ if ne .chezmoi.hostname \"mbp-m5\" }}
.ssh/work_*
{{ end }}
";
    let rules = ignore(source, &fixture.machine()).unwrap();
    let rule = |pattern: &str, platforms, except: Option<&[&str]>| IgnoreRule {
        pattern: pattern.into(),
        platforms,
        except: except.map(|hosts| hosts.iter().map(|host| host.to_string()).collect()),
    };
    assert_eq!(
        rules,
        [
            rule("README.md", ALL, None),
            rule("Library/**", LINUX, None),
            rule(".config/i3", MACOS, None),
            rule(".ssh/work_*", ALL, Some(&["xinshoutw-macbook-pro"])),
        ]
    );
    assert!(
        ignore(
            "{{ if eq .chezmoi.hostname \"x\" }}a{{ end }}",
            &fixture.machine()
        )
        .is_err()
    );
}
