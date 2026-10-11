//! Checking and compiling sources (spec compiler §13.3.1): every Lua and
//! wowlua file is parsed once with the compiler's own parser; then, per
//! build configuration and flavor, the package's modules are analyzed
//! together (imports, exports, `@if`), `.wowlua` files are lowered and
//! emitted, and Lua 5.1's code-generation limits are checked with the
//! embedded VM.

use crate::luacheck;
use crate::sources::{Source, SourceKind};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use wowlua_diagnostics::Diagnostic;
use wowlua_lower::{Context, ImportContext};
use wowlua_luair::{CommentMode, EmitOptions, LineMode};
use wowlua_manifest::Profile;
use wowlua_manifest::model::LineMode as ManifestLineMode;
use wowlua_sema::meta::{Meta, MetaKind};
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
    /// Why each edge exists (the use and a description).
    pub edge_reasons: HashMap<(String, String), (wowlua_span::Span, String)>,
    /// Module IDs registered with the runtime.
    pub modules: Vec<String>,
    pub needs_runtime: bool,
    /// `wowlua:meta` paths used as runtime values.
    pub meta_used: BTreeSet<(MetaKind, Vec<String>)>,
    /// Runtime slices the compiled files use.
    pub slices: BTreeSet<wowlua_runtime::Slice>,
    /// Static types of `typeName(expr)` arguments (from the checker).
    pub type_names: HashMap<wowlua_span::Span, String>,
}

/// A declaration file with its contents.
pub struct Declaration {
    pub source: Source,
    pub file: FileId,
    pub bytes: Vec<u8>,
}

/// The `check` settings of the manifest (spec §9.8).
pub struct CheckConfig {
    pub mode: wowlua_manifest::model::CheckMode,
    pub strict: globset::GlobSet,
    pub lua: bool,
    /// `check.globals` and the SavedVariables.
    pub globals: std::collections::HashSet<String>,
}

/// A declaration file of one flavor (the wow-api types), parsed.
pub struct ParsedDeclaration {
    pub source: Source,
    pub parse: Parse,
}

/// How a source is checked (spec §2.1), or `None`.
fn check_settings(
    source: &Source,
    parse: &Parse,
    cfg: &CheckConfig,
) -> Option<wowlua_sema::check::FileSettings> {
    use wowlua_manifest::model::CheckMode;
    use wowlua_sema::check::{FileSettings, query::Mode};
    let directive = |n: &str| parse.chunk.directives.iter().any(|d| d.name.as_ref() == n);
    if directive("nocheck") {
        return None;
    }
    let opted_strict = directive("strict")
        || cfg.strict.is_match(&source.rel)
        || cfg.strict.is_match(&source.src_rel);
    match source.kind {
        SourceKind::Wowlua => {
            let pure = wowlua_syntax::features::features(&parse.chunk).is_empty();
            if opted_strict {
                Some(FileSettings {
                    mode: Mode::Strict,
                    pure_lua: false,
                })
            } else {
                match cfg.mode {
                    CheckMode::Nocheck => None,
                    // A project-wide strict mode does not apply to pure Lua.
                    CheckMode::Strict if !pure => Some(FileSettings {
                        mode: Mode::Strict,
                        pure_lua: false,
                    }),
                    _ => Some(FileSettings {
                        mode: Mode::Basic,
                        pure_lua: pure,
                    }),
                }
            }
        }
        SourceKind::Lua if opted_strict => Some(FileSettings {
            mode: Mode::Strict,
            pure_lua: false,
        }),
        SourceKind::Lua if cfg.lua && cfg.mode != CheckMode::Nocheck => Some(FileSettings {
            mode: Mode::Basic,
            pure_lua: true,
        }),
        _ => None,
    }
}

/// The parsed sources of a package (shared by all flavors).
pub struct Parsed {
    pub(crate) parses: Vec<Option<Parse>>,
    pub(crate) ok: Vec<bool>,
    declarations: Vec<Parse>,
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

/// Parses every source once and checks the plain Lua files.
pub fn parse_sources(
    db: &SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &[(Source, FileId)],
    declarations: &[Declaration],
    raw: &HashMap<String, Vec<u8>>,
) -> Parsed {
    let mut parses = Vec::with_capacity(files.len());
    let mut ok = vec![true; files.len()];
    for (i, (source, file)) in files.iter().enumerate() {
        if source.kind == SourceKind::Xml {
            parses.push(None);
            continue;
        }
        let bytes = &raw[&source.rel];
        let parsed = wowlua_syntax::parse(bytes, *file);
        if parsed.has_errors() {
            diagnostics.extend(parsed.diagnostics.clone());
            ok[i] = false;
        } else if source.kind == SourceKind::Lua {
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
                ok[i] = false;
            } else if let Some(d) = luacheck::check(db, *file, bytes) {
                diagnostics.push(d);
            }
        }
        parses.push(Some(parsed));
    }
    let declarations = declarations
        .iter()
        .map(|d| {
            let parsed = wowlua_syntax::parse(&d.bytes, d.file);
            if parsed.has_errors() {
                diagnostics.extend(parsed.diagnostics.clone());
            }
            parsed
        })
        .collect();
    Parsed {
        parses,
        ok,
        declarations,
    }
}

/// Adds the standard-library modules the sources import, transitively
/// (tree-shaking at module granularity, spec §8.1): each is compiled into
/// the addon as `wowlua/std/<name>.lua`.
pub fn add_stdlib(
    db: &mut SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &mut Vec<(Source, FileId)>,
    declarations: &mut Vec<Declaration>,
    raw: &mut HashMap<String, Vec<u8>>,
    parsed: &mut Parsed,
) {
    let mut loaded: BTreeSet<String> = BTreeSet::new();
    let mut scanned = 0;
    loop {
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        for parse in parsed.parses[scanned..].iter().flatten() {
            for s in &parse.chunk.block.stmts {
                if let wowlua_syntax::ast::StmtKind::Import(import) = &s.kind {
                    let spec = String::from_utf8_lossy(&import.from.value).into_owned();
                    if let Some(name) = spec.strip_prefix("wowlua:")
                        && name != "meta"
                        && !loaded.contains(name)
                    {
                        wanted.insert(name.to_string());
                    }
                }
            }
        }
        scanned = parsed.parses.len();
        if wanted.is_empty() {
            return;
        }
        for name in wanted {
            loaded.insert(name.clone());
            // Unknown modules are reported when the import is resolved.
            let Some(module) = wowlua_stdlib::module(&name) else {
                continue;
            };
            let src_rel = wowlua_stdlib::src_rel(&name);
            let rel = format!("<wowlua>/{name}.wowlua");
            let file = db.add(&rel, module.source.clone());
            let source = Source {
                rel: rel.clone(),
                src_rel,
                abs: std::path::PathBuf::new(),
                kind: SourceKind::Wowlua,
            };
            let parse = wowlua_syntax::parse(module.source.as_bytes(), file);
            let ok = !parse.has_errors();
            if !ok {
                diagnostics.extend(parse.diagnostics.clone());
            }
            raw.insert(rel, module.source.into_bytes());
            files.push((source, file));
            parsed.parses.push(Some(parse));
            parsed.ok.push(ok);
            if let Some(text) = module.declaration {
                let rel = format!("<wowlua>/{name}.d.wowlua");
                let file = db.add(&rel, text.clone());
                let parse = wowlua_syntax::parse(text.as_bytes(), file);
                if parse.has_errors() {
                    diagnostics.extend(parse.diagnostics.clone());
                }
                declarations.push(Declaration {
                    source: Source {
                        rel,
                        src_rel: format!("{}{name}.d.wowlua", wowlua_sema::STDLIB_PREFIX),
                        abs: std::path::PathBuf::new(),
                        kind: SourceKind::Wowlua,
                    },
                    file,
                    bytes: text.into_bytes(),
                });
                parsed.declarations.push(parse);
            }
        }
    }
}

/// Compiles the package for one build configuration and flavor.
#[allow(clippy::too_many_arguments)]
pub fn compile_flavor(
    db: &SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &[(Source, FileId)],
    declarations: &[Declaration],
    raw: &HashMap<String, Vec<u8>>,
    parsed: &Parsed,
    profile: &Profile,
    meta: Arc<Meta>,
    layout: wowlua_sema::loadorder::Layout,
    api: &[ParsedDeclaration],
    check: &CheckConfig,
    shared: Option<&wowlua_sema::check::flavors::SharedFlavors>,
) -> CompileOutput {
    let options = EmitOptions {
        line_mode: line_mode(profile),
        // Minification (which strips comments) is not available yet.
        comments: CommentMode::All,
        indent: "\t".into(),
    };
    let mut out = CompileOutput::default();

    let mut inputs: Vec<FileInput> = files
        .iter()
        .zip(&parsed.parses)
        .map(|((source, file), parse)| FileInput {
            src_rel: source.src_rel.clone(),
            kind: match source.kind {
                SourceKind::Wowlua => FileKind::Wowlua,
                SourceKind::Lua => FileKind::Lua,
                SourceKind::Xml => FileKind::Xml,
            },
            parse: parse.as_ref(),
            xml: (source.kind == SourceKind::Xml)
                .then(|| {
                    std::str::from_utf8(&raw[&source.rel])
                        .ok()
                        .map(|t| (t, *file))
                })
                .flatten(),
        })
        .collect();
    for (d, parse) in declarations.iter().zip(&parsed.declarations) {
        inputs.push(FileInput {
            src_rel: d.source.src_rel.clone(),
            kind: FileKind::Declaration,
            parse: Some(parse),
            xml: None,
        });
    }
    for d in api {
        inputs.push(FileInput {
            src_rel: d.source.src_rel.clone(),
            kind: FileKind::Declaration,
            parse: Some(&d.parse),
            xml: None,
        });
    }
    let analysis = wowlua_sema::analyze(&inputs, Some(&meta));
    diagnostics.extend(analysis.diagnostics.iter().cloned());
    // Load order (spec §5.4): files listed in `build.load-order` keep their
    // listed order; files outside this flavor do not take part.
    let markers = wowlua_sema::usage::Markers::from_table();
    let order = wowlua_sema::loadorder::analyze(&inputs, &analysis, layout, &markers);
    diagnostics.extend(order.diagnostics.iter().cloned());
    for &(a, b) in &order.constraints {
        if a < files.len() && b < files.len() {
            let pair = (files[a].0.rel.clone(), files[b].0.rel.clone());
            if let Some(reason) = order.reasons.get(&(a, b)) {
                out.edge_reasons.insert(pair.clone(), reason.clone());
            }
            out.edges.push(pair);
        }
    }
    // Type checking (spec chapter 2).
    let settings: Vec<Option<wowlua_sema::check::FileSettings>> = (0..inputs.len())
        .map(
            |i| match (files.get(i), parsed.parses.get(i).and_then(Option::as_ref)) {
                (Some((source, _)), Some(parse))
                    if parsed.ok[i] && !wowlua_sema::is_compiler_known(&source.src_rel) =>
                {
                    check_settings(source, parse, check)
                }
                _ => None,
            },
        )
        .collect();
    if settings.iter().any(Option::is_some) {
        let checked = wowlua_sema::check::check_package_full(
            &inputs,
            &analysis,
            &settings,
            &check.globals,
            shared,
        );
        diagnostics.extend(checked.diagnostics);
        out.type_names = checked.type_names;
    }
    for info in analysis.modules.iter().flatten() {
        if info.is_module {
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
    }
    out.modules.sort();
    out.modules.dedup();

    for (i, (source, file)) in files.iter().enumerate() {
        if source.kind != SourceKind::Wowlua || !parsed.ok[i] {
            continue;
        }
        let parse = parsed.parses[i].as_ref().expect("parsed");
        let bytes = &raw[&source.rel];
        let mut ctx = context(&analysis, i, &inputs, meta.clone(), &order.lazy);
        ctx.file = source.rel.clone();
        ctx.debug = profile.debug;
        ctx.tools = wowlua_lower::Tools {
            assertions: profile.assertions,
            dev_console: profile.dev_console,
            profiling: profile.instrument != wowlua_manifest::model::Instrument::None,
        };
        ctx.type_names = out
            .type_names
            .iter()
            .filter(|(span, _)| span.file == *file)
            .map(|(span, name)| ((span.start, span.end), name.clone()))
            .collect();
        ctx.compiler_known = wowlua_sema::is_compiler_known(&source.src_rel);
        let lowered = match wowlua_lower::lower_module(parse, bytes, &ctx) {
            Ok(lowered) => lowered,
            Err(errors) => {
                diagnostics.extend(errors);
                continue;
            }
        };
        out.needs_runtime |= lowered.uses_runtime;
        out.meta_used.extend(lowered.meta_used);
        out.slices.extend(lowered.slices);
        diagnostics.extend(lowered.warnings);
        let emitted = wowlua_luair::emit(&lowered.chunk, &options);
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
fn context(
    analysis: &wowlua_sema::Analysis,
    i: usize,
    inputs: &[FileInput],
    meta: Arc<Meta>,
    lazy: &std::collections::HashSet<(usize, u32)>,
) -> Context {
    let mut ctx = Context {
        meta: Some(meta),
        ..Context::default()
    };
    let Some(info) = &analysis.modules[i] else {
        return ctx;
    };
    ctx.module = info.is_module.then(|| info.id.clone());
    ctx.classes = info
        .classes
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    ctx.enums = info
        .enums
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for import in &info.imports {
        let target = match &import.target {
            Target::Module(m) => {
                let module = analysis.modules[*m].as_ref();
                let exports = module
                    .map(|t| {
                        t.exports
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                ImportContext::Module {
                    id: wowlua_sema::module_id(&inputs[*m].src_rel),
                    lazy: lazy.contains(&(i, import.start)),
                    exports,
                    attrs: module.map(|t| t.export_attrs.clone()).unwrap_or_default(),
                    intrinsics: module.map(|t| t.intrinsics.clone()).unwrap_or_default(),
                }
            }
            Target::Lua {
                file,
                declared: true,
            } => ImportContext::Module {
                id: wowlua_sema::module_id(&inputs[*file].src_rel),
                lazy: lazy.contains(&(i, import.start)),
                exports: HashMap::<String, Export>::new(),
                attrs: HashMap::new(),
                intrinsics: HashMap::new(),
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
