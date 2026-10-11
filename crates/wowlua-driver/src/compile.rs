//! Checking and compiling sources (spec compiler §13.3.1, stages 4 and
//! 14–19 as far as milestone M1 implements them): every Lua and wowlua file
//! is parsed with the compiler's own parser; `.wowlua` files are lowered and
//! emitted; Lua 5.1's code-generation limits are checked with the embedded
//! VM.

use crate::luacheck;
use crate::sources::{Source, SourceKind};
use std::collections::HashMap;
use wowlua_diagnostics::Diagnostic;
use wowlua_luair::{CommentMode, EmitOptions, LineMode};
use wowlua_manifest::Profile;
use wowlua_manifest::model::LineMode as ManifestLineMode;
use wowlua_span::{FileId, SourceDb};

/// The compiled form of a `.wowlua` source.
pub struct Compiled {
    pub code: Vec<u8>,
    /// Source Map v3 (when the profile enables source maps).
    pub map: Option<String>,
}

/// The line mode for unbundled output: `preserve` unless the profile sets
/// one (spec codegen §14.5; bundling is not available yet).
fn line_mode(profile: &Profile) -> LineMode {
    match profile.line_mode {
        Some(ManifestLineMode::Statement) => LineMode::Statement,
        Some(ManifestLineMode::Pretty) => LineMode::Pretty,
        _ => LineMode::Preserve,
    }
}

/// Checks every Lua and wowlua source; returns the compiled `.wowlua`
/// sources by `Source::rel`.
pub fn compile_sources(
    db: &SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &[(Source, FileId)],
    raw: &HashMap<String, Vec<u8>>,
    profile: &Profile,
) -> HashMap<String, Compiled> {
    let options = EmitOptions {
        line_mode: line_mode(profile),
        // Minification (which strips comments) is not available yet.
        comments: CommentMode::All,
        indent: "\t".into(),
    };
    let mut compiled = HashMap::new();
    for (source, file) in files.iter().filter(|(s, _)| s.kind != SourceKind::Xml) {
        let bytes = &raw[&source.rel];
        let parsed = wowlua_syntax::parse(bytes, *file);
        if parsed.has_errors() {
            diagnostics.extend(parsed.diagnostics);
            continue;
        }
        if source.kind == SourceKind::Lua {
            if let Some((feature, span)) = wowlua_syntax::features::features(&parsed.chunk).first()
            {
                diagnostics.push(
                    Diagnostic::error(
                        "E0101",
                        format!(
                            "{} are wowlua syntax, but this is a `.lua` file",
                            feature.describe()
                        ),
                    )
                    .with_label(*span, "not Lua 5.1")
                    .with_help("rename the file to `.wowlua`"),
                );
                continue;
            }
            if let Some(d) = luacheck::check(db, *file, bytes) {
                diagnostics.push(d);
            }
            continue;
        }
        let chunk = match wowlua_lower::lower(&parsed, bytes) {
            Ok(chunk) => chunk,
            Err(errors) => {
                diagnostics.extend(errors);
                continue;
            }
        };
        let emitted = wowlua_luair::emit(&chunk, &options);
        // The output must parse (spec compiler §13.8, item 2).
        let reparsed = wowlua_syntax::parse(&emitted.text, *file);
        if reparsed.has_errors() {
            diagnostics.push(
                Diagnostic::plain_error(format!(
                    "internal compiler error: the code emitted for this file does not parse: {}",
                    reparsed.diagnostics[0].message
                ))
                .with_path(&source.rel),
            );
            continue;
        }
        if let Some(d) = luacheck::check(db, *file, &emitted.text) {
            diagnostics.push(d);
        }
        let output = source.output_path();
        let map = profile.source_maps.then(|| {
            let name = output.rsplit('/').next().unwrap_or(&output);
            wowlua_luair::sourcemap::source_map(&emitted.mappings, name, &source.rel)
        });
        compiled.insert(
            source.rel.clone(),
            Compiled {
                code: emitted.text,
                map,
            },
        );
    }
    compiled
}
