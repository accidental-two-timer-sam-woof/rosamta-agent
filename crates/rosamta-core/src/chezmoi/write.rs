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

/// What the import writes in the repo.
const PATHS: [&str; 7] = [
    "home",
    crate::dotfiles::MANIFEST,
    "scripts",
    crate::scripts::MANIFEST,
    crate::externals::MANIFEST,
    crate::secrets::MANIFEST,
    ASIDE,
];

/// Fails when the repo has changes that would be committed with the
/// import, or undone with it.
pub(crate) fn writable(repo: &Path) -> Result<()> {
    let mut args = vec!["status", "--porcelain", "--untracked-files=all", "--"];
    args.extend_from_slice(&PATHS);
    let staged = !crate::git::succeeds(repo, &["diff", "--cached", "--quiet"])?;
    if staged || !crate::git::run(repo, &args)?.trim().is_empty() {
        return Err(Error::Usage(
            "the repo has changes that are not published yet; run `rsmt publish` first".into(),
        ));
    }
    Ok(())
}

/// Write the plan, record what is only this host's (vars and the scripts
/// chezmoi ran), then commit. A failure leaves the repo as it was; the
/// host's records do no harm without the commit.
pub(crate) fn write(ctx: &Context, root: &Path, plan: Plan) -> Result<Report> {
    let repo = ctx.paths.repo();
    writable(&repo)?;
    let written = write_repo(ctx, root, &plan)
        .and_then(|scripts| record(ctx, &plan, &scripts))
        .and_then(|()| commit(ctx, &plan));
    if let Err(err) = written {
        undo(&repo);
        return Err(err);
    }
    Ok(plan.report)
}

fn record(ctx: &Context, plan: &Plan, scripts: &[Script]) -> Result<()> {
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
    remember_scripts(ctx, scripts)
}

fn commit(ctx: &Context, plan: &Plan) -> Result<()> {
    let repo = ctx.paths.repo();
    // A key made for this import goes with the files it seals.
    let paths: Vec<&str> = PATHS
        .into_iter()
        .chain([crate::dotfiles::crypt::REPO_KEY])
        .filter(|path| repo.join(path).exists())
        .collect();
    let message = format!(
        "import(chezmoi): {} dotfiles, {} scripts, {} externals\n\n{}",
        plan.dotfiles.len(),
        plan.scripts.len(),
        plan.externals.len(),
        plan.report.summary()
    );
    crate::sync::commit_scanned(ctx, &paths, Some(&message)).map(drop)
}

/// Put the import's paths back as committed; they were clean before it.
fn undo(repo: &Path) {
    for path in PATHS {
        let tracked = crate::git::run(repo, &["ls-files", "--", path])
            .is_ok_and(|files| !files.trim().is_empty());
        if tracked {
            let _ = crate::git::run(
                repo,
                &[
                    "restore",
                    "--source=HEAD",
                    "--staged",
                    "--worktree",
                    "--",
                    path,
                ],
            );
        }
        let _ = crate::git::run(repo, &["clean", "-fdq", "--", path]);
    }
}

fn write_repo(ctx: &Context, root: &Path, plan: &Plan) -> Result<Vec<Script>> {
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
        let to = repo.join(ASIDE).join(source);
        // What the user converted there from an earlier import stays.
        if to.exists() {
            continue;
        }
        let from = root.join(source);
        let bytes = std::fs::read(&from).map_err(Error::read(&from))?;
        put(&to, &bytes, 0o644)?;
    }
    Ok(scripts)
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
