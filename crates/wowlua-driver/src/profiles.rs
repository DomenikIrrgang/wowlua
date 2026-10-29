//! A package's world profiles (spec Tooling §16.1.4, decisions T213,
//! T401): `tests/profiles/<name>.json`, checked with the test side (E4400,
//! E4401), and the profiles the manifest's `test.profile` and the test
//! files' `config({ profile = "…" })` name, which must be there (E4402).

use crate::Session;
use crate::project::Project;
use std::ops::Range;
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::world::PROFILES_DIR;
use wowlua_span::Span;
use wowlua_syntax::view::*;

/// A literal text a test file's `config({ … })` gives `key` (read from its
/// syntax, before anything loads: `profile`, `platform`), and where the
/// literal is in `source`.
pub fn config_text(source: &[u8], key: &str) -> Option<(String, Range<usize>)> {
    let file = wowlua_span::SourceDb::new().add("", "");
    let parse = wowlua_syntax::parse(source, file);
    parse.view().block().stmts().iter().find_map(|s| {
        let StmtKind::Call(call) = s.kind() else {
            return None;
        };
        let ExprKind::Call { callee, args } = call.kind() else {
            return None;
        };
        if !matches!(callee.kind(), ExprKind::Name(n) if n.text() == "config") {
            return None;
        }
        let ExprKind::Table(fields) = args.exprs().first()?.kind() else {
            return None;
        };
        fields.iter().find_map(|f| match f {
            TableField::Named { name, value, .. } if name.text() == key => match value.kind() {
                ExprKind::String(lit) => Some((
                    String::from_utf8_lossy(&lit.value()).into_owned(),
                    value.span().range(),
                )),
                _ => None,
            },
            _ => None,
        })
    })
}

/// The world profiles of the package at `root`: names and files, by name.
pub fn list(root: &Path) -> Vec<(String, PathBuf)> {
    let dir = root.join(PROFILES_DIR);
    let mut out: Vec<(String, PathBuf)> = crate::host::read_dir(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        })
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
        .collect();
    out.sort();
    out
}

/// Checks the package's world profiles and the names that refer to them;
/// `tests` are the test files (in the session's sources) and their text.
pub(crate) fn check<'a>(
    session: &mut Session,
    project: &Project,
    tests: impl Iterator<Item = (wowlua_span::FileId, &'a [u8])>,
) {
    let root = project.root();
    let profiles = list(root);
    for (name, path) in &profiles {
        let Ok(text) = crate::host::read_to_string(path) else {
            continue;
        };
        let rel = PathBuf::from(PROFILES_DIR).join(format!(
            "{name}.{}",
            path.extension()
                .map(|e| e.to_string_lossy())
                .unwrap_or_default()
        ));
        let (_, found) = wowlua_manifest::world::check_for(
            &mut session.db,
            &rel,
            text,
            &project.manifest.flavors,
        );
        for d in found.into_vec() {
            session.push(d);
        }
    }
    let known = |name: &str| profiles.iter().any(|(n, _)| n == name);
    let missing = |name: &str| {
        let mut names: Vec<&str> = profiles.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        let help = if names.is_empty() {
            format!("add `{PROFILES_DIR}/{name}.json`")
        } else {
            format!(
                "add `{PROFILES_DIR}/{name}.json`, or name one of {}",
                names.join(", ")
            )
        };
        (
            format!("no world profile `{name}` (`{PROFILES_DIR}/{name}.json`)"),
            help,
        )
    };
    // The manifest's `test.profile`.
    let manifest = &project.manifest;
    if let Some(name) = &manifest.manifest.test.profile
        && !known(name)
    {
        let text = session.db.get(manifest.file).text().to_string();
        let at = wowlua_manifest::locate::locate(
            &text,
            &wowlua_manifest::locate::JsonPath::root()
                .key("test")
                .key("profile"),
        )
        .map(|l| l.value)
        .unwrap_or(0..0);
        let (message, help) = missing(name);
        session.push(
            Diagnostic::error("E4402", message)
                .with_label(Span::new(manifest.file, at), "names no profile")
                .with_help(help),
        );
    }
    // The test files' `config({ profile = "…" })`.
    for (file, source) in tests {
        let Some((name, at)) = config_text(source, "profile") else {
            continue;
        };
        if known(&name) {
            continue;
        }
        let (message, help) = missing(&name);
        session.push(
            Diagnostic::error("E4402", message)
                .with_label(Span::new(file, at), "names no profile")
                .with_help(help),
        );
    }
}
