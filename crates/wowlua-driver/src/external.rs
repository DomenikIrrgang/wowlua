//! External addons (`workspace.external-addons`, decision T127): AddOns that
//! are fetched and shipped as they are, like a `.pkgmeta` external that
//! `move-folders` turns into an AddOn of its own (DBM's voice packs). Every
//! file is copied (minus dotfiles and the external's own `.pkgmeta`
//! `ignore`), its own TOC included; with the packager's processing, text
//! files get keywords and LF line endings like the packager's externals.

use crate::Session;
use crate::build::{BuildOutput, Content};
use crate::project::ExternalAddon;
use std::collections::BTreeMap;
use std::path::Path;
use wowlua_diagnostics::Diagnostic;

/// Builds an external addon into `<out>/<profile>/<name>/`.
pub fn build_external(
    session: &mut Session,
    ext: &ExternalAddon,
    profile: &wowlua_manifest::Profile,
    out_root: &Path,
    write: bool,
) -> Option<BuildOutput> {
    let Some(package) = &ext.package else {
        session.push(Diagnostic::plain_error(format!(
            "external addon `{}` was not fetched",
            ext.name
        )));
        return None;
    };
    let dir = &package.dir;
    // The external's own `.pkgmeta`: `ignore` and `plain-copy` (patterns
    // relative to its repository root, where the package sits at `sub`).
    let (own, sub) = package
        .project_file
        .as_ref()
        .and_then(|(file, sub)| {
            let text = std::fs::read_to_string(file).ok()?;
            Some((crate::pkgmeta::parse(&text).ok()?, sub.clone()))
        })
        .unwrap_or_default();
    // An archive is copied as it is, like the packager does.
    let archive = matches!(package.source, wowlua_pkg::Source::Archive { .. });
    let mut files: BTreeMap<String, Content> = BTreeMap::new();
    let mut info: Option<crate::keywords::KeywordInfo> = None;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(dir) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let in_root = if sub.is_empty() {
            rel.clone()
        } else {
            format!("{sub}/{rel}")
        };
        if crate::keywords::plain_copy(&own.ignore, &[&in_root]) {
            continue;
        }
        let abs = entry.path().to_path_buf();
        let processed = ext.packager
            && !archive
            && crate::keywords::processed_extension(&rel)
            && !crate::keywords::plain_copy(&own.plain_copy, &[&in_root]);
        if !processed {
            files.insert(rel, Content::Copy(abs));
            continue;
        }
        let bytes = std::fs::read(&abs).unwrap_or_default();
        let normalized = crate::keywords::lf(&bytes);
        let lower = rel.to_ascii_lowercase();
        let out = match std::str::from_utf8(&normalized) {
            Ok(text) if text.contains('@') => {
                let info = info.get_or_insert_with(|| {
                    crate::keywords::package_info(
                        &package.source,
                        package.pin.as_deref(),
                        profile.debug,
                        text.contains("@project-author@"),
                    )
                });
                let mut out = if lower.ends_with(".lua") || lower.ends_with(".xml") {
                    crate::keywords::apply_library(text, info, lower.ends_with(".xml"))
                } else {
                    crate::keywords::substitute(text, info)
                };
                if out.contains("@file-")
                    && let (Some(cache), Some(pin)) =
                        (wowlua_pkg::Cache::new(), package.pin.as_deref())
                {
                    let project = wowlua_pkg::fetch::PackageVcs {
                        version: info.version.clone(),
                        hash: info.hash.clone(),
                        short_hash: info.short_hash.clone(),
                        revision: info.revision.clone(),
                        author: info.author.clone(),
                        timestamp: info.timestamp,
                    };
                    let file = wowlua_pkg::fetch::package_file_vcs(
                        &cache,
                        &package.source,
                        pin,
                        &rel,
                        &bytes,
                        &project,
                    );
                    out = crate::keywords::substitute_file(&out, &file);
                }
                out.into_bytes()
            }
            _ => normalized,
        };
        let out = if ext.crlf {
            crate::keywords::crlf(&out)
        } else {
            out
        };
        if out == bytes {
            files.insert(rel, Content::Copy(abs));
        } else {
            files.insert(rel, Content::Bytes(out));
        }
    }
    // Its own TOC files, shipped unchanged.
    let tocs: Vec<wowlua_toc::TocFile> = files
        .keys()
        .filter(|f| !f.contains('/') && f.to_ascii_lowercase().ends_with(".toc"))
        .map(|f| {
            let content = match &files[f] {
                Content::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
                Content::Copy(p) => std::fs::read(p)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default(),
            };
            wowlua_toc::TocFile {
                name: f.clone(),
                flavors: Vec::new(),
                content,
            }
        })
        .collect();
    if tocs.is_empty() {
        session.push(
            Diagnostic::plain_error(format!(
                "external addon `{}` has no .toc file in `{}`",
                ext.name,
                dir.display()
            ))
            .with_help("check its `subdir` / `strip-prefix`"),
        );
        return None;
    }
    let version = tocs
        .iter()
        .find_map(|t| {
            wowlua_toc::parse(&t.content)
                .get("Version")
                .map(str::to_string)
        })
        .or_else(|| package.pin.clone())
        .unwrap_or_default();
    let out_dir = wowlua_pkg::source::normalize(&out_root.join(&profile.name).join(&ext.name));
    if write && let Err(error) = crate::build::write_tree(&out_dir, &files) {
        session.push(
            Diagnostic::plain_error(format!("cannot write the build output: {error}"))
                .with_path(&out_dir),
        );
        return None;
    }
    Some(BuildOutput {
        addon_name: ext.name.clone(),
        dir: out_dir,
        profile: profile.name.clone(),
        flavors: ext.flavors.clone(),
        version,
        build_id: "external".into(),
        tocs,
        files,
    })
}
