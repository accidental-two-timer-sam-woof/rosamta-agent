//! Turn the vendored gitleaks rules (rules/gitleaks.toml) into a Rust table,
//! keeping only the patterns Rust's regex engine compiles; gitleaks targets Go.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let source = Path::new("rules/gitleaks.toml");
    println!("cargo::rerun-if-changed={}", source.display());
    let text = std::fs::read_to_string(source).expect("read rules/gitleaks.toml");
    let config: toml::Table = text.parse().expect("parse rules/gitleaks.toml");
    // (id, pattern, keywords): a rule only runs on text containing a keyword.
    let mut out = String::from("pub(crate) const GITLEAKS: &[(&str, &str, &[&str])] = &[\n");
    for rule in config["rules"].as_array().expect("a rules array") {
        let id = rule["id"].as_str().expect("a rule id");
        // Path-only rules have no pattern; our name-and-value rule replaces the
        // noisy generic one.
        let Some(pattern) = rule.get("regex").and_then(|value| value.as_str()) else {
            continue;
        };
        if id == "generic-api-key" {
            continue;
        }
        let keywords: Vec<&str> = rule
            .get("keywords")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|keyword| keyword.as_str())
            .collect();
        match regex::Regex::new(pattern) {
            Ok(_) => writeln!(out, "    ({id:?}, {pattern:?}, &{keywords:?}),").unwrap(),
            Err(err) => {
                let reason = err.to_string();
                let reason = reason.lines().last().unwrap_or_default();
                println!("cargo::warning=gitleaks rule {id} skipped: {reason}");
            }
        }
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("gitleaks.rs");
    std::fs::write(dest, out).expect("write gitleaks.rs");
}
