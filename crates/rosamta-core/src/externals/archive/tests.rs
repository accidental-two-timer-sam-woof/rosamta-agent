use std::fs;
use std::io::Cursor;

use tar::{EntryType, Header};

use super::*;

/// A tar entry written byte for byte, so names with `..` get through.
fn raw(builder: &mut tar::Builder<Vec<u8>>, name: &str, kind: EntryType, data: &[u8], link: &str) {
    let mut header = Header::new_gnu();
    let gnu = header.as_gnu_mut().unwrap();
    gnu.name[..name.len()].copy_from_slice(name.as_bytes());
    gnu.linkname[..link.len()].copy_from_slice(link.as_bytes());
    header.set_entry_type(kind);
    header.set_size(data.len() as u64);
    header.set_mode(0o4755);
    header.set_cksum();
    builder.append(&header, data).unwrap();
}

fn tar(entries: &[(&str, EntryType, &[u8], &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, kind, data, link) in entries {
        raw(&mut builder, name, *kind, data, link);
    }
    builder.into_inner().unwrap()
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// Extract `bytes` into `<dir>/out`, which is left alone on failure checks.
fn unpack(bytes: &[u8], strip: u32) -> (tempfile::TempDir, Result<()>) {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("archive");
    fs::write(&archive, bytes).unwrap();
    let out = dir.path().join("out");
    fs::create_dir(&out).unwrap();
    let result = extract(&archive, strip, &out);
    (dir, result)
}

#[test]
fn tar_gz_strips_and_drops_special_bits() {
    let bytes = gzip(&tar(&[
        (
            "pax_global_header",
            EntryType::XGlobalHeader,
            b"52 comment=abc\n",
            "",
        ),
        ("pkg-1.0/", EntryType::Directory, b"", ""),
        ("pkg-1.0/bin/tool", EntryType::Regular, b"#!/bin/sh\n", ""),
        ("pkg-1.0/tool", EntryType::Symlink, b"", "bin/tool"),
    ]));
    let (dir, result) = unpack(&bytes, 1);
    result.unwrap();
    let out = dir.path().join("out");
    assert_eq!(
        fs::read_to_string(out.join("bin/tool")).unwrap(),
        "#!/bin/sh\n"
    );
    let mode = fs::metadata(out.join("bin/tool"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o755);
    assert_eq!(
        fs::read_link(out.join("tool")).unwrap(),
        Path::new("bin/tool")
    );
    assert!(!out.join("pax_global_header").exists());
}

#[test]
fn entries_outside_the_target_fail() {
    for entries in [
        vec![("../evil", EntryType::Regular, &b"x"[..], "")],
        vec![("/abs", EntryType::Regular, &b"x"[..], "")],
        vec![("a", EntryType::Symlink, &b""[..], "../../x")],
        vec![("a", EntryType::Symlink, &b""[..], "/etc/passwd")],
        // Each fine counted alone; together `M` climbs above the target.
        vec![
            ("d1/d2/d3/L", EntryType::Symlink, &b""[..], "../../.."),
            ("M", EntryType::Symlink, &b""[..], "d1/d2/d3/L/../.."),
        ],
        // A link placed through another link lands wherever that one points.
        vec![
            ("M", EntryType::Symlink, &b""[..], "d"),
            ("M/x", EntryType::Symlink, &b""[..], "y"),
        ],
        vec![("a", EntryType::Link, &b""[..], "b")],
        vec![("a", EntryType::Fifo, &b""[..], "")],
    ] {
        let (dir, result) = unpack(&tar(&entries), 0);
        let err = result.unwrap_err().to_string();
        assert!(
            [
                "outside the target",
                "hard link or special",
                "through another link"
            ]
            .iter()
            .any(|reason| err.contains(reason)),
            "{entries:?}: {err}"
        );
        assert!(!dir.path().join("evil").exists());
        assert!(!dir.path().join("out/a").exists());
    }
}

#[test]
fn zip_extracts_and_other_files_are_refused() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    writer.start_file("fonts/A.ttf", options).unwrap();
    writer.write_all(b"font").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let (dir, result) = unpack(&bytes, 0);
    result.unwrap();
    let font = dir.path().join("out/fonts/A.ttf");
    assert_eq!(fs::read(&font).unwrap(), b"font");
    assert_eq!(
        fs::metadata(&font).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let (_dir, result) = unpack(b"plain text", 0);
    assert!(result.unwrap_err().to_string().contains("not a .tar.gz"));
    let (_dir, result) = unpack(&bytes, 2);
    assert!(result.unwrap_err().to_string().contains("nothing is left"));
}
