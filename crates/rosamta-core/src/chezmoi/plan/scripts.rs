//! Scripts in the chezmoi source.

use super::*;

impl Planner<'_> {
    pub(super) fn script(
        &mut self,
        source: &str,
        target: &str,
        attrs: &FileAttrs,
        script: ScriptAttrs,
    ) -> Result<()> {
        let name = target.strip_prefix(".chezmoiscripts/").unwrap_or(target);
        let repo_path = format!("scripts/{name}");
        let scope = self.scope(target);
        if scope.platforms == 0 {
            self.report(
                source,
                Some(&repo_path),
                Outcome::Skipped,
                Reason::Ignored,
                None,
            );
            return Ok(());
        }
        let declared = crate::scripts::load(&self.inputs.ctx.paths.repo())?
            .script
            .iter()
            .any(|script| script.source == repo_path);
        if declared || self.inputs.ctx.paths.repo().join(&repo_path).exists() {
            self.report(
                source,
                Some(&repo_path),
                Outcome::Skipped,
                Reason::Exists,
                None,
            );
            return Ok(());
        }
        // chezmoi tells apart scripts that become the same repo path.
        if self.plan.scripts.iter().any(|new| new.source == repo_path) {
            self.aside(source, &repo_path, Reason::Exists, None);
            return Ok(());
        }
        let Ok(text) = String::from_utf8(self.read(source)?) else {
            self.aside(
                source,
                &repo_path,
                Reason::Template,
                Some("not UTF-8 text".into()),
            );
            return Ok(());
        };
        let converted = if attrs.template {
            convert::script(&text, &self.machine)
        } else {
            convert::check_script(&text).map(|()| Converted {
                text: text.clone(),
                template: false,
                may_be_empty: false,
                scope: Scope::default(),
                uses: Default::default(),
            })
        };
        let converted = match converted {
            Ok(converted) => converted,
            Err(err) => {
                let detail = format!("line {}: {}", err.line, err.what);
                self.aside(source, &repo_path, Reason::Template, Some(detail));
                return Ok(());
            }
        };
        let scope = scope.and(&converted.scope);
        if scope.is_empty() || converted.text.trim().is_empty() {
            self.report(
                source,
                Some(&repo_path),
                Outcome::Skipped,
                Reason::Empty,
                None,
            );
            return Ok(());
        }
        if let Some(rule) = crate::adopt::secret_rule(&converted.text) {
            self.report(
                source,
                Some(&repo_path),
                Outcome::Manual,
                Reason::Secret,
                Some(rule.into()),
            );
            return Ok(());
        }
        let Some(when) = scope.when() else {
            self.aside(source, &repo_path, Reason::Scope, None);
            return Ok(());
        };
        let mut watch = Vec::new();
        for path in &converted.uses.watch {
            match self.imported.get(path.trim_start_matches('/')) {
                Some(repo) => watch.push(repo.clone()),
                None => {
                    let detail = format!("`include \"{path}\"` of a file that was not imported");
                    self.aside(source, &repo_path, Reason::Template, Some(detail));
                    return Ok(());
                }
            }
        }
        if script.run.is_none() {
            self.report(
                source,
                Some(&repo_path),
                Outcome::Note,
                Reason::EveryApply,
                None,
            );
        }
        // Scripts in `.chezmoiscripts/` run in the home directory there too.
        if name == target && target.contains('/') {
            self.report(
                source,
                Some(&repo_path),
                Outcome::Note,
                Reason::ScriptDir,
                None,
            );
        }
        if !converted.uses.hostnames.is_empty() {
            self.used(source, &repo_path, &converted);
        }
        self.report(
            source,
            Some(&repo_path),
            Outcome::Imported,
            Reason::Script,
            None,
        );
        self.plan.scripts.push(NewScript {
            content: with_interpreter(&repo_path, converted.text).into_bytes(),
            source: repo_path,
            before: script.before,
            watch,
            when: when.unwrap_or_default(),
        });
        Ok(())
    }
}

/// chezmoi runs a script without `#!` by its extension; rosamta runs it
/// with `sh`, so the interpreter goes in the first line.
fn with_interpreter(path: &str, text: String) -> String {
    let interpreter = match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("py") => "python3",
        Some("rb") => "ruby",
        Some("pl") => "perl",
        Some("nu") => "nu",
        _ => return text,
    };
    if text.starts_with("#!") {
        return text;
    }
    format!("#!/usr/bin/env {interpreter}\n{text}")
}
