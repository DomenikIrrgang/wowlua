//! Checking and compiling sources (spec compiler §13.3.1): every Lua and
//! wowlua file is parsed with the compiler's own parser, the package's
//! modules are analyzed together (imports, exports, order), `.wowlua` files
//! are lowered and emitted, and Lua 5.1's code-generation limits are checked
//! with the embedded VM.

use crate::luacheck;
use crate::sources::{Source, SourceKind};
use std::collections::HashMap;
use wowlua_diagnostics::Diagnostic;
use wowlua_lower::{Context, ImportContext};
use wowlua_luair::{CommentMode, EmitOptions, LineMode};
use wowlua_manifest::Profile;
use wowlua_manifest::model::LineMode as ManifestLineMode;
use wowlua_sema::{Export, FileInput, FileKind, Target};
use wowlua_span::{FileId, SourceDb};
use wowlua_syntax::Parse;

/// The compiled form of a `.wowlua` source.
pub struct Compiled {
    pub code: Vec<u8>,
    /// Source Map v3 (when the profile enables source maps).
    pub map: Option<String>,
}

#[derive(Default)]
pub struct CompileOutput {
    /// Compiled `.wowlua` sources by `Source::rel`.
    pub compiled: HashMap<String, Compiled>,
    /// `(a, b)`: source `a` must load before source `b` (by `Source::rel`).
    pub edges: Vec<(String, String)>,
    /// Module IDs registered with the runtime; empty when no runtime is
    /// needed.
    pub modules: Vec<String>,
    pub needs_runtime: bool,
}

/// A declaration file with its contents.
pub struct Declaration {
    pub source: Source,
    pub file: FileId,
    pub bytes: Vec<u8>,
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

/// Checks and compiles the package's sources.
pub fn compile_sources(
    db: &SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &[(Source, FileId)],
    declarations: &[Declaration],
    raw: &HashMap<String, Vec<u8>>,
    profile: &Profile,
) -> CompileOutput {
    let options = EmitOptions {
        line_mode: line_mode(profile),
        // Minification (which strips comments) is not available yet.
        comments: CommentMode::All,
        indent: "\t".into(),
    };
    let mut out = CompileOutput::default();

    // Parse every Lua and wowlua file.
    let mut parses: Vec<Option<Parse>> = Vec::with_capacity(files.len());
    let mut ok = vec![true; files.len()];
    for (i, (source, file)) in files.iter().enumerate() {
        if source.kind == SourceKind::Xml {
            parses.push(None);
            continue;
        }
        let parsed = wowlua_syntax::parse(&raw[&source.rel], *file);
        if parsed.has_errors() {
            diagnostics.extend(parsed.diagnostics.clone());
            ok[i] = false;
        }
        parses.push(Some(parsed));
    }
    let decl_parses: Vec<Parse> = declarations
        .iter()
        .map(|d| {
            let parsed = wowlua_syntax::parse(&d.bytes, d.file);
            if parsed.has_errors() {
                diagnostics.extend(parsed.diagnostics.clone());
            }
            parsed
        })
        .collect();

    // Analyze the package's modules together.
    let mut inputs: Vec<FileInput> = files
        .iter()
        .zip(&parses)
        .map(|((source, _), parse)| FileInput {
            src_rel: source.src_rel.clone(),
            kind: match source.kind {
                SourceKind::Wowlua => FileKind::Wowlua,
                SourceKind::Lua => FileKind::Lua,
                SourceKind::Xml => FileKind::Xml,
            },
            parse: parse.as_ref(),
        })
        .collect();
    for (d, parse) in declarations.iter().zip(&decl_parses) {
        inputs.push(FileInput {
            src_rel: d.source.src_rel.clone(),
            kind: FileKind::Declaration,
            parse: Some(parse),
        });
    }
    let analysis = wowlua_sema::analyze(&inputs);
    diagnostics.extend(analysis.diagnostics.iter().cloned());
    for &(a, b) in &analysis.edges {
        if a < files.len() && b < files.len() {
            out.edges
                .push((files[a].0.rel.clone(), files[b].0.rel.clone()));
        }
    }
    for (i, info) in analysis.modules.iter().enumerate() {
        if let Some(info) = info
            && info.is_module
        {
            out.needs_runtime = true;
            out.modules.push(info.id.clone());
            for import in &info.imports {
                if let Target::Lua {
                    file,
                    declared: true,
                } = import.target
                {
                    out.modules
                        .push(wowlua_sema::module_id(&inputs[file].src_rel));
                }
            }
        }
        let _ = i;
    }
    out.modules.sort();
    out.modules.dedup();

    for (i, (source, file)) in files.iter().enumerate() {
        if source.kind == SourceKind::Xml || !ok[i] {
            continue;
        }
        let parsed = parses[i].as_ref().expect("parsed");
        let bytes = &raw[&source.rel];
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
        let ctx = context(&analysis, i, &inputs);
        let chunk = match wowlua_lower::lower_with(parsed, bytes, &ctx) {
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
        out.compiled.insert(
            source.rel.clone(),
            Compiled {
                code: emitted.text,
                map,
            },
        );
    }
    out
}

/// The lowering context of input `i` from the package analysis.
fn context(analysis: &wowlua_sema::Analysis, i: usize, inputs: &[FileInput]) -> Context {
    let Some(info) = &analysis.modules[i] else {
        return Context::default();
    };
    let mut ctx = Context {
        module: info.is_module.then(|| info.id.clone()),
        imports: HashMap::new(),
    };
    for import in &info.imports {
        let target = match &import.target {
            Target::Module(m) => {
                let exports = analysis.modules[*m]
                    .as_ref()
                    .map(|t| {
                        t.exports
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                ImportContext::Module {
                    id: wowlua_sema::module_id(&inputs[*m].src_rel),
                    lazy: false,
                    exports,
                }
            }
            Target::Lua {
                file,
                declared: true,
            } => ImportContext::Module {
                id: wowlua_sema::module_id(&inputs[*file].src_rel),
                lazy: false,
                exports: HashMap::<String, Export>::new(),
            },
            Target::Lua {
                declared: false, ..
            }
            | Target::Xml(_) => ImportContext::LoadOnly,
            Target::Meta => ImportContext::Meta,
        };
        ctx.imports.insert(import.start, target);
    }
    ctx
}
