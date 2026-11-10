//! The example files parse into their types and pass their JSON Schemas.

use rosamta_core::config::{Deploy, DotfileManifest, HostConfig, Module, Os, Phase, RepoManifest};
use rosamta_core::schema::{Kind, schema};

fn example(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/examples/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn as_json(toml_text: &str) -> serde_json::Value {
    serde_json::to_value(toml::from_str::<toml::Table>(toml_text).unwrap()).unwrap()
}

fn schema_errors(kind: Kind, toml_text: &str) -> Vec<String> {
    let validator = jsonschema::validator_for(&schema(kind)).unwrap();
    validator
        .iter_errors(&as_json(toml_text))
        .map(|err| err.to_string())
        .collect()
}

#[test]
fn examples_parse() {
    let manifest: RepoManifest = toml::from_str(&example("rosamta.toml")).unwrap();
    assert_eq!(manifest.hosts, ["mbp", "lab-server"]);
    let module: Module = toml::from_str(&example("module.toml")).unwrap();
    assert_eq!(
        (module.phase, module.order, module.when.os.as_slice()),
        (Some(Phase::Prompt), 10, &[Os::Macos][..])
    );
    let host: HostConfig = toml::from_str(&example("host.toml")).unwrap();
    assert_eq!(host.tags, ["lab", "personal"]);
    let structured: Module = toml::from_str(&example("structured.toml")).unwrap();
    assert_eq!(
        structured.env.keys().collect::<Vec<_>>(),
        ["EDITOR", "VISUAL"]
    );
    assert_eq!(
        structured.path.prepend,
        ["~/.local/bin", "$HOME/.cargo/bin"]
    );
    assert_eq!(structured.aliases["ll"], "ls -lh");
    let raw: Module = toml::from_str(&example("raw.toml")).unwrap();
    assert!(raw.raw.zsh.as_deref().unwrap().contains("ngrok completion"));
    assert_eq!(
        (raw.phase, raw.raw.bash.as_deref()),
        (Some(Phase::Tools), None)
    );
    let dotfiles: DotfileManifest = toml::from_str(&example("dotfiles.toml")).unwrap();
    assert_eq!(
        dotfiles
            .dotfile
            .iter()
            .map(|entry| entry.deploy)
            .collect::<Vec<_>>(),
        [Deploy::Link, Deploy::Copy, Deploy::Link]
    );
    assert_eq!(dotfiles.dotfile[1].mode(), Some(0o600));
}

#[test]
fn examples_pass_their_schemas() {
    for (kind, file) in [
        (Kind::Repo, "rosamta.toml"),
        (Kind::Module, "module.toml"),
        (Kind::Module, "structured.toml"),
        (Kind::Module, "raw.toml"),
        (Kind::Host, "host.toml"),
        (Kind::Dotfiles, "dotfiles.toml"),
    ] {
        assert_eq!(
            schema_errors(kind, &example(file)),
            Vec::<String>::new(),
            "{file}"
        );
    }
}

#[test]
fn schemas_reject_what_parsing_rejects() {
    assert!(!schema_errors(Kind::Module, "phse = \"tools\"").is_empty());
    assert!(!schema_errors(Kind::Module, "[when]\nos = [\"darwin\"]").is_empty());
    assert!(!schema_errors(Kind::Repo, "hosts = []").is_empty());
    assert!(!schema_errors(Kind::Host, "id = \"mbp\"\npull_interval = 0").is_empty());
    for bad in [
        "[[dotfile]]\ndeploy = \"copy\"",
        "[[dotfile]]\npath = \"x\"\ndeploy = \"hardlink\"",
        "[[dotfile]]\npath = \"x\"\npermissions = \"rw\"",
    ] {
        assert!(!schema_errors(Kind::Dotfiles, bad).is_empty(), "{bad}");
    }
}
