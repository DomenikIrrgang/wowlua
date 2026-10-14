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
use wowlua_luair::LineMode;
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
    /// Runtime checks (from the checker, spec §7.5).
    pub value_checks: HashMap<wowlua_span::Span, wowlua_sema::check::runtime::ValueCheck>,
    /// Modules `dead-code` removed (by `Source::rel`, spec optimization
    /// §15.2.1).
    pub removed: BTreeSet<String>,
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
    /// The SavedVariables, for `wowlua:saved`'s checks.
    pub stdlib: wowlua_sema::stdlib::StdlibConfig,
}

/// A declaration file of one flavor (the wow-api types), parsed.
pub struct ParsedDeclaration {
    pub source: Source,
    pub file: FileId,
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

/// What minification applies to (spec optimization §15.4).
#[derive(Default)]
pub struct Minify {
    pub passes: wowlua_luair::Passes,
    /// Plain Lua sources minified and optimized too (by `Source::rel`).
    pub lua: std::collections::HashSet<String>,
    /// `rename-private`: text renamed methods must not occur in (the XML
    /// and embedded libraries).
    pub foreign: Vec<Vec<u8>>,
    /// `build.keep`: modules `dead-code` never removes.
    pub keep: Vec<String>,
}

/// The profile's optimizations (spec optimization §15.2).
#[derive(Clone, Copy, Debug, Default)]
pub struct Optimizations {
    pub dead_code: bool,
    pub inline: bool,
    pub cache_globals: bool,
}

/// A chunk optimized, minified and emitted.
pub struct Optimized {
    pub emitted: wowlua_luair::Emitted,
    /// Original names of renamed locals by source position.
    pub original: HashMap<wowlua_luair::ir::Pos, String>,
    /// Local functions not inlined, and why.
    pub not_inlined: Vec<(wowlua_luair::ir::Pos, wowlua_luair::optimize::NotInlined)>,
    /// The passes that add locals, upvalues or expression size were left
    /// out, as they broke a Lua 5.1 limit.
    pub reduced: bool,
}

/// Optimizes, minifies and emits `chunk`. When that breaks a Lua 5.1 limit
/// (registers, locals, upvalues, constants) that the code without
/// `dedupe-strings`, `cache-globals` and `inline` keeps, those are left out:
/// optimization never creates a limit error (spec optimization §15.2.2).
pub fn optimize_and_emit(
    chunk: wowlua_luair::ir::Chunk,
    optimizations: &Optimizations,
    passes: &wowlua_luair::Passes,
    options: &wowlua_luair::EmitOptions,
) -> Optimized {
    let run = |mut chunk: wowlua_luair::ir::Chunk, reduced: bool| {
        if optimizations.dead_code {
            wowlua_luair::optimize::remove_dead_locals(&mut chunk);
        }
        let mut not_inlined = Vec::new();
        if optimizations.inline && !reduced {
            not_inlined = wowlua_luair::optimize::inline(&mut chunk);
        }
        if optimizations.cache_globals && !reduced {
            wowlua_luair::optimize::cache_globals(&mut chunk);
        }
        let mut passes = *passes;
        if reduced {
            passes.dedupe_strings = false;
        }
        let original = wowlua_luair::minify::apply(&mut chunk, &passes, options.line_mode);
        Optimized {
            emitted: wowlua_luair::emit(&chunk, options),
            original,
            not_inlined,
            reduced,
        }
    };
    let grows = optimizations.inline || optimizations.cache_globals || passes.dedupe_strings;
    if !grows {
        return run(chunk, false);
    }
    let full = run(chunk.clone(), false);
    let compiles = |code: &[u8]| mlua::Lua::new().load(code).into_function().is_ok();
    if compiles(&full.emitted.text) {
        return full;
    }
    let reduced = run(chunk, true);
    if compiles(&reduced.emitted.text) {
        reduced
    } else {
        full
    }
}

/// Minifies Lua the compiler did not compile (the runtime, library files);
/// `None` when there is nothing to do or it does not parse as Lua 5.1, and
/// it then ships as it is.
pub fn minify_lua(
    db: &mut SourceDb,
    path: &str,
    code: &[u8],
    passes: &wowlua_luair::Passes,
    line_mode: LineMode,
) -> Option<Vec<u8>> {
    if passes.is_empty() {
        return None;
    }
    let file = db.add(path, String::from_utf8_lossy(code).into_owned());
    let parse = wowlua_syntax::parse(code, file);
    if parse.has_errors() || !wowlua_syntax::features::features(&parse.chunk).is_empty() {
        return None;
    }
    let mut chunk = wowlua_lower::lower(&parse, code).ok()?;
    wowlua_luair::minify::apply(&mut chunk, passes, line_mode);
    let emitted = wowlua_luair::emit(&chunk, &passes.emit_options(line_mode));
    let ok = !wowlua_syntax::parse(&emitted.text, file).has_errors()
        && luacheck::check(db, file, &emitted.text).is_none();
    ok.then_some(emitted.text)
}

/// The parsed sources of a package (shared by all flavors).
pub struct Parsed {
    pub(crate) parses: Vec<Option<Parse>>,
    pub(crate) ok: Vec<bool>,
    declarations: Vec<Parse>,
}

/// The line mode (spec codegen §14.5): the profile's, else `preserve` for
/// unbundled and `statement` for bundled output.
pub(crate) fn line_mode(profile: &Profile) -> LineMode {
    match profile.effective_line_mode() {
        ManifestLineMode::Statement => LineMode::Statement,
        ManifestLineMode::Pretty => LineMode::Pretty,
        ManifestLineMode::Preserve => LineMode::Preserve,
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
    // Modules added by an earlier call stay.
    let mut loaded: BTreeSet<String> = files
        .iter()
        .filter_map(|(s, _)| {
            s.src_rel
                .strip_prefix(wowlua_sema::STDLIB_PREFIX)
                .and_then(|n| n.strip_suffix(".wowlua"))
                .map(str::to_string)
        })
        .collect();
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
                output: None,
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
                        output: None,
                    },
                    file,
                    bytes: text.into_bytes(),
                });
                parsed.declarations.push(parse);
            }
        }
    }
}

/// Adds the modules of wowlua library dependencies the sources import,
/// transitively (tree-shaking at module granularity, spec §10.5.1). Each is
/// a source `@pkg/<name>/<path>`, emitted under the library's output folder.
/// Files an added XML file references come along.
pub fn add_libraries(
    db: &mut SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &mut Vec<(Source, FileId)>,
    declarations: &mut Vec<Declaration>,
    raw: &mut HashMap<String, Vec<u8>>,
    parsed: &mut Parsed,
    libs: &[crate::embed::WowluaLib],
) {
    if libs.is_empty() {
        return;
    }
    let mut present: std::collections::HashSet<String> = files
        .iter()
        .map(|(s, _)| s.src_rel.clone())
        .chain(declarations.iter().map(|d| d.source.src_rel.clone()))
        .collect();
    let mut scanned = 0;
    loop {
        // (library index, path relative to its src) still to add.
        let mut wanted: Vec<(usize, String)> = Vec::new();
        let lookup = |lib: usize, path: &str, wanted: &mut Vec<(usize, String)>| {
            let files = &libs[lib].files;
            let has_ext = [".xml", ".lua", ".wowlua"]
                .iter()
                .any(|e| path.ends_with(e));
            let candidates: Vec<String> = if has_ext {
                vec![path.to_string()]
            } else {
                vec![
                    format!("{path}.wowlua"),
                    format!("{path}.d.wowlua"),
                    format!("{path}.lua"),
                    format!("{path}/init.wowlua"),
                ]
            };
            for c in candidates {
                // Exact first; a case mismatch is added so that the import
                // reports E1013.
                let found = files
                    .keys()
                    .find(|k| **k == c)
                    .or_else(|| files.keys().find(|k| k.eq_ignore_ascii_case(&c)));
                if let Some(k) = found {
                    wanted.push((lib, k.clone()));
                    if let Some(stem) = k.strip_suffix(".d.wowlua") {
                        let lua = format!("{stem}.lua");
                        if files.contains_key(&lua) {
                            wanted.push((lib, lua));
                        }
                    }
                    return;
                }
            }
        };
        for (i, (source, _)) in files.iter().enumerate().skip(scanned) {
            let importer = wowlua_sema::package_of(&source.src_rel)
                .and_then(|n| libs.iter().position(|l| l.name == n));
            let dir_of = |src_rel: &str| -> String {
                src_rel
                    .rsplit_once('/')
                    .map(|(d, _)| d.to_string())
                    .unwrap_or_default()
            };
            if source.kind == SourceKind::Xml {
                // A library's XML loads files next to it.
                let Some(lib) = importer else { continue };
                let Ok(refs) = wowlua_xml::references(&String::from_utf8_lossy(&raw[&source.rel]))
                else {
                    continue;
                };
                let prefix = libs[lib].src_rel("");
                let dir = dir_of(&source.src_rel);
                for r in refs {
                    let joined = format!("{dir}/{}", r.normalized());
                    if let Some(rel) = normalize_rel(&joined)
                        .and_then(|p| p.strip_prefix(&prefix).map(str::to_string))
                    {
                        lookup(lib, &rel, &mut wanted);
                    }
                }
                continue;
            }
            let Some(parse) = parsed.parses.get(i).and_then(Option::as_ref) else {
                continue;
            };
            for s in &parse.chunk.block.stmts {
                let wowlua_syntax::ast::StmtKind::Import(import) = &s.kind else {
                    continue;
                };
                let spec = String::from_utf8_lossy(&import.from.value).into_owned();
                if spec.contains(':') {
                    continue;
                }
                if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
                    let Some(lib) = importer else { continue };
                    let prefix = libs[lib].src_rel("");
                    let joined = match spec.strip_prefix('/') {
                        Some(rest) => format!("{prefix}{rest}"),
                        None => format!("{}/{spec}", dir_of(&source.src_rel)),
                    };
                    if let Some(rel) = normalize_rel(&joined)
                        .and_then(|p| p.strip_prefix(&prefix).map(str::to_string))
                    {
                        lookup(lib, &rel, &mut wanted);
                    }
                    continue;
                }
                let (name, rest) = spec.split_once('/').unwrap_or((spec.as_str(), ""));
                let Some(lib) = libs.iter().position(|l| l.name == name) else {
                    continue;
                };
                if rest.is_empty() {
                    if let Some(main) = &libs[lib].main {
                        wanted.push((lib, main.clone()));
                    }
                } else {
                    lookup(lib, rest, &mut wanted);
                }
            }
        }
        scanned = files.len();
        let mut added = false;
        for (lib, rel) in wanted {
            let l = &libs[lib];
            let src_rel = l.src_rel(&rel);
            if !present.insert(src_rel.clone()) {
                continue;
            }
            added = true;
            let abs = l.files[&rel].clone();
            let bytes = match std::fs::read(&abs) {
                Ok(b) => b,
                Err(e) => {
                    diagnostics.push(Diagnostic::plain_error(format!(
                        "cannot read `{}`: {e}",
                        abs.display()
                    )));
                    continue;
                }
            };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let file = db.add(&src_rel, text);
            let lower = rel.to_ascii_lowercase();
            if lower.ends_with(".d.wowlua") {
                let parse = wowlua_syntax::parse(&bytes, file);
                if parse.has_errors() {
                    diagnostics.extend(parse.diagnostics.clone());
                }
                declarations.push(Declaration {
                    source: Source {
                        rel: src_rel.clone(),
                        src_rel,
                        abs,
                        kind: SourceKind::Wowlua,
                        output: None,
                    },
                    file,
                    bytes,
                });
                parsed.declarations.push(parse);
                continue;
            }
            let kind = if lower.ends_with(".wowlua") {
                SourceKind::Wowlua
            } else if lower.ends_with(".lua") {
                SourceKind::Lua
            } else {
                SourceKind::Xml
            };
            let source = Source {
                rel: src_rel.clone(),
                src_rel: src_rel.clone(),
                abs,
                kind,
                output: Some(l.output(&rel)),
            };
            if kind == SourceKind::Xml {
                parsed.parses.push(None);
                parsed.ok.push(true);
            } else {
                let parse = wowlua_syntax::parse(&bytes, file);
                let ok = !parse.has_errors();
                if !ok {
                    diagnostics.extend(parse.diagnostics.clone());
                }
                parsed.parses.push(Some(parse));
                parsed.ok.push(ok);
            }
            raw.insert(src_rel, bytes);
            files.push((source, file));
        }
        if !added {
            return;
        }
    }
}

/// `.` and `..` resolved; `None` if the path leaves the root.
fn normalize_rel(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// What compiling a wowlua library's files needs (per flavor).
pub struct LibraryUnit {
    pub check: CheckConfig,
    /// `src_rel` of its main module.
    pub main: Option<String>,
    /// Its `wowlua:meta`: the consumer's addon, build and flavor; its own
    /// features and config.
    pub meta: Arc<Meta>,
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
    lower_lua: &std::collections::HashSet<String>,
    libraries: &HashMap<String, LibraryUnit>,
    minify: &Minify,
) -> CompileOutput {
    let options = minify.passes.emit_options(line_mode(profile));
    let mut out = CompileOutput::default();

    let unit_of = |src_rel: &str| wowlua_sema::package_of(src_rel).and_then(|n| libraries.get(n));
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
            meta: unit_of(&source.src_rel).map(|u| u.meta.as_ref()),
            main_of: unit_of(&source.src_rel)
                .filter(|u| u.main.as_deref() == Some(source.src_rel.as_str()))
                .and(wowlua_sema::package_of(&source.src_rel).map(str::to_string)),
        })
        .collect();
    for (d, parse) in declarations.iter().zip(&parsed.declarations) {
        inputs.push(FileInput {
            src_rel: d.source.src_rel.clone(),
            kind: FileKind::Declaration,
            parse: Some(parse),
            xml: None,
            meta: None,
            main_of: None,
        });
    }
    for d in api {
        inputs.push(FileInput {
            src_rel: d.source.src_rel.clone(),
            kind: FileKind::Declaration,
            parse: Some(&d.parse),
            xml: None,
            meta: None,
            main_of: None,
        });
    }
    let analysis = wowlua_sema::analyze(&inputs, Some(&meta));
    diagnostics.extend(analysis.diagnostics.iter().cloned());
    // The standard library's compile-time checks (spec stdlib, "core").
    diagnostics.extend(wowlua_sema::stdlib::check(
        &inputs,
        &analysis,
        &check.stdlib,
    ));
    // Dead modules (spec optimization §15.2.1); the build leaves out those
    // dead in every flavor.
    let dead: std::collections::HashSet<usize> = if profile.optimize.dead_code {
        dead_modules(
            files,
            parsed,
            &analysis,
            raw,
            layout.loaded_by,
            &minify.keep,
        )
    } else {
        std::collections::HashSet::new()
    };
    out.removed = dead.iter().map(|&i| files[i].0.rel.clone()).collect();

    // Load order (spec §5.4): files listed in `build.load-order` keep their
    // listed order; files outside this flavor do not take part.
    let markers = wowlua_sema::usage::Markers::from_table().with_api(&inputs);
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
                // The standard library's modules are checked in basic mode
                // whatever the project's settings, so their exports are typed
                // for the code that imports them.
                (Some((source, _)), Some(_))
                    if parsed.ok[i]
                        && wowlua_sema::module_id(&source.src_rel).starts_with("wowlua:") =>
                {
                    Some(wowlua_sema::check::FileSettings {
                        mode: wowlua_sema::check::query::Mode::Basic,
                        pure_lua: false,
                    })
                }
                (Some((source, _)), Some(parse))
                    if parsed.ok[i] && !wowlua_sema::is_compiler_known(&source.src_rel) =>
                {
                    // A library is checked with its own settings.
                    let cfg = unit_of(&source.src_rel).map_or(check, |u| &u.check);
                    check_settings(source, parse, cfg)
                }
                _ => None,
            },
        )
        .collect();
    let mut private_methods = std::collections::HashSet::new();
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
        out.value_checks = checked.value_checks;
        private_methods = checked.private_methods;
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

    // Enums register their order when the package uses `wowlua:reflect`.
    let reflect_used = analysis.modules.iter().flatten().any(|m| {
        m.imports.iter().any(|imp| {
            matches!(imp.target, Target::Module(t)
                if analysis.modules[t].as_ref().is_some_and(|x| x.id == "wowlua:reflect"))
        })
    });
    // `@inline` hints (spec §4.3): local functions by position, and methods,
    // which are not inlined.
    let mut inline_hints: HashMap<FileId, Vec<(wowlua_luair::ir::Pos, wowlua_span::Span)>> =
        HashMap::new();
    if profile.optimize.inline {
        use wowlua_syntax::ast;
        let is_inline = |d: &[ast::Decorator]| {
            d.iter()
                .any(|d| d.path.len() == 1 && d.path[0].text.as_ref() == "inline")
        };
        for (i, (source, file)) in files.iter().enumerate() {
            let Some(parse) = parsed.parses[i].as_ref() else {
                continue;
            };
            if source.kind != SourceKind::Wowlua {
                continue;
            }
            let lexed = &parse.lexed;
            wowlua_syntax::visit::walk_stmts(&parse.chunk.block, &mut |st| match &st.kind {
                ast::StmtKind::LocalFunction { name, .. } if is_inline(&st.decorators) => {
                    let pos = wowlua_luair::ir::Pos::new(
                        lexed.line_of(name.span.start),
                        lexed.column_of(name.span.start),
                    );
                    inline_hints
                        .entry(*file)
                        .or_default()
                        .push((pos, name.span));
                }
                ast::StmtKind::Class(c) => {
                    for m in &c.members {
                        if is_inline(&m.decorators) {
                            diagnostics.push(
                                Diagnostic::warning(
                                    "W3031",
                                    "`@inline` is ignored: only local functions with a single `return` expression are inlined",
                                )
                                .with_label(m.span, "a method"),
                            );
                        }
                    }
                }
                _ => {}
            });
        }
    }
    // Emits a lowered chunk: optimized, minified, checked to parse and to
    // stay within Lua's limits.
    let finish = |chunk: wowlua_luair::ir::Chunk,
                  source: &Source,
                  file: FileId,
                  diagnostics: &mut Vec<Diagnostic>|
     -> Option<Compiled> {
        let optimized = optimize_and_emit(
            chunk,
            &Optimizations {
                dead_code: profile.optimize.dead_code,
                inline: profile.optimize.inline,
                cache_globals: profile.optimize.cache_globals
                    == wowlua_manifest::model::CacheGlobals::Safe,
            },
            &minify.passes,
            &options,
        );
        for (pos, span) in inline_hints.get(&file).into_iter().flatten() {
            use wowlua_luair::optimize::NotInlined;
            let Some((_, why)) = optimized.not_inlined.iter().find(|(p, _)| p == pos) else {
                continue;
            };
            let why = match why {
                NotInlined::NotAnExpression => "its body is not a single `return` expression",
                NotInlined::Recursive => "it is recursive",
                NotInlined::Escapes => "it is used other than by calling it",
            };
            diagnostics.push(
                Diagnostic::warning("W3031", format!("`@inline` is ignored: {why}"))
                    .with_label(*span, "not inlined"),
            );
        }
        if optimized.reduced {
            diagnostics.push(
                Diagnostic::note(
                    "`dedupe-strings`, `cache-globals` and `inline` are left out for this file: with them it would exceed a Lua 5.1 limit",
                )
                .with_path(&source.rel),
            );
        }
        let (emitted, original) = (optimized.emitted, optimized.original);
        // The output must parse (spec compiler §13.8, item 2).
        let reparsed = wowlua_syntax::parse(&emitted.text, file);
        if reparsed.has_errors() {
            diagnostics.push(
                Diagnostic::plain_error(format!(
                    "internal compiler error: the code emitted for this file does not parse: {}",
                    reparsed.diagnostics[0].message
                ))
                .with_path(&source.rel),
            );
            return None;
        }
        if let Some(d) = luacheck::check(db, file, &emitted.text) {
            diagnostics.push(d);
        }
        let output = source.output_path();
        let map = profile.source_maps.then(|| {
            let name = output.rsplit('/').next().unwrap_or(&output);
            wowlua_luair::sourcemap::source_map_named(
                &emitted.mappings,
                name,
                &source.rel,
                &original,
            )
        });
        Some(Compiled {
            code: emitted.text,
            map,
        })
    };
    let mut lowered_files: Vec<(usize, wowlua_luair::ir::Chunk)> = Vec::new();
    for (i, (source, file)) in files.iter().enumerate() {
        let plain_lua = source.kind == SourceKind::Lua && lower_lua.contains(&source.rel);
        let minified_lua = source.kind == SourceKind::Lua && minify.lua.contains(&source.rel);
        // Dead modules are compiled still: the build removes a module only
        // if it is dead in every flavor.
        if (source.kind != SourceKind::Wowlua && !plain_lua && !minified_lua) || !parsed.ok[i] {
            continue;
        }
        let parse = parsed.parses[i].as_ref().expect("parsed");
        let bytes = &raw[&source.rel];
        // Plain Lua is only minified: it compiles to itself (spec §14.2).
        if minified_lua && !plain_lua {
            match wowlua_lower::lower(parse, bytes) {
                Ok(chunk) => lowered_files.push((i, chunk)),
                Err(errors) => diagnostics.extend(errors),
            }
            continue;
        }
        let unit = unit_of(&source.src_rel);
        let file_meta = unit.map_or_else(|| meta.clone(), |u| u.meta.clone());
        let mut ctx = context(&analysis, i, &inputs, file_meta, &order.lazy);
        ctx.file = source.rel.clone();
        ctx.debug = profile.debug;
        ctx.tools = wowlua_lower::Tools {
            assertions: profile.assertions,
            dev_console: profile.dev_console,
            profiling: profile.instrument != wowlua_manifest::model::Instrument::None,
            log_level: profile.log_level as u8,
            instrument: profile.instrument as u8,
            runtime_checks: profile.runtime_checks as u8,
            reflection: profile.reflection as u8,
            reflect_used,
            monitor: profile.monitor,
        };
        ctx.value_checks = out
            .value_checks
            .iter()
            .filter(|(span, _)| span.file == *file)
            .map(|(span, c)| ((span.start, span.end), c.clone()))
            .collect();
        ctx.type_names = out
            .type_names
            .iter()
            .filter(|(span, _)| span.file == *file)
            .map(|(span, name)| ((span.start, span.end), name.clone()))
            .collect();
        ctx.compiler_known = wowlua_sema::is_compiler_known(&source.src_rel);
        ctx.dependency = unit.is_some();
        let lowered = match wowlua_lower::lower_module(parse, bytes, &ctx) {
            Ok(lowered) => lowered,
            Err(errors) => {
                diagnostics.extend(errors);
                continue;
            }
        };
        // The runtime's `config` and `features` tables are the addon's: a
        // library may only use their constant (scalar) values.
        if unit.is_some()
            && let Some((kind, path)) = lowered
                .meta_used
                .iter()
                .find(|(k, _)| matches!(k, MetaKind::Config | MetaKind::Features))
        {
            diagnostics.push(
                Diagnostic::plain_error(format!(
                    "library code reads `{}{}` at run time; a library may only use constant `config` and `features` values",
                    if *kind == MetaKind::Config { "config" } else { "features" },
                    path.iter().map(|p| format!(".{p}")).collect::<String>()
                ))
                .with_path(&source.rel),
            );
        }
        out.needs_runtime |= lowered.uses_runtime;
        out.meta_used.extend(lowered.meta_used);
        out.slices.extend(lowered.slices);
        diagnostics.extend(lowered.warnings);
        lowered_files.push((i, lowered.chunk));
    }
    if minify.passes.rename_private {
        let mut parsed_files: Vec<crate::private_names::Parsed> = Vec::new();
        for (i, (source, file)) in files.iter().enumerate() {
            if let (Some(parse), Some(bytes)) = (parsed.parses[i].as_ref(), raw.get(&source.rel)) {
                parsed_files.push(crate::private_names::Parsed {
                    file: *file,
                    parse,
                    bytes,
                });
            }
        }
        for (d, parse) in declarations.iter().zip(&parsed.declarations) {
            parsed_files.push(crate::private_names::Parsed {
                file: d.file,
                parse,
                bytes: &d.bytes,
            });
        }
        for d in api {
            parsed_files.push(crate::private_names::Parsed {
                file: d.file,
                parse: &d.parse,
                bytes: db.get(d.file).text().as_bytes(),
            });
        }
        rename_private_methods(
            &mut lowered_files,
            files,
            &parsed_files,
            &private_methods,
            minify,
        );
    }
    for (i, chunk) in lowered_files {
        let (source, file) = &files[i];
        if let Some(compiled) = finish(chunk, source, *file, diagnostics) {
            out.compiled.insert(source.rel.clone(), compiled);
        }
    }
    out
}

/// The class half of `rename-private` over every file of the package.
fn rename_private_methods(
    lowered: &mut [(usize, wowlua_luair::ir::Chunk)],
    files: &[(Source, FileId)],
    parsed: &[crate::private_names::Parsed],
    recorded: &std::collections::HashSet<wowlua_span::Span>,
    minify: &Minify,
) {
    if recorded.is_empty() {
        return;
    }
    let mut plan = crate::private_names::plan(recorded, parsed, &minify.foreign);
    if plan.at.is_empty() {
        return;
    }
    let mut found: HashMap<FileId, Vec<wowlua_luair::ir::Pos>> = HashMap::new();
    for (i, chunk) in lowered.iter_mut() {
        let file = files[*i].1;
        if let Some(at) = plan.at.get(&file) {
            found.insert(file, wowlua_luair::minify::rename_members(chunk, at, false));
        }
    }
    crate::private_names::verify(&mut plan, &found);
    for (i, chunk) in lowered.iter_mut() {
        if let Some(at) = plan.at.get(&files[*i].1) {
            wowlua_luair::minify::rename_members(chunk, at, true);
        }
    }
}

/// The modules `dead-code` removes (spec optimization §15.2.1): modules
/// whose top level only declares things, that no kept file imports, that
/// no XML file loads, not in `build.keep` and not required by name from
/// plain Lua (`__wowlua.require("id")`). Plain Lua files are never removed.
fn dead_modules(
    files: &[(Source, FileId)],
    parsed: &Parsed,
    analysis: &wowlua_sema::Analysis,
    raw: &HashMap<String, Vec<u8>>,
    loaded_by: &[Option<usize>],
    keep: &[String],
) -> std::collections::HashSet<usize> {
    use wowlua_syntax::ast;
    fn pure(e: &ast::Expr) -> bool {
        match &e.kind {
            ast::ExprKind::Nil
            | ast::ExprKind::True
            | ast::ExprKind::False
            | ast::ExprKind::Number(_)
            | ast::ExprKind::String(_)
            | ast::ExprKind::Function(_) => true,
            ast::ExprKind::Table(fields) => fields.iter().all(|f| match f {
                ast::TableField::Keyed { key, value, .. } => {
                    matches!(
                        key.kind,
                        ast::ExprKind::String(_) | ast::ExprKind::Number(_)
                    ) && pure(value)
                }
                ast::TableField::Named { value, .. } | ast::TableField::Positional(value) => {
                    pure(value)
                }
            }),
            _ => false,
        }
    }
    fn declaration(s: &ast::Stmt) -> bool {
        s.decorators.is_empty()
            && match &s.kind {
                ast::StmtKind::Local { values, .. } => values.iter().all(pure),
                ast::StmtKind::LocalFunction { .. }
                | ast::StmtKind::Interface(_)
                | ast::StmtKind::Enum(_)
                | ast::StmtKind::TypeAlias(_)
                | ast::StmtKind::Declare(_)
                | ast::StmtKind::Import(_) => true,
                ast::StmtKind::Const { value, .. } => value.as_ref().is_none_or(pure),
                ast::StmtKind::Class(c) => c.members.iter().all(|m| {
                    m.decorators.is_empty()
                        && match &m.kind {
                            ast::ClassMemberKind::Field { value, .. } => {
                                !m.modifiers.contains(&ast::Modifier::Static)
                                    || value.as_ref().is_none_or(pure)
                            }
                            _ => true,
                        }
                }),
                ast::StmtKind::Export(inner) => {
                    // `export function f()` defines an export, not a global.
                    matches!(&inner.kind, ast::StmtKind::Function { name, .. } if name.path.len() == 1 && name.method.is_none())
                        || declaration(inner)
                }
                _ => false,
            }
    }
    // Module IDs plain Lua requires by name.
    let required =
        regex::Regex::new(r#"require\s*\(?\s*["']([^"']+)["']"#).expect("a valid pattern");
    let mut by_name: std::collections::HashSet<String> = keep.iter().cloned().collect();
    for (source, _) in files {
        if source.kind == SourceKind::Lua
            && let Some(text) = raw.get(&source.rel)
        {
            let text = String::from_utf8_lossy(text);
            by_name.extend(required.captures_iter(&text).map(|c| c[1].to_string()));
        }
    }
    let candidate = |i: usize| -> bool {
        let Some(info) = analysis.modules.get(i).and_then(Option::as_ref) else {
            return false;
        };
        files[i].0.kind == SourceKind::Wowlua
            && info.is_module
            && loaded_by.get(i).copied().flatten().is_none()
            && !by_name.contains(&info.id)
            && parsed.parses[i]
                .as_ref()
                .is_some_and(|p| p.chunk.block.stmts.iter().all(declaration))
    };
    // Everything else is kept, and so is what kept files import.
    let mut kept: Vec<bool> = (0..files.len()).map(|i| !candidate(i)).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..files.len() {
            if !kept[i] {
                continue;
            }
            let Some(info) = analysis.modules.get(i).and_then(Option::as_ref) else {
                continue;
            };
            for import in &info.imports {
                if let wowlua_sema::Target::Module(m) | wowlua_sema::Target::Lua { file: m, .. } =
                    import.target
                    && m < kept.len()
                    && !kept[m]
                {
                    kept[m] = true;
                    changed = true;
                }
            }
        }
    }
    (0..files.len()).filter(|&i| !kept[i]).collect()
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

#[cfg(test)]
mod limit_tests {
    use super::*;

    /// A table so large and nested that hoisting its strings into locals
    /// would run out of registers: the passes that add locals are left out.
    #[test]
    fn optimizations_never_break_lua_limits() {
        // 170 long strings used twice each (170 hoisted locals), and lists
        // of 49 items three levels deep (pending items take registers).
        let mut src = String::from("Strings = {\n");
        for i in 0..170 {
            src.push_str(&format!(
                "\"a long string number {i}\", \"a long string number {i}\",\n"
            ));
        }
        src.push_str("}\nData = {\n");
        for _ in 0..3 {
            for _ in 0..49 {
                src.push_str("{ 1 },\n");
            }
            src.push_str("{\n");
        }
        src.push('1');
        for _ in 0..3 {
            src.push_str("},\n");
        }
        src.push_str("}\n");
        let mut db = SourceDb::new();
        let file = db.add("t.lua", src.clone());
        let parse = wowlua_syntax::parse(src.as_bytes(), file);
        let chunk = wowlua_lower::lower(&parse, src.as_bytes()).unwrap();
        let passes = wowlua_luair::Passes::from_names(["dedupe-strings", "rename-locals"]);
        let out = optimize_and_emit(
            chunk,
            &Optimizations::default(),
            &passes,
            &passes.emit_options(LineMode::Statement),
        );
        assert!(out.reduced);
        assert!(
            mlua::Lua::new()
                .load(&out.emitted.text[..])
                .into_function()
                .is_ok()
        );
    }
}
