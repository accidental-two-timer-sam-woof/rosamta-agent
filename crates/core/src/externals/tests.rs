use super::*;

const REV: &str = "0123456789abcdef0123456789abcdef01234567";
const SUM: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn check_text(externals: &str, dotfiles: &str) -> Vec<String> {
    let manifest: ExternalManifest = validate::parse(externals).unwrap();
    let dotfiles: DotfileManifest = validate::parse(dotfiles).unwrap();
    check(&manifest, &dotfiles)
}

fn git(path: &str) -> String {
    format!(
        "[[external]]\npath = \"{path}\"\ngit = \"https://example.com/a.git\"\nrev = \"{REV}\"\n"
    )
}

fn archive(path: &str) -> String {
    format!(
        "[[external]]\npath = \"{path}\"\narchive = \"https://example.com/a.zip\"\nsha256 = \"{SUM}\"\n"
    )
}

#[test]
fn locked_declarations_pass() {
    let text = git(".oh-my-zsh") + &archive(".local/share/fonts/a");
    assert_eq!(check_text(&text, ""), Vec::<String>::new());
}

#[test]
fn kinds_and_locks_are_checked() {
    let both = "[[external]]\npath = \"a\"\ngit = \"x\"\narchive = \"https://e/a.zip\"\n";
    assert_eq!(
        check_text(both, ""),
        ["external `a`: set `git` or `archive`, not both"]
    );
    let unlocked = "[[external]]\npath = \"a\"\ngit = \"https://e/a.git\"\n";
    assert_eq!(
        check_text(unlocked, ""),
        ["external `a` is not locked yet; run `rsmt externals update`"]
    );
    let short =
        "[[external]]\npath = \"a\"\ngit = \"https://e/a.git\"\nrev = \"abc123\"\nstrip = 1\n";
    assert_eq!(
        check_text(short, ""),
        [
            "external `a`: `sha256` and `strip` are for archives",
            "external `a`: `rev` must be a full commit hash",
        ]
    );
    let plain = format!(
        "[[external]]\npath = \"a\"\narchive = \"ftp://e/a.zip\"\nsha256 = \"{SUM}\"\nbranch = \"main\"\n"
    );
    assert_eq!(
        check_text(&plain, ""),
        [
            "external `a`: `rev` and `branch` are for git",
            "external `a`: `archive` must be an http or https URL",
        ]
    );
    let option =
        format!("[[external]]\npath = \"a\"\ngit = \"--upload-pack=x\"\nrev = \"{REV}\"\n");
    assert_eq!(
        check_text(&option, ""),
        ["external `a`: `git` and `branch` cannot start with `-`"]
    );
    assert_eq!(
        check_text(&git("../a"), ""),
        ["external `../a`: path must be relative to the home directory, without `..`"]
    );
}

#[test]
fn paths_nest_only_inside_git_externals() {
    let plugin = git(".oh-my-zsh") + &git(".oh-my-zsh/custom/plugins/fzf-tab");
    let aliases = "[[dotfile]]\npath = \".oh-my-zsh/custom/aliases.zsh\"\n";
    assert_eq!(check_text(&plugin, aliases), Vec::<String>::new());

    let in_archive = archive(".fonts") + &git(".fonts/extra");
    assert_eq!(
        check_text(&in_archive, "[[dotfile]]\npath = \".fonts/a.conf\"\n"),
        [
            "dotfile `.fonts/a.conf` is inside external `.fonts`, which is not a git repo",
            "external `.fonts/extra` is inside external `.fonts`, which is not a git repo",
        ]
    );
    // Writing into a linked directory would write into the repo.
    assert_eq!(
        check_text(
            &git(".config/zsh/plugins/a"),
            "[[dotfile]]\npath = \".config/zsh\"\n"
        ),
        ["external `.config/zsh/plugins/a` is inside dotfile `.config/zsh`"]
    );
    // Host variants of one path are settled when selecting.
    let variants = archive(".fonts") + &archive(".fonts");
    assert_eq!(check_text(&variants, ""), Vec::<String>::new());
}
