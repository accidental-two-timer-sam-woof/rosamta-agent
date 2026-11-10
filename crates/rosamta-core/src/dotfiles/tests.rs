use super::*;
use crate::config::{Arch, Os};

fn host(id: &str, tags: &[&str]) -> HostFacts {
    HostFacts {
        os: Os::Macos,
        arch: Arch::Aarch64,
        id: id.into(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
    }
}

fn manifest(text: &str) -> DotfileManifest {
    toml::from_str(text).unwrap()
}

#[test]
fn clean_paths() {
    assert_eq!(clean(".config/nvim/"), Some(PathBuf::from(".config/nvim")));
    assert_eq!(clean("./.gitconfig"), Some(PathBuf::from(".gitconfig")));
    assert_eq!(clean("/etc/hosts"), None);
    assert_eq!(clean("../x"), None);
    assert_eq!(clean(".config/../x"), None);
    assert_eq!(clean(""), None);
}

#[test]
fn defaults_and_mode() {
    let parsed = manifest("[[dotfile]]\npath = \".config/nvim\"\n");
    assert_eq!(parsed.dotfile, [Dotfile::new(".config/nvim")]);
    assert_eq!(parsed.dotfile[0].source(), "home/.config/nvim");
    let mut entry = Dotfile::new("x");
    for (permissions, mode) in [
        ("0600", Some(0o600)),
        ("755", Some(0o755)),
        ("0800", None),
        ("1755", None),
        ("rw", None),
    ] {
        entry.permissions = Some(permissions.into());
        assert_eq!(entry.mode(), mode, "{permissions}");
    }
}

#[test]
fn host_specific_declaration_wins() {
    let parsed = manifest(
        r#"
[[dotfile]]
path = ".gitconfig"

[[dotfile]]
path = ".gitconfig"
source = "hosts/mbp/.gitconfig"
when = { hosts = ["mbp"] }

[[dotfile]]
path = ".zprofile"
when = { tags = ["work"] }

[[dotfile]]
path = ".zprofile"
when = { os = ["macos"] }
"#,
    );
    let on_mbp = select(&parsed, &host("mbp", &["work"]));
    let Choice::Use(gitconfig) = &on_mbp[Path::new(".gitconfig")] else {
        panic!("{on_mbp:?}");
    };
    assert_eq!(gitconfig.source(), "hosts/mbp/.gitconfig");
    assert_eq!(on_mbp[Path::new(".zprofile")], Choice::Duplicate(2));

    let elsewhere = select(&parsed, &host("lab", &[]));
    let Choice::Use(gitconfig) = &elsewhere[Path::new(".gitconfig")] else {
        panic!("{elsewhere:?}");
    };
    assert_eq!(gitconfig.source(), "home/.gitconfig");
    let Choice::Use(_) = &elsewhere[Path::new(".zprofile")] else {
        panic!("{elsewhere:?}");
    };
}

#[test]
fn checks() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join("home/.config/nvim")).unwrap();
    std::fs::write(repo.path().join("home/.config/nvim/init.lua"), "").unwrap();
    std::fs::write(repo.path().join("home/.netrc"), "").unwrap();
    let parsed = manifest(
        r#"
[[dotfile]]
path = ".config/nvim"

[[dotfile]]
path = ".config/nvim/init.lua"

[[dotfile]]
path = "/etc/hosts"

[[dotfile]]
path = ".missing"

[[dotfile]]
path = ".netrc"
encrypted = true
permissions = "600x"
"#,
    );
    let problems = check(&parsed, repo.path());
    let expected = [
        "path must be relative",
        "source `home/.missing` does not exist",
        "permissions must be an octal mode",
        "an encrypted dotfile cannot be a link",
        "dotfile `.config/nvim/init.lua` is inside dotfile `.config/nvim`",
    ];
    for want in expected {
        assert!(
            problems.iter().any(|problem| problem.contains(want)),
            "{want}: {problems:#?}"
        );
    }
    assert_eq!(problems.len(), expected.len() + 1, "{problems:#?}");
}

#[test]
fn appended_declarations_stay_compact() {
    let repo = tempfile::tempdir().unwrap();
    let manifest = repo.path().join(MANIFEST);
    std::fs::write(
        &manifest,
        "# mine\n[[dotfile]]\npath = \".gitconfig\" # shared\n",
    )
    .unwrap();
    let mut entry = Dotfile::new(".gitconfig");
    entry.source = Some("hosts/mbp/.gitconfig".into());
    entry.deploy = Deploy::Copy;
    entry.when.hosts = vec!["mbp".into()];
    review::append(repo.path(), &entry).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        "# mine\n[[dotfile]]\npath = \".gitconfig\" # shared\n\n[[dotfile]]\npath = \".gitconfig\"\nsource = \"hosts/mbp/.gitconfig\"\ndeploy = \"copy\"\nwhen = { hosts = [\"mbp\"] }\n"
    );
    let parsed: DotfileManifest =
        toml::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    assert_eq!(parsed.dotfile[1], entry);
}
