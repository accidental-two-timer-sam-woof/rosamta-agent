//! Writing an import plan into the repo, as one commit.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use toml_edit::{Array, Table, value};

use super::plan::Plan;
use super::report::Report;
use crate::apply::Context;
use crate::dotfiles::review::{push_table, when_table};
use crate::error::{Error, Result};
use crate::scripts::{Run, Script, Timing};

/// Where sources that were not converted are kept, for the user.
pub const ASIDE: &str = "chezmoi-import";

fn put(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(Error::write(parent))?;
    }
    std::fs::write(path, bytes).map_err(Error::write(path))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(Error::write(path))
}

pub(crate) fn write(ctx: &Context, root: &Path, plan: Plan) -> Result<Report> {
    let repo = ctx.paths.repo();
    for new in &plan.dotfiles {
        let bytes = if new.entry.encrypted {
            crate::dotfiles::crypt::encrypt(ctx, &new.content)?
        } else {
            new.content.clone()
        };
        let mode = if new.executable { 0o755 } else { 0o644 };
        put(&repo.join("home").join(&new.entry.path), &bytes, mode)?;
        crate::dotfiles::review::append(&repo, &new.entry)?;
    }
    let mut scripts = Vec::new();
    for new in &plan.scripts {
        put(&repo.join(&new.source), &new.content, 0o755)?;
        let script = Script {
            source: new.source.clone(),
            run: Run::Onchange,
            timing: if new.before {
                Timing::Before
            } else {
                Timing::After
            },
            watch: new.watch.clone(),
            when: new.when.clone(),
        };
        let mut table = Table::new();
        table["source"] = value(&script.source);
        table["run"] = value("onchange");
        if new.before {
            table["timing"] = value("before");
        }
        if !script.watch.is_empty() {
            table["watch"] = value(script.watch.iter().collect::<Array>());
        }
        if let Some(when) = when_table(&script.when) {
            table["when"] = value(when);
        }
        push_table(&repo.join(crate::scripts::MANIFEST), "script", table)?;
        scripts.push(script);
    }
    for external in &plan.externals {
        let mut table = Table::new();
        table["path"] = value(&external.path);
        let fields = [
            ("git", &external.git),
            ("rev", &external.rev),
            ("branch", &external.branch),
            ("archive", &external.archive),
            ("sha256", &external.sha256),
        ];
        for (key, field) in fields {
            if let Some(text) = field {
                table[key] = value(text);
            }
        }
        if external.strip > 0 {
            table["strip"] = value(i64::from(external.strip));
        }
        if let Some(when) = when_table(&external.when) {
            table["when"] = value(when);
        }
        push_table(&repo.join(crate::externals::MANIFEST), "external", table)?;
    }
    if plan.op {
        add_op(&repo.join(crate::secrets::MANIFEST))?;
    }
    for source in &plan.aside {
        let from = root.join(source);
        let bytes = std::fs::read(&from).map_err(Error::read(&from))?;
        put(&repo.join(ASIDE).join(source), &bytes, 0o644)?;
    }
    let host = crate::adopt::host_config(ctx)?;
    let vars: Vec<(String, String)> = plan
        .vars
        .iter()
        .filter(|(name, _)| !host.vars.contains_key(*name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if !vars.is_empty() {
        crate::host::set_vars(&ctx.paths, &vars)?;
    }
    let candidates = [
        "home",
        crate::dotfiles::MANIFEST,
        "scripts",
        crate::scripts::MANIFEST,
        crate::externals::MANIFEST,
        crate::secrets::MANIFEST,
        ASIDE,
    ];
    let paths: Vec<&str> = candidates
        .into_iter()
        .filter(|path| repo.join(path).exists())
        .collect();
    let message = format!(
        "import(chezmoi): {} dotfiles, {} scripts, {} externals\n\n{}",
        plan.dotfiles.len(),
        plan.scripts.len(),
        plan.externals.len(),
        plan.report.summary()
    );
    crate::sync::commit_scanned(ctx, &paths, Some(&message))?;
    remember_scripts(ctx, &scripts)?;
    Ok(plan.report)
}

/// `secret("op", …)` reads with the 1Password CLI.
fn add_op(path: &Path) -> Result<()> {
    if !path.exists() {
        crate::edit::write_atomic(path, "")?;
    }
    crate::edit::edit_toml(path, |doc| {
        if doc
            .get("commands")
            .and_then(|commands| commands.get("op"))
            .is_some()
        {
            return;
        }
        let mut op = Table::new();
        op["run"] = value(["op", "read", "{arg}"].into_iter().collect::<Array>());
        if !doc.contains_key("commands") {
            let mut commands = Table::new();
            commands.set_implicit(true);
            doc["commands"] = toml_edit::Item::Table(commands);
        }
        doc["commands"]["op"] = toml_edit::Item::Table(op);
    })
    .map(drop)
}

/// chezmoi already ran them on this host.
fn remember_scripts(ctx: &Context, scripts: &[Script]) -> Result<()> {
    if scripts.is_empty() {
        return Ok(());
    }
    let repo = ctx.paths.repo();
    let state = ctx.paths.state();
    let mut ran = crate::scripts::apply::load(state)?;
    for script in scripts {
        let hash = crate::scripts::apply::hash(&repo, script)?;
        ran.entries.insert(
            crate::scripts::apply::key(script),
            crate::scripts::apply::Record {
                hash: Some(hash),
                failed: false,
            },
        );
    }
    crate::scripts::apply::save(state, &ran)
}
