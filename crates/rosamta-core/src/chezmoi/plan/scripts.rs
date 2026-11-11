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
        let repo_path = format!("scripts/{target}");
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
        if target.contains('/') {
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
            source: repo_path,
            content: converted.text.into_bytes(),
            before: script.before,
            watch,
            when: when.unwrap_or_default(),
        });
        Ok(())
    }
}
