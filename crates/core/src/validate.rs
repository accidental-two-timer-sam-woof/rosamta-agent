//! `rsmt validate`: check every TOML file in the repo without changing any.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::{CURRENT_FORMAT, DotfileManifest, Module, RepoManifest};
use crate::error::{Error, Result};
use crate::repo;

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub files: Vec<FileReport>,
}

#[derive(Debug, Serialize)]
pub struct FileReport {
    /// Relative to the repo root.
    pub path: PathBuf,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

impl Issue {
    fn at(message: impl Into<String>) -> Self {
        Self {
            line: None,
            column: None,
            message: message.into(),
        }
    }
}

impl Report {
    pub fn is_valid(&self) -> bool {
        self.files.iter().all(|file| file.errors.is_empty())
    }

    fn file(&mut self, path: PathBuf) -> &mut FileReport {
        let index = match self.files.iter().position(|file| file.path == path) {
            Some(index) => index,
            None => {
                self.files.push(FileReport {
                    path,
                    errors: Vec::new(),
                    warnings: Vec::new(),
                });
                self.files.len() - 1
            }
        };
        &mut self.files[index]
    }
}

/// Parse `text` as `T`, turning a failure into an issue with a line number.
pub(crate) fn parse<T: DeserializeOwned>(text: &str) -> Result<T, Issue> {
    toml::from_str(text).map_err(|err| {
        let (line, column) = err
            .span()
            .map(|span| line_col(text, span.start))
            .map_or((None, None), |(line, column)| (Some(line), Some(column)));
        Issue {
            line,
            column,
            message: err.message().trim().to_string(),
        }
    })
}

fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    (line, column)
}

pub fn validate(repo_dir: &Path) -> Result<Report> {
    let mut report = Report::default();
    let manifest = check_manifest(repo_dir, &mut report)?;
    let hosts = manifest.map(|manifest| manifest.hosts).unwrap_or_default();

    let mut names: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in repo::module_files(repo_dir)? {
        let rel = relative(repo_dir, &path);
        report.file(rel.clone());
        names
            .entry(repo::module_name(&path))
            .or_default()
            .push(rel.clone());
        match read(&path)?.and_then(|text| parse::<Module>(&text)) {
            Ok(module) => {
                let problems = crate::render::check(&module).into_iter().map(Issue::at);
                report.file(rel.clone()).errors.extend(problems);
                for host in module
                    .when
                    .hosts
                    .iter()
                    .filter(|host| !hosts.contains(host))
                {
                    report.file(rel.clone()).warnings.push(Issue::at(format!(
                        "host ID `{host}` is not in the host list in rosamta.toml"
                    )));
                }
            }
            Err(issue) => report.file(rel).errors.push(issue),
        }
    }
    for (name, paths) in names.iter().filter(|(_, paths)| paths.len() > 1) {
        let all = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        for path in paths {
            report.file(path.clone()).errors.push(Issue::at(format!(
                "module name `{name}` is used by more than one file: {all}"
            )));
        }
    }

    let mut dotfiles = DotfileManifest::default();
    let path = repo_dir.join(crate::dotfiles::MANIFEST);
    if path.is_file() {
        let rel = PathBuf::from(crate::dotfiles::MANIFEST);
        match read(&path)?.and_then(|text| parse::<DotfileManifest>(&text)) {
            Ok(manifest) => {
                dotfiles = manifest.clone();
                let problems = crate::dotfiles::check(&manifest, repo_dir);
                report
                    .file(rel.clone())
                    .errors
                    .extend(problems.into_iter().map(Issue::at));
                for entry in &manifest.dotfile {
                    for host in entry.when.hosts.iter().filter(|host| !hosts.contains(host)) {
                        report.file(rel.clone()).warnings.push(Issue::at(format!(
                            "dotfile `{}`: host ID `{host}` is not in the host list in rosamta.toml",
                            entry.path
                        )));
                    }
                }
            }
            Err(issue) => report.file(rel).errors.push(issue),
        }
    }
    let path = repo_dir.join(crate::externals::MANIFEST);
    if path.is_file() {
        let rel = PathBuf::from(crate::externals::MANIFEST);
        match read(&path)?.and_then(|text| parse::<crate::externals::ExternalManifest>(&text)) {
            Ok(manifest) => {
                let problems = crate::externals::check(&manifest, &dotfiles);
                let file = report.file(rel);
                file.errors.extend(problems.into_iter().map(Issue::at));
                for entry in &manifest.external {
                    for host in entry.when.hosts.iter().filter(|host| !hosts.contains(host)) {
                        file.warnings.push(Issue::at(format!(
                            "external `{}`: host ID `{host}` is not in the host list in rosamta.toml",
                            entry.path
                        )));
                    }
                }
            }
            Err(issue) => report.file(rel).errors.push(issue),
        }
    }
    let path = repo_dir.join(crate::scripts::MANIFEST);
    if path.is_file() {
        let rel = PathBuf::from(crate::scripts::MANIFEST);
        match read(&path)?.and_then(|text| parse::<crate::scripts::ScriptManifest>(&text)) {
            Ok(manifest) => {
                let problems = crate::scripts::check(&manifest, repo_dir);
                let file = report.file(rel);
                file.errors.extend(problems.into_iter().map(Issue::at));
                for entry in &manifest.script {
                    for host in entry.when.hosts.iter().filter(|host| !hosts.contains(host)) {
                        file.warnings.push(Issue::at(format!(
                            "script `{}`: host ID `{host}` is not in the host list in rosamta.toml",
                            entry.source
                        )));
                    }
                }
            }
            Err(issue) => report.file(rel).errors.push(issue),
        }
    }
    let path = repo_dir.join(crate::secrets::MANIFEST);
    if path.is_file() {
        let rel = PathBuf::from(crate::secrets::MANIFEST);
        match read(&path)?.and_then(|text| parse::<crate::secrets::SecretCommands>(&text)) {
            Ok(commands) => {
                let problems = crate::secrets::check(&commands);
                report
                    .file(rel)
                    .errors
                    .extend(problems.into_iter().map(Issue::at));
            }
            Err(issue) => report.file(rel).errors.push(issue),
        }
    }
    let path = repo_dir.join(crate::adopt::Allowed::MANIFEST);
    if path.is_file() {
        let issues = read(&path)?
            .and_then(|text| parse::<crate::adopt::Allowed>(&text))
            .err();
        report
            .file(PathBuf::from(crate::adopt::Allowed::MANIFEST))
            .errors
            .extend(issues);
    }
    // Its fields belong to a later change; for now only the syntax is checked.
    let path = repo_dir.join("packages.toml");
    if path.is_file() {
        let issues = read(&path)?
            .and_then(|text| parse::<toml::Table>(&text))
            .err();
        report
            .file(PathBuf::from("packages.toml"))
            .errors
            .extend(issues);
    }
    Ok(report)
}

fn check_manifest(repo_dir: &Path, report: &mut Report) -> Result<Option<RepoManifest>> {
    let path = repo_dir.join(repo::MANIFEST);
    if !path.try_exists().map_err(Error::read(&path))? {
        return Err(Error::Usage(format!(
            "{} is not a rosamta repo: missing {}",
            repo_dir.display(),
            repo::MANIFEST
        )));
    }
    let file = report.file(PathBuf::from(repo::MANIFEST));
    match read(&path)?.and_then(|text| parse::<RepoManifest>(&text)) {
        Ok(manifest) => {
            repo::ensure_supported(manifest.format)?;
            if manifest.format == 0 {
                file.errors
                    .push(Issue::at("format must be a positive integer"));
            } else if manifest.format < CURRENT_FORMAT {
                file.warnings.push(Issue::at(format!(
                    "format {} is older than {CURRENT_FORMAT}; the next change to the repo migrates it",
                    manifest.format
                )));
            }
            Ok(Some(manifest))
        }
        Err(issue) => {
            file.errors.push(issue);
            Ok(None)
        }
    }
}

/// The text of `path`; text that is not UTF-8 is an issue in the file.
fn read(path: &Path) -> Result<Result<String, Issue>> {
    let bytes = std::fs::read(path).map_err(Error::read(path))?;
    Ok(String::from_utf8(bytes).map_err(|err| {
        let valid = err.utf8_error().valid_up_to();
        let (line, column) = line_col(&String::from_utf8_lossy(&err.as_bytes()[..valid]), valid);
        Issue {
            line: Some(line),
            column: Some(column),
            message: "not valid UTF-8".into(),
        }
    }))
}

fn relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn errors<'a>(report: &'a Report, path: &str) -> &'a [Issue] {
        &report
            .files
            .iter()
            .find(|file| file.path == Path::new(path))
            .unwrap()
            .errors
    }

    #[test]
    fn minimal_repo_is_valid() {
        let dir = repo(&[("rosamta.toml", "format = 1\n")]);
        let report = validate(dir.path()).unwrap();
        assert!(report.is_valid());
        assert_eq!(report.files.len(), 1);
    }

    #[test]
    fn unknown_field_reports_line() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            (
                "modules/bun.toml",
                "# bun\nenabled = true\nphse = \"tools\"\n",
            ),
        ]);
        let report = validate(dir.path()).unwrap();
        let issue = &errors(&report, "modules/bun.toml")[0];
        assert_eq!(issue.line, Some(3));
        assert!(issue.message.contains("phse"), "{issue:?}");
    }

    #[test]
    fn duplicate_names_list_both_files() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            ("modules/bun.toml", ""),
            ("modules/adopted/bun.toml", ""),
        ]);
        let report = validate(dir.path()).unwrap();
        assert!(!report.is_valid());
        for path in ["modules/bun.toml", "modules/adopted/bun.toml"] {
            let message = &errors(&report, path)[0].message;
            assert!(
                message.contains("modules/bun.toml")
                    && message.contains("modules/adopted/bun.toml"),
                "{message}"
            );
        }
    }

    #[test]
    fn editor_lock_and_hidden_files_are_not_modules() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            ("modules/bun.toml", ""),
            ("modules/.bun.toml", "broken ="),
        ]);
        // Emacs's lock file: a dangling symlink.
        std::os::unix::fs::symlink("me@host.1234:1", dir.path().join("modules/.#bun.toml"))
            .unwrap();
        std::os::unix::fs::symlink("gone", dir.path().join("modules/x.toml")).unwrap();
        let report = validate(dir.path()).unwrap();
        assert!(report.is_valid());
        assert_eq!(
            repo::module_files(dir.path()).unwrap(),
            [dir.path().join("modules/bun.toml")]
        );
    }

    #[test]
    fn non_utf8_file_is_a_file_error() {
        let dir = repo(&[("rosamta.toml", "format = 1\n"), ("modules/ok.toml", "")]);
        fs::write(
            dir.path().join("modules/bad.toml"),
            b"# ok\nphase = \"\xff\"\n",
        )
        .unwrap();
        let report = validate(dir.path()).unwrap();
        assert!(!report.is_valid());
        let issue = &errors(&report, "modules/bad.toml")[0];
        assert_eq!((issue.line, issue.column), (Some(2), Some(10)));
        assert!(issue.message.contains("UTF-8"), "{issue:?}");
        assert!(errors(&report, "modules/ok.toml").is_empty());
    }

    #[test]
    fn bad_os_lists_allowed_values() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            ("modules/x.toml", "[when]\nos = [\"darwin\"]\n"),
        ]);
        let report = validate(dir.path()).unwrap();
        let message = &errors(&report, "modules/x.toml")[0].message;
        assert!(
            message.contains("macos") && message.contains("linux"),
            "{message}"
        );
    }

    #[test]
    fn bad_arch_is_an_error() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            ("modules/x.toml", "[when]\narch = [\"arm64\"]\n"),
        ]);
        let report = validate(dir.path()).unwrap();
        let message = &errors(&report, "modules/x.toml")[0].message;
        assert!(
            message.contains("x86_64") && message.contains("aarch64"),
            "{message}"
        );
    }

    #[test]
    fn unknown_host_is_a_warning() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\nhosts = [\"mbp\"]\n"),
            ("modules/x.toml", "[when]\nhosts = [\"new-laptop\"]\n"),
        ]);
        let report = validate(dir.path()).unwrap();
        assert!(report.is_valid());
        let file = report
            .files
            .iter()
            .find(|file| file.path == Path::new("modules/x.toml"))
            .unwrap();
        assert!(file.warnings[0].message.contains("new-laptop"));
    }

    #[test]
    fn bad_names_are_errors() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            ("modules/x.toml", "[env]\n\"MY-VAR\" = \"1\"\n"),
        ]);
        let report = validate(dir.path()).unwrap();
        assert!(
            errors(&report, "modules/x.toml")[0]
                .message
                .contains("MY-VAR")
        );
    }

    #[test]
    fn newer_format_is_refused() {
        let dir = repo(&[("rosamta.toml", "format = 99\n")]);
        assert!(validate(dir.path()).unwrap_err().is_usage());
    }

    #[test]
    fn not_a_repo() {
        let dir = repo(&[]);
        assert!(validate(dir.path()).unwrap_err().is_usage());
    }

    #[test]
    fn other_files_are_syntax_checked() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            ("dotfiles.toml", "[broken\n"),
        ]);
        let report = validate(dir.path()).unwrap();
        assert_eq!(errors(&report, "dotfiles.toml")[0].line, Some(1));
    }

    #[test]
    fn validate_writes_nothing() {
        let dir = repo(&[
            ("rosamta.toml", "format = 1\n"),
            ("modules/a.toml", "phase = \"env\"\n"),
        ]);
        let before = fs::read_to_string(dir.path().join("modules/a.toml")).unwrap();
        validate(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("modules/a.toml")).unwrap(),
            before
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
