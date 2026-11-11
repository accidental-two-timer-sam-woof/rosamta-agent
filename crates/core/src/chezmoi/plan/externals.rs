//! `.chezmoiexternal` files.

use super::*;

impl Planner<'_> {
    pub(super) fn externals(&mut self, source: &str, target_dir: &str) -> Result<()> {
        let path = self.inputs.root.join(source);
        let text = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
        if text.contains("{{") {
            self.plan.aside.push(source.to_string());
            self.report(
                source,
                None,
                Outcome::Manual,
                Reason::ExternalTemplate,
                None,
            );
            return Ok(());
        }
        let name = source.trim_end_matches(".tmpl");
        let read = if name.ends_with(".toml") {
            toml::from_str::<toml::Table>(&text)
                .ok()
                .and_then(|table| serde_json::to_value(table).ok())
        } else if name.ends_with(".json") {
            serde_json::from_str(&text).ok()
        } else {
            None
        };
        let Some(Value::Object(entries)) = read else {
            self.plan.aside.push(source.to_string());
            self.report(source, None, Outcome::Manual, Reason::Format, None);
            return Ok(());
        };
        let existing = crate::externals::load(&self.inputs.ctx.paths.repo())?;
        for (key, entry) in entries {
            let target = join(target_dir, key.trim_matches('/'));
            let item = format!("{source}: {key}");
            if existing
                .external
                .iter()
                .any(|external| external.path == target)
            {
                self.report(&item, Some(&target), Outcome::Skipped, Reason::Exists, None);
                continue;
            }
            let scope = self.scope(&target);
            if scope.platforms == 0 {
                self.report(
                    &item,
                    Some(&target),
                    Outcome::Skipped,
                    Reason::Ignored,
                    None,
                );
                continue;
            }
            let Some(when) = scope.when() else {
                self.report(&item, Some(&target), Outcome::Manual, Reason::Scope, None);
                continue;
            };
            if let Some(external) = self.external(&item, &target, &entry, when.unwrap_or_default())
            {
                self.report(
                    &item,
                    Some(&target),
                    Outcome::Imported,
                    Reason::External,
                    None,
                );
                self.plan.externals.push(external);
            }
        }
        Ok(())
    }

    /// One `.chezmoiexternal` entry, or `None` once reported.
    fn external(
        &mut self,
        item: &str,
        target: &str,
        entry: &Value,
        when: When,
    ) -> Option<External> {
        let kind = entry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let url = entry.get("url").and_then(Value::as_str);
        let allowed: &[&str] = match kind {
            "git-repo" => &["type", "url", "clone", "pull", "refreshPeriod"],
            "archive" => &[
                "type",
                "url",
                "stripComponents",
                "checksum",
                "format",
                "refreshPeriod",
                "exact",
            ],
            _ => {
                self.report(
                    item,
                    Some(target),
                    Outcome::Manual,
                    Reason::ExternalType,
                    Some(kind.into()),
                );
                return None;
            }
        };
        let mut extra: Vec<&str> = entry
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key.as_str())
            .filter(|key| !allowed.contains(key))
            .collect();
        // Arguments that change what is cloned, such as `--recurse-submodules`.
        if clone_args(entry).iter().any(|arg| !harmless(arg)) {
            extra.push("clone.args");
        }
        let checksum = entry.get("checksum").and_then(Value::as_object);
        if checksum.is_some_and(|sums| sums.keys().any(|key| key != "sha256")) {
            extra.push("checksum");
        }
        let format = entry.get("format").and_then(Value::as_str);
        let compressed = |url: &str| {
            [
                ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tbz", ".tar.zst", ".tzst", ".xz", ".bz2",
                ".zst", ".7z", ".rar",
            ]
            .iter()
            .any(|suffix| url.ends_with(suffix))
        };
        if format.is_some_and(|format| !["tar", "tar.gz", "tgz", "zip"].contains(&format))
            || (format.is_none() && kind == "archive" && url.is_some_and(compressed))
        {
            extra.push("format");
        }
        let Some(url) = url else {
            extra.push("urls");
            return self.extra(item, target, extra);
        };
        if !extra.is_empty() {
            return self.extra(item, target, extra);
        }
        if entry.get("refreshPeriod").is_some() {
            self.report(
                item,
                Some(target),
                Outcome::Note,
                Reason::RefreshPeriod,
                None,
            );
        }
        if entry.get("exact").and_then(Value::as_bool) == Some(true) {
            self.report(item, Some(target), Outcome::Note, Reason::ExactDir, None);
        }
        let mut external = External {
            path: target.to_string(),
            when,
            ..Default::default()
        };
        if kind == "git-repo" {
            external.git = Some(url.to_string());
            external.branch = branch(entry);
            external.rev = self.cloned_rev(target, url);
        } else {
            external.archive = Some(url.to_string());
            external.strip = entry
                .get("stripComponents")
                .and_then(Value::as_u64)
                .unwrap_or_default() as u32;
            external.sha256 = checksum
                .and_then(|sums| sums.get("sha256"))
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            if self.inputs.ctx.home.join(target).exists() {
                self.report(
                    item,
                    Some(target),
                    Outcome::Note,
                    Reason::ArchiveConflict,
                    None,
                );
            }
        }
        if external.rev.is_none() && external.sha256.is_none() {
            self.report(item, Some(target), Outcome::Note, Reason::Unlocked, None);
        }
        Some(external)
    }

    fn extra(&mut self, item: &str, target: &str, extra: Vec<&str>) -> Option<External> {
        self.report(
            item,
            Some(target),
            Outcome::Manual,
            Reason::ExternalField,
            Some(extra.join(", ")),
        );
        None
    }

    /// The commit chezmoi's clone at `target` is on, if it clones `url`.
    fn cloned_rev(&self, target: &str, url: &str) -> Option<String> {
        let dir = self.inputs.ctx.home.join(target);
        if !dir.join(".git").exists() {
            return None;
        }
        let origin = crate::git::run_foreign(&dir, &["remote", "get-url", "origin"]).ok()?;
        let plain = |url: &str| {
            url.trim()
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .to_string()
        };
        if plain(&origin) != plain(url) {
            return None;
        }
        crate::git::run_foreign(&dir, &["rev-parse", "--verify", "HEAD"])
            .ok()
            .map(|rev| rev.trim().to_string())
    }

    /// Externals rosamta would refuse for how they nest are reported instead.
    pub(super) fn nesting(&mut self) -> Result<()> {
        let repo = self.inputs.ctx.paths.repo();
        let mut externals: ExternalManifest = crate::externals::load(&repo)?;
        let first = externals.external.len();
        externals
            .external
            .extend(self.plan.externals.iter().cloned());
        let mut dotfiles: DotfileManifest = crate::dotfiles::load(&repo)?;
        dotfiles
            .dotfile
            .extend(self.plan.dotfiles.iter().map(|new| new.entry.clone()));
        let bad: BTreeSet<usize> = crate::externals::problems(&externals, &dotfiles)
            .into_iter()
            // Only how paths nest; a missing lock is reported as unlocked.
            .filter(|(index, problem)| *index >= first && problem.contains(" is inside "))
            .map(|(index, _)| index - first)
            .collect();
        for index in bad.iter().rev() {
            let external = self.plan.externals.remove(*index);
            let item = self.plan.report.items.iter_mut().find(|item| {
                item.reason == Reason::External && item.target.as_deref() == Some(&external.path)
            });
            if let Some(item) = item {
                item.outcome = Outcome::Manual;
                item.reason = Reason::Nested;
            }
        }
        Ok(())
    }
}

fn clone_args(entry: &Value) -> Vec<&str> {
    entry
        .pointer("/clone/args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// A clone argument rosamta's clone does without: the branch is kept, and
/// the others only make the clone smaller or quieter.
fn harmless(arg: &str) -> bool {
    let name = arg.split('=').next().unwrap_or(arg);
    !arg.starts_with('-')
        || [
            "--branch",
            "-b",
            "--depth",
            "--single-branch",
            "--no-single-branch",
            "--filter",
            "--quiet",
            "-q",
        ]
        .contains(&name)
}

/// `--branch x` or `-b x` in `clone.args`.
fn branch(entry: &Value) -> Option<String> {
    let args = clone_args(entry);
    args.iter().enumerate().find_map(|(index, arg)| {
        if let Some(name) = arg.strip_prefix("--branch=") {
            return Some(name.to_string());
        }
        (*arg == "--branch" || *arg == "-b")
            .then(|| args.get(index + 1).map(|name| name.to_string()))
            .flatten()
    })
}
