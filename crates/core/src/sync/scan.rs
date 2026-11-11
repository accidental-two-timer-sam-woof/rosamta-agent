//! The secret check before a push: every line the push would add to the remote.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use crate::adopt::Allowed;
use crate::error::Result;
use crate::git;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    /// Empty for changes not committed yet.
    pub commit: String,
    /// Relative to the repo.
    pub file: String,
    pub line: usize,
    pub rule: &'static str,
    /// What `allow.toml` lists to let this line through.
    pub hash: String,
}

/// Plain patches with zero context, whatever the user's diff settings;
/// files git would call binary included.
const PATCH: [&str; 8] = [
    "--text",
    "--unified=0",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--no-renames",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

const ARMOR: &str = "-----BEGIN AGE ENCRYPTED FILE-----";
const ARMOR_END: &str = "-----END AGE ENCRYPTED FILE-----";

/// An encrypted dotfile: one armored block and nothing else.
fn armored(text: &str) -> bool {
    text.starts_with(ARMOR)
        && text.trim_end().ends_with(ARMOR_END)
        && text.matches(ARMOR).count() == 1
}

/// Lines added by each commit in `range` (arguments to `git log`), with
/// their line numbers in that commit's version of the file.
pub(crate) fn scan(repo: &Path, range: &[&str]) -> Result<Vec<Hit>> {
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "log",
        "--patch",
        // The first commit's diff too, whatever `log.showRoot` says.
        "--root",
        "--reverse",
        "--format=commit %H",
        "--no-show-signature",
        // A merge can add lines of its own.
        "--diff-merges=first-parent",
    ];
    args.extend(PATCH);
    args.extend_from_slice(range);
    let mut revs = vec!["rev-list"];
    revs.extend_from_slice(range);
    let commits = git::run(repo, &revs)?;
    let hosts = host_ids(repo, commits.lines().map(|sha| format!("{sha}:")));
    let allowed = Allowed::load(repo)?;
    Ok(unencrypted(
        repo,
        check(&added_lines(&git::run(repo, &args)?), &hosts, &allowed),
    ))
}

/// Lines added by the staged changes.
pub(crate) fn staged(repo: &Path, paths: &[&str]) -> Result<Vec<Hit>> {
    let mut args = vec!["-c", "core.quotePath=false", "diff", "--cached"];
    args.extend(PATCH);
    if !paths.is_empty() {
        args.push("--");
        args.extend_from_slice(paths);
    }
    let hosts = host_ids(repo, [":".to_string()]);
    let allowed = Allowed::load(repo)?;
    Ok(unencrypted(
        repo,
        check(&added_lines(&git::run(repo, &args)?), &hosts, &allowed),
    ))
}

/// Host IDs come from hostnames, which can look random (`Mac-1762…`) but
/// are never secrets: those registered now and in each of `snapshots` (a
/// `<commit>:` or the index `:`), so a host renamed since still counts.
fn host_ids(repo: &Path, snapshots: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut hosts = crate::repo::read_manifest(repo)
        .map(|manifest| manifest.hosts)
        .unwrap_or_default();
    for snapshot in snapshots {
        let blob = format!("{snapshot}{}", crate::repo::MANIFEST);
        if let Ok(text) = git::run(repo, &["cat-file", "blob", &blob])
            && let Ok(manifest) = crate::validate::parse::<crate::config::RepoManifest>(&text)
        {
            hosts.extend(manifest.hosts);
        }
    }
    hosts.sort();
    hosts.dedup();
    hosts
}

/// Hits outside encrypted dotfiles. A changed ciphertext adds only base64
/// lines, so the file itself is checked: in the hit's commit, or the index.
fn unencrypted(repo: &Path, hits: Vec<Hit>) -> Vec<Hit> {
    hits.into_iter()
        .filter(|hit| {
            let blob = format!("{}:{}", hit.commit, hit.file);
            !git::run(repo, &["cat-file", "blob", &blob]).is_ok_and(|text| armored(&text))
        })
        .collect()
}

/// One file in one commit, and its added lines.
struct Added {
    commit: String,
    file: String,
    lines: Vec<(usize, String)>,
}

fn added_lines(log: &str) -> Vec<Added> {
    let mut out: Vec<Added> = Vec::new();
    let mut commit = String::new();
    let mut file: Option<String> = None;
    let mut next = 0;
    let mut in_hunk = false;
    for line in log.lines() {
        // Diff lines start with `+`, `-` or `\`, so these headers are unambiguous.
        if let Some(sha) = line.strip_prefix("commit ") {
            commit = sha.to_string();
            in_hunk = false;
        } else if line.starts_with("diff --git ") {
            file = None;
            in_hunk = false;
        } else if !in_hunk && let Some(path) = line.strip_prefix("+++ ") {
            file = path
                .trim_matches('"')
                .strip_prefix("b/")
                .map(str::to_string);
        } else if let Some(header) = line.strip_prefix("@@ ") {
            in_hunk = true;
            next = header
                .split_whitespace()
                .find_map(|part| part.strip_prefix('+'))
                .and_then(|range| range.split(',').next()?.parse().ok())
                .unwrap_or(0);
        } else if in_hunk && let (Some(text), Some(file)) = (line.strip_prefix('+'), &file) {
            match out.last_mut() {
                Some(last) if last.commit == commit && &last.file == file => {
                    last.lines.push((next, text.to_string()))
                }
                _ => out.push(Added {
                    commit: commit.clone(),
                    file: file.clone(),
                    lines: vec![(next, text.to_string())],
                }),
            }
            next += 1;
        }
    }
    out
}

/// `text` with the registered host IDs taken out of `hosts = [...]` lists
/// and of the `# Adopted from <file> on <host>.` line adopted modules start
/// with; the same words anywhere else are still checked.
fn without_host_ids(text: &str, hosts: &[String]) -> String {
    static LIST: OnceLock<Regex> = OnceLock::new();
    static ADOPTED: OnceLock<Regex> = OnceLock::new();
    let list = LIST.get_or_init(|| Regex::new(r"\bhosts\s*=\s*\[[^\]]*\]").unwrap());
    let adopted =
        ADOPTED.get_or_init(|| Regex::new(r"(?m)^(# Adopted from .+ on )(\S+)\.$").unwrap());
    let text = list.replace_all(text, |caps: &regex::Captures| {
        hosts.iter().fold(caps[0].to_string(), |list, host| {
            list.replace(&format!("\"{host}\""), "\"\"")
        })
    });
    adopted
        .replace_all(&text, |caps: &regex::Captures| {
            match hosts.contains(&caps[2].to_string()) {
                true => format!("{}.", &caps[1]),
                false => caps[0].to_string(),
            }
        })
        .into_owned()
}

/// Check each file's added lines together, so rules spanning lines (private
/// keys) still match; report the first line that matches on its own. What
/// `allowed` lists is left out.
fn check(files: &[Added], hosts: &[String], allowed: &Allowed) -> Vec<Hit> {
    let matched = |text: &str| allowed.rule(&without_host_ids(text, hosts));
    files
        .iter()
        .filter(|added| !allowed.file(&added.file))
        .filter_map(|added| {
            let text: Vec<&str> = added.lines.iter().map(|(_, text)| text.as_str()).collect();
            let text = text.join("\n");
            // A new encrypted dotfile is ciphertext, which looks like a key to
            // the rules; an armored block in a plain file is not one.
            if added.lines.first().is_some_and(|(line, _)| *line == 1) && armored(&text) {
                return None;
            }
            let rule = matched(&text)?;
            let (line, text) = added
                .lines
                .iter()
                .find(|(_, text)| matched(text).is_some())
                .unwrap_or(&added.lines[0]);
            Some(Hit {
                commit: added.commit.clone(),
                file: added.file.clone(),
                line: *line,
                rule,
                hash: Allowed::hash(text),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adopt::secret_rule;

    #[test]
    fn changed_ciphertext_is_not_a_secret() {
        crate::git::tests::isolate();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let armored = |body: &str| format!("{ARMOR}\n{body}\n-----END AGE ENCRYPTED FILE-----\n");
        let file = repo.join("netrc");
        crate::git::run(repo, &["init", "--quiet"]).unwrap();
        std::fs::write(
            &file,
            armored("YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBhYmNk"),
        )
        .unwrap();
        crate::git::run(repo, &["add", "netrc"]).unwrap();
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "v1"]).unwrap();
        // Saved again: only the base64 body changes, which looks like a key.
        let body = "Qm9ndXNLZXlNYXRlcmlhbDlYcVpQd0V0UjJ0eUJrN0xtTnFQcVJzVHVWd1h5WjAxMjM0NTY3ODk=";
        std::fs::write(&file, armored(body)).unwrap();
        crate::git::run(repo, &["add", "netrc"]).unwrap();
        assert!(
            secret_rule(body).is_some(),
            "the body must look like a secret"
        );
        assert_eq!(staged(repo, &[]).unwrap(), []);
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "v2"]).unwrap();
        assert_eq!(scan(repo, &["HEAD~1..HEAD"]).unwrap(), []);

        // A plain file holding an armored block is still scanned.
        let notes = format!(
            "# notes\n{}export GITHUB_TOKEN=ghp_0123456789abcdefghijABCDEFGHIJ012345\n",
            armored("YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBhYmNk")
        );
        std::fs::write(repo.join("notes"), notes).unwrap();
        crate::git::run(repo, &["add", "notes"]).unwrap();
        let hits = staged(repo, &[]).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].file, "notes");
        crate::git::run(repo, &["reset", "--quiet"]).unwrap();

        // So is one that starts with an armored block, new or added to.
        let token = "export GITHUB_TOKEN=ghp_0123456789abcdefghijABCDEFGHIJ012345\n";
        let block = armored("YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBhYmNk");
        std::fs::write(repo.join("tail"), format!("{block}{token}")).unwrap();
        crate::git::run(repo, &["add", "tail"]).unwrap();
        assert_eq!(staged(repo, &["tail"]).unwrap().len(), 1);
        std::fs::write(repo.join("tail"), &block).unwrap();
        crate::git::run(repo, &["add", "tail"]).unwrap();
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "v3"]).unwrap();
        std::fs::write(repo.join("tail"), format!("{block}{token}")).unwrap();
        crate::git::run(repo, &["add", "tail"]).unwrap();
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "v4"]).unwrap();
        let hits = scan(repo, &["HEAD~1..HEAD"]).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
    }

    /// The first commit is scanned whatever the user's `log.showRoot`.
    #[test]
    fn the_first_commit_is_scanned() {
        crate::git::tests::isolate();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        crate::git::run(repo, &["init", "--quiet"]).unwrap();
        crate::git::run(repo, &["config", "log.showRoot", "false"]).unwrap();
        let token = "export GITHUB_TOKEN=ghp_0123456789abcdefghijABCDEFGHIJ012345\n";
        std::fs::write(repo.join("env"), token).unwrap();
        crate::git::run(repo, &["add", "env"]).unwrap();
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "root"]).unwrap();
        assert_eq!(scan(repo, &["HEAD"]).unwrap().len(), 1);
    }

    /// A host renamed since a commit was made still counts for that commit.
    #[test]
    fn a_renamed_host_still_counts() {
        crate::git::tests::isolate();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let host = "mac-zk3q9lmp2vr8ty6wb1nc4xj7";
        crate::git::run(repo, &["init", "--quiet"]).unwrap();
        let manifest = |host: &str| format!("format = 1\nhosts = [\"{host}\"]\n");
        std::fs::write(repo.join("rosamta.toml"), manifest(host)).unwrap();
        crate::git::run(repo, &["add", "--all"]).unwrap();
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "join"]).unwrap();
        std::fs::write(repo.join("rosamta.toml"), manifest("mbp")).unwrap();
        crate::git::run(repo, &["add", "--all"]).unwrap();
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "rename"]).unwrap();
        assert_eq!(scan(repo, &["HEAD"]).unwrap(), []);
        // The index, not the working tree, for staged changes.
        std::fs::write(repo.join("rosamta.toml"), manifest(host)).unwrap();
        crate::git::run(repo, &["add", "--all"]).unwrap();
        std::fs::write(repo.join("rosamta.toml"), manifest("mbp")).unwrap();
        assert_eq!(staged(repo, &[]).unwrap(), []);
    }

    /// A host named by a random-looking hostname is not a secret.
    #[test]
    fn host_ids_are_not_secrets() {
        crate::git::tests::isolate();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let host = "mac-zk3q9lmp2vr8ty6wb1nc4xj7";
        assert!(secret_rule(&format!("hosts = [\"{host}\"]")).is_some());
        crate::git::run(repo, &["init", "--quiet"]).unwrap();
        let manifest = format!("format = 1\nhosts = [\"{host}\"]\n");
        std::fs::write(repo.join("rosamta.toml"), manifest).unwrap();
        std::fs::create_dir(repo.join("modules")).unwrap();
        let module = format!(
            "# Adopted from ~/.zshrc on {host}.\n[when]\nhosts = [\"{host}\"]\n\n[env]\nA = \"1\"\n"
        );
        std::fs::write(repo.join("modules/a.toml"), module).unwrap();
        crate::git::run(repo, &["add", "--all"]).unwrap();
        assert_eq!(staged(repo, &[]).unwrap(), []);

        std::fs::write(
            repo.join("modules/b.toml"),
            "[env]\nB = \"Zk3q9LmP2vR8tY6wB1nC4xJ7\"\n",
        )
        .unwrap();
        // The same word as a password is still one.
        let password = format!("[env]\nDB_PASSWORD = \"{host}\"\n");
        std::fs::write(repo.join("modules/c.toml"), password).unwrap();
        crate::git::run(repo, &["add", "--all"]).unwrap();
        let hits = staged(repo, &[]).unwrap();
        let files: Vec<&str> = hits.iter().map(|hit| hit.file.as_str()).collect();
        assert_eq!(files, ["modules/b.toml", "modules/c.toml"], "{hits:?}");
    }

    const LOG: &str = "commit 1111
diff --git a/modules/env.toml b/modules/env.toml
new file mode 100644
--- /dev/null
+++ b/modules/env.toml
@@ -0,0 +1,3 @@
+[env]
+EDITOR = \"nvim\"
+GH = \"ghp_0123456789abcdefghijABCDEFGHIJ012345\"
commit 2222
diff --git a/modules/ll.toml b/modules/ll.toml
--- a/modules/ll.toml
+++ b/modules/ll.toml
@@ -4 +4,2 @@ x
-old
++++ not a header
+ll = \"ls -lh\"
diff --git a/gone.toml b/gone.toml
deleted file mode 100644
--- a/gone.toml
+++ /dev/null
@@ -1 +0,0 @@
-SECRET_TOKEN=abc
";

    #[test]
    fn added_lines_carry_new_line_numbers() {
        let added = added_lines(LOG);
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].commit, "1111");
        assert_eq!(
            added[0].lines[2],
            (
                3,
                "GH = \"ghp_0123456789abcdefghijABCDEFGHIJ012345\"".into()
            )
        );
        assert_eq!(added[1].file, "modules/ll.toml");
        assert_eq!(
            added[1].lines,
            [
                (4, "+++ not a header".into()),
                (5, "ll = \"ls -lh\"".into())
            ]
        );
    }

    /// Files git calls binary and lines a merge adds are scanned too.
    #[test]
    fn binary_files_and_merges_are_scanned() {
        git::tests::isolate();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let token = "GH = \"ghp_0123456789abcdefghijABCDEFGHIJ012345\"\n";
        let commit = |message: &str| {
            git::run(repo, &["add", "--all"]).unwrap();
            git::run(repo, &["commit", "--quiet", "--message", message]).unwrap();
        };
        git::run(repo, &["init", "--quiet", "--initial-branch", "main"]).unwrap();
        std::fs::write(repo.join(".gitattributes"), "*.bin -diff\n").unwrap();
        commit("base");
        std::fs::write(repo.join("marked.bin"), token).unwrap();
        std::fs::write(repo.join("nul.toml"), format!("a\0b\n{token}")).unwrap();
        let files =
            |hits: Vec<Hit>| -> Vec<String> { hits.into_iter().map(|hit| hit.file).collect() };
        git::run(repo, &["add", "--all"]).unwrap();
        assert_eq!(
            files(staged(repo, &[]).unwrap()),
            ["marked.bin", "nul.toml"]
        );
        commit("binary");
        assert_eq!(
            files(scan(repo, &["HEAD~1..HEAD"]).unwrap()),
            ["marked.bin", "nul.toml"]
        );

        git::run(repo, &["checkout", "--quiet", "-b", "side"]).unwrap();
        std::fs::write(repo.join("side.toml"), "x = 1\n").unwrap();
        commit("side");
        git::run(repo, &["checkout", "--quiet", "main"]).unwrap();
        git::run(
            repo,
            &["merge", "--quiet", "--no-ff", "--no-commit", "side"],
        )
        .unwrap();
        std::fs::write(repo.join("merge.toml"), token).unwrap();
        commit("merge");
        assert_eq!(
            files(scan(repo, &["HEAD~1..HEAD"]).unwrap()),
            ["merge.toml"]
        );
    }

    #[test]
    fn hits_name_file_line_and_rule() {
        assert_eq!(
            check(&added_lines(LOG), &[], &Allowed::default()),
            [Hit {
                commit: "1111".into(),
                file: "modules/env.toml".into(),
                line: 3,
                rule: "github-pat",
                hash: Allowed::hash("GH = \"ghp_0123456789abcdefghijABCDEFGHIJ012345\""),
            }]
        );
    }
}
