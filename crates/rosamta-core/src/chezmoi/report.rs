//! What an import did with each part of the source, as codes the CLI and
//! the app put in their own words.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Imported,
    Skipped,
    /// Not imported; the user converts it by hand.
    Manual,
    /// Imported, with something to know about it.
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    // Imported.
    Dotfile,
    Script,
    External,
    // Skipped.
    /// `.chezmoiignore` excludes it on every host.
    Ignored,
    /// Only white space: chezmoi does not create it.
    Empty,
    /// Already declared or present in the repo.
    Exists,
    /// A shell startup file: rosamta adopts its content into modules.
    StartupFile,
    // Manual.
    /// A template construct the import does not convert; the detail names it.
    Template,
    /// Looks like a secret and is not encrypted.
    Secret,
    /// Could not be decrypted, or the repo key is not available here.
    Encrypted,
    /// The platforms it applies on are not one `when`.
    Scope,
    /// Renders differently from `chezmoi cat` on this host.
    Differs,
    Modify,
    Remove,
    Symlink,
    /// `create_` with `.tmpl`: rosamta's create deploys without rendering.
    CreateTemplate,
    /// chezmoi removes what it does not manage in this directory.
    ExactDir,
    /// The directory's own permissions.
    DirPermissions,
    /// chezmoi creates this empty directory.
    EmptyDir,
    /// An external type rosamta has no equivalent for.
    ExternalType,
    /// External settings rosamta has no equivalent for; the detail names them.
    ExternalField,
    /// An externals file that is a template.
    ExternalTemplate,
    /// A path inside another external or dotfile that rosamta does not allow.
    Nested,
    /// A data or externals file in a format the import does not read.
    Format,
    /// `.chezmoiremove` or `.chezmoitemplates`.
    Special,
    /// A symlink in the source: what it points to is not read.
    Link,
    // Notes.
    /// chezmoi hostnames compared in it became host IDs; the detail lists them.
    Hostname,
    /// chezmoi ran it on every apply; it now runs when it changes.
    EveryApply,
    /// chezmoi ran it in another directory; rosamta runs it in the home directory.
    ScriptDir,
    /// rosamta keeps the locked version until `rsmt externals update`.
    RefreshPeriod,
    /// chezmoi's copy is already there, so the first apply lists a conflict.
    ArchiveConflict,
    /// Not locked yet: `rsmt externals update` locks it.
    Unlocked,
    /// Renders to nothing on some hosts, where rosamta still writes the file.
    MayBeEmpty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    /// The path in the chezmoi source.
    pub source: String,
    /// What it became: a home-relative target, or a repo path for scripts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub outcome: Outcome,
    pub reason: Reason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub items: Vec<Item>,
}

impl Report {
    pub(crate) fn push(
        &mut self,
        source: &str,
        target: Option<&str>,
        outcome: Outcome,
        reason: Reason,
        detail: Option<String>,
    ) {
        self.items.push(Item {
            source: source.to_string(),
            target: target.map(str::to_string),
            outcome,
            reason,
            detail,
        });
    }

    pub fn count(&self, outcome: Outcome) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == outcome)
            .count()
    }

    /// For the commit: one line per item that is not simply imported.
    pub(crate) fn summary(&self) -> String {
        let mut lines = vec![format!(
            "Imported {}, skipped {}, to convert by hand {}.",
            self.count(Outcome::Imported),
            self.count(Outcome::Skipped),
            self.count(Outcome::Manual)
        )];
        for item in self
            .items
            .iter()
            .filter(|item| item.outcome != Outcome::Imported)
        {
            let mut line = format!(
                "- {} {}: {}",
                code(item.outcome),
                item.source,
                code(item.reason)
            );
            if let Some(detail) = &item.detail {
                line.push_str(&format!(" ({detail})"));
            }
            lines.push(line);
        }
        lines.join("\n")
    }
}

/// The snake_case code serde gives a value.
pub fn code(value: impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}
