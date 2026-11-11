//! `secrets.toml` checks and the stored bws token, without any secret tool.

mod common;

use std::fs;

use common::Home;
use predicates::prelude::*;

#[test]
fn schema_describes_secrets_toml() {
    let home = Home::new();
    let out = home
        .cmd("rsmt")
        .args(["schema", "secrets"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["commands"].is_object());
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn validate_checks_secrets_toml() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write(
        "secrets.toml",
        "[commands.op]\nrun = [\"op\", \"read\", \"{arg}\"]\n",
    );
    home.cmd("rsmt").arg("validate").assert().success();

    home.write("secrets.toml", "[commands.op]\nrn = [\"op\"]\n");
    let path = home.repo().join("secrets.toml");
    home.cmd("rsmt").arg("validate").assert().code(2).stderr(
        predicate::str::contains(format!("{}:2:", path.display()))
            .and(predicate::str::contains("rn")),
    );

    home.write("secrets.toml", "[commands.op]\nrun = []\n");
    home.cmd("rsmt")
        .arg("validate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "command `op`: run must start with a program",
        ));
}

#[test]
fn bws_logout_deletes_the_stored_token() {
    let home = Home::new();
    let token = home.root.join(".config/rosamta/bws-token");
    fs::create_dir_all(token.parent().unwrap()).unwrap();
    fs::write(&token, "0.abc\n").unwrap();
    home.cmd("rsmt")
        .args(["secrets", "bws", "logout"])
        .assert()
        .success()
        .stdout("Deleted the Bitwarden Secrets Manager access token\n");
    assert!(!token.exists());
    let out = home
        .cmd("rsmt")
        .args(["secrets", "bws", "logout", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(reply, serde_json::json!({ "deleted": false }));
}
