use std::os::unix::fs::PermissionsExt;

use super::*;

/// A fake tool in `dir`: a shell script answering by its first argument.
/// Each call appends its arguments to `dir/calls`.
fn tool(dir: &Path, name: &str, cases: &str) {
    let path = dir.join(name);
    let script = format!(
        "#!/bin/sh\necho \"{name} $*\" >> \"$(dirname \"$0\")/calls\"\ncase \"$1\" in\n{cases}\nesac\n"
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

const ITEM: &str = r#"{"name":"GitHub","notes":"n","fields":[{"name":"token","value":"ghp_1"}]"#;

fn rbw(dir: &Path) {
    tool(
        dir,
        "rbw",
        &format!(
            r#"config) echo '{{"email":"me@example.com"}}' ;;
unlocked) [ -f "$(dirname "$0")/unlocked" ] ;;
get) echo '{ITEM},"data":{{"username":"me","password":"rbw-pw","totp":"seed"}}}}' ;;"#
        ),
    );
}

fn bw(dir: &Path) {
    tool(
        dir,
        "bw",
        &format!(
            r#"status) echo '{{"status":"unlocked"}}' ;;
list) echo '[{ITEM},"login":{{"username":"me","password":"bw-pw"}}}},{{"name":"GitHub Enterprise"}}]' ;;"#
        ),
    );
}

fn session(dir: &Path, interactive: bool, order: &[BitwardenTool]) -> Session {
    Session::with(
        interactive,
        order,
        SecretCommands::default(),
        dir.display().to_string(),
        None,
    )
}

fn calls(dir: &Path, prefix: &str) -> usize {
    std::fs::read_to_string(dir.join("calls"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with(prefix))
        .count()
}

#[test]
fn rbw_comes_first_and_each_item_is_read_once() {
    let dir = tempfile::tempdir().unwrap();
    rbw(dir.path());
    bw(dir.path());
    std::fs::write(dir.path().join("unlocked"), "").unwrap();
    let secrets = session(dir.path(), false, &[]);
    assert_eq!(secrets.bitwarden("GitHub", None).unwrap(), "rbw-pw");
    assert_eq!(secrets.bitwarden("GitHub", Some("username")).unwrap(), "me");
    assert_eq!(secrets.bitwarden("GitHub", Some("notes")).unwrap(), "n");
    assert_eq!(secrets.bitwarden("GitHub", Some("totp")).unwrap(), "seed");
    assert_eq!(secrets.bitwarden("GitHub", Some("token")).unwrap(), "ghp_1");
    let err = secrets.bitwarden("GitHub", Some("pin")).unwrap_err();
    assert_eq!(
        err,
        Failure::Error("Bitwarden item `GitHub` has no pin".into())
    );
    assert_eq!(calls(dir.path(), "rbw get"), 1);
    assert_eq!(calls(dir.path(), "bw "), 0);
}

#[test]
fn the_order_in_host_toml_wins() {
    let dir = tempfile::tempdir().unwrap();
    rbw(dir.path());
    bw(dir.path());
    let secrets = session(dir.path(), false, &[BitwardenTool::Bw]);
    assert_eq!(secrets.bitwarden("GitHub", None).unwrap(), "bw-pw");
    assert_eq!(calls(dir.path(), "rbw"), 0);
}

#[test]
fn only_the_item_named_counts() {
    let dir = tempfile::tempdir().unwrap();
    rbw(dir.path());
    std::fs::write(dir.path().join("unlocked"), "").unwrap();
    // rbw falls back to a partial match: "Git" finds "GitHub".
    let err = session(dir.path(), false, &[])
        .bitwarden("Git", None)
        .unwrap_err();
    assert_eq!(
        err,
        Failure::Error("rbw cannot read `Git`: no item is named `Git`".into())
    );
    assert_eq!(calls(dir.path(), "rbw get --raw -- Git"), 1);

    tool(
        dir.path(),
        "bw",
        r#"status) echo '{"status":"unlocked"}' ;;
list) echo '[{"name":"a"},{"name":"a"}]' ;;"#,
    );
    let err = session(dir.path(), false, &[BitwardenTool::Bw])
        .bitwarden("a", None)
        .unwrap_err();
    assert_eq!(
        err,
        Failure::Error("bw cannot read `a`: several items are named `a`".into())
    );
}

#[test]
fn a_locked_tool_waits_outside_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    rbw(dir.path());
    let secrets = session(dir.path(), false, &[]);
    let waiting = Waiting::Locked(BitwardenTool::Rbw);
    assert_eq!(
        secrets.bitwarden("GitHub", None).unwrap_err(),
        Failure::Waiting(waiting.clone())
    );
    assert_eq!(secrets.take_waiting(), std::slice::from_ref(&waiting));
    assert!(secrets.take_waiting().is_empty());
    // Checked once; the next render waits again without asking rbw.
    assert_eq!(
        secrets.bitwarden("Other", None).unwrap_err(),
        Failure::Waiting(waiting.clone())
    );
    assert_eq!(secrets.take_waiting(), [waiting]);
    assert_eq!(calls(dir.path(), "rbw unlocked"), 1);
    assert_eq!(calls(dir.path(), "rbw get"), 0);
}

#[test]
fn signed_out_tools_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    tool(dir.path(), "rbw", r#"config) echo '{"email":null}' ;;"#);
    tool(
        dir.path(),
        "bw",
        r#"status) echo '{"status":"unauthenticated"}' ;;"#,
    );
    tool(dir.path(), "bws", "*) exit 1 ;;");
    let err = session(dir.path(), false, &[])
        .bitwarden("GitHub", None)
        .unwrap_err();
    assert_eq!(
        err,
        Failure::Error("no Bitwarden tool is installed and signed in (tried rbw, bw, bws)".into())
    );
}

#[test]
fn bws_finds_a_secret_by_key_with_the_token_in_its_environment() {
    let dir = tempfile::tempdir().unwrap();
    tool(
        dir.path(),
        "bws",
        r#"secret) [ "$BWS_ACCESS_TOKEN" = "t0k" ] || exit 1
echo '[{"key":"NPM_TOKEN","value":"npm_1","note":"ci"},{"key":"TWICE"},{"key":"TWICE"}]' ;;"#,
    );
    let secrets = Session::with(
        false,
        &[],
        SecretCommands::default(),
        dir.path().display().to_string(),
        Some("t0k".into()),
    );
    assert_eq!(secrets.bitwarden("NPM_TOKEN", None).unwrap(), "npm_1");
    assert_eq!(secrets.bitwarden("NPM_TOKEN", Some("note")).unwrap(), "ci");
    assert!(matches!(
        secrets.bitwarden("NPM_TOKEN", Some("username")),
        Err(Failure::Error(_))
    ));
    assert_eq!(
        secrets.bitwarden("OTHER", None).unwrap_err(),
        Failure::Error("bws has no secret with key `OTHER`".into())
    );
    assert_eq!(
        secrets.bitwarden("TWICE", None).unwrap_err(),
        Failure::Error("bws has several secrets with key `TWICE`".into())
    );
    assert_eq!(calls(dir.path(), "bws secret list"), 1);
    // The token never reaches the command line.
    let log = std::fs::read_to_string(dir.path().join("calls")).unwrap();
    assert!(!log.contains("t0k"));
}

fn commands(dir: &Path) -> SecretCommands {
    tool(
        dir,
        "fetch",
        r#"fail) echo "first" >&2; echo "denied: $2" >&2; exit 3 ;;
*) printf '%s\n' "$1" ;;"#,
    );
    let program = dir.join("fetch").display().to_string();
    toml::from_str(&format!(
        "[commands.fetch]\nrun = [\"{program}\", \"{{arg}}\"]\n\
         [commands.fail]\nrun = [\"{program}\", \"fail\", \"{{arg}}\"]\n"
    ))
    .unwrap()
}

#[test]
fn secret_commands_run_once_in_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Session::with(true, &[], commands(dir.path()), String::new(), None);
    assert_eq!(secrets.secret("fetch", "op://a b").unwrap(), "op://a b");
    assert_eq!(secrets.secret("fetch", "op://a b").unwrap(), "op://a b");
    assert_eq!(calls(dir.path(), "fetch op://a b"), 1);
    assert_eq!(
        secrets.secret("fail", "x").unwrap_err(),
        Failure::Error("secret command `fail` failed: denied: x".into())
    );
    assert_eq!(
        secrets.secret("vault", "x").unwrap_err(),
        Failure::Error("secret command `vault` is not declared in secrets.toml".into())
    );
}

#[test]
fn secret_commands_wait_outside_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Session::with(false, &[], commands(dir.path()), String::new(), None);
    assert_eq!(
        secrets.secret("fetch", "x").unwrap_err(),
        Failure::Waiting(Waiting::Command("fetch".into()))
    );
    assert_eq!(calls(dir.path(), "fetch"), 0);
}

#[test]
fn programs_must_be_fixed_names_or_absolute_paths() {
    let commands: SecretCommands = toml::from_str(
        "[commands.a]\nrun = []\n[commands.b]\nrun = [\"\"]\n[commands.c]\nrun = [\"pass\"]\n\
         [commands.d]\nrun = [\"{arg}\"]\n[commands.e]\nrun = [\"scripts/op.sh\"]\n\
         [commands.f]\nrun = [\"/usr/bin/op\", \"{arg}\"]\n",
    )
    .unwrap();
    assert_eq!(
        check(&commands),
        [
            "command `a`: run must start with a program",
            "command `b`: run must start with a program",
            "command `d`: the program cannot come from {arg}",
            "command `e`: the program must be a name on PATH or an absolute path",
        ]
    );
}

#[test]
fn the_ssh_socket_follows_how_the_app_was_installed() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let none = Path::new("/nonexistent");
    assert_eq!(socket(home, "", none, none), None);

    let apps = tempfile::tempdir().unwrap();
    std::fs::create_dir(apps.path().join("Bitwarden.app")).unwrap();
    assert_eq!(
        socket(home, "", apps.path(), none),
        Some(home.join(".bitwarden-ssh-agent.sock"))
    );

    std::fs::create_dir_all(home.join(".var/app/com.bitwarden.desktop")).unwrap();
    assert_eq!(
        socket(home, "", none, none),
        Some(home.join(".var/app/com.bitwarden.desktop/data/.bitwarden-ssh-agent.sock"))
    );
    std::fs::create_dir_all(home.join("snap/bitwarden")).unwrap();
    assert_eq!(
        socket(home, "", none, none),
        Some(home.join("snap/bitwarden/current/.bitwarden-ssh-agent.sock"))
    );
    std::fs::create_dir_all(home.join("Library/Containers/com.bitwarden.desktop")).unwrap();
    assert_eq!(
        socket(home, "", none, none),
        Some(home.join("Library/Containers/com.bitwarden.desktop/Data/.bitwarden-ssh-agent.sock"))
    );
}
