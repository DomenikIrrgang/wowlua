//! The wowlua registry's protocol (spec §10.10), shared by the client and
//! the server (`wowlua-registry`): the sparse index's lines, the package
//! archive (§10.10.3) and its checksum, package names and the registry URLs
//! wowlua accepts.

use crate::safety::{self, Unsafe};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;

/// The default registry (spec §10.3).
pub const DEFAULT_REGISTRY: &str = "https://registry.wowlua.dev";

/// Files an archive may hold.
pub const MAX_FILES: usize = 10_000;

/// Bytes an archive may unpack to (a guard against compression bombs).
pub const MAX_UNPACKED: u64 = 256 * 1024 * 1024;

/// One line of `GET {registry}/index/<name>`: one published version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    pub vers: String,
    #[serde(default)]
    pub deps: Vec<IndexDep>,
    /// The checksum of the package's file tree (`sha256-…`, spec §10.6):
    /// the lockfile's `checksum` of the version.
    pub cksum: String,
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub yanked: bool,
    /// The package's `package.wowlua` requirement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wowlua: Option<String>,
}

/// A dependency of an index entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDep {
    pub name: String,
    pub req: String,
    /// `normal` (the only kind published: dev-dependencies stay with the
    /// package's own repository).
    #[serde(default = "normal")]
    pub kind: String,
    /// Another registry than the package's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// Only included when a feature of the package enables it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

fn normal() -> String {
    "normal".into()
}

impl IndexEntry {
    pub fn version(&self) -> Option<semver::Version> {
        semver::Version::parse(&self.vers).ok()
    }
}

/// The lines of an index file; lines that are not entries are skipped (a
/// newer server may add fields, never change these).
pub fn parse_index(text: &str) -> Vec<IndexEntry> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// An index file's text: one JSON line per entry.
pub fn write_index(entries: &[IndexEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&serde_json::to_string(e).unwrap_or_default());
        out.push('\n');
    }
    out
}

/// `GET {registry}/config.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// The base of the publish, yank and owner requests.
    pub api: String,
    /// The base of the downloads.
    pub dl: String,
}

/// A registry package name: lowercase `[a-z0-9-]`, 2–64 characters
/// (spec §10.10.2, the same rule as `package.name`).
pub fn valid_name(name: &str) -> bool {
    (2..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A registry URL without its trailing `/`.
pub fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Whether wowlua talks to a registry at `url`: HTTPS, or plain HTTP to
/// this computer (a registry run locally, and the tests).
pub fn allowed_url(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Why an archive is not a valid package archive.
#[derive(Debug)]
pub enum ArchiveError {
    /// Not a gzip-compressed tar, or a damaged one.
    Invalid(String),
    /// An entry that could write outside the package, or a link.
    Unsafe(Unsafe),
    /// More than [`MAX_FILES`] files, or more than [`MAX_UNPACKED`] bytes.
    TooLarge(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Invalid(why) => write!(f, "not a package archive: {why}"),
            ArchiveError::Unsafe(u) => write!(f, "unsafe entry `{}`: {}", u.path, u.reason),
            ArchiveError::TooLarge(why) => write!(f, "archive too large: {why}"),
        }
    }
}

/// The files of a package archive (spec §10.10.3) by their `/`-separated
/// paths. Only regular files and folders are allowed: absolute paths, `..`
/// and links are unsafe (E4323).
pub fn read_archive(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, ArchiveError> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(ArchiveError::Invalid("not gzip-compressed".into()));
    }
    let decoder = flate2::read::GzDecoder::new(bytes).take(MAX_UNPACKED + 1);
    let mut archive = tar::Archive::new(decoder);
    let mut files = BTreeMap::new();
    let mut total: u64 = 0;
    let entries = archive
        .entries()
        .map_err(|e| ArchiveError::Invalid(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| ArchiveError::Invalid(e.to_string()))?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        let name = entry
            .path()
            .map_err(|e| ArchiveError::Invalid(e.to_string()))?
            .to_string_lossy()
            .into_owned();
        let rel = safety::entry_path(&name).map_err(ArchiveError::Unsafe)?;
        if rel.as_os_str().is_empty() || kind.is_dir() {
            continue;
        }
        if !(kind.is_file() || kind == tar::EntryType::Continuous) {
            return Err(ArchiveError::Unsafe(Unsafe {
                path: name,
                reason: "registry archives hold regular files only",
            }));
        }
        let rel = rel.to_string_lossy().replace('\\', "/");
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|e| ArchiveError::Invalid(e.to_string()))?;
        total += contents.len() as u64;
        if total > MAX_UNPACKED {
            return Err(ArchiveError::TooLarge(format!(
                "it unpacks to more than {} MiB",
                MAX_UNPACKED / 1024 / 1024
            )));
        }
        if files.insert(rel.clone(), contents).is_some() {
            return Err(ArchiveError::Invalid(format!("`{rel}` appears twice")));
        }
        if files.len() > MAX_FILES {
            return Err(ArchiveError::TooLarge(format!(
                "it holds more than {MAX_FILES} files"
            )));
        }
    }
    Ok(files)
}

/// A reproducible package archive: the files at the archive's root, sorted,
/// with modes 0644 and the modification time `mtime` (seconds).
pub fn write_archive(files: &BTreeMap<String, Vec<u8>>, mtime: u64) -> std::io::Result<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    for (path, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(mtime);
        header.set_uid(0);
        header.set_gid(0);
        builder.append_data(&mut header, path, &contents[..])?;
    }
    builder.into_inner()?.finish()
}

/// The checksum of an archive's files: the same as [`crate::checksum::tree`]
/// of them extracted.
pub fn checksum(files: &BTreeMap<String, Vec<u8>>) -> String {
    crate::checksum::of_files(files.iter().map(|(p, c)| (p.as_str(), c.as_slice())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_package_name_rule() {
        assert!(valid_name("wowlua-ui"));
        assert!(valid_name("ab"));
        assert!(!valid_name("a"));
        assert!(!valid_name(&"a".repeat(65)));
        assert!(valid_name(&"a".repeat(64)));
        assert!(!valid_name("LibStub"));
        assert!(!valid_name("lib_stub"));
        assert!(!valid_name("lib.stub"));
    }

    #[test]
    fn only_https_or_this_computer() {
        assert!(allowed_url("https://registry.wowlua.dev"));
        assert!(allowed_url("http://127.0.0.1:8080"));
        assert!(allowed_url("http://localhost/reg"));
        assert!(allowed_url("http://[::1]:9000"));
        assert!(!allowed_url("http://registry.wowlua.dev"));
        assert!(!allowed_url("http://127.0.0.1.example.com"));
        assert!(!allowed_url("ftp://localhost"));
    }

    #[test]
    fn index_lines_round_trip_in_the_spec_s_form() {
        let line = r#"{"name":"wowlua-ui","vers":"0.4.2","deps":[{"name":"libstub","req":"^1.0","kind":"normal"}],"cksum":"sha256-ab","features":{},"yanked":false,"wowlua":">=0.3"}"#;
        let entries = parse_index(&format!("{line}\nnot json\n\n"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].deps[0].req, "^1.0");
        assert_eq!(write_index(&entries), format!("{line}\n"));
    }

    #[test]
    fn archives_are_reproducible_and_checked() {
        let mut files = BTreeMap::new();
        files.insert("wowlua.json".to_string(), b"{}".to_vec());
        files.insert("src/init.wowlua".to_string(), b"return 1".to_vec());
        let a = write_archive(&files, 0).unwrap();
        let b = write_archive(&files, 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(read_archive(&a).unwrap(), files);
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in &files {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        assert_eq!(checksum(&files), crate::checksum::tree(dir.path()).unwrap());
        assert!(matches!(
            read_archive(b"PK\x03\x04"),
            Err(ArchiveError::Invalid(_))
        ));
    }

    #[test]
    fn archives_with_links_are_unsafe() {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        builder
            .append_link(&mut header, "lib.lua", "other.lua")
            .unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();
        match read_archive(&bytes) {
            Err(ArchiveError::Unsafe(u)) => assert_eq!(u.path, "lib.lua"),
            other => panic!("expected an unsafe entry, got {other:?}"),
        }
    }
}
