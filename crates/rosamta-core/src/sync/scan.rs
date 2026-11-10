//! The secret check before a push: every line the push would add to the remote.

use std::path::Path;

use serde::Serialize;

use crate::adopt::secret_rule;
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

/// Lines added by each commit in `range` (arguments to `git log`), with
/// their line numbers in that commit's version of the file.
pub(crate) fn scan(repo: &Path, range: &[&str]) -> Result<Vec<Hit>> {
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "log",
        "--patch",
        "--reverse",
        "--format=commit %H",
        "--no-show-signature",
        // A merge can add lines of its own.
        "--diff-merges=first-parent",
    ];
    args.extend(PATCH);
    args.extend_from_slice(range);
    Ok(unencrypted(
        repo,
        check(&added_lines(&git::run(repo, &args)?)),
    ))
}

/// Lines added by the staged changes.
pub(crate) fn staged(repo: &Path) -> Result<Vec<Hit>> {
    let mut args = vec!["-c", "core.quotePath=false", "diff", "--cached"];
    args.extend(PATCH);
    Ok(unencrypted(
        repo,
        check(&added_lines(&git::run(repo, &args)?)),
    ))
}

/// Hits outside encrypted dotfiles. A changed ciphertext adds only base64
/// lines, so the file itself is checked: in the hit's commit, or the index.
fn unencrypted(repo: &Path, hits: Vec<Hit>) -> Vec<Hit> {
    hits.into_iter()
        .filter(|hit| {
            let blob = format!("{}:{}", hit.commit, hit.file);
            !git::run(repo, &["cat-file", "blob", &blob]).is_ok_and(|text| text.starts_with(ARMOR))
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

/// Check each file's added lines together, so rules spanning lines (private
/// keys) still match; report the first line that matches on its own.
fn check(files: &[Added]) -> Vec<Hit> {
    files
        .iter()
        // Encrypted dotfiles are ciphertext, which looks like a key to the rules.
        .filter(|added| !added.lines.iter().any(|(_, text)| text == ARMOR))
        .filter_map(|added| {
            let text: Vec<&str> = added.lines.iter().map(|(_, text)| text.as_str()).collect();
            let rule = secret_rule(&text.join("\n"))?;
            let line = added
                .lines
                .iter()
                .find(|(_, text)| secret_rule(text).is_some())
                .unwrap_or(&added.lines[0])
                .0;
            Some(Hit {
                commit: added.commit.clone(),
                file: added.file.clone(),
                line,
                rule,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(staged(repo).unwrap(), []);
        crate::git::run(repo, &["commit", "--quiet", "--no-verify", "-m", "v2"]).unwrap();
        assert_eq!(scan(repo, &["HEAD~1..HEAD"]).unwrap(), []);
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
        assert_eq!(files(staged(repo).unwrap()), ["marked.bin", "nul.toml"]);
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
            check(&added_lines(LOG)),
            [Hit {
                commit: "1111".into(),
                file: "modules/env.toml".into(),
                line: 3,
                rule: "github-pat",
            }]
        );
    }
}
