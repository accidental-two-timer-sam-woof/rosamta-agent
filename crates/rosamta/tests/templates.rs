//! Templates rendered by `rsmt apply` outside a terminal, with fake
//! Bitwarden tools and secret commands on PATH.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::Home;

const ITEM: &str = r#"{"name":"GitHub","login":{"username":"me","password":"pw-1"},"data":{"username":"me","password":"pw-1"}}"#;

struct Tools {
    home: Home,
    bin: PathBuf,
}

impl Tools {
    fn new(template: &str, host: &str) -> Self {
        let home = Home::new();
        home.cmd("rsmt").arg("init").assert().success();
        let bin = home.root.join("bin");
        fs::create_dir(&bin).unwrap();
        home.write("home/.npmrc", template);
        home.write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".npmrc\"\ndeploy = \"template\"\n",
        );
        let config = fs::read_to_string(home.host_config()).unwrap();
        fs::write(home.host_config(), format!("{config}{host}")).unwrap();
        Self { home, bin }
    }

    /// A script answering by its first argument; each call is logged.
    fn tool(&self, name: &str, cases: &str) {
        let path = self.bin.join(name);
        let script = format!(
            "#!/bin/sh\necho \"{name} $*\" >> \"$(dirname \"$0\")/calls\"\ncase \"$1\" in\n{cases}\nesac\n"
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn calls(&self, prefix: &str) -> usize {
        fs::read_to_string(self.bin.join("calls"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    }

    fn rsmt(&self, args: &[&str]) -> std::process::Output {
        let path = std::env::var("PATH").unwrap_or_default();
        self.home
            .cmd("rsmt")
            .args(args)
            .env("PATH", format!("{}:{path}", self.bin.display()))
            .output()
            .unwrap()
    }

    fn waiting(&self) -> serde_json::Value {
        let out = self.rsmt(&["review", "--json"]);
        assert!(out.status.success(), "{out:?}");
        let queue: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        queue[0]["dotfile"]["waiting"].clone()
    }

    fn target(&self) -> PathBuf {
        self.home.root.join(".npmrc")
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn a_locked_rbw_waits_until_it_is_unlocked() {
    let tools = Tools::new("pw={{ bitwarden(\"GitHub\") }}\n", "");
    tools.tool(
        "rbw",
        &format!(
            r#"config) echo '{{"email":"me@example.com"}}' ;;
unlocked) [ -f "$(dirname "$0")/open" ] ;;
unlock) touch "$(dirname "$0")/open" ;;
get) echo '{ITEM}' ;;"#
        ),
    );
    let out = tools.rsmt(&["apply"]);
    assert!(out.status.success(), "{out:?}");
    assert!(!tools.target().exists());
    assert_eq!(
        tools.waiting(),
        serde_json::json!({"for": "locked", "tool": "rbw"})
    );
    // Outside a terminal rosamta never unlocks and never reads.
    assert_eq!(tools.calls("rbw unlock "), 0);
    assert_eq!(tools.calls("rbw get"), 0);
    let out = tools.rsmt(&["review", "accept", "1"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");

    fs::write(tools.bin.join("open"), "").unwrap();
    assert!(tools.rsmt(&["apply"]).status.success());
    assert_eq!(read(&tools.target()), "pw=pw-1\n");
    assert_eq!(
        fs::metadata(tools.target()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(tools.waiting(), serde_json::Value::Null);
}

#[test]
fn a_locked_bw_waits_and_an_unlocked_one_is_read() {
    let tools = Tools::new(
        "user={{ bitwarden(\"GitHub\", \"username\") }}\n",
        "\n[secrets]\nbitwarden = [\"bw\"]\n",
    );
    tools.tool(
        "bw",
        &format!(
            r#"status) [ -f "$(dirname "$0")/open" ] && echo '{{"status":"unlocked"}}' || echo '{{"status":"locked"}}' ;;
list) echo '[{ITEM}]' ;;"#
        ),
    );
    assert!(tools.rsmt(&["apply"]).status.success());
    assert_eq!(
        tools.waiting(),
        serde_json::json!({"for": "locked", "tool": "bw"})
    );
    assert_eq!(tools.calls("bw list"), 0);

    fs::write(tools.bin.join("open"), "").unwrap();
    assert!(tools.rsmt(&["apply"]).status.success());
    assert_eq!(read(&tools.target()), "user=me\n");
    assert_eq!(tools.calls("bw unlock"), 0);
}

#[test]
fn bws_gets_the_stored_token_in_its_environment() {
    let tools = Tools::new(
        "npm={{ bitwarden(\"NPM_TOKEN\") }}\n",
        "\n[secrets]\nbitwarden = [\"bws\"]\n",
    );
    tools.tool(
        "bws",
        r#"secret) [ "$BWS_ACCESS_TOKEN" = "0.t0k" ] || exit 1
echo '[{"key":"NPM_TOKEN","value":"npm_1"}]' ;;"#,
    );
    // Not signed in: no token stored.
    let out = tools.rsmt(&["apply"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no Bitwarden tool"),
        "{out:?}"
    );

    let token = tools.home.root.join(".config/rosamta/bws-token");
    fs::write(&token, "0.t0k\n").unwrap();
    assert!(tools.rsmt(&["apply"]).status.success());
    assert_eq!(read(&tools.target()), "npm=npm_1\n");
    assert!(!read(&tools.bin.join("calls")).contains("t0k"));
}

#[test]
fn a_secret_command_runs_only_in_a_terminal() {
    let tools = Tools::new("token={{ secret(\"fetch\", \"npm\") }}\n", "");
    tools.tool("fetch", r#"*) echo "value-of-$1" ;;"#);
    tools.home.write(
        "secrets.toml",
        "[commands.fetch]\nrun = [\"fetch\", \"{arg}\"]\n",
    );
    assert!(tools.rsmt(&["apply"]).status.success());
    assert_eq!(
        tools.waiting(),
        serde_json::json!({"for": "command", "name": "fetch"})
    );
    assert_eq!(tools.calls("fetch"), 0);
    let out = tools.rsmt(&["review"]);
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("secret command `fetch` runs only in `rsmt apply` in a terminal"),
        "{out:?}"
    );
}
