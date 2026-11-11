//! The one marked block rosamta keeps in each rc file.
//!
//! | Shell | File | Loads |
//! |---|---|---|
//! | sh | `~/.profile` | env, PATH order |
//! | bash | the profile file login bash reads, and `~/.bashrc` | env, PATH order, then `~/.bashrc` when interactive; env and rc |
//! | zsh | `.zshenv`; `$ZDOTDIR/.zshrc`; `$ZDOTDIR/.zprofile` | env; rc; PATH order |
//! | fish | `conf.d/rosamta.fish` (a file of its own) | env, and rc when interactive |

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Shell;
use crate::error::{Error, Result};
use crate::process::output_with_timeout;
use crate::render;
use crate::snapshot::Snapshots;

pub const BEGIN: &str = "# >>> rosamta >>>";
pub const END: &str = "# <<< rosamta <<<";

/// The first line of a `~/.bash_profile` rosamta creates, so login bash
/// still reads `~/.profile`. It is rosamta's, so adoption leaves it alone.
pub const BASH_PROFILE_SEED: &str = "[ -r \"$HOME/.profile\" ] && . \"$HOME/.profile\"\n";

/// Facts about this host that decide hook locations and contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEnv {
    pub home: PathBuf,
    /// The `current` link of the generated scripts, written into hooks as is:
    /// shells may not see the `XDG_DATA_HOME` rsmt saw.
    pub current: PathBuf,
    /// Where zsh reads `.zshenv`: `$ZDOTDIR` after the global zshenv, or HOME.
    pub zshenv_dir: PathBuf,
    /// Where zsh reads `.zshrc` and `.zprofile`, after `.zshenv` ran.
    pub zdotdir: PathBuf,
    pub fish_config: PathBuf,
}

impl HookEnv {
    /// Ask zsh and fish themselves; fall back to their defaults.
    pub fn detect(home: &Path, current: &Path, zsh: Option<&Path>, fish: Option<&Path>) -> Self {
        let query = |exe: &Path, args: &[&str], zdotdir: Option<&Path>| {
            let mut cmd = std::process::Command::new(exe);
            cmd.args(args)
                .env("HOME", home)
                .env("ROSAMTA_CURRENT", "/nonexistent");
            match zdotdir {
                Some(dir) => cmd.env("ZDOTDIR", dir),
                None => cmd.env_remove("ZDOTDIR"),
            };
            let output = output_with_timeout(&mut cmd, Duration::from_secs(5)).ok()??;
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (output.status.success() && text.starts_with('/')).then(|| PathBuf::from(text))
        };
        let zdotdir_script = "printf %s \"${ZDOTDIR:-$HOME}\"";
        let fresh = zsh.and_then(|zsh| query(zsh, &["-c", zdotdir_script], None));
        // rsmt inherits a ZDOTDIR from the session, which every zsh sees, or
        // from a zsh whose ~/.zshenv set it, which a new zsh sees only after
        // reading ~/.zshenv. Only the first kind moves .zshenv.
        let session = std::env::var_os("ZDOTDIR")
            .map(PathBuf::from)
            .filter(|dir| fresh.as_ref() != Some(dir));
        // -f still runs the global zshenv, which may set ZDOTDIR (Alpine does
        // when HOME has no zsh startup files), but not the user's .zshenv.
        let zshenv_dir = zsh
            .and_then(|zsh| query(zsh, &["-f", "-c", zdotdir_script], session.as_deref()))
            .unwrap_or_else(|| home.to_path_buf());
        let zdotdir = match &session {
            Some(dir) => zsh.and_then(|zsh| query(zsh, &["-c", zdotdir_script], Some(dir))),
            None => fresh,
        }
        .unwrap_or_else(|| zshenv_dir.clone());
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let fish_config = fish
            .and_then(|fish| query(fish, &["-c", "printf %s $__fish_config_dir"], None))
            .unwrap_or_else(|| config_home.join("fish"));
        Self {
            home: home.to_path_buf(),
            current: current.to_path_buf(),
            zshenv_dir,
            zdotdir,
            fish_config,
        }
    }
}

/// A planned write: `None` means the file is absent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl FileChange {
    pub fn is_noop(&self) -> bool {
        self.before == self.after
    }
}

fn read(path: &Path) -> Result<Option<String>> {
    let text = read_raw(path)?;
    if let Some(text) = &text {
        // An unpaired start line would make the next write pair it with
        // a later end line and drop the user's lines in between.
        let starts = text.lines().filter(|line| line.trim_end() == BEGIN).count();
        if starts > usize::from(find_block(text).is_some()) {
            return Err(Error::Usage(format!(
                "{}: the `{BEGIN}` and `{END}` lines do not pair up; keep one of each around rosamta's block and try again",
                path.display()
            )));
        }
    }
    Ok(text)
}

fn read_raw(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Error::Read {
            path: path.to_path_buf(),
            source: err,
        }),
    }
}

/// `current` under HOME as `$HOME/...`, so a block stays right when the rc
/// file moves to a host with another home directory.
fn home_relative(env: &HookEnv) -> Option<&Path> {
    env.current.strip_prefix(&env.home).ok()
}

/// One stage's block. `ROSAMTA_CURRENT` lets apply's smoke test point a
/// shell at a candidate; `_rosamta_version`, set by the env loader, keeps
/// later stages on the version the env stage loaded. The last stage drops it.
fn posix_block(env: &HookEnv, shell: Shell, body: &str, last: bool) -> String {
    let (mut dir, rest) = match home_relative(env) {
        Some(rel) => ("$HOME/".to_string(), rel),
        None => (String::new(), env.current.as_path()),
    };
    for c in rest.display().to_string().chars() {
        if matches!(c, '"' | '\\' | '`' | '$') {
            dir.push('\\');
        }
        dir.push(c);
    }
    let unset = if last {
        "_rosamta _rosamta_version"
    } else {
        "_rosamta"
    };
    format!(
        "{BEGIN}\n_rosamta=\"${{ROSAMTA_CURRENT:-${{_rosamta_version:-{dir}}}}}/{shell}\"\n{body}unset {unset}\n{END}\n"
    )
}

fn source_line(file: &str) -> String {
    format!("[ -r \"$_rosamta/{file}\" ] && . \"$_rosamta/{file}\"\n")
}

fn fish_file(env: &HookEnv) -> String {
    let dir = match home_relative(env) {
        Some(rel) => format!(
            "\"$HOME\"{}",
            render::fish_quote(&format!("/{}", rel.display()))
        ),
        None => render::fish_quote(&env.current.display().to_string()),
    };
    format!(
        "{BEGIN}\nset -l __rosamta_dir $ROSAMTA_CURRENT\n\
         test -n \"$__rosamta_dir\"; or set __rosamta_dir {dir}\n\
         test -r $__rosamta_dir/fish/env.fish; and source $__rosamta_dir/fish/env.fish\n\
         status is-interactive; and test -r $__rosamta_dir/fish/rc.fish; and source $__rosamta_dir/fish/rc.fish\n{END}\n"
    )
}

/// Replace the block in `text`, or append one; nothing outside it changes.
pub fn upsert_block(text: &str, block: &str) -> String {
    match find_block(text) {
        Some((start, end)) => format!("{}{block}{}", &text[..start], &text[end..]),
        None if text.is_empty() || text.ends_with('\n') => format!("{text}{block}"),
        None => format!("{text}\n{block}"),
    }
}

/// `text` without its block.
pub fn strip_block(text: &str) -> String {
    match find_block(text) {
        Some((start, end)) => format!("{}{}", &text[..start], &text[end..]),
        None => text.to_string(),
    }
}

/// The byte range of the block, from its first line through its last newline.
pub fn find_block(text: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    let mut start = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if start.is_none() && trimmed == BEGIN {
            start = Some(offset);
        } else if start.is_some() && trimmed == END {
            return start.map(|start| (start, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

/// The profile file login bash reads first, if any exists.
fn bash_profile(home: &Path) -> Option<PathBuf> {
    [".bash_profile", ".bash_login", ".profile"]
        .iter()
        .map(|name| home.join(name))
        .find(|path| path.exists())
}

/// Changes that install `shell`'s hooks. Nothing is written.
pub fn plan_install(shell: Shell, env: &HookEnv) -> Result<Vec<FileChange>> {
    plan_install_with(shell, env, &read)
}

fn plan_install_with(
    shell: Shell,
    env: &HookEnv,
    read: &dyn Fn(&Path) -> Result<Option<String>>,
) -> Result<Vec<FileChange>> {
    let mut changes = Vec::new();
    let mut upsert = |path: PathBuf, block: String, seed: Option<String>| -> Result<()> {
        let before = read(&path)?;
        let base = before.clone().or(seed).unwrap_or_default();
        changes.push(FileChange {
            after: Some(upsert_block(&base, &block)),
            before,
            path,
        });
        Ok(())
    };
    match shell {
        Shell::Sh => {
            let body = format!("{}{}", source_line("env.sh"), source_line("profile.sh"));
            upsert(
                env.home.join(".profile"),
                posix_block(env, shell, &body, true),
                None,
            )?;
        }
        Shell::Bash => {
            let profile = bash_profile(&env.home).filter(|path| !path.ends_with(".profile"));
            let path = profile
                .clone()
                .unwrap_or_else(|| env.home.join(".bash_profile"));
            // Login bash reads only this file, so a new one must keep ~/.profile
            // working, and every one must reach ~/.bashrc in interactive shells,
            // once: Debian's ~/.profile sources it already.
            let seed = profile.is_none().then(|| BASH_PROFILE_SEED.to_string());
            let text = strip_block(&read(&path)?.or_else(|| seed.clone()).unwrap_or_default());
            let dot_profile = read(&env.home.join(".profile"))?.unwrap_or_default();
            let reaches_bashrc = text.contains(".bashrc")
                || (text.contains(".profile") && strip_block(&dot_profile).contains(".bashrc"));
            let mut body = format!("{}{}", source_line("env.bash"), source_line("profile.bash"));
            if !reaches_bashrc {
                body.push_str(
                    "case $- in *i*) [ -r \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\" ;; esac\n",
                );
            }
            upsert(path, posix_block(env, shell, &body, false), seed)?;
            let rc = format!(
                "{}case $- in *i*) {};; esac\n",
                source_line("env.bash"),
                source_line("rc.bash").trim_end()
            );
            upsert(
                env.home.join(".bashrc"),
                posix_block(env, shell, &rc, true),
                None,
            )?;
        }
        Shell::Zsh => {
            upsert(
                env.zshenv_dir.join(".zshenv"),
                posix_block(env, shell, &source_line("env.zsh"), false),
                None,
            )?;
            upsert(
                env.zdotdir.join(".zshrc"),
                posix_block(env, shell, &source_line("rc.zsh"), true),
                None,
            )?;
            // The global zprofile may reorder PATH (macOS path_helper) or reset
            // it (Alpine's /etc/profile) after .zshenv ran.
            upsert(
                env.zdotdir.join(".zprofile"),
                posix_block(env, shell, &source_line("profile.zsh"), false),
                None,
            )?;
        }
        Shell::Fish => {
            let path = env.fish_config.join("conf.d/rosamta.fish");
            changes.push(FileChange {
                before: read(&path)?,
                after: Some(fish_file(env)),
                path,
            });
        }
    }
    Ok(changes)
}

/// Files that hold `shell`'s hook today.
pub fn hooked_files(shell: Shell, env: &HookEnv) -> Result<Vec<PathBuf>> {
    let candidates: Vec<PathBuf> = match shell {
        Shell::Sh => vec![env.home.join(".profile")],
        Shell::Bash => vec![
            env.home.join(".bash_profile"),
            env.home.join(".bash_login"),
            env.home.join(".bashrc"),
        ],
        Shell::Zsh => vec![
            env.zshenv_dir.join(".zshenv"),
            env.zdotdir.join(".zshrc"),
            env.zdotdir.join(".zprofile"),
        ],
        Shell::Fish => vec![env.fish_config.join("conf.d/rosamta.fish")],
    };
    let mut out = Vec::new();
    for path in candidates {
        if read(&path)?.is_some_and(|text| find_block(&text).is_some()) && !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// Files that should hold `shell`'s block and do not, as when another tool
/// writes its own copy of an rc file over rosamta's.
pub fn missing(shell: Shell, env: &HookEnv) -> Result<Vec<PathBuf>> {
    // Markers that do not pair up are apply's to report, in that file
    // alone: any start line counts as a block here.
    Ok(plan_install_with(shell, env, &read_raw)?
        .into_iter()
        .filter(|change| {
            change
                .before
                .as_deref()
                .is_none_or(|text| !text.lines().any(|line| line.trim_end() == BEGIN))
        })
        .map(|change| change.path)
        .collect())
}

/// Changes that remove `shell`'s hooks. fish's file goes away entirely.
pub fn plan_remove(shell: Shell, env: &HookEnv) -> Result<Vec<FileChange>> {
    let mut changes = Vec::new();
    for path in hooked_files(shell, env)? {
        let before = read(&path)?;
        let after = if shell == Shell::Fish {
            None
        } else {
            before.as_deref().map(strip_block)
        };
        changes.push(FileChange {
            path,
            before,
            after,
        });
    }
    Ok(changes)
}

/// Snapshot, then write every change that does something.
pub fn commit(changes: &[FileChange], snapshots: &Snapshots, reason: &str) -> Result<()> {
    let real: Vec<&FileChange> = changes.iter().filter(|change| !change.is_noop()).collect();
    // An edit made after the plan (while a prompt waited) is never overwritten.
    for change in &real {
        crate::adopt::unchanged(&change.path, change.before.as_deref())?;
    }
    snapshots.save(
        &real
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>(),
        reason,
    )?;
    for change in real {
        match &change.after {
            Some(text) => {
                if let Some(dir) = change.path.parent() {
                    std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
                }
                crate::edit::write_atomic(&change.path, text)?;
            }
            None => std::fs::remove_file(&change.path).map_err(Error::write(&change.path))?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_after_the_plan_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".zshrc");
        std::fs::write(&path, "planned\n").unwrap();
        let change = FileChange {
            path: path.clone(),
            before: Some("planned\n".into()),
            after: Some("written\n".into()),
        };
        std::fs::write(&path, "edited meanwhile\n").unwrap();
        let snapshots = Snapshots::new(&dir.path().join("state"), dir.path());
        assert!(commit(std::slice::from_ref(&change), &snapshots, "test").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "edited meanwhile\n"
        );
    }

    #[test]
    fn upsert_and_strip() {
        let block = format!("{BEGIN}\nx\n{END}\n");
        assert_eq!(upsert_block("", &block), block);
        assert_eq!(upsert_block("a\n", &block), format!("a\n{block}"));
        assert_eq!(upsert_block("a", &block), format!("a\n{block}"));
        let installed = format!("a\n{BEGIN}\nold\n{END}\nb\n");
        assert_eq!(upsert_block(&installed, &block), format!("a\n{block}b\n"));
        assert_eq!(strip_block(&installed), "a\nb\n");
        assert_eq!(strip_block("plain\n"), "plain\n");
    }

    #[test]
    fn unterminated_block_is_left_alone() {
        let text = format!("{BEGIN}\nhalf\n");
        assert_eq!(find_block(&text), None);
    }

    #[test]
    fn unpaired_markers_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".zshrc");
        let block = format!("{BEGIN}\nx\n{END}\n");
        for text in [
            format!("a\n{BEGIN}\nb\n"),
            format!("{BEGIN}\nmine\n{block}"),
            format!("{block}{block}"),
        ] {
            std::fs::write(&path, &text).unwrap();
            assert!(read(&path).is_err(), "{text}");
        }
        std::fs::write(&path, format!("a\n{block}b\n")).unwrap();
        assert!(read(&path).unwrap().is_some());
    }

    #[test]
    fn unpaired_markers_hide_no_other_file_that_lost_its_block() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let env = HookEnv {
            home: home.into(),
            current: home.join("current"),
            zshenv_dir: home.into(),
            zdotdir: home.into(),
            fish_config: home.join(".config/fish"),
        };
        std::fs::write(home.join(".zshenv"), format!("{BEGIN}\nx\n{END}\n")).unwrap();
        std::fs::write(home.join(".zprofile"), format!("{BEGIN}\nx\n")).unwrap();
        std::fs::write(home.join(".zshrc"), "export A=1\n").unwrap();
        assert_eq!(missing(Shell::Zsh, &env).unwrap(), [home.join(".zshrc")]);
    }
}
