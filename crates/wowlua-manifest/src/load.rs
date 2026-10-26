//! Loading `wowlua.json`: JSON syntax (E4001), the compiler-version gate
//! (E4300, checked before anything else), schema errors (E4002) and then the
//! semantic checks in [`crate::validate`].

use crate::locate::{JsonPath, PathSeg, locate};
use crate::model::Manifest;
use crate::{LoadOutput, LoadedManifest, validate};
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;
use wowlua_diagnostics::{Diagnostic, Diagnostics};
use wowlua_span::{FileId, SourceDb, Span};

/// Reads and validates the manifest at `path`.
pub fn load(db: &mut SourceDb, path: &Path, compiler: &semver::Version) -> LoadOutput {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let mut output = parse(db, path, text, compiler);
            if let Some(manifest) = &mut output.manifest {
                manifest.path = path.to_path_buf();
                manifest.root = path.parent().map(Path::to_path_buf).unwrap_or_default();
            }
            output
        }
        Err(error) => {
            let mut diagnostics = Diagnostics::new();
            diagnostics.push(
                Diagnostic::plain_error(format!("cannot read the manifest: {error}"))
                    .with_path(path),
            );
            LoadOutput {
                manifest: None,
                diagnostics,
            }
        }
    }
}

/// Validates manifest text. `display_path` names the file in diagnostics.
pub fn parse(
    db: &mut SourceDb,
    display_path: &Path,
    text: String,
    compiler: &semver::Version,
) -> LoadOutput {
    let file = db.add(display_path, text);
    let text = db.get(file).text();
    let mut diagnostics = Diagnostics::new();
    let fail = |diagnostics| LoadOutput {
        manifest: None,
        diagnostics,
    };

    // 1. Strict JSON syntax.
    let raw: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(json_syntax_error(db, file, &error, "E4001", "wowlua.json"));
            return fail(diagnostics);
        }
    };
    if !raw.is_object() {
        diagnostics.push(
            Diagnostic::error("E4002", "the manifest must be a JSON object")
                .with_label(Span::new(file, 0..text.len().min(1)), "expected `{`"),
        );
        return fail(diagnostics);
    }

    // 2. Compiler version gate, before schema validation (§9.16).
    if let Some(required) = raw.pointer("/package/wowlua").and_then(Value::as_str) {
        let path = JsonPath::root().key("package").key("wowlua");
        let span = span_of(text, file, &path, false);
        match semver::VersionReq::parse(required) {
            Ok(req) if !req.matches(compiler) => {
                diagnostics.push(
                    Diagnostic::error(
                        "E4300",
                        format!(
                            "this package requires wowlua {required}, but this is wowlua {compiler}"
                        ),
                    )
                    .with_label(span, "required compiler version")
                    .with_help("install a matching wowlua version"),
                );
                return fail(diagnostics);
            }
            Ok(_) => {}
            Err(error) => {
                diagnostics.push(
                    Diagnostic::error("E4002", format!("invalid version requirement: {error}"))
                        .with_label(span, "not a semver requirement"),
                );
                return fail(diagnostics);
            }
        }
    }

    // 3. Schema: types, required and unknown keys.
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let manifest: Manifest = match serde_path_to_error::deserialize(&mut deserializer) {
        Ok(manifest) => manifest,
        Err(error) => {
            diagnostics.push(schema_error(text, file, &error, "E4002"));
            return fail(diagnostics);
        }
    };

    // 4. Semantic checks.
    let flavors = validate::validate(&manifest, &raw, text, file, &mut diagnostics);
    if diagnostics.has_errors() {
        return fail(diagnostics);
    }
    LoadOutput {
        manifest: Some(LoadedManifest {
            path: display_path.to_path_buf(),
            root: display_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            file,
            manifest,
            flavors,
        }),
        diagnostics,
    }
}

pub(crate) fn json_syntax_error(
    db: &SourceDb,
    file: FileId,
    error: &serde_json::Error,
    code: &'static str,
    what: &str,
) -> Diagnostic {
    let source = db.get(file);
    let offset = source.offset(error.line(), error.column());
    let end = (offset + 1).min(source.text().len());
    let message = strip_position(&error.to_string());
    Diagnostic::error(code, format!("invalid JSON: {message}"))
        .with_label(Span::new(file, offset.min(end)..end), "here")
        .with_note(format!(
            "{what} is strict JSON: no comments and no trailing commas"
        ))
}

/// Removes serde_json's " at line X column Y" suffix.
fn strip_position(message: &str) -> String {
    match message.rfind(" at line ") {
        Some(i) => message[..i].to_string(),
        None => message.to_string(),
    }
}

/// Span of a path's key (or value) in the manifest, falling back to the
/// nearest existing ancestor and finally the file start.
pub(crate) fn span_of(text: &str, file: FileId, path: &JsonPath, value: bool) -> Span {
    let mut current = path.clone();
    loop {
        if let Some(found) = locate(text, &current) {
            let range = if value {
                found.value
            } else {
                found.key_or_value()
            };
            return Span::new(file, range);
        }
        if current.is_root() {
            return Span::file_start(file);
        }
        current = current.parent();
    }
}

static UNKNOWN_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^unknown field `([^`]*)`, (?:expected (.*)|there are no fields)").unwrap()
});

pub(crate) fn schema_error(
    text: &str,
    file: FileId,
    error: &serde_path_to_error::Error<serde_json::Error>,
    code: &'static str,
) -> Diagnostic {
    let path = JsonPath(
        error
            .path()
            .iter()
            .filter_map(|segment| match segment {
                serde_path_to_error::Segment::Map { key } => Some(PathSeg::Key(key.clone())),
                serde_path_to_error::Segment::Seq { index } => Some(PathSeg::Index(*index)),
                _ => None,
            })
            .collect(),
    );
    let message = strip_position(&error.inner().to_string());

    if let Some(captures) = UNKNOWN_FIELD.captures(&message) {
        let key = &captures[1];
        let parent = path.parent();
        let location = if parent.is_root() {
            "at the top level".to_string()
        } else {
            format!("in `{parent}`")
        };
        let mut diagnostic = Diagnostic::error(code, format!("unknown key `{key}` {location}"))
            .with_label(span_of(text, file, &path, false), "unknown key");
        let candidates: Vec<&str> = captures
            .get(2)
            .map(|m| m.as_str().split('`').skip(1).step_by(2).collect())
            .unwrap_or_default();
        if let Some(best) = closest(key, &candidates) {
            diagnostic = diagnostic.with_help(format!("did you mean `{best}`?"));
        } else if !candidates.is_empty() {
            diagnostic = diagnostic.with_note(format!("valid keys: {}", candidates.join(", ")));
        }
        return diagnostic;
    }

    let (label, at) = if message.starts_with("missing field") {
        ("in this object", span_of(text, file, &path, false))
    } else {
        ("here", span_of(text, file, &path, true))
    };
    let location = if path.is_root() {
        String::new()
    } else {
        format!(" at `{path}`")
    };
    Diagnostic::error(code, format!("{message}{location}")).with_label(at, label)
}

/// The candidate closest to `word` by edit distance, if it is close enough to
/// be a likely typo.
pub(crate) fn closest<'a>(word: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .map(|c| (edit_distance(word, c), *c))
        .filter(|(d, c)| *d <= (c.len().max(word.len()) / 3).max(1))
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// Optimal string alignment distance: insertions, deletions, substitutions
/// and transpositions of adjacent characters each cost one.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use wowlua_diagnostics::{ColorChoice, MessageFormat, Renderer};

    fn run(text: &str) -> (SourceDb, LoadOutput) {
        let mut db = SourceDb::new();
        let output = parse(
            &mut db,
            Path::new("wowlua.json"),
            text.to_string(),
            &semver::Version::new(0, 1, 0),
        );
        (db, output)
    }

    fn short(db: &SourceDb, output: &LoadOutput) -> Vec<String> {
        let renderer = Renderer::new(MessageFormat::Short, ColorChoice::Never);
        output
            .diagnostics
            .iter()
            .map(|d| renderer.render_to_string(db, d).trim_end().to_string())
            .collect()
    }

    #[test]
    fn minimal_manifest_loads() {
        let (_, out) = run(r#"{ "package": { "name": "my-addon", "version": "1.0.0" } }"#);
        let m = out.manifest.expect("valid");
        assert_eq!(m.addon_name(), "MyAddon");
        assert_eq!(m.flavors, vec!["standard"]);
    }

    #[test]
    fn json_syntax_errors_are_e4001() {
        let (db, out) = run("{ \"package\": { \"name\": \"a\", } }");
        assert!(out.manifest.is_none());
        let lines = short(&db, &out);
        assert!(lines[0].contains("error[E4001]"), "{lines:?}");
    }

    #[test]
    fn unknown_keys_suggest_fixes() {
        let (db, out) = run(
            r#"{ "package": { "name": "a-b", "version": "1.0.0" }, "addon": { "titel": "x" } }"#,
        );
        let lines = short(&db, &out);
        assert_eq!(
            lines,
            vec!["wowlua.json:1:64: error[E4002]: unknown key `titel` in `addon`"]
        );
        let d = out.diagnostics.iter().next().unwrap();
        assert_eq!(d.help.as_deref(), Some("did you mean `title`?"));
    }

    #[test]
    fn wrong_types_point_at_values() {
        let (db, out) = run(r#"{ "package": { "name": "ab", "version": 1 } }"#);
        let lines = short(&db, &out);
        assert!(
            lines[0].contains("1:41: error[E4002]: invalid type"),
            "{lines:?}"
        );
        assert!(lines[0].contains("at `package.version`"), "{lines:?}");
    }

    #[test]
    fn compiler_version_is_checked_first() {
        let (db, out) = run(
            r#"{ "package": { "name": "ab", "version": "1.0.0", "wowlua": "^9" }, "future-key": 1 }"#,
        );
        let lines = short(&db, &out);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("error[E4300]"), "{lines:?}");
    }

    #[test]
    fn edit_distance_suggestions() {
        assert_eq!(closest("titel", &["title", "notes"]), Some("title"));
        assert_eq!(closest("zzz", &["title", "notes"]), None);
    }
}
