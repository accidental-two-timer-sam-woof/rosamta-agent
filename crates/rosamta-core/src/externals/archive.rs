//! Archive externals: downloaded whole, checked against the locked SHA-256,
//! then extracted without letting any entry land outside the target.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

const MAX_DOWNLOAD: u64 = 1 << 30;
/// Room for large font families, not for a decompression bomb.
const MAX_EXTRACTED: u64 = 4 << 30;
const MAX_ENTRIES: usize = 200_000;

fn fail(message: impl Into<String>) -> Error {
    Error::External(message.into())
}

/// Download `url` into `to`, returning the SHA-256 of what arrived.
pub(crate) fn download(url: &str, to: &Path) -> Result<String> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent = ureq::Agent::config_builder()
        .user_agent(concat!("rosamta/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(600)))
        .tls_config(tls)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|err| fail(format!("cannot download {url}: {err}")))?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .reader();
    let mut file = File::create(to).map_err(Error::write(to))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|err| fail(format!("cannot download {url}: {err}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        file.write_all(&buf[..read]).map_err(Error::write(to))?;
    }
    Ok(hex(&hasher.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    TarGz,
    Tar,
    Zip,
}

fn format(file: &mut File) -> std::io::Result<Option<Format>> {
    let mut head = [0; 512];
    let mut len = 0;
    while len < head.len() {
        match file.read(&mut head[len..])? {
            0 => break,
            read => len += read,
        }
    }
    file.seek(SeekFrom::Start(0))?;
    let head = &head[..len];
    Ok(if head.starts_with(&[0x1f, 0x8b]) {
        Some(Format::TarGz)
    } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        Some(Format::Zip)
    } else if head.get(257..262) == Some(b"ustar") {
        Some(Format::Tar)
    } else {
        None
    })
}

/// Extract `archive` into the empty directory `into`, dropping `strip`
/// leading directories from every path.
pub(crate) fn extract(archive: &Path, strip: u32, into: &Path) -> Result<()> {
    let mut file = File::open(archive).map_err(Error::read(archive))?;
    let format = format(&mut file).map_err(Error::read(archive))?;
    let mut out = Extractor {
        into,
        strip,
        written: 0,
        entries: 0,
        links: Vec::new(),
    };
    match format {
        Some(Format::TarGz) => out.tar(flate2::read::GzDecoder::new(file))?,
        Some(Format::Tar) => out.tar(file)?,
        Some(Format::Zip) => out.zip(file)?,
        None => return Err(fail("not a .tar.gz, .tar or .zip archive")),
    }
    if out.entries == 0 {
        return Err(fail(format!(
            "nothing is left after dropping {strip} directories"
        )));
    }
    // Links come last, so no entry is ever written through one.
    for (at, target) in &out.links {
        let path = into.join(at);
        std::os::unix::fs::symlink(target, &path).map_err(Error::write(&path))?;
    }
    Ok(())
}

struct Extractor<'a> {
    into: &'a Path,
    strip: u32,
    written: u64,
    entries: usize,
    links: Vec<(PathBuf, PathBuf)>,
}

impl Extractor<'_> {
    /// Where `name` goes inside the target; `None` for entries the strip
    /// removes entirely.
    fn place(&mut self, name: &Path) -> Result<Option<PathBuf>> {
        let mut parts = Vec::new();
        for component in name.components() {
            match component {
                Component::Normal(part) => parts.push(part),
                Component::CurDir => {}
                _ => {
                    return Err(fail(format!(
                        "`{}` points outside the target",
                        name.display()
                    )));
                }
            }
        }
        let rest: PathBuf = parts.into_iter().skip(self.strip as usize).collect();
        if rest.as_os_str().is_empty() {
            return Ok(None);
        }
        self.entries += 1;
        if self.entries > MAX_ENTRIES {
            return Err(fail(format!("more than {MAX_ENTRIES} entries")));
        }
        Ok(Some(rest))
    }

    fn dir(&self, rel: &Path) -> Result<()> {
        let path = self.into.join(rel);
        std::fs::create_dir_all(&path).map_err(Error::write(&path))
    }

    fn file(&mut self, rel: &Path, mode: Option<u32>, data: &mut impl Read) -> Result<()> {
        let path = self.into.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::write(parent))?;
        }
        let mut file = File::create(&path).map_err(Error::write(&path))?;
        let budget = MAX_EXTRACTED - self.written;
        let copied = std::io::copy(&mut data.take(budget + 1), &mut file)
            .map_err(|err| fail(format!("cannot extract {}: {err}", rel.display())))?;
        if copied > budget {
            return Err(fail("extracts to more than 4 GiB"));
        }
        self.written += copied;
        // No setuid, setgid or sticky bits.
        let mode = mode.map_or(0o644, |mode| mode & 0o777);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .map_err(Error::write(&path))
    }

    fn link(&mut self, rel: PathBuf, target: PathBuf) -> Result<()> {
        // A relative target that stays inside, counted from the link's directory.
        let mut depth = rel.components().count() as i64 - 1;
        for component in target.components() {
            match component {
                Component::Normal(_) => depth += 1,
                Component::CurDir => {}
                Component::ParentDir => depth -= 1,
                _ => depth = -1,
            }
            if depth < 0 {
                return Err(fail(format!(
                    "link `{}` points outside the target",
                    rel.display()
                )));
            }
        }
        self.links.push((rel, target));
        Ok(())
    }

    fn tar(&mut self, reader: impl Read) -> Result<()> {
        use tar::EntryType;
        let bad = |err: std::io::Error| fail(format!("cannot read the archive: {err}"));
        let mut archive = tar::Archive::new(reader);
        for entry in archive.entries().map_err(bad)? {
            let mut entry = entry.map_err(bad)?;
            let kind = entry.header().entry_type();
            // Metadata such as the pax header GitHub's tarballs start with.
            if matches!(
                kind,
                EntryType::XGlobalHeader
                    | EntryType::XHeader
                    | EntryType::GNULongName
                    | EntryType::GNULongLink
            ) {
                continue;
            }
            let name = entry.path().map_err(bad)?.into_owned();
            let Some(rel) = self.place(&name)? else {
                continue;
            };
            match kind {
                EntryType::Directory => self.dir(&rel)?,
                EntryType::Regular | EntryType::Continuous => {
                    let mode = entry.header().mode().ok();
                    self.file(&rel, mode, &mut entry)?;
                }
                EntryType::Symlink => {
                    let target = entry
                        .link_name()
                        .map_err(bad)?
                        .ok_or_else(|| fail(format!("link `{}` has no target", rel.display())))?
                        .into_owned();
                    self.link(rel, target)?;
                }
                _ => {
                    return Err(fail(format!(
                        "`{}` is a hard link or special file",
                        name.display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn zip(&mut self, file: File) -> Result<()> {
        let bad = |err: zip::result::ZipError| fail(format!("cannot read the archive: {err}"));
        let mut archive = zip::ZipArchive::new(file).map_err(bad)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(bad)?;
            let name = PathBuf::from(entry.name().map_err(bad)?.into_owned());
            let Some(rel) = self.place(&name)? else {
                continue;
            };
            if entry.is_dir() {
                self.dir(&rel)?;
            } else if entry.is_symlink() {
                let mut target = String::new();
                entry
                    .read_to_string(&mut target)
                    .map_err(|err| fail(format!("cannot read link `{}`: {err}", rel.display())))?;
                self.link(rel, PathBuf::from(target))?;
            } else {
                let mode = entry.unix_mode();
                self.file(&rel, mode, &mut entry)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
