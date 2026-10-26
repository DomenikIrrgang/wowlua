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
#[derive(Debug)]
pub struct Compiled {
    pub code: Vec<u8>,
    /// Source Map v3 (when the profile enables source maps and the build
    /// writes its output), encoded until it is written.
    pub map: Option<Arc<wowlua_luair::sourcemap::EncodedMap>>,
    /// `code` has no byte order mark, and Lua 5.1 compiled exactly it.
    pub compiles: bool,
    /// The code was dropped once checked (`BuildRequest::keep_code`):
    /// `code` is empty, and this is the BLAKE3 hash of what it was.
    pub dropped: Option<[u8; 32]>,
}

impl Compiled {
    /// Whether the two hold the same code, also where it was dropped.
    pub fn same_code(&self, other: &Compiled) -> bool {
        let hash = |c: &Compiled| {
            c.dropped
                .unwrap_or_else(|| *blake3::hash(&c.code).as_bytes())
        };
        match (self.dropped, other.dropped) {
            (None, None) => self.code == other.code,
            _ => hash(self) == hash(other),
        }
    }
}

#[derive(Default)]
pub struct CompileOutput {
    /// The sources the type check ran on, by index.
    pub checked: std::collections::HashSet<usize>,
    /// Compiled `.wowlua` sources by `Source::rel`. Flavors that compile a
    /// file the same share it; it is replaced, never changed.
    pub compiled: HashMap<String, Arc<Compiled>>,
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
    /// What strict files' fast forms may rely on (from the checker, spec
    /// §1.2.4, §1.4.1, §1.4.2).
    pub fast_forms: HashMap<wowlua_span::Span, u8>,
    /// Modules `dead-code` removed (by `Source::rel`, spec optimization
    /// §15.2.1).
    pub removed: BTreeSet<String>,
    /// `@binding` declarations, for the generated Bindings.xml.
    pub bindings: Vec<wowlua_sema::stdlib::BindingDecl>,
    /// With `CheckConfig::index`: each name the check met and the span of
    /// what it refers to.
    pub definitions: Vec<(wowlua_span::Span, wowlua_span::Span)>,
    /// With `CheckConfig::hover`: those files' names and expressions, with
    /// their types.
    pub hovers: Vec<(wowlua_span::Span, String)>,
    /// With `CheckConfig::complete`: what could stand at the cursor.
    pub completions: Option<wowlua_sema::check::complete::Completions>,
    /// With `CheckConfig::complete`: the signatures of the call around the
    /// cursor.
    pub signature: Option<Vec<wowlua_sema::check::complete::Signature>>,
}

/// A declaration file with its contents.
pub struct Declaration {
    pub source: Source,
    pub file: FileId,
    /// Borrowed for the compiler's own declarations, compiled into it.
    pub bytes: std::borrow::Cow<'static, [u8]>,
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
    /// Find what each name refers to (the language server's go to
    /// definition): `CompileOutput::definitions`.
    pub index: bool,
    /// `Session::focus`: type-check only these files and their imports.
    pub focus: Option<std::collections::HashSet<std::path::PathBuf>>,
    /// `Session::hover`: record these files' types for hover.
    pub hover: Option<std::collections::HashSet<std::path::PathBuf>>,
    /// `Session::complete`: a completion check, of the word typed.
    pub complete: Option<String>,
}

/// A declaration file of one flavor (the wow-api types), parsed.
pub struct ParsedDeclaration {
    pub source: Source,
    pub file: FileId,
    pub parse: Parse,
    /// The WoW API's globals of a primitive type, which `parse` leaves out.
    pub globals: wowlua_api::Globals,
}

/// How a source is checked (spec §2.1), or `None`.
fn check_settings(
    source: &Source,
    parse: &Parse,
    cfg: &CheckConfig,
) -> Option<wowlua_sema::check::FileSettings> {
    use wowlua_manifest::model::CheckMode;
    use wowlua_sema::check::{FileSettings, query::Mode};
    let directive = |n: &str| {
        parse
            .view()
            .directives()
            .iter()
            .any(|d| d.name.as_ref() == n)
    };
    if directive("nocheck") {
        return None;
    }
    let opted_strict = directive("strict")
        || cfg.strict.is_match(&source.rel)
        || cfg.strict.is_match(&source.src_rel);
    match source.kind {
        SourceKind::Wowlua => {
            let pure = wowlua_syntax::features::features(parse.view()).is_empty();
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
    /// Lua 5.1 compiled `emitted.text` as it is, in a fresh state, while
    /// choosing between the full and the reduced variant.
    pub compiles: bool,
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
            compiles: false,
        }
    };
    let grows = optimizations.inline || optimizations.cache_globals || passes.dedupe_strings;
    if !grows {
        return run(chunk, false);
    }
    let full = run(chunk.clone(), false);
    let compiles = luacheck::compiles;
    if compiles(&full.emitted.text) {
        return Optimized {
            compiles: true,
            ..full
        };
    }
    let reduced = run(chunk, true);
    if compiles(&reduced.emitted.text) {
        Optimized {
            compiles: true,
            ..reduced
        }
    } else {
        full
    }
}

/// The output must parse (spec compiler §13.8, item 2): the internal error
/// when `text`, emitted for `file` (at `rel`), does not.
fn emitted_parse_error(text: &[u8], file: FileId, rel: &str) -> Option<Diagnostic> {
    if wowlua_syntax::parses(text, file) {
        return None;
    }
    // Only to word the error: the full parse has its first message.
    let reparsed = wowlua_syntax::parse(text, file);
    Some(
        Diagnostic::plain_error(format!(
            "internal compiler error: the code emitted for this file does not parse: {}",
            reparsed.diagnostics[0].message
        ))
        .with_path(rel),
    )
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
    if parse.has_errors() || !wowlua_syntax::features::features(parse.view()).is_empty() {
        return None;
    }
    let mut chunk = wowlua_lower::lower(&parse, code).ok()?;
    wowlua_luair::minify::apply(&mut chunk, passes, line_mode);
    // Only the text is used: these files get no source maps.
    let mut options = passes.emit_options(line_mode);
    options.mappings = false;
    let emitted = wowlua_luair::emit(&chunk, &options);
    let ok = wowlua_syntax::parses(&emitted.text, file)
        && luacheck::check(db, file, &emitted.text).is_none();
    ok.then_some(emitted.text)
}

/// The parsed sources of a package (shared by all flavors).
pub struct Parsed {
    /// `None` for XML files and summarized ones (`Front::Summarized`).
    pub(crate) parses: Vec<Option<Parse>>,
    pub(crate) ok: Vec<bool>,
    /// What `wowlua dev` knows of plain Lua sources without their parse.
    pub(crate) fronts: Vec<Front>,
    declarations: Vec<Parse>,
    /// The parses keep their tokens (for `rename-private`); otherwise each
    /// frees them once parsed (#19): no later pass reads them.
    keep_tokens: bool,
}

impl Parsed {
    /// The package's syntax trees, by file; the rest is dropped.
    pub(crate) fn into_parses(self) -> Vec<Option<Parse>> {
        self.parses
    }
}

/// A plain Lua source in `wowlua dev` (#7 stage 3; see `BuildCache::front`).
#[derive(Clone)]
pub(crate) enum Front {
    None,
    /// Unchanged since an earlier build: not parsed.
    Summarized {
        usage: Arc<wowlua_sema::usage::FileUsage>,
        nolint: bool,
    },
    /// Parsed cleanly: kept for later builds if no flavor type-checks it.
    Fresh {
        hash: blake3::Hash,
        usage: Arc<wowlua_sema::usage::FileUsage>,
        nolint: bool,
    },
}

impl Parsed {
    fn push(&mut self, mut parse: Option<Parse>, ok: bool) {
        if let Some(parse) = &mut parse {
            self.finish(parse);
        }
        self.parses.push(parse);
        self.ok.push(ok);
        self.fronts.push(Front::None);
    }

    fn push_declaration(&mut self, mut parse: Parse) {
        self.finish(&mut parse);
        self.declarations.push(parse);
    }

    /// Frees the tokens of a parse kept here, unless they are kept.
    fn finish(&self, parse: &mut Parse) {
        if !self.keep_tokens {
            parse.lexed.drop_tokens();
        }
    }

    /// The load-order usage of file `i` known without a flavor.
    pub(crate) fn usage(&self, i: usize) -> Option<Arc<wowlua_sema::usage::FileUsage>> {
        match self.fronts.get(i)? {
            Front::None => None,
            Front::Summarized { usage, .. } | Front::Fresh { usage, .. } => Some(usage.clone()),
        }
    }
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

/// Parses `bytes`, the source of `file`. The tree shares the source
/// database's text where it is these very bytes (valid UTF-8, nearly
/// always) instead of copying them.
fn parse_source(db: &SourceDb, file: FileId, bytes: &[u8]) -> Parse {
    match db.get(file).shared_text() {
        Some(text) if std::ptr::eq(text.as_bytes(), bytes) => {
            wowlua_syntax::parse_shared(text, file)
        }
        _ => wowlua_syntax::parse(bytes, file),
    }
}

/// Parses every source once and checks the plain Lua files, on worker
/// threads when `jobs` allows (#8); diagnostics and the cache are handled
/// on the calling thread, in file order, as when parsing one at a time.
#[allow(clippy::too_many_arguments)]
pub fn parse_sources(
    db: &SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &[(Source, FileId)],
    declarations: &[Declaration],
    raw: &HashMap<String, Arc<[u8]>>,
    mut cache: Option<&mut crate::BuildCache>,
    summarize: bool,
    keep_tokens: bool,
    jobs: &crate::parallel::Jobs,
) -> Parsed {
    /// What to do with a source or declaration.
    #[derive(Clone)]
    enum Item {
        /// An unchanged plain Lua file kept by an earlier build (#7): its
        /// Lua 5.1 check is replayed as for a parsed one.
        Summarized {
            i: usize,
            lua: Option<Diagnostic>,
            usage: Arc<wowlua_sema::usage::FileUsage>,
            nolint: bool,
        },
        /// Parse source `i`; `lua_kept`: its Lua 5.1 check is in the cache.
        Source {
            i: usize,
            lua_kept: bool,
        },
        Declaration(usize),
    }
    /// What parsing an item gave (one per file, for the region only).
    #[allow(clippy::large_enum_variant)]
    enum Done {
        Summarized {
            i: usize,
            lua: Option<Diagnostic>,
            usage: Arc<wowlua_sema::usage::FileUsage>,
            nolint: bool,
        },
        Source {
            i: usize,
            parse: Parse,
            /// The parse failed, or the file has wowlua syntax (E0101).
            failed: Option<Vec<Diagnostic>>,
            /// The Lua 5.1 check, unless it is in the cache.
            lua: Option<Option<Diagnostic>>,
            usage: Option<Arc<wowlua_sema::usage::FileUsage>>,
            nolint: bool,
        },
        Declaration(Parse),
    }
    // The usage of a plain Lua file depends only on the table markers.
    let markers = summarize.then(wowlua_sema::usage::Markers::from_table);
    let mut items = Vec::new();
    for (i, (source, file)) in files.iter().enumerate() {
        if source.kind == SourceKind::Xml {
            continue;
        }
        let bytes = &raw[&source.rel];
        if summarize
            && source.kind == SourceKind::Lua
            && let Some(cache) = cache.as_deref_mut()
            && let Some((usage, nolint)) = cache.front(&source.rel, *file, bytes)
        {
            let lua = cache.lua_check(db, &source.rel, *file, bytes);
            items.push(Item::Summarized {
                i,
                lua,
                usage,
                nolint,
            });
            continue;
        }
        let lua_kept = source.kind == SourceKind::Lua
            && cache
                .as_deref_mut()
                .is_some_and(|c| c.has_lua_check(&source.rel, *file, bytes));
        items.push(Item::Source { i, lua_kept });
    }
    items.extend((0..declarations.len()).map(Item::Declaration));
    let weight = |item: &Item| match item {
        Item::Summarized { .. } => 0,
        Item::Source { i, .. } => raw[&files[*i].0.rel].len() as u64,
        Item::Declaration(d) => declarations[*d].bytes.len() as u64,
    };
    let compute = |item: Item| match item {
        Item::Summarized {
            i,
            lua,
            usage,
            nolint,
        } => Done::Summarized {
            i,
            lua,
            usage,
            nolint,
        },
        Item::Source { i, lua_kept } => {
            let (source, file) = &files[i];
            let bytes = &raw[&source.rel];
            let mut parse = parse_source(db, *file, bytes);
            let mut failed = None;
            let mut lua = None;
            let mut usage = None;
            let mut nolint = false;
            if parse.has_errors() {
                failed = Some(parse.diagnostics.clone());
            } else if source.kind == SourceKind::Lua {
                if let Some((feature, span)) =
                    wowlua_syntax::features::features(parse.view()).first()
                {
                    failed = Some(vec![
                        Diagnostic::error(
                            "E0101",
                            format!(
                                "this `.lua` file uses {}, which Lua 5.1 doesn't have",
                                feature.describe()
                            ),
                        )
                        .with_label(*span, "not Lua 5.1")
                        .with_help("rename the file to `.wowlua`"),
                    ]);
                } else {
                    if !lua_kept {
                        lua = Some(luacheck::check(db, *file, bytes));
                    }
                    if let Some(markers) = &markers {
                        usage = Some(Arc::new(wowlua_sema::usage::file_usage(&parse, markers)));
                        nolint = parse
                            .view()
                            .directives()
                            .iter()
                            .any(|d| d.name.as_ref() == "nolint");
                    }
                }
            }
            if !keep_tokens {
                parse.lexed.drop_tokens();
            }
            Done::Source {
                i,
                parse,
                failed,
                lua,
                usage,
                nolint,
            }
        }
        Item::Declaration(d) => {
            let mut parse = wowlua_syntax::parse(&declarations[d].bytes, declarations[d].file);
            if !keep_tokens {
                parse.lexed.drop_tokens();
            }
            Done::Declaration(parse)
        }
    };
    let mut parses: Vec<Option<Parse>> = (0..files.len()).map(|_| None).collect();
    let mut ok = vec![true; files.len()];
    let mut fronts = vec![Front::None; files.len()];
    let mut parsed_declarations = Vec::with_capacity(declarations.len());
    // Parsing takes about this much memory per source byte while it runs
    // (the tree it keeps is the build's anyway).
    jobs.region(items, weight, 20, compute, |done| match done {
        Done::Summarized {
            i,
            lua,
            usage,
            nolint,
        } => {
            diagnostics.extend(lua);
            fronts[i] = Front::Summarized { usage, nolint };
        }
        Done::Source {
            i,
            parse,
            failed,
            lua,
            usage,
            nolint,
        } => {
            let (source, file) = &files[i];
            if let Some(failed) = failed {
                diagnostics.extend(failed);
                ok[i] = false;
            } else if source.kind == SourceKind::Lua {
                let bytes = &raw[&source.rel];
                let checked = match cache.as_deref_mut() {
                    Some(cache) => cache.lua_check_with(db, &source.rel, *file, bytes, lua),
                    None => lua.unwrap_or_else(|| luacheck::check(db, *file, bytes)),
                };
                diagnostics.extend(checked);
                if let (Some(usage), Some(cache)) = (usage, cache.as_deref_mut()) {
                    fronts[i] = Front::Fresh {
                        hash: cache.source_hash(&source.rel, bytes),
                        usage,
                        nolint,
                    };
                }
            }
            parses[i] = Some(parse);
        }
        Done::Declaration(parse) => {
            if parse.has_errors() {
                diagnostics.extend(parse.diagnostics.clone());
            }
            parsed_declarations.push(parse);
        }
    });
    Parsed {
        parses,
        ok,
        fronts,
        declarations: parsed_declarations,
        keep_tokens,
    }
}

/// Adds the standard-library modules the sources import, transitively
/// (tree-shaking at module granularity, spec §8.1): each is compiled into
/// the addon as `wowlua/std/<name>.lua`.
/// `declaration_overrides`: declarations generated for the project (the
/// type of `wowlua:locale`'s `L`), by module name.
pub fn add_stdlib(
    db: &mut SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &mut Vec<(Source, FileId)>,
    declarations: &mut Vec<Declaration>,
    raw: &mut HashMap<String, Arc<[u8]>>,
    parsed: &mut Parsed,
    declaration_overrides: &HashMap<&str, String>,
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
            for s in parse.view().block().stmts() {
                if let wowlua_syntax::view::StmtKind::Import(import) = s.kind() {
                    let spec = String::from_utf8_lossy(&import.from().value()).into_owned();
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
            raw.insert(rel, module.source.into_bytes().into());
            files.push((source, file));
            parsed.push(Some(parse), ok);
            let declaration = declaration_overrides
                .get(name.as_str())
                .cloned()
                .or(module.declaration);
            if let Some(text) = declaration {
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
                    bytes: text.into_bytes().into(),
                });
                parsed.push_declaration(parse);
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
    raw: &mut HashMap<String, Arc<[u8]>>,
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
            for s in parse.view().block().stmts() {
                let wowlua_syntax::view::StmtKind::Import(import) = s.kind() else {
                    continue;
                };
                let spec = String::from_utf8_lossy(&import.from().value()).into_owned();
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
            let bytes = match crate::overlay::read(&abs) {
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
                    bytes: bytes.into(),
                });
                parsed.push_declaration(parse);
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
                parsed.push(None, true);
            } else {
                let parse = wowlua_syntax::parse(&bytes, file);
                let ok = !parse.has_errors();
                if !ok {
                    diagnostics.extend(parse.diagnostics.clone());
                }
                parsed.push(Some(parse), ok);
            }
            raw.insert(src_rel, bytes.into());
            files.push((source, file));
        }
        if !added {
            return;
        }
    }
}

/// `.` and `..` resolved; `None` if the path leaves the root.
pub(crate) fn normalize_rel(path: &str) -> Option<String> {
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

/// What compiling files gave in the earlier flavors of one build, and in
/// `wowlua dev` in earlier builds. Lowering reads nothing of the flavor but
/// the file's parse, bytes and context, and finishing nothing but the
/// lowered chunk, the file's source and `FileId`, and the build's settings:
/// a file with the same bytes and `FileId` whose context is the same apart
/// from `wowlua:meta`, and whose lowering never read that, compiles to the
/// same code with the same diagnostics. Across builds, the build's settings
/// are the same while [`crate::incremental::BuildCache`] keeps the entries.
#[derive(Default)]
pub(crate) struct FlavorCache {
    /// By `Source::rel`; one entry per context the file was compiled with.
    pub(crate) entries: HashMap<String, Vec<CacheEntry>>,
    /// The module IDs plain Lua requires by name, and `build.keep`: the
    /// same in every flavor, found the first time a flavor asks.
    required: std::cell::OnceCell<std::collections::HashSet<String>>,
    /// The build's number when the entries are kept for later builds
    /// (`wowlua dev`): every flavor stores, and the bytes of a file are
    /// compared by hash.
    pub(crate) generation: Option<u64>,
    /// The hashes of the files' bytes, once per build.
    hashes: HashMap<String, blake3::Hash>,
    pub(crate) hits: usize,
    pub(crate) misses: usize,
    /// The type checks of plain Lua files that later flavors of this build
    /// can reuse; per build, where a path and `FileId` mean the same bytes.
    pub(crate) checks: wowlua_sema::check::reuse::CheckCache,
    /// What checking each file reads of its syntax, by path and `FileId`
    /// (the same in every flavor of the build).
    prepared: HashMap<(String, FileId), Arc<wowlua_sema::check::body::Prepared>>,
    /// Each source's load-order usage, by path, kept across builds (the
    /// language server's checker): the same `FileId`, bytes and game API
    /// give the same usage.
    pub(crate) usages: HashMap<String, UsageEntry>,
    /// Modules' exports, kept across builds (decision T375).
    pub(crate) modules: wowlua_sema::check::portable::ModuleCache,
}

/// A source's load-order usage and what it was computed from.
pub(crate) struct UsageEntry {
    file: FileId,
    hash: blake3::Hash,
    /// The declaration files the markers came from (the flavor's API).
    api: blake3::Hash,
    usage: Arc<wowlua_sema::usage::FileUsage>,
    /// The last build that used it.
    pub(crate) generation: u64,
}

pub(crate) struct CacheEntry {
    /// The source it was compiled from (with `rel`, the map's key).
    src_rel: String,
    kind: SourceKind,
    output: Option<String>,
    file: FileId,
    /// The hash of its bytes, when kept for later builds.
    hash: Option<blake3::Hash>,
    /// The lowering context without `meta`; `None` for plain Lua lowered
    /// as a script, which has no context.
    ctx: Option<Context>,
    /// What lowering reported.
    lowering: Vec<Diagnostic>,
    uses_runtime: bool,
    slices: BTreeSet<wowlua_runtime::Slice>,
    /// What finishing reported.
    finishing: Vec<Diagnostic>,
    /// `None` when finishing failed.
    compiled: Option<Arc<Compiled>>,
    /// The last build that compiled or reused it.
    pub(crate) generation: u64,
}

impl FlavorCache {
    /// A cache for build `generation` that starts with `entries` and keeps
    /// what it compiles for later builds.
    pub(crate) fn kept(
        entries: HashMap<String, Vec<CacheEntry>>,
        hashes: HashMap<String, blake3::Hash>,
        usages: HashMap<String, UsageEntry>,
        modules: wowlua_sema::check::portable::ModuleCache,
        generation: u64,
    ) -> Self {
        FlavorCache {
            entries,
            hashes,
            usages,
            modules,
            generation: Some(generation),
            ..FlavorCache::default()
        }
    }

    /// The hash of `source`'s bytes when entries are kept for later builds;
    /// within one build the bytes are the same.
    fn hash(&mut self, source: &Source, bytes: &[u8]) -> Option<blake3::Hash> {
        self.generation?;
        Some(
            *self
                .hashes
                .entry(source.rel.clone())
                .or_insert_with(|| blake3::hash(bytes)),
        )
    }

    fn get(
        &mut self,
        source: &Source,
        file: FileId,
        hash: Option<blake3::Hash>,
        ctx: Option<&Context>,
    ) -> Option<&CacheEntry> {
        let generation = self.generation.unwrap_or(0);
        let found = self.entries.get_mut(&source.rel)?.iter_mut().find(|e| {
            e.file == file
                && e.hash == hash
                && e.src_rel == source.src_rel
                && e.kind == source.kind
                && e.output == source.output
                && match (&e.ctx, ctx) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.same_except_meta(b),
                    _ => false,
                }
        })?;
        found.generation = generation;
        Some(found)
    }

    fn insert(&mut self, source: &Source, entry: CacheEntry) {
        self.entries
            .entry(source.rel.clone())
            .or_default()
            .push(entry);
    }
}

/// Compiles the package for one build configuration and flavor. Files
/// compiled the same in an earlier flavor are taken from `cache`; with
/// `store`, a later flavor can take this flavor's from it.
#[allow(clippy::too_many_arguments)]
pub fn compile_flavor(
    db: &SourceDb,
    diagnostics: &mut Vec<Diagnostic>,
    files: &[(Source, FileId)],
    declarations: &[Declaration],
    raw: &HashMap<String, Arc<[u8]>>,
    parsed: &Parsed,
    profile: &Profile,
    source_maps: bool,
    meta: Arc<Meta>,
    layout: wowlua_sema::loadorder::Layout,
    api: &[&ParsedDeclaration],
    check: &CheckConfig,
    shared: Option<&wowlua_sema::check::flavors::SharedFlavors>,
    lower_lua: &std::collections::HashSet<String>,
    libraries: &HashMap<String, LibraryUnit>,
    // The sources of the addons it depends on, checked with it, types only.
    foreign: &crate::foreign::Foreign,
    minify: &Minify,
    cache: &mut FlavorCache,
    store: bool,
    // `Some(kept)`: drop the code of every file but `kept` once checked.
    drop_code: Option<&BTreeSet<String>>,
    jobs: &crate::parallel::Jobs,
) -> CompileOutput {
    let mut options = minify.passes.emit_options(line_mode(profile));
    options.mappings = source_maps;
    // `wowlua test`'s build: it has the test side.
    let test_build = files
        .iter()
        .any(|(s, _)| crate::sources::TestRole::of(&s.rel).is_some());
    let mut out = CompileOutput::default();
    wowlua_luair::deep::note(wowlua_syntax::deep::links());

    let unit_of = |src_rel: &str| wowlua_sema::package_of(src_rel).and_then(|n| libraries.get(n));
    let mut inputs: Vec<FileInput> = files
        .iter()
        .zip(&parsed.parses)
        .enumerate()
        .map(|(i, ((source, file), parse))| FileInput {
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
            globals: Default::default(),
            usage: parsed.usage(i),
            prepared: None,
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
            globals: Default::default(),
            usage: None,
            prepared: None,
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
            globals: d.globals,
            usage: None,
            prepared: None,
        });
    }
    // The types-only sources: their declarations, then their files, last.
    let foreign_from = inputs.len();
    for (d, parse) in &foreign.declarations {
        inputs.push(FileInput {
            src_rel: d.source.src_rel.clone(),
            kind: FileKind::Declaration,
            parse: Some(parse),
            xml: None,
            meta: None,
            main_of: None,
            globals: Default::default(),
            usage: None,
            prepared: None,
        });
    }
    let foreign_start = inputs.len();
    for ((source, _), parse) in foreign.files.iter().zip(&foreign.parses) {
        inputs.push(FileInput {
            src_rel: source.src_rel.clone(),
            kind: FileKind::Wowlua,
            parse: Some(parse),
            xml: None,
            meta: unit_of(&source.src_rel).map(|u| u.meta.as_ref()),
            main_of: unit_of(&source.src_rel)
                .filter(|u| u.main.as_deref() == Some(source.src_rel.as_str()))
                .and(wowlua_sema::package_of(&source.src_rel).map(str::to_string)),
            globals: Default::default(),
            usage: None,
            prepared: None,
        });
    }
    let foreign_ids = foreign.file_ids();
    let analysis = wowlua_sema::analyze(&inputs, Some(&meta));
    diagnostics.extend(analysis.diagnostics.iter().cloned());
    // Only types cross to another addon: its modules are not in this
    // one's build (E1041).
    if !foreign.is_empty() {
        for (i, parse) in parsed.parses.iter().enumerate().take(files.len()) {
            let (Some(info), Some(parse)) = (analysis.modules[i].as_ref(), parse) else {
                continue;
            };
            for import in &info.imports {
                let foreign_module = matches!(import.target,
                    Target::Module(m) | Target::Lua { file: m, .. } if m >= foreign_from);
                if import.type_only || !foreign_module {
                    continue;
                }
                let Some(s) = parse
                    .view()
                    .block()
                    .stmts()
                    .into_iter()
                    .find(|s| s.span().start == import.start)
                else {
                    continue;
                };
                diagnostics.push(
                    Diagnostic::error(
                        "E1041",
                        "imports a module of another addon, which is not part of this one",
                    )
                    .with_label(s.span(), "another addon's module")
                    .with_help(
                        "use `import type` for its types, and its `@global` API for its values",
                    ),
                );
            }
        }
    }
    // The standard library's compile-time checks (spec stdlib, "core").
    let stdlib = wowlua_sema::stdlib::check(&inputs, &analysis, &check.stdlib, Some(&meta));
    diagnostics.extend(stdlib.diagnostics);
    out.bindings = stdlib.bindings;
    // Dead modules (spec optimization §15.2.1); the build leaves out those
    // dead in every flavor.
    let dead: std::collections::HashSet<usize> = if profile.optimize.dead_code {
        dead_modules(
            files,
            parsed,
            &analysis,
            layout.loaded_by,
            &cache.required,
            || required_by_name(files, raw, &minify.keep),
        )
    } else {
        std::collections::HashSet::new()
    };
    out.removed = dead.iter().map(|&i| files[i].0.rel.clone()).collect();

    // Load order (spec §5.4): files listed in `build.load-order` keep their
    // listed order; files outside this flavor do not take part.
    let markers = wowlua_sema::usage::Markers::from_table().with_api(&inputs);
    // The active sources' usages, on worker threads (#8): the load order
    // takes them from `FileInput::usage`, and they go after it.
    // (kept across builds: those of unchanged files are taken from there)
    let api_files = {
        let mut h = blake3::Hasher::new();
        for input in inputs.iter().filter(|i| i.kind == FileKind::Declaration) {
            h.update(input.src_rel.as_bytes());
            h.update(b"\0");
        }
        h.finalize()
    };
    let mut hashes: HashMap<usize, blake3::Hash> = HashMap::new();
    let candidates: Vec<usize> = (0..files.len())
        .filter(|&i| {
            matches!(inputs[i].kind, FileKind::Wowlua | FileKind::Lua)
                && inputs[i].parse.is_some()
                && inputs[i].usage.is_none()
                && layout.active.get(i).copied().unwrap_or(true)
        })
        .collect();
    let mut wanted: Vec<usize> = Vec::with_capacity(candidates.len());
    for i in candidates {
        let (source, file) = &files[i];
        let Some(hash) = cache.hash(source, &raw[&source.rel]) else {
            wanted.push(i);
            continue;
        };
        let generation = cache.generation.unwrap_or(0);
        match cache.usages.get_mut(&source.rel) {
            Some(e) if e.file == *file && e.hash == hash && e.api == api_files => {
                e.generation = generation;
                inputs[i].usage = Some(e.usage.clone());
            }
            _ => {
                hashes.insert(i, hash);
                wanted.push(i);
            }
        }
    }
    let mut usages = Vec::with_capacity(wanted.len());
    jobs.region(
        wanted,
        |&i| raw[&files[i].0.rel].len() as u64,
        20,
        |i| {
            let parse = inputs[i].parse.expect("parsed");
            (i, Arc::new(wowlua_sema::usage::file_usage(parse, &markers)))
        },
        |usage| usages.push(usage),
    );
    for (i, usage) in &usages {
        inputs[*i].usage = Some(usage.clone());
        if let (Some(&hash), Some(generation)) = (hashes.get(i), cache.generation) {
            cache.usages.insert(
                files[*i].0.rel.clone(),
                UsageEntry {
                    file: files[*i].1,
                    hash,
                    api: api_files,
                    usage: usage.clone(),
                    generation,
                },
            );
        }
    }
    // The types-only sources load with another addon, not this one.
    let mut active = layout.active.to_vec();
    if !foreign.is_empty() {
        active.resize(foreign_start, true);
        active.resize(inputs.len(), false);
    }
    let layout = wowlua_sema::loadorder::Layout {
        active: &active,
        ..layout
    };
    // (a focused check needs the globals files assign, not the order: its
    // diagnostics wait for the full check)
    let order = if check.focus.is_some() {
        wowlua_sema::loadorder::global_defs_only(&inputs)
    } else {
        wowlua_sema::loadorder::analyze_with(&inputs, &analysis, layout, &markers, jobs)
    };
    for (i, _) in usages {
        inputs[i].usage = None;
    }
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
    let foreign_files = |i: usize| {
        i.checked_sub(foreign_start)
            .and_then(|k| Some((foreign.files.get(k)?, foreign.parses.get(k)?)))
            .map(|((source, file), parse)| ((source.clone(), *file), parse))
    };
    // A focused check: the focused files and the modules they import,
    // directly or not.
    // The files the editor has open: their names and expressions' types
    // are recorded (hover).
    let hovered: Option<std::collections::HashSet<usize>> = check.hover.as_ref().map(|open| {
        files
            .iter()
            .enumerate()
            .filter(|(_, (s, _))| {
                open.contains(&s.abs)
                    || crate::host::canonicalize(&s.abs).is_ok_and(|p| open.contains(&p))
            })
            .map(|(i, _)| i)
            .collect()
    });
    let mut focus_files: std::collections::HashSet<usize> = Default::default();
    let focused: Option<std::collections::HashSet<usize>> = check.focus.as_ref().map(|focus| {
        let mut keep = std::collections::HashSet::new();
        let mut todo: Vec<usize> = files
            .iter()
            .enumerate()
            .filter(|(_, (s, _))| {
                focus.contains(&s.abs)
                    || crate::host::canonicalize(&s.abs).is_ok_and(|p| focus.contains(&p))
            })
            .map(|(i, _)| i)
            .collect();
        focus_files.extend(todo.iter().copied());
        while let Some(i) = todo.pop() {
            if !keep.insert(i) {
                continue;
            }
            for import in analysis
                .modules
                .get(i)
                .into_iter()
                .flatten()
                .flat_map(|m| &m.imports)
            {
                if let Target::Module(m) = import.target {
                    todo.push(m);
                }
            }
        }
        keep
    });
    let settings: Vec<Option<wowlua_sema::check::FileSettings>> = (0..inputs.len())
        .map(|i| {
            let foreign_file = foreign_files(i);
            let (file, parse, ok) = match &foreign_file {
                Some((file, parse)) => (Some(file), Some(*parse), true),
                None => (
                    files.get(i),
                    parsed.parses.get(i).and_then(Option::as_ref),
                    parsed.ok.get(i).copied().unwrap_or(false),
                ),
            };
            match (file, parse) {
                // The standard library's modules are checked in strict mode
                // whatever the project's settings, so their exports are typed
                // for the code that imports them.
                (Some((source, _)), Some(_))
                    if ok && wowlua_sema::module_id(&source.src_rel).starts_with("wowlua:") =>
                {
                    Some(wowlua_sema::check::FileSettings {
                        mode: wowlua_sema::check::query::Mode::Strict,
                        pure_lua: false,
                    })
                }
                (Some((source, _)), Some(parse))
                    if ok && !wowlua_sema::is_compiler_known(&source.src_rel) =>
                {
                    // A library is checked with its own settings.
                    let cfg = unit_of(&source.src_rel).map_or(check, |u| &u.check);
                    check_settings(source, parse, cfg)
                }
                _ => None,
            }
        })
        .collect();
    out.checked = settings
        .iter()
        .enumerate()
        .take(files.len())
        .filter(|(_, s)| s.is_some())
        .map(|(i, _)| i)
        .collect();
    // What checking each file reads of its syntax: prepared once per
    // build, on worker threads (#8).
    let unprepared: Vec<usize> = (0..files.len())
        .filter(|&i| settings.get(i).copied().flatten().is_some() && inputs[i].parse.is_some())
        // (a focused check prepares the files it checks)
        .filter(|&i| focused.as_ref().is_none_or(|keep| keep.contains(&i)))
        .filter(|&i| {
            !cache
                .prepared
                .contains_key(&(files[i].0.rel.clone(), files[i].1))
        })
        .collect();
    let mut prepared = Vec::with_capacity(unprepared.len());
    jobs.region(
        unprepared,
        |&i| raw[&files[i].0.rel].len() as u64,
        20,
        |i| {
            let parse = inputs[i].parse.expect("parsed");
            (i, Arc::new(wowlua_sema::check::body::prepare(parse)))
        },
        |p| prepared.push(p),
    );
    for (i, p) in prepared {
        cache
            .prepared
            .insert((files[i].0.rel.clone(), files[i].1), p);
    }
    for (i, input) in inputs.iter_mut().enumerate().take(files.len()) {
        if settings.get(i).copied().flatten().is_some() {
            input.prepared = cache
                .prepared
                .get(&(files[i].0.rel.clone(), files[i].1))
                .cloned();
        }
    }
    let mut private_methods = std::collections::HashSet::new();
    let mut inline_calls: HashMap<wowlua_span::Span, wowlua_span::Span> = HashMap::new();
    if settings.iter().any(Option::is_some) {
        let checked = wowlua_sema::check::check_package_full(
            &inputs,
            &analysis,
            &settings,
            &check.globals,
            shared,
            shared
                .is_none()
                .then_some(wowlua_sema::check::reuse::Reuse {
                    cache: &mut cache.checks,
                    store,
                    // kept across builds (the language server's checker): a
                    // first pass takes unchanged modules' exports, a full
                    // check only keeps them (decision T375)
                    modules: cache.generation.is_some().then(|| {
                        wowlua_sema::check::portable::ModuleReuse {
                            cache: &mut cache.modules,
                            take_except: focused.is_some().then(|| focus_files.clone()),
                        }
                    }),
                }),
            &order.global_defs,
            &wowlua_sema::check::Extras {
                definitions: check.index,
                // (a focused check checks these; the rest count for the
                // globals)
                only: focused.as_ref(),
                hover: hovered.as_ref(),
                complete: check.complete.as_deref(),
            },
        );
        diagnostics.extend(checked.diagnostics);
        out.definitions = checked.definitions;
        out.hovers = checked.hovers;
        out.completions = checked.completions;
        out.signature = checked.signature;
        out.type_names = checked.type_names;
        out.value_checks = checked.value_checks;
        out.fast_forms = checked.fast_forms;
        private_methods = checked.private_methods;
        inline_calls = checked.inline_calls;
    }
    // What the types-only sources report is their addon's.
    if !foreign_ids.is_empty() {
        diagnostics.retain(|d| {
            !d.labels
                .first()
                .is_some_and(|l| foreign_ids.contains(&l.span.file))
        });
    }
    // A focused check ends with the type check: nothing is lowered.
    if focused.is_some() {
        return out;
    }
    for info in analysis.modules[..foreign_from.min(analysis.modules.len())]
        .iter()
        .flatten()
    {
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
    let reflect_used = analysis.modules[..files.len()].iter().flatten().any(|m| {
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
        use wowlua_syntax::view;
        let is_inline = |d: view::Decorators<'_>| d.iter().any(|d| d.is_named("inline"));
        for (i, (source, file)) in files.iter().enumerate() {
            let Some(parse) = parsed.parses[i].as_ref() else {
                continue;
            };
            if source.kind != SourceKind::Wowlua {
                continue;
            }
            let lexed = &parse.lexed;
            wowlua_syntax::visit::walk_stmts(parse.view().block(), &mut |st| match st.kind() {
                view::StmtKind::LocalFunction { name, .. } if is_inline(st.decorators()) => {
                    let pos = wowlua_luair::ir::Pos::new(
                        lexed.line_of(name.span().start),
                        lexed.column_of(name.span().start),
                    );
                    inline_hints
                        .entry(*file)
                        .or_default()
                        .push((pos, name.span()));
                }
                // `function f()` and `function t.f()`: not local functions
                // (an exported one has the `export`'s decorators).
                view::StmtKind::Function { name, .. } if is_inline(st.decorators()) => {
                    diagnostics.push(
                        Diagnostic::warning(
                            "W3031",
                            "`@inline` is ignored: only local functions and private methods with a single `return` expression are inlined",
                        )
                        .with_label(name.span(), "not a local function"),
                    );
                }
                view::StmtKind::Export(inner)
                    if is_inline(st.decorators())
                        && matches!(inner.kind(), view::StmtKind::Function { .. }) =>
                {
                    diagnostics.push(
                        Diagnostic::warning(
                            "W3031",
                            "`@inline` is ignored: only local functions and private methods with a single `return` expression are inlined",
                        )
                        .with_label(inner.span(), "not a local function"),
                    );
                }
                view::StmtKind::Class(c) => {
                    for m in &c.members() {
                        if !is_inline(m.decorators()) {
                            continue;
                        }
                        // A private method is inlined like a local function
                        // (lowering reports what keeps its calls from it).
                        if let view::ClassMemberKind::Method { name, func, .. } = m.kind()
                            && wowlua_sema::classes::inline_private_method(m)
                        {
                            if !wowlua_sema::classes::inline_shape(func) {
                                diagnostics.push(
                                    Diagnostic::warning(
                                        "W3031",
                                        "`@inline` is ignored: its body is not a single `return` expression",
                                    )
                                    .with_label(name.span(), "not inlined"),
                                );
                            }
                            continue;
                        }
                        diagnostics.push(
                            Diagnostic::warning(
                                "W3031",
                                "`@inline` is ignored: only local functions and private methods with a single `return` expression are inlined",
                            )
                            .with_label(m.span(), "not a private method"),
                        );
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
        // Lowered code is as deep as its source (`wowlua_luair::deep`).
        wowlua_luair::deep::run(|| {
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
            let compiles = optimized.compiles;
            let (emitted, original) = (optimized.emitted, optimized.original);
            if let Some(d) = emitted_parse_error(&emitted.text, file, &source.rel) {
                diagnostics.push(d);
                return None;
            }
            // Without a byte order mark, no error means these bytes compiled.
            let checked = match luacheck::check_unless_compiled(db, file, &emitted.text, compiles) {
                Some(d) => {
                    diagnostics.push(d);
                    false
                }
                None => !emitted.text.starts_with(b"\xEF\xBB\xBF"),
            };
            let output = source.output_path();
            let map = source_maps.then(|| {
                let name = output.rsplit('/').next().unwrap_or(&output);
                Arc::new(wowlua_luair::sourcemap::encode_named(
                    &emitted.mappings,
                    name,
                    &source.rel,
                    &original,
                ))
            });
            let drop = drop_code.is_some_and(|kept| !kept.contains(&source.rel));
            Some(Compiled {
                dropped: drop.then(|| *blake3::hash(&emitted.text).as_bytes()),
                code: if drop { Vec::new() } else { emitted.text },
                map,
                compiles: checked,
            })
        })
    };
    // Each file is finished right after it is lowered, so only one file's
    // IR is alive at a time. Renaming private methods needs every lowered
    // file at once; without private methods it renames nothing.
    let stream = !minify.passes.rename_private || private_methods.is_empty();
    // What finishing reports comes after everything lowering reports, as
    // when every file was lowered before any was finished.
    let mut finish_diagnostics = Vec::new();
    let mut lowered_files: Vec<(usize, wowlua_luair::ir::Chunk)> = Vec::new();
    // Lowers file `i` with its context, or as a script without one, and
    // adds what lowering reports to `diagnostics`; `None` when it fails.
    let lower_file = |i: usize, ctx: Option<&Context>, diagnostics: &mut Vec<Diagnostic>| {
        // Lowered code is as deep as its source (`wowlua_luair::deep`).
        wowlua_luair::deep::run(|| {
            let source = &files[i].0;
            let bytes = &raw[&source.rel];
            // A summarized file (`Front::Summarized`) is parsed where a compile
            // misses, and the parse dropped after lowering.
            let reparsed;
            let parse = match parsed.parses[i].as_ref() {
                Some(parse) => parse,
                None => {
                    reparsed = wowlua_syntax::parse(bytes, files[i].1);
                    &reparsed
                }
            };
            let Some(ctx) = ctx else {
                return match wowlua_lower::lower(parse, bytes) {
                    Ok(chunk) => Some(wowlua_lower::Lowered {
                        chunk,
                        meta_used: BTreeSet::new(),
                        uses_runtime: false,
                        slices: BTreeSet::new(),
                        warnings: Vec::new(),
                        reads_meta: false,
                    }),
                    Err(errors) => {
                        diagnostics.extend(errors);
                        None
                    }
                };
            };
            let mut lowered = match wowlua_lower::lower_module(parse, bytes, ctx) {
                Ok(lowered) => lowered,
                Err(errors) => {
                    diagnostics.extend(errors);
                    return None;
                }
            };
            // The runtime's `config` and `features` tables are the addon's: a
            // library may only use their constant (scalar) values.
            if ctx.dependency
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
            diagnostics.append(&mut lowered.warnings);
            Some(lowered)
        })
    };
    // The checker's runtime checks and `typeName` texts by file, once (each
    // file's context gets its own).
    let mut value_checks_of: HashMap<FileId, HashMap<(u32, u32), _>> = HashMap::new();
    for (span, c) in &out.value_checks {
        value_checks_of
            .entry(span.file)
            .or_default()
            .insert((span.start, span.end), c.clone());
    }
    // Calls of `@inline` private methods, by file (spec §4.3).
    type Offsets = (u32, u32);
    let mut inline_calls_of: HashMap<FileId, HashMap<Offsets, Offsets>> = HashMap::new();
    if profile.optimize.inline {
        for (call, at) in &inline_calls {
            inline_calls_of
                .entry(call.file)
                .or_default()
                .insert((call.start, call.end), (at.start, at.end));
        }
    }
    let mut fast_forms_of: HashMap<FileId, HashMap<(u32, u32), u8>> = HashMap::new();
    for (span, &f) in &out.fast_forms {
        fast_forms_of
            .entry(span.file)
            .or_default()
            .insert((span.start, span.end), f);
    }
    let mut type_names_of: HashMap<FileId, HashMap<(u32, u32), String>> = HashMap::new();
    for (span, name) in &out.type_names {
        type_names_of
            .entry(span.file)
            .or_default()
            .insert((span.start, span.end), name.clone());
    }
    /// A file to compile, planned in file order: taken from the cache, or
    /// lowered (and in stream mode finished) by a job (one per file).
    #[allow(clippy::large_enum_variant)]
    enum Slot {
        Hit {
            i: usize,
            lowering: Vec<Diagnostic>,
            uses_runtime: bool,
            slices: BTreeSet<wowlua_runtime::Slice>,
            finishing: Vec<Diagnostic>,
            compiled: Option<Arc<Compiled>>,
        },
        Miss {
            i: usize,
            ctx: Option<Context>,
            hash: Option<blake3::Hash>,
        },
    }
    /// What a job gave for a miss.
    struct FileResult {
        lowering: Vec<Diagnostic>,
        /// `None` when lowering failed.
        lowered: Option<LoweredFile>,
    }
    struct LoweredFile {
        meta_used: BTreeSet<(MetaKind, Vec<String>)>,
        uses_runtime: bool,
        slices: BTreeSet<wowlua_runtime::Slice>,
        reads_meta: bool,
        /// The IR, kept for rename-private outside stream mode.
        chunk: Option<wowlua_luair::ir::Chunk>,
        finishing: Vec<Diagnostic>,
        compiled: Option<Compiled>,
    }
    let mut slots: Vec<Slot> = Vec::new();
    for (i, (source, file)) in files.iter().enumerate() {
        let plain_lua = source.kind == SourceKind::Lua && lower_lua.contains(&source.rel);
        let minified_lua = source.kind == SourceKind::Lua && minify.lua.contains(&source.rel);
        // Dead modules are compiled still: the build removes a module only
        // if it is dead in every flavor.
        if (source.kind != SourceKind::Wowlua && !plain_lua && !minified_lua) || !parsed.ok[i] {
            continue;
        }
        // Plain Lua is only minified: it compiles to itself (spec §14.2).
        let ctx = if minified_lua && !plain_lua {
            None
        } else {
            let unit = unit_of(&source.src_rel);
            let file_meta = unit.map_or_else(|| meta.clone(), |u| u.meta.clone());
            let mut ctx = context(&analysis, i, &inputs, file_meta, &order.lazy, test_build);
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
            ctx.value_checks = value_checks_of.remove(file).unwrap_or_default();
            ctx.type_names = type_names_of.remove(file).unwrap_or_default();
            ctx.inline_calls = inline_calls_of.remove(file).unwrap_or_default();
            ctx.fast_forms = fast_forms_of.remove(file).unwrap_or_default();
            ctx.compiler_known = wowlua_sema::is_compiler_known(&source.src_rel);
            ctx.dependency = unit.is_some();
            Some(ctx)
        };
        // A file an earlier flavor compiled the same: what it reported goes
        // where compiling it again would put it. Only streamed files are
        // reused, as nothing renames their chunks.
        let hash = cache.hash(source, &raw[&source.rel]);
        if stream && let Some(reused) = cache.get(source, *file, hash, ctx.as_ref()) {
            // Debug builds (and so the tests) compile it again and compare.
            #[cfg(debug_assertions)]
            {
                let mut lowering = Vec::new();
                let lowered = lower_file(i, ctx.as_ref(), &mut lowering).expect("it lowered");
                let mut finishing = Vec::new();
                let compiled = finish(lowered.chunk, source, *file, &mut finishing);
                assert_eq!(
                    format!(
                        "{:?}",
                        (
                            &lowering,
                            lowered.uses_runtime,
                            &lowered.slices,
                            lowered.meta_used.len(),
                            lowered.reads_meta,
                            &finishing,
                            &compiled
                        )
                    ),
                    format!(
                        "{:?}",
                        (
                            &reused.lowering,
                            reused.uses_runtime,
                            &reused.slices,
                            0,
                            false,
                            &reused.finishing,
                            &reused.compiled.as_deref()
                        )
                    ),
                    "{} is reused but compiles differently",
                    source.rel
                );
            }
            slots.push(Slot::Hit {
                i,
                lowering: reused.lowering.clone(),
                uses_runtime: reused.uses_runtime,
                slices: reused.slices.clone(),
                finishing: reused.finishing.clone(),
                compiled: reused.compiled.clone(),
            });
            cache.hits += 1;
            continue;
        }
        cache.misses += 1;
        slots.push(Slot::Miss { i, ctx, hash });
    }
    // Lowering and finishing are pure per file: on worker threads (#8),
    // merged below in file order as compiling one file at a time would.
    let misses: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, Slot::Miss { .. }))
        .map(|(k, _)| k)
        .collect();
    let weight = |k: &usize| match &slots[*k] {
        Slot::Miss { i, .. } => raw[&files[*i].0.rel].len() as u64,
        Slot::Hit { .. } => 0,
    };
    let compute = |k: usize| {
        let Slot::Miss { i, ctx, .. } = &slots[k] else {
            unreachable!("only misses are jobs")
        };
        let (source, file) = &files[*i];
        let mut lowering = Vec::new();
        let lowered = lower_file(*i, ctx.as_ref(), &mut lowering).map(|lowered| {
            let mut finishing = Vec::new();
            let (chunk, compiled) = if stream {
                (None, finish(lowered.chunk, source, *file, &mut finishing))
            } else {
                (Some(lowered.chunk), None)
            };
            LoweredFile {
                meta_used: lowered.meta_used,
                uses_runtime: lowered.uses_runtime,
                slices: lowered.slices,
                reads_meta: lowered.reads_meta,
                chunk,
                finishing,
                compiled,
            }
        });
        FileResult { lowering, lowered }
    };
    // A job takes about this much memory per source byte (the release
    // optimizations clone the IR).
    let per_byte = if minify.passes.is_empty() && !profile.optimize.dead_code {
        50
    } else {
        100
    };
    let mut results: Vec<FileResult> = Vec::with_capacity(misses.len());
    jobs.region(misses, weight, per_byte, compute, |r| results.push(r));
    let mut results = results.into_iter();
    for slot in slots {
        match slot {
            Slot::Hit {
                i,
                lowering,
                uses_runtime,
                slices,
                finishing,
                compiled,
            } => {
                diagnostics.extend(lowering);
                out.needs_runtime |= uses_runtime;
                out.slices.extend(slices);
                finish_diagnostics.extend(finishing);
                if let Some(compiled) = compiled {
                    out.compiled.insert(files[i].0.rel.clone(), compiled);
                }
            }
            Slot::Miss { i, ctx, hash } => {
                let (source, file) = &files[i];
                let FileResult {
                    lowering: lowering_diagnostics,
                    lowered,
                } = results.next().expect("a result per miss");
                let mark = diagnostics.len();
                diagnostics.extend(lowering_diagnostics);
                let Some(lowered) = lowered else {
                    continue;
                };
                // Kept only for a later flavor or build, and only when the
                // flavor's `wowlua:meta` did not shape it.
                let keep = stream && (store || cache.generation.is_some()) && !lowered.reads_meta;
                out.needs_runtime |= lowered.uses_runtime;
                out.meta_used.extend(lowered.meta_used);
                out.slices.extend(lowered.slices.iter().copied());
                if let Some(chunk) = lowered.chunk {
                    lowered_files.push((i, chunk));
                    continue;
                }
                let fmark = finish_diagnostics.len();
                finish_diagnostics.extend(lowered.finishing);
                let compiled = lowered.compiled.map(Arc::new);
                if let Some(compiled) = &compiled {
                    out.compiled
                        .insert(source.rel.clone(), Arc::clone(compiled));
                }
                let lowering = &diagnostics[mark..];
                let finishing = &finish_diagnostics[fmark..];
                // A later build may change another file a diagnostic points into.
                let own = |d: &Diagnostic| d.labels.iter().all(|l| l.span.file == *file);
                let foreign =
                    cache.generation.is_some() && !lowering.iter().chain(finishing).all(own);
                if keep && !foreign {
                    let entry = CacheEntry {
                        src_rel: source.src_rel.clone(),
                        kind: source.kind,
                        output: source.output.clone(),
                        file: *file,
                        hash,
                        ctx: ctx.map(|ctx| Context { meta: None, ..ctx }),
                        lowering: lowering.to_vec(),
                        uses_runtime: lowered.uses_runtime,
                        slices: lowered.slices,
                        finishing: finishing.to_vec(),
                        compiled,
                        generation: cache.generation.unwrap_or(0),
                    };
                    cache.insert(source, entry);
                }
            }
        }
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
        // The words of the API's globals its parsed text leaves out, as
        // their tokens would give them.
        let mut api_words = Vec::new();
        for d in api {
            // The build keeps the API's tokens only for rename-private.
            debug_assert!(!d.parse.lexed.tokens.is_empty(), "API tokens freed");
            parsed_files.push(crate::private_names::Parsed {
                file: d.file,
                parse: &d.parse,
                bytes: db.get(d.file).text().as_bytes(),
            });
            for g in d.globals.iter() {
                let ty = format!("{:?}", g.ty).to_ascii_lowercase();
                let addon = g.on_demand.map(|a| format!(" onDemand {a}"));
                api_words.extend(
                    format!(
                        "declare global {} {ty}{}\n",
                        g.name,
                        addon.unwrap_or_default()
                    )
                    .into_bytes(),
                );
            }
        }
        rename_private_methods(
            &mut lowered_files,
            files,
            &parsed_files,
            &private_methods,
            minify,
            &api_words,
        );
    }
    for (i, chunk) in lowered_files {
        let (source, file) = &files[i];
        if let Some(compiled) = finish(chunk, source, *file, diagnostics) {
            out.compiled.insert(source.rel.clone(), Arc::new(compiled));
        }
    }
    diagnostics.append(&mut finish_diagnostics);
    out
}

/// The class half of `rename-private` over every file of the package.
fn rename_private_methods(
    lowered: &mut [(usize, wowlua_luair::ir::Chunk)],
    files: &[(Source, FileId)],
    parsed: &[crate::private_names::Parsed],
    recorded: &std::collections::HashSet<wowlua_span::Span>,
    minify: &Minify,
    api_words: &[u8],
) {
    if recorded.is_empty() {
        return;
    }
    let foreign: Vec<&[u8]> = minify
        .foreign
        .iter()
        .map(Vec::as_slice)
        .chain([api_words])
        .collect();
    let mut plan = crate::private_names::plan(recorded, parsed, &foreign);
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
/// The module IDs plain Lua requires by name (`require("id")`), and `keep`.
fn required_by_name(
    files: &[(Source, FileId)],
    raw: &HashMap<String, Arc<[u8]>>,
    keep: &[String],
) -> std::collections::HashSet<String> {
    static REQUIRED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"require\s*\(?\s*["']([^"']+)["']"#).expect("a valid pattern")
    });
    let mut by_name: std::collections::HashSet<String> = keep.iter().cloned().collect();
    for (source, _) in files {
        if source.kind == SourceKind::Lua
            && let Some(text) = raw.get(&source.rel)
        {
            let text = String::from_utf8_lossy(text);
            by_name.extend(REQUIRED.captures_iter(&text).map(|c| c[1].to_string()));
        }
    }
    by_name
}

/// `required` holds [`required_by_name`] once a flavor needed it; it is
/// found only when a module could be left out.
fn dead_modules(
    files: &[(Source, FileId)],
    parsed: &Parsed,
    analysis: &wowlua_sema::Analysis,
    loaded_by: &[Option<usize>],
    required: &std::cell::OnceCell<std::collections::HashSet<String>>,
    find_required: impl Fn() -> std::collections::HashSet<String>,
) -> std::collections::HashSet<usize> {
    use wowlua_syntax::view;
    fn pure(e: view::Expr<'_>) -> bool {
        match e.kind() {
            view::ExprKind::Nil
            | view::ExprKind::True
            | view::ExprKind::False
            | view::ExprKind::Number(_)
            | view::ExprKind::String(_)
            | view::ExprKind::Function(_) => true,
            view::ExprKind::Table(fields) => fields.iter().all(|f| match f {
                view::TableField::Keyed { key, value, .. } => {
                    matches!(
                        key.kind(),
                        view::ExprKind::String(_) | view::ExprKind::Number(_)
                    ) && pure(value)
                }
                view::TableField::Named { value, .. } | view::TableField::Positional(value) => {
                    pure(value)
                }
            }),
            _ => false,
        }
    }
    fn declaration(s: view::Stmt<'_>) -> bool {
        s.decorators().is_empty()
            && match s.kind() {
                view::StmtKind::Local { values, .. } => values.iter().all(pure),
                view::StmtKind::LocalFunction { .. }
                | view::StmtKind::Interface(_)
                | view::StmtKind::Enum(_)
                | view::StmtKind::TypeAlias(_)
                | view::StmtKind::Declare(_)
                | view::StmtKind::Import(_) => true,
                view::StmtKind::Const { value, .. } => value.is_none_or(pure),
                view::StmtKind::Class(c) => c.members().iter().all(|m| {
                    m.decorators().is_empty()
                        && match m.kind() {
                            view::ClassMemberKind::Field { value, .. } => {
                                !m.modifiers().contains(&view::Modifier::Static)
                                    || value.is_none_or(pure)
                            }
                            _ => true,
                        }
                }),
                view::StmtKind::Export(inner) => {
                    // `export function f()` defines an export, not a global.
                    matches!(inner.kind(), view::StmtKind::Function { name, .. } if name.path().len() == 1 && name.method().is_none())
                        || declaration(inner)
                }
                _ => false,
            }
    }
    let candidate = |i: usize| -> bool {
        let Some(info) = analysis.modules.get(i).and_then(Option::as_ref) else {
            return false;
        };
        files[i].0.kind == SourceKind::Wowlua
            && info.is_module
            && loaded_by.get(i).copied().flatten().is_none()
            && !required.get_or_init(&find_required).contains(&info.id)
            && parsed.parses[i]
                .as_ref()
                .is_some_and(|p| p.view().block().stmts().iter().all(declaration))
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
    test_build: bool,
) -> Context {
    let mut ctx = Context {
        meta: Some(meta),
        ..Context::default()
    };
    let Some(info) = &analysis.modules[i] else {
        return ctx;
    };
    ctx.module = info.is_module.then(|| info.id.clone());
    ctx.exposed = info.exposed.clone();
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
                    // A test build reads imports at each use, so that
                    // `mock.module` replaces them (spec Tooling §16.1.3.1).
                    lazy: lazy.contains(&(i, import.start))
                        || (test_build && !wowlua_sema::is_compiler_known(&inputs[*m].src_rel)),
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

    // Dropped code compares by its hash, with dropped and kept code alike.
    #[test]
    fn dropped_code_compares_by_its_hash() {
        let kept = |code: &[u8]| Compiled {
            code: code.to_vec(),
            map: None,
            compiles: true,
            dropped: None,
        };
        let dropped = |code: &[u8]| Compiled {
            code: Vec::new(),
            map: None,
            compiles: true,
            dropped: Some(*blake3::hash(code).as_bytes()),
        };
        assert!(kept(b"x = 1").same_code(&kept(b"x = 1")));
        assert!(!kept(b"x = 1").same_code(&kept(b"x = 2")));
        assert!(dropped(b"x = 1").same_code(&dropped(b"x = 1")));
        assert!(!dropped(b"x = 1").same_code(&dropped(b"x = 2")));
        assert!(dropped(b"x = 1").same_code(&kept(b"x = 1")));
        assert!(!kept(b"x = 2").same_code(&dropped(b"x = 1")));
    }

    /// Emitted code that does not parse is an internal error, worded with
    /// the parser's first message.
    #[test]
    fn emitted_code_that_does_not_parse_is_an_internal_error() {
        let mut db = SourceDb::new();
        let file = db.add("src/a.lua", "");
        assert!(emitted_parse_error(b"local a = {1, {2}}\n", file, "src/a.lua").is_none());
        let d = emitted_parse_error(b"x = = 1\ny = (\n", file, "src/a.lua").expect("an error");
        assert_eq!(
            d.message,
            "internal compiler error: the code emitted for this file does not parse: unexpected symbol near '='"
        );
        assert!(d.is_error() && d.code.is_none());
        assert_eq!(d.path.as_deref(), Some(std::path::Path::new("src/a.lua")));
    }

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
