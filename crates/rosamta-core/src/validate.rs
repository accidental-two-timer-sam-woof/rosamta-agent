//! `rsmt validate`: check every TOML file in the repo without changing any.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::{CURRENT_FORMAT, Module, RepoManifest};
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
        let text = read(&path)?;
        report.file(rel.clone());
        names
            .entry(repo::module_name(&path))
            .or_default()
            .push(rel.clone());
        match parse::<Module>(&text) {
            Ok(module) => {
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

    // Their fields belong to later changes; for now only the syntax is checked.
    for name in ["dotfiles.toml", "packages.toml"] {
        let path = repo_dir.join(name);
        if path.is_file() {
            let issues = parse::<toml::Table>(&read(&path)?).err();
            report.file(PathBuf::from(name)).errors.extend(issues);
        }
    }
    Ok(report)
}

fn check_manifest(repo_dir: &Path, report: &mut Report) -> Result<Option<RepoManifest>> {
    let path = repo_dir.join(repo::MANIFEST);
    if !path.is_file() {
        return Err(Error::Usage(format!(
            "{} is not a rosamta repo: missing {}",
            repo_dir.display(),
            repo::MANIFEST
        )));
    }
    let file = report.file(PathBuf::from(repo::MANIFEST));
    match parse::<RepoManifest>(&read(&path)?) {
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

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(Error::read(path))
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
