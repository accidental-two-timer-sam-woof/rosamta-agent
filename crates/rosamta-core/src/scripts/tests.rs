use super::*;

fn check_text(text: &str) -> Vec<String> {
    let repo = tempfile::tempdir().unwrap();
    for file in ["scripts/install.sh", "Brewfile"] {
        let path = repo.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    let manifest: ScriptManifest = validate::parse(text).unwrap();
    check(&manifest, repo.path())
}

#[test]
fn declared_scripts_pass() {
    let text = "[[script]]\nsource = \"scripts/install.sh\"\nrun = \"onchange\"\ntiming = \"before\"\nwatch = [\"Brewfile\"]\n";
    assert_eq!(check_text(text), Vec::<String>::new());
}

#[test]
fn sources_watches_and_duplicates_are_checked() {
    let text = "[[script]]\nsource = \"scripts/missing.sh\"\nrun = \"once\"\n\n\
                [[script]]\nsource = \"scripts\"\nrun = \"onchange\"\nwatch = [\"../x\", \"gone\"]\n\n\
                [[script]]\nsource = \"scripts/install.sh\"\nrun = \"once\"\nwatch = [\"Brewfile\"]\n\n\
                [[script]]\nsource = \"./scripts/install.sh\"\nrun = \"once\"\n";
    assert_eq!(
        check_text(text),
        [
            "script `scripts/missing.sh` does not exist",
            "script `scripts` is not a file",
            "script `scripts`: watched `../x` must be relative to the repo, without `..`",
            "script `scripts`: watched `gone` does not exist",
            "script `scripts/install.sh`: `watch` is for onchange scripts",
            "script `./scripts/install.sh` is declared more than once",
        ]
    );
}

#[test]
fn run_is_required_and_timing_defaults_to_after() {
    let err = validate::parse::<ScriptManifest>("[[script]]\nsource = \"a.sh\"\n").unwrap_err();
    assert!(err.message.contains("run"), "{}", err.message);
    let manifest: ScriptManifest =
        validate::parse("[[script]]\nsource = \"a.sh\"\nrun = \"once\"\n").unwrap();
    assert_eq!(manifest.script[0].timing, Timing::After);
}
