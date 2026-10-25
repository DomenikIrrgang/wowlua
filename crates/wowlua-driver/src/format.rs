//! `wowlua fmt` (spec tooling §16.3): formats `.wowlua` files of the
//! package (and `.lua` files with `fmt.lua = true`), configured by the
//! manifest's `fmt` section.

use crate::Session;
use crate::project::Project;
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_fmt::{Config, Error, QuoteStyle};
use wowlua_manifest::model::{Fmt, Indent, QuoteStyle as ManifestQuote};

#[derive(Clone, Debug, Default)]
pub struct FormatRequest {
    /// Files or directories; empty means the whole package.
    pub paths: Vec<PathBuf>,
    /// Report files that would change instead of writing them.
    pub check: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FormatReport {
    /// Files that changed (or would change with `check`), relative paths.
    pub changed: Vec<String>,
    pub unchanged: usize,
    /// Files left unchanged because they use wowlua syntax M1 cannot format.
    pub skipped: usize,
}

/// The formatter configuration from the manifest's `fmt` section.
pub fn config(fmt: &Fmt) -> Config {
    let (indent, indent_width) = match fmt.indent {
        Indent::Tab(_) => ("\t".to_string(), 4),
        Indent::Spaces(n) => (" ".repeat(usize::from(n)), usize::from(n)),
    };
    Config {
        indent,
        indent_width,
        line_width: fmt.line_width as usize,
        quote: match fmt.quote_style {
            ManifestQuote::Double => QuoteStyle::Double,
            ManifestQuote::Single => QuoteStyle::Single,
            ManifestQuote::Preserve => QuoteStyle::Preserve,
        },
    }
}

fn wanted(path: &Path, lua: bool) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.ends_with(".wowlua") || (lua && name.ends_with(".lua"))
}

/// The files to format: explicit files, or the matching files below the
/// given directories (the package root by default), respecting `.gitignore`
/// and skipping the build output and `fmt.exclude`.
fn files(project: &Project, request: &FormatRequest, lua: bool) -> Vec<PathBuf> {
    let root = &project.manifest.root;
    let out_dir = project.manifest.out_dir();
    let exclude = crate::sources::glob_set(&project.manifest.manifest.fmt.exclude);
    let starts: Vec<PathBuf> = if request.paths.is_empty() {
        vec![root.clone()]
    } else {
        request.paths.clone()
    };
    let mut found = Vec::new();
    for start in starts {
        if crate::host::is_file(&start) {
            found.push(start);
            continue;
        }
        for path in walk_files(&start) {
            let path = path.as_path();
            if path.starts_with(&out_dir) {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            if wanted(path, lua) && !exclude.is_match(&rel) {
                found.push(path.to_path_buf());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

pub fn format(session: &mut Session, project: &Project, request: &FormatRequest) -> FormatReport {
    let fmt = &project.manifest.manifest.fmt;
    let config = config(fmt);
    let root = &project.manifest.root;
    let mut report = FormatReport::default();
    for path in files(project, request, fmt.lua) {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = match crate::host::read(&path) {
            Ok(b) => b,
            Err(e) => {
                session.push(
                    Diagnostic::plain_error(format!("cannot read file: {e}")).with_path(&rel),
                );
                continue;
            }
        };
        let file = session
            .db
            .add(&rel, String::from_utf8_lossy(&bytes).into_owned());
        match wowlua_fmt::format(&bytes, file, &config) {
            Ok(out) if out == bytes => report.unchanged += 1,
            Ok(out) => {
                if !request.check
                    && let Err(e) = crate::host::write(&path, &out)
                {
                    session.push(
                        Diagnostic::plain_error(format!("cannot write file: {e}")).with_path(&rel),
                    );
                    continue;
                }
                report.changed.push(rel);
            }
            Err(Error::Syntax(diagnostics)) => session.diagnostics.extend(diagnostics),
            Err(Error::Unsupported(note)) => {
                report.skipped += 1;
                session.push(note);
            }
            Err(Error::Unstable(reason)) => session.push(
                Diagnostic::plain_error(format!(
                    "internal formatter error: {reason}; the file was left unchanged"
                ))
                .with_path(&rel)
                .with_note("please report this as a bug"),
            ),
        }
    }
    report
}

/// The files below `start` that are not hidden; on a computer also not
/// ignored by `.gitignore` or `.ignore`.
fn walk_files(start: &Path) -> Vec<PathBuf> {
    if crate::host::installed().is_some() {
        return crate::host::walk(start, false, &|_| true);
    }
    #[allow(clippy::disallowed_methods)] // (no host installed: the disk's)
    let walker = ignore::WalkBuilder::new(start)
        .hidden(true)
        .require_git(false)
        .build();
    walker
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .collect()
}
