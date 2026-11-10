//! Writes that touch only the target fields, keeping comments and order.

use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Value};

use crate::error::{Error, Result};

/// Apply `change` to the TOML file at `path`. Returns whether the file changed.
pub fn edit_toml(path: &Path, change: impl FnOnce(&mut DocumentMut)) -> Result<bool> {
    let text = std::fs::read_to_string(path).map_err(Error::read(path))?;
    let mut doc: DocumentMut = text.parse().map_err(|err: toml_edit::TomlError| {
        Error::Usage(format!("{}: {}", path.display(), err.message().trim()))
    })?;
    change(&mut doc);
    let updated = doc.to_string();
    if updated == text {
        return Ok(false);
    }
    write_atomic(path, &updated)?;
    Ok(true)
}

/// Write `text` to a file only its owner can read.
pub(crate) fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = path
        .parent()
        .expect("a private file has a parent directory");
    std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(Error::write(path))?;
    // `mode` applies only when the file is created.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(Error::write(path))?;
    file.write_all(text.as_bytes()).map_err(Error::write(path))
}

/// Write via a temporary file and rename, so readers never see half a file.
/// A symlink (stow, yadm) stays one: the file it points at is replaced,
/// keeping its permissions.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = target.with_file_name(format!(".{name}.rosamta-tmp"));
    let permissions = std::fs::metadata(&target)
        .ok()
        .map(|meta| meta.permissions());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    if let Some(permissions) = &permissions {
        options.mode(permissions.mode());
    }
    let mut file = options.open(&tmp).map_err(Error::write(&tmp))?;
    // `mode` applies only when the file is created, and through the umask.
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)
            .map_err(Error::write(&tmp))?;
    }
    file.write_all(text.as_bytes())
        .map_err(Error::write(&tmp))?;
    std::fs::rename(&tmp, &target).map_err(Error::write(path))
}

pub fn set_module_enabled(path: &Path, enabled: bool) -> Result<bool> {
    edit_toml(path, |doc| doc["enabled"] = toml_edit::value(enabled))
}

/// The array at `key` inside the table-like `item`, if any.
pub(crate) fn array_mut<'a>(item: &'a mut Item, key: &str) -> Option<&'a mut Array> {
    item.as_table_like_mut()?.get_mut(key)?.as_array_mut()
}

/// Replace `old` with `new` in a string array, dropping duplicates of `new`.
/// Decorations (comments, spacing) of kept elements are preserved.
pub(crate) fn rename_in_array(array: &mut Array, old: &str, new: &str) -> bool {
    if !array.iter().any(|value| value.as_str() == Some(old)) {
        return false;
    }
    let mut seen_new = array.iter().any(|value| value.as_str() == Some(new));
    let mut index = 0;
    while index < array.len() {
        if array.get(index).and_then(Value::as_str) == Some(old) {
            if seen_new {
                array.remove(index);
                continue;
            }
            let value = array.get_mut(index).expect("index in range");
            let decor = value.decor().clone();
            *value = Value::from(new);
            *value.decor_mut() = decor;
            seen_new = true;
        }
        index += 1;
    }
    true
}

/// Rewrite a string array as the sorted, de-duplicated version of `values`.
pub(crate) fn set_sorted_strings(item: &mut Item, values: impl IntoIterator<Item = String>) {
    let mut values: Vec<String> = values.into_iter().collect();
    values.sort();
    values.dedup();
    *item = toml_edit::value(values.into_iter().collect::<Array>());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Module;

    const MODULE: &str = "# Prompt for every shell\nphase = \"prompt\" # keep early\n\n[when]\n# only my laptop\nhosts = [\"mbp\"]\n";

    #[test]
    fn disable_keeps_comments_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("starship.toml");
        std::fs::write(&path, MODULE).unwrap();

        assert!(set_module_enabled(&path, false).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# Prompt for every shell\nphase = \"prompt\" # keep early\nenabled = false\n\n[when]\n# only my laptop\nhosts = [\"mbp\"]\n"
        );
        let module: Module = toml::from_str(&text).unwrap();
        assert!(!module.enabled);
        assert_eq!(module.when.hosts, ["mbp"]);
    }

    #[test]
    fn atomic_writes_keep_symlinks_and_modes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles/zshrc");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "old\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join(".zshrc");
        std::os::unix::fs::symlink("dotfiles/zshrc", &link).unwrap();

        write_atomic(&link, "new\n").unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new\n");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn unchanged_file_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.toml");
        std::fs::write(&path, "enabled = true\n").unwrap();
        assert!(!set_module_enabled(&path, true).unwrap());
    }

    #[test]
    fn rename_keeps_decor_and_dedupes() {
        let mut doc: DocumentMut = "hosts = [\"mbp\", # laptop\n  \"lab\", \"work-mbp\"]\n"
            .parse()
            .unwrap();
        let array = doc["hosts"].as_array_mut().unwrap();
        assert!(rename_in_array(array, "lab", "server"));
        assert!(rename_in_array(array, "mbp", "work-mbp"));
        assert!(!rename_in_array(array, "gone", "x"));
        let values: Vec<_> = array.iter().filter_map(Value::as_str).collect();
        assert_eq!(values, ["server", "work-mbp"]);
    }

    #[test]
    fn sorted_strings() {
        let mut doc: DocumentMut = "tags = [\"personal\"]\n".parse().unwrap();
        let values = ["personal", "lab", "lab"].map(String::from);
        set_sorted_strings(&mut doc["tags"], values);
        assert_eq!(doc.to_string(), "tags = [\"lab\", \"personal\"]\n");
    }
}
