use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;

/// An HTTP server that answers each request with the next canned reply and
/// records `METHOD path` plus the body of every request.
struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

fn mock(replies: &[(u16, &str)]) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let replies: Vec<(u16, String)> = replies
        .iter()
        .map(|(status, body)| (*status, body.to_string()))
        .collect();
    std::thread::spawn(move || {
        for (status, body) in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut content = vec![0; length];
            reader.read_exact(&mut content).unwrap();
            let target: Vec<&str> = request_line.split_whitespace().take(2).collect();
            seen.lock().unwrap().push(format!(
                "{}\n{}",
                target.join(" "),
                String::from_utf8_lossy(&content)
            ));
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    Mock { url, requests }
}

fn github(mock: &Mock) -> GitHub {
    GitHub::with_urls(&mock.url, &mock.url, "test-client")
}

const CODE: &str = r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":0}"#;

fn sign_in(replies: &[(u16, &str)]) -> (Mock, Result<String>) {
    let mock = mock(replies);
    let github = github(&mock);
    let result = github
        .request_code()
        .and_then(|code| github.wait_for_token(&code));
    (mock, result)
}

#[test]
fn device_flow_waits_through_pending_and_slow_down() {
    let (mock, token) = sign_in(&[
        (200, CODE),
        (200, r#"{"error":"authorization_pending"}"#),
        (200, r#"{"error":"slow_down","interval":0}"#),
        (
            200,
            r#"{"access_token":"gho_abc","token_type":"bearer","scope":"repo"}"#,
        ),
    ]);
    assert_eq!(token.unwrap(), "gho_abc");
    let requests = mock.requests.lock().unwrap();
    assert!(requests[0].starts_with("POST /login/device/code\n"));
    assert!(requests[0].contains("client_id=test-client") && requests[0].contains("scope=repo"));
    assert!(requests[3].contains("device_code=dc"));
}

#[test]
fn device_flow_reports_expiry_and_refusal() {
    let (_, expired) = sign_in(&[(200, CODE), (200, r#"{"error":"expired_token"}"#)]);
    assert!(expired.unwrap_err().to_string().contains("expired"));
    let (_, timed_out) = sign_in(&[(200, &CODE.replace("900", "0"))]);
    assert!(timed_out.unwrap_err().to_string().contains("expired"));
    let (_, denied) = sign_in(&[(200, CODE), (200, r#"{"error":"access_denied"}"#)]);
    assert!(denied.unwrap_err().to_string().contains("declined"));
    let (_, disabled) = sign_in(&[
        (200, CODE),
        (
            200,
            r#"{"error":"device_flow_disabled","error_description":"Device Flow must be explicitly enabled"}"#,
        ),
    ]);
    assert!(
        disabled
            .unwrap_err()
            .to_string()
            .contains("explicitly enabled")
    );
}

#[test]
fn no_client_id_means_no_sign_in() {
    let github = GitHub::with_urls("http://127.0.0.1:9", "http://127.0.0.1:9", "");
    assert!(matches!(github.request_code(), Err(Error::GitHub(_))));
}

#[test]
fn created_repos_are_private() {
    let mock = mock(&[(
        201,
        r#"{"full_name":"me/dotfiles","ssh_url":"git@github.com:me/dotfiles.git","clone_url":"https://github.com/me/dotfiles.git","private":true}"#,
    )]);
    let repo = github(&mock).create_repo("gho_abc", "dotfiles").unwrap();
    assert_eq!(repo.full_name, "me/dotfiles");
    let requests = mock.requests.lock().unwrap();
    let (target, body) = requests[0].split_once('\n').unwrap();
    assert_eq!(target, "POST /user/repos");
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({ "name": "dotfiles", "private": true })
    );
}

#[test]
fn api_errors_carry_githubs_message() {
    let mock = mock(&[(
        422,
        r#"{"message":"Repository creation failed.","errors":[{"message":"name already exists on this account"}]}"#,
    )]);
    let err = github(&mock)
        .create_repo("gho_abc", "dotfiles")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "GitHub: name already exists on this account (422 Unprocessable Entity)"
    );
}

/// A local failure must not leave a GitHub repo behind that makes every
/// retry fail with "name already exists".
#[test]
fn a_new_github_repo_waits_for_the_local_repo() {
    crate::git::tests::isolate();
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::from_vars(|name| (name == "HOME").then(|| home.path().into())).unwrap();
    let init = |mock: &Mock| {
        init_new(
            &paths,
            &github(mock),
            "gho_abc",
            "dotfiles",
            &Transport::Ssh,
        )
    };
    std::fs::create_dir_all(paths.host_config().parent().unwrap()).unwrap();
    std::fs::write(paths.host_config(), "id = 1\n").unwrap();
    let created = mock(&[(
        201,
        r#"{"full_name":"me/dotfiles","ssh_url":"git@github.com:me/dotfiles.git","clone_url":"https://github.com/me/dotfiles.git","private":true}"#,
    )]);
    assert!(init(&created).is_err());
    assert!(created.requests.lock().unwrap().is_empty());

    // GitHub refusing leaves no local repo in the way either.
    std::fs::write(paths.host_config(), "id = \"a\"\n").unwrap();
    let refused = mock(&[(422, r#"{"message":"Repository creation failed."}"#)]);
    assert!(init(&refused).is_err());
    assert!(!paths.repo().exists());

    let outcome = init(&created).unwrap();
    let url = git::run(&outcome.repo, &["remote", "get-url", "origin"]).unwrap();
    assert_eq!(url, "git@github.com:me/dotfiles.git\n");
}

#[test]
fn credential_replies_only_for_github_over_https() {
    let request = "protocol=https\nhost=github.com\n";
    assert_eq!(
        credential_reply(request, Some("gho_abc")),
        "username=x-access-token\npassword=gho_abc\n"
    );
    assert_eq!(credential_reply(request, None), "");
    assert_eq!(
        credential_reply("protocol=https\nhost=gitlab.com\n", Some("gho_abc")),
        ""
    );
}

#[test]
fn helper_lives_in_the_repo_config_only() {
    crate::git::tests::isolate();
    let dir = tempfile::tempdir().unwrap();
    git::run(dir.path(), &["init", "--quiet"]).unwrap();
    configure_https(dir.path(), "/opt/it's here/rosamta").unwrap();
    configure_https(dir.path(), "/usr/bin/rosamta").unwrap();
    let values = git::run(dir.path(), &["config", "--local", "--get-all", HELPER_KEY]).unwrap();
    assert_eq!(values, "\n!'/usr/bin/rosamta' git-credential\n");
    assert!(remove_helper(dir.path()).unwrap());
    assert!(!remove_helper(dir.path()).unwrap());
    assert_eq!(
        helper_command("/opt/it's here/rosamta"),
        r"!'/opt/it'\''s here/rosamta' git-credential"
    );
}

/// Reaches the real GitHub over TLS with the OS trust store.
#[test]
#[ignore = "needs the network"]
fn tls_reaches_github() {
    let err = GitHub::default().login("not-a-token").unwrap_err();
    assert!(err.to_string().contains("(401 Unauthorized)"), "{err}");
}
