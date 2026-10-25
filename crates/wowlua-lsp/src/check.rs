//! A package checked as `wowlua check` checks it (the test side too), with
//! the editor's unsaved texts over the disk's (`wowlua_driver::overlay`):
//! its diagnostics by file, as the protocol has them, and what each name
//! refers to (go to definition).

use crate::text;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::{Diagnostic, Severity};
use wowlua_driver::Session;
use wowlua_driver::build::BuildRequest;
use wowlua_driver::project::LoadOptions;

/// A name of a file, by its bytes, and where what it refers to is.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub start: u32,
    pub end: u32,
    pub target: Target,
}

/// A name or expression of a file, by its bytes, and its type as messages
/// show it.
#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub start: u32,
    pub end: u32,
    pub ty: String,
}

/// What could stand at the cursor (a completion check's).
#[derive(Clone, Debug, PartialEq)]
pub struct Completions {
    /// Where the cursor is: `names`, `members`, `methods`, `types`,
    /// `strings`, `decorators`, or `none` (where the check found nothing to
    /// complete).
    pub context: &'static str,
    /// Names were left out for the prefix: a longer one gives others.
    pub incomplete: bool,
    pub items: Vec<Completion>,
}

/// One completion: its name, kind (the protocol's number), type, rank
/// (lower first) and where it is declared (a URI and line, for its doc
/// comment).
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    pub label: String,
    pub kind: u8,
    pub detail: String,
    pub rank: u8,
    pub declared: Option<(String, u32)>,
    /// What it does, where no doc comment says (a core decorator).
    pub doc: Option<String>,
}

/// A place in a file: its URI (shared by every name that leads there) and
/// the lines and UTF-16 columns of its start and end. Kept this small, as a
/// large package has a definition for hundreds of thousands of names.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub uri: std::sync::Arc<str>,
    pub range: [u32; 4],
}

impl Target {
    /// The protocol's `Location`.
    pub fn location(&self) -> Json {
        let [l0, c0, l1, c1] = self.range;
        json!({
            "uri": &*self.uri,
            "range": {
                "start": { "line": l0, "character": c0 },
                "end": { "line": l1, "character": c1 },
            },
        })
    }
}

/// What a check found in one package (its manifest), by file (canonical
/// absolute paths).
#[derive(Default)]
pub struct Checked {
    pub manifest: PathBuf,
    /// A focused check's files: what it says about them replaces what was
    /// known (`None`: the whole package).
    pub scope: Option<BTreeSet<PathBuf>>,
    /// Diagnostics; one about the package as a whole goes to its manifest.
    pub diagnostics: BTreeMap<PathBuf, Vec<Json>>,
    /// Each file's names that refer to something, by start.
    pub definitions: HashMap<PathBuf, Vec<Definition>>,
    /// Each open file's names and expressions with their types, by start.
    pub hovers: HashMap<PathBuf, Vec<Hover>>,
    /// A completion check's: what could stand at the cursor.
    pub completions: Option<Completions>,
    /// A completion check's: the forms of the call around the cursor.
    pub signature: Option<Vec<SignatureForm>>,
    /// Why the package could not be loaded (its manifest's first error),
    /// when it could not: nothing of it is checked.
    pub unloaded: Option<String>,
    /// A references or rename check's answer.
    pub found: Option<crate::refs::Found>,
}

/// One form of the function called around the cursor (signature help):
/// its parameters as written (`name: T`), its rest, its results.
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureForm {
    pub params: Vec<String>,
    pub rest: Option<String>,
    pub results: String,
}

/// Checks the package whose manifest is `manifest`; with `whole`, every
/// package of its workspace (those that depend on it too). What loading
/// the workspace reported goes with `manifest`'s package. Definitions are
/// kept for the `open` files (canonical paths) only: an editor asks for
/// those, and a large package has hundreds of thousands.
///
/// With `focus` (canonical paths), only those files and the modules they
/// import are type-checked, and nothing is built: the language server's
/// first pass after an edit, its diagnostics for those files alone.
///
/// With `complete` (the word typed), a completion check: a focused file
/// has the placeholder where the cursor is, and what could stand there is
/// the answer (its diagnostics and the rest are of no use).
pub fn check(
    manifest: &Path,
    whole: bool,
    open: &HashSet<PathBuf>,
    focus: Option<&HashSet<PathBuf>>,
    complete: Option<&str>,
) -> Vec<Checked> {
    check_and(manifest, whole, open, focus, complete, None, None, drop)
}

/// A references or rename check (`find`) of the whole workspace of
/// `manifest`'s package, its results those of a full check. A rename is
/// checked again with the new name, and refused when that check finds
/// errors the first did not, or names that refer to it other than the
/// renamed ones.
pub fn find(
    manifest: &Path,
    open: &HashSet<PathBuf>,
    find: &crate::refs::Find,
    mut warm: Option<&mut Warm>,
    done: fn(Session),
) -> Vec<Checked> {
    let mut out = check_and(
        manifest,
        true,
        open,
        None,
        None,
        Some(find),
        warm.as_deref_mut(),
        done,
    );
    let Some(new) = &find.rename else {
        return out;
    };
    let edits = match out.first().and_then(|c| c.found.as_ref()) {
        Some(crate::refs::Found {
            rename: Some(Ok(edits)),
            ..
        }) => edits.clone(),
        _ => return out,
    };
    let verdict = if edits.is_empty() {
        Err("wowlua found nothing here to rename".to_string())
    } else {
        verify(manifest, open, &out, find, new, &edits, warm)
    };
    if let Err(why) = verdict
        && let Some(found) = out.first_mut().and_then(|c| c.found.as_mut())
    {
        found.rename = Some(Err(why));
    }
    out
}

/// Checks the workspace with `edits` given the name `new` (over the texts
/// the first check read, put back after): no new errors, and the renamed
/// names, and no others, spelled as `new` and referring to the same.
fn verify(
    manifest: &Path,
    open: &HashSet<PathBuf>,
    before: &[Checked],
    find: &crate::refs::Find,
    new: &str,
    edits: &[crate::refs::Place],
    mut warm: Option<&mut Warm>,
) -> Result<(), String> {
    let mut by_file: BTreeMap<&Path, Vec<&crate::refs::Place>> = BTreeMap::new();
    for e in edits {
        by_file.entry(e.path.as_path()).or_default().push(e);
    }
    // each file renamed, and how far its bytes move: (start, end, by)
    let mut originals = Restore {
        files: Vec::new(),
        open,
    };
    let mut moved: HashMap<PathBuf, Vec<(u32, u32, i64)>> = HashMap::new();
    for (path, list) in &mut by_file {
        list.sort_by_key(|e| e.start);
        let Ok(bytes) = wowlua_driver::overlay::read(path) else {
            return Err(format!("{} can't be read", path.display()));
        };
        let mut last = 0u32;
        let mut shifts = Vec::new();
        for e in list.iter() {
            if e.start < last || e.end as usize > bytes.len() {
                return Err("the names to rename overlap".into());
            }
            shifts.push((e.start, e.end, new.len() as i64 - (e.end - e.start) as i64));
            last = e.end;
        }
        moved.insert(path.to_path_buf(), shifts);
        originals.files.push((path.to_path_buf(), bytes));
    }
    for (path, bytes) in &originals.files {
        wowlua_driver::overlay::set(path, renamed_text(bytes, &moved[path], new));
    }
    // where byte `at` of `path` is after the rename (a renamed name: its
    // start)
    let shift = |path: &Path, at: u32| -> u32 {
        let mut delta = 0i64;
        for &(s, e, by) in moved.get(path).into_iter().flatten() {
            if s <= at && at < e {
                return (s as i64 + delta) as u32;
            }
            if e <= at {
                delta += by;
            }
        }
        (at as i64 + delta) as u32
    };
    let again = crate::refs::Find {
        path: find.path.clone(),
        offset: shift(&text::canonical(&find.path), find.offset),
        rename: None,
    };
    // (not kept on disk: the next check reads the texts as they are)
    let store = warm.as_deref_mut().and_then(|w| w.store.take());
    let after = check_and(
        manifest,
        true,
        &HashSet::new(),
        None,
        None,
        Some(&again),
        warm.as_deref_mut(),
        drop,
    );
    if let Some(w) = warm {
        w.store = store;
    }
    drop(originals);
    // errors by file and code
    let errors = |checked: &[Checked]| -> HashMap<(PathBuf, String), Vec<Json>> {
        let mut out: HashMap<(PathBuf, String), Vec<Json>> = HashMap::new();
        for c in checked {
            for (path, items) in &c.diagnostics {
                for d in items.iter().filter(|d| d["severity"] == 1) {
                    let code = d["code"].as_str().unwrap_or_default().to_string();
                    out.entry((path.clone(), code)).or_default().push(d.clone());
                }
            }
        }
        out
    };
    let (old, now) = (errors(before), errors(&after));
    let mut fresh: Vec<_> = now
        .iter()
        .filter(|(key, items)| items.len() > old.get(*key).map_or(0, Vec::len))
        .collect();
    fresh.sort_by(|a, b| a.0.cmp(b.0));
    if let Some(((path, _), items)) = fresh.first() {
        let d = &items[items.len() - 1];
        let line = d["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
        let message = d["message"].as_str().unwrap_or_default();
        let message = message.lines().next().unwrap_or_default();
        return Err(format!(
            "renaming to `{new}` would be an error in {}:{line}: {message}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    let expected: BTreeSet<(PathBuf, u32)> = edits
        .iter()
        .map(|e| (e.path.clone(), shift(&e.path, e.start)))
        .collect();
    let found: BTreeSet<(PathBuf, u32)> = after
        .iter()
        .filter_map(|c| c.found.as_ref())
        .flat_map(|f| f.renamed.iter().map(|p| (p.path.clone(), p.start)))
        .collect();
    if found != expected {
        let file = found
            .symmetric_difference(&expected)
            .next()
            .map(|(path, _)| path.file_name().unwrap_or_default().to_string_lossy())
            .unwrap_or_default();
        return Err(format!(
            "renaming to `{new}` would change what names refer to in {file}: is `{new}` taken there?"
        ));
    }
    Ok(())
}

/// Files whose texts a rename's check replaced, put back when dropped (a
/// panic too): an open document's text, else the disk's.
struct Restore<'a> {
    files: Vec<(PathBuf, Vec<u8>)>,
    open: &'a HashSet<PathBuf>,
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        for (path, bytes) in self.files.drain(..) {
            if self.open.contains(&text::canonical(&path)) {
                wowlua_driver::overlay::set(&path, bytes);
            } else {
                wowlua_driver::overlay::remove(&path);
            }
        }
    }
}

/// `bytes` with the names at `moved`'s places replaced by `new`.
fn renamed_text(bytes: &[u8], moved: &[(u32, u32, i64)], new: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut last = 0usize;
    for &(s, e, _) in moved {
        out.extend_from_slice(&bytes[last..s as usize]);
        out.extend_from_slice(new.as_bytes());
        last = e as usize;
    }
    out.extend_from_slice(&bytes[last..]);
    out
}

/// What a long-lived checker keeps between checks: each package's build
/// cache (by its manifest), so a check repeats only the work of what
/// changed.
#[derive(Default)]
pub struct Warm {
    caches: HashMap<PathBuf, wowlua_driver::BuildCache>,
    /// Checks run, for the server's restarts.
    pub checks: usize,
    /// Where each package's modules are kept between runs (spec compiler
    /// §13.9 Phase 2): `None` keeps them in memory only.
    pub store: Option<Store>,
}

/// Where a checker keeps packages' module caches on disk.
#[derive(Clone, Debug)]
pub enum Store {
    /// `$WOWLUA_HOME/cache/check/` (the language server's).
    Home,
    /// A folder of its own (the tests').
    Dir(PathBuf),
}

impl Store {
    fn path(&self, manifest: &Path) -> Option<PathBuf> {
        match self {
            Store::Home => wowlua_driver::module_cache_path(manifest),
            Store::Dir(dir) => Some(wowlua_driver::module_cache_in(dir, manifest)),
        }
    }
}

impl Warm {
    /// A checker that keeps the packages' modules on disk too.
    pub fn persistent(store: Store) -> Warm {
        Warm {
            store: Some(store),
            ..Warm::default()
        }
    }

    /// The modules kept for `manifest`'s package, for the tests.
    pub fn kept_modules(&self, manifest: &Path) -> usize {
        self.caches.get(manifest).map_or(0, |c| c.kept_modules())
    }

    /// What the last build of `manifest`'s package took from its cache.
    pub fn stats(&self, manifest: &Path) -> Option<wowlua_driver::CacheStats> {
        self.caches.get(manifest).map(|c| c.stats)
    }
}

/// [`check`] with what `warm` kept from earlier checks (a long-lived
/// checker's), keeping this one's there.
pub fn check_warm(
    manifest: &Path,
    whole: bool,
    open: &HashSet<PathBuf>,
    focus: Option<&HashSet<PathBuf>>,
    complete: Option<&str>,
    warm: Option<&mut Warm>,
) -> Vec<Checked> {
    check_and(manifest, whole, open, focus, complete, None, warm, drop)
}

/// [`check`], handing the session to `done` (a process about to end
/// leaves it unfreed: a large package's take tens of milliseconds).
#[allow(clippy::too_many_arguments)]
fn check_and(
    manifest: &Path,
    whole: bool,
    open: &HashSet<PathBuf>,
    focus: Option<&HashSet<PathBuf>>,
    complete: Option<&str>,
    find: Option<&crate::refs::Find>,
    mut warm: Option<&mut Warm>,
    done: fn(Session),
) -> Vec<Checked> {
    let dir = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut session = Session::new();
    session.focus = focus.cloned();
    session.complete = complete.map(str::to_string);
    // (a completion check's too: signature help reads the callee's, which
    // stands before the cursor, where its text is the document's)
    session.definitions = Some(Vec::new());
    if complete.is_none() {
        // (the open files' types, for hover)
        session.hover = Some(open.clone());
    }
    let whole = whole && focus.is_none();
    let options = LoadOptions {
        manifest: Some(manifest.to_path_buf()),
        cwd: dir.clone(),
        package: None,
        locked: false,
        // the editor leaves wowlua.lock alone
        lock_in_memory: true,
        // the editor never reaches the network (spec §16.2)
        offline: true,
        skip_dependencies: false,
        update: None,
        precise: None,
        status: None,
        bundled: false,
        // (a member's whole check: those of its workspace that depend on it)
        all_members: whole,
    };
    let mut roots = Roots {
        dirs: vec![dir.clone()],
        ..Roots::default()
    };
    // (diagnostics and definitions by package, as each build makes them)
    let mut parts: Vec<Part> = Vec::new();
    let loaded = wowlua_driver::load_projects(&mut session, &options);
    let loading: Vec<Diagnostic> = session.take_diagnostics().into_iter().collect();
    // (nothing is checked: the editor is told why)
    let unloaded = loaded
        .is_none()
        .then(|| unloaded(&session, &roots, manifest, &loading));
    parts.push((manifest.to_path_buf(), loading, Vec::new(), Vec::new()));
    if let Some(projects) = loaded {
        // (where a dependency's files are: the workspace's packages', and
        // those they depend on)
        for project in &projects {
            if let Some(p) = project.manifest.package() {
                let src = &project.manifest.manifest.build.src;
                roots.package(&p.name, project.root(), src);
            }
            for p in &project.deps.packages {
                let src = p.manifest.as_ref().map_or("src", |m| m.build.src.as_str());
                roots.package(&p.name, &p.dir, src);
            }
        }
        let own = text::canonical(manifest);
        // an edit checks its own package (a workspace root that is no
        // package: every member)
        let only_own = !whole
            && projects
                .iter()
                .any(|p| text::canonical(&p.manifest.path) == own);
        for project in projects {
            if only_own && text::canonical(&project.manifest.path) != own {
                continue;
            }
            let request = BuildRequest {
                profile: "dev".into(),
                flavors: None,
                out_dir: None,
                write: false,
                nolib: false,
                minify: None,
                bundle: None,
                only: None,
                jobs: wowlua_driver::parallel::Jobs::resolve(None),
                keep_code: false,
                tests: true,
                left_out: Vec::new(),
            };
            let _ = match warm.as_deref_mut() {
                Some(w) => {
                    let store = w
                        .store
                        .as_ref()
                        .and_then(|s| s.path(&project.manifest.path));
                    let cache = w
                        .caches
                        .entry(project.manifest.path.clone())
                        .or_insert_with(|| {
                            // (a fresh server: the modules an earlier one kept)
                            let mut cache = wowlua_driver::BuildCache::default();
                            if let Some(path) = &store {
                                cache.load_modules(path);
                            }
                            cache
                        });
                    let built =
                        wowlua_driver::build_with_cache(&mut session, &project, &request, cache);
                    // a full check keeps every module: kept for the next
                    // server too
                    if focus.is_none()
                        && let Some(path) = &store
                        && let Err(e) = cache.save_modules(path)
                    {
                        eprintln!("cannot keep the checked modules in {}: {e}", path.display());
                    }
                    built
                }
                None => wowlua_driver::build(&mut session, &project, &request),
            };
            project.apply_lints(&mut session);
            roots.dirs.push(project.root().to_path_buf());
            let defs = session.definitions.replace(Vec::new()).unwrap_or_default();
            let hovers = std::mem::take(&mut session.hovers);
            parts.push((
                project.manifest.path.clone(),
                session.take_diagnostics().into_iter().collect(),
                defs,
                hovers,
            ));
        }
    }
    let mut out: Vec<Checked> = Vec::new();
    if complete.is_some() {
        // (none when the check never met the placeholder)
        let completions = match session.completions.take() {
            Some(found) => completions_of(&session, &roots, found),
            None => Completions {
                context: "none",
                incomplete: false,
                items: Vec::new(),
            },
        };
        let signature = session.signature.take().map(|forms| {
            forms
                .into_iter()
                .map(|s| SignatureForm {
                    params: s.params,
                    rest: s.rest,
                    results: s.results,
                })
                .collect()
        });
        let defs: Vec<Spans> = parts.iter().flat_map(|p| p.2.iter().cloned()).collect();
        let mut definitions = definitions(&session, &roots, &defs, open);
        if let Some(f) = focus {
            definitions.retain(|path, _| f.contains(path));
        }
        done(session);
        return vec![Checked {
            manifest: text::canonical(manifest),
            completions: Some(completions),
            signature,
            definitions,
            ..Checked::default()
        }];
    }
    // a references or rename check: every name the workspace's checks met
    let found = find.map(|f| {
        let defs: Vec<Spans> = parts.iter().flat_map(|p| p.2.iter().copied()).collect();
        crate::refs::find(&session, &roots, &defs, f)
    });
    for (package, diagnostics, defs, hovers) in parts {
        let package = text::canonical(&package);
        let mut checked = Checked {
            definitions: definitions(&session, &roots, &defs, open),
            hovers: hovers_of(&session, &roots, &hovers, open),
            scope: focus.map(|f| f.iter().cloned().collect()),
            ..Checked::default()
        };
        for d in &diagnostics {
            if let Some((path, item)) = convert(&session, &roots, &package, d) {
                // (a focused check speaks for its files only)
                if focus.is_some_and(|f| !f.contains(&path)) {
                    continue;
                }
                checked.diagnostics.entry(path).or_default().push(item);
            }
        }
        if let Some(f) = focus {
            checked.definitions.retain(|path, _| f.contains(path));
            checked.hovers.retain(|path, _| f.contains(path));
        }
        // (a package loaded and built: one part)
        match out.iter_mut().find(|c| c.manifest == package) {
            Some(c) => {
                for (path, items) in checked.diagnostics {
                    c.diagnostics.entry(path).or_default().extend(items);
                }
                c.definitions.extend(checked.definitions);
                c.hovers.extend(checked.hovers);
            }
            None => {
                checked.manifest = package;
                out.push(checked);
            }
        }
    }
    if let Some(first) = out.first_mut() {
        first.unloaded = unloaded;
        first.found = found;
    }
    done(session);
    out
}

/// Why a package could not be loaded, as the editor shows it: its name (as
/// its manifest says, else its folder's), the first error and its file.
fn unloaded(session: &Session, roots: &Roots, manifest: &Path, found: &[Diagnostic]) -> String {
    let error = found.iter().find(|d| d.severity == Severity::Error);
    let file = error
        .and_then(|d| {
            d.labels
                .first()
                .map(|l| session.db.get(l.span.file).path().to_path_buf())
                .or_else(|| d.path.clone())
        })
        .unwrap_or_else(|| manifest.to_path_buf());
    let file = absolute(roots, &file).unwrap_or(file);
    let json = wowlua_driver::host::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Json>(&t).ok());
    let folder = file
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = json
        .as_ref()
        .and_then(|j| j["package"]["name"].as_str())
        .map(str::to_string)
        .unwrap_or(folder);
    let why = error.map_or("its manifest has errors", |d| d.message.as_str());
    let shown = file.file_name().unwrap_or_default().to_string_lossy();
    format!("{name} can't be checked: {why} ({shown})")
}

/// What a completion check found, for the protocol.
fn completions_of(
    session: &Session,
    roots: &Roots,
    found: wowlua_sema::check::complete::Completions,
) -> Completions {
    use wowlua_sema::check::complete::{Context, Kind};
    let context = match found.context {
        Context::Names => "names",
        Context::Members { method: false } => "members",
        Context::Members { method: true } => "methods",
        Context::Types => "types",
        Context::Strings => "strings",
        Context::Decorators => "decorators",
    };
    // (each file's URI and lines once)
    let mut files: HashMap<wowlua_span::FileId, Option<(String, text::Lines<'_>)>> = HashMap::new();
    let items = found
        .items
        .into_iter()
        .map(|item| {
            let declared = item.at.and_then(|at| {
                let (uri, lines) = files
                    .entry(at.file)
                    .or_insert_with(|| {
                        let file = session.db.get(at.file);
                        let path = absolute(roots, file.path())
                            .or_else(|| materialize(file.path(), file.text()))?;
                        Some((text::uri_of(&path), text::Lines::new(file.text())))
                    })
                    .as_ref()?;
                Some((uri.clone(), lines.line_column(at.start as usize).0))
            });
            // the protocol's `CompletionItemKind`
            let kind = match item.kind {
                Kind::Variable => 6,
                Kind::Constant => 21,
                Kind::Function => 3,
                Kind::Method => 2,
                Kind::Field => 5,
                Kind::Property => 10,
                Kind::Class => 7,
                Kind::Interface => 8,
                Kind::Alias => 22,
                Kind::Enum => 13,
                Kind::EnumMember => 20,
                Kind::Module => 9,
                Kind::TypeParameter => 25,
                Kind::Builtin => 14,
                Kind::Literal => 12,
                Kind::Decorator => 14,
            };
            Completion {
                label: item.label,
                kind,
                detail: item.detail,
                rank: item.rank,
                declared,
                doc: item.doc,
            }
        })
        .collect();
    Completions {
        context,
        incomplete: found.incomplete,
        items,
    }
}

/// A name's span and its definition's.
type Spans = (wowlua_span::Span, wowlua_span::Span);

/// What one package's build gave: its manifest, diagnostics, definitions
/// and hovers.
type Part = (
    PathBuf,
    Vec<wowlua_diagnostics::Diagnostic>,
    Vec<Spans>,
    Vec<(wowlua_span::Span, String)>,
);

/// The hovers of the open files, by file.
fn hovers_of(
    session: &Session,
    roots: &Roots,
    found: &[(wowlua_span::Span, String)],
    open: &HashSet<PathBuf>,
) -> HashMap<PathBuf, Vec<Hover>> {
    let mut out: HashMap<PathBuf, Vec<Hover>> = HashMap::new();
    let mut paths: HashMap<wowlua_span::FileId, Option<PathBuf>> = HashMap::new();
    for (span, ty) in found {
        let Some(path) = paths
            .entry(span.file)
            .or_insert_with(|| {
                absolute(roots, session.db.get(span.file).path()).filter(|p| open.contains(p))
            })
            .clone()
        else {
            continue;
        };
        out.entry(path).or_default().push(Hover {
            start: span.start,
            end: span.end,
            ty: ty.clone(),
        });
    }
    for list in out.values_mut() {
        list.sort_by_key(|h| (h.start, h.end));
        list.dedup_by(|a, b| a.start == b.start && a.end == b.end);
    }
    out
}

/// (name, definition) spans as each file's [`Definition`]s.
fn definitions(
    session: &Session,
    roots: &Roots,
    found: &[Spans],
    open: &HashSet<PathBuf>,
) -> HashMap<PathBuf, Vec<Definition>> {
    let mut out: HashMap<PathBuf, Vec<Definition>> = HashMap::new();
    // Each file's path (or none) and line index, once: a file on disk for
    // names, also one written for the compiler's own as a target.
    let mut paths: HashMap<wowlua_span::FileId, Option<PathBuf>> = HashMap::new();
    let mut targets: HashMap<wowlua_span::FileId, Option<std::sync::Arc<str>>> = HashMap::new();
    let mut lines: HashMap<wowlua_span::FileId, text::Lines<'_>> = HashMap::new();
    for (at, to) in found {
        let Some(path) = paths
            .entry(at.file)
            .or_insert_with(|| {
                absolute(roots, session.db.get(at.file).path()).filter(|p| open.contains(p))
            })
            .clone()
        else {
            continue;
        };
        let uri = targets
            .entry(to.file)
            .or_insert_with(|| {
                let file = session.db.get(to.file);
                absolute(roots, file.path())
                    .or_else(|| materialize(file.path(), file.text()))
                    .map(|p| text::uri_of(&p).into())
            })
            .clone();
        let Some(uri) = uri else {
            continue;
        };
        let lines = lines
            .entry(to.file)
            .or_insert_with(|| text::Lines::new(session.db.get(to.file).text()));
        let (l0, c0) = lines.line_column(to.start as usize);
        let (l1, c1) = lines.line_column(to.end as usize);
        out.entry(path).or_default().push(Definition {
            start: at.start,
            end: at.end,
            target: Target {
                uri,
                range: [l0, c0, l1, c1],
            },
        });
    }
    for list in out.values_mut() {
        list.sort_by_key(|a| (a.start, a.end));
        list.dedup();
    }
    out
}

/// Where [`materialize`] writes the compiler's built-in files.
pub fn builtin_dir() -> PathBuf {
    // (a host of its own, such as a page's in-memory files, keeps them in
    // a hidden folder at its root: no temporary directory there)
    if wowlua_driver::host::installed().is_some() {
        return PathBuf::from(format!("/.wowlua-{}", env!("CARGO_PKG_VERSION")));
    }
    #[allow(clippy::disallowed_methods)] // (the computer's: no host installed)
    let temp = std::env::temp_dir();
    temp.join(format!("wowlua-{}", env!("CARGO_PKG_VERSION")))
}

/// A file the compiler has built in (the game's API, the standard
/// library's declarations) written where the editor can open it: the
/// system's temporary directory, one folder per wowlua version.
pub(crate) fn materialize(rel: &Path, contents: &str) -> Option<PathBuf> {
    let name: String = rel
        .to_string_lossy()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '@' | '*' | '?' | '"' | '|' => '_',
            c => c,
        })
        .collect();
    let path = builtin_dir().join(name.trim_start_matches(['/', '\\']));
    let same =
        |path: &Path| wowlua_driver::host::read(path).is_ok_and(|b| b.len() == contents.len());
    if !same(&path) {
        wowlua_driver::host::create_dir_all(path.parent()?).ok()?;
        // written whole, then renamed: a reader never sees half a file;
        // each writer has its own partial file, so two checks writing the
        // same declaration at once don't take each other's
        static WRITES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial =
            path.with_extension(format!("{}-{n}.partial", wowlua_driver::host::process_id()));
        let written = wowlua_driver::host::write(&partial, contents)
            .and_then(|()| wowlua_driver::host::rename(&partial, &path));
        if written.is_err() {
            let _ = wowlua_driver::host::remove_file(&partial);
            if !same(&path) {
                return None;
            }
        }
    }
    Some(path)
}

/// Where a check's files are: the folders their paths are relative to, and
/// each package's folder and source folder by name (a dependency's files
/// are `@pkg/<name>/<path>`).
#[derive(Default)]
pub(crate) struct Roots {
    pub dirs: Vec<PathBuf>,
    pub packages: HashMap<String, (PathBuf, PathBuf)>,
}

impl Roots {
    /// A package's folders, by its name.
    fn package(&mut self, name: &str, dir: &Path, src: &str) {
        self.packages
            .entry(name.to_string())
            .or_insert_with(|| (dir.join(src), dir.to_path_buf()));
    }
}

impl std::ops::Deref for Roots {
    type Target = [PathBuf];

    fn deref(&self) -> &[PathBuf] {
        &self.dirs
    }
}

/// A file of the session as a canonical absolute path, if it is one on
/// disk.
pub(crate) fn absolute(roots: &Roots, rel: &Path) -> Option<PathBuf> {
    let found = if rel.is_absolute() {
        wowlua_driver::host::exists(rel).then(|| rel.to_path_buf())
    } else if let Some((name, rest)) = rel
        .to_str()
        .and_then(|r| r.strip_prefix(wowlua_sema::PACKAGE_PREFIX))
        .and_then(|r| r.split_once('/'))
    {
        let (src, dir) = roots.packages.get(name)?;
        [src.join(rest), dir.join(rest)]
            .into_iter()
            .find(|p| wowlua_driver::host::exists(p))
    } else {
        roots
            .iter()
            .map(|r| r.join(rel))
            .find(|p| wowlua_driver::host::exists(p))
    }?;
    Some(text::canonical(&found))
}

fn convert(
    session: &Session,
    roots: &Roots,
    manifest: &Path,
    d: &Diagnostic,
) -> Option<(PathBuf, Json)> {
    let severity = match d.severity {
        Severity::Error => 1,
        Severity::Warning => 2,
        Severity::Note => 3,
    };
    let mut message = d.message.clone();
    if let Some(label) = d.labels.first().filter(|l| !l.message.is_empty()) {
        message.push_str(&format!(": {}", label.message));
    }
    for note in &d.notes {
        message.push_str(&format!("\n{note}"));
    }
    if let Some(help) = &d.help {
        message.push_str(&format!("\nhelp: {help}"));
    }
    let (path, range) = match d.labels.first() {
        Some(label) => {
            let file = session.db.get(label.span.file);
            let path = absolute(roots, file.path())?;
            let r = label.span.range();
            (path, text::range(file.text(), r.start, r.end))
        }
        None => {
            let path = d
                .path
                .as_deref()
                .and_then(|p| absolute(roots, p))
                .unwrap_or_else(|| manifest.to_path_buf());
            (path, text::range("", 0, 0))
        }
    };
    let mut item = json!({
        "range": range,
        "severity": severity,
        "source": "wowlua",
        "message": message,
    });
    if let Some(code) = d.code {
        item["code"] = json!(code);
    }
    Some((path, item))
}

// ------------------------------------------------------------ the child --

/// A check as one process asks another for it (`wowlua lsp --check`): the
/// package, whether its whole workspace, and the editor's open documents
/// (their unsaved texts are what is checked).
pub fn request(
    manifest: &Path,
    whole: bool,
    docs: &[(PathBuf, String)],
    focus: Option<&HashSet<PathBuf>>,
    rescan: bool,
    complete: Option<&str>,
) -> Json {
    json!({
        "manifest": manifest,
        "whole": whole,
        "focus": focus,
        "rescan": rescan,
        "complete": complete,
        "docs": docs.iter().map(|(p, t)| json!({ "path": p, "text": t })).collect::<Vec<_>>(),
    })
}

/// A [`request`] answered by a long-lived checker (`wowlua lsp --worker`):
/// the open documents are exactly the request's, and what `warm` kept is
/// reused.
pub fn worker(warm: &mut Warm, request: &Json) -> Json {
    let manifest = PathBuf::from(request["manifest"].as_str().unwrap_or_default());
    let whole = request["whole"].as_bool().unwrap_or(false);
    let mut open = HashSet::new();
    let mut docs = Vec::new();
    for doc in request["docs"].as_array().into_iter().flatten() {
        let (Some(path), Some(text)) = (doc["path"].as_str(), doc["text"].as_str()) else {
            continue;
        };
        let path = PathBuf::from(path);
        open.insert(text::canonical(&path));
        docs.push((path, std::sync::Arc::from(text.as_bytes())));
    }
    wowlua_driver::overlay::replace_all(docs);
    // files were created or deleted: the folders are walked again
    if request["rescan"].as_bool() == Some(true) {
        wowlua_driver::forget_walks();
    }
    let focus: Option<HashSet<PathBuf>> = request["focus"].as_array().map(|f| {
        f.iter()
            .filter_map(Json::as_str)
            .map(PathBuf::from)
            .collect()
    });
    warm.checks += 1;
    // (the session is freed on another thread: the answer goes out first)
    let complete = request["complete"].as_str();
    let free: fn(Session) = |s| {
        std::thread::spawn(move || drop(s));
    };
    let packages = match crate::refs::Find::from_json(&request["find"]) {
        Some(f) => find(&manifest, &open, &f, Some(warm), free),
        None => check_and(
            &manifest,
            whole,
            &open,
            focus.as_ref(),
            complete,
            None,
            Some(warm),
            free,
        ),
    };
    Json::Array(packages.iter().map(Checked::to_json).collect())
}

/// What a worker's [`worker`] wrote, read back.
pub fn answer(json: &Json) -> Option<Vec<Checked>> {
    json.as_array()?.iter().map(Checked::from_json).collect()
}

impl Checked {
    /// The definitions as rows of numbers, their URIs listed once.
    fn to_json(&self) -> Json {
        let mut uris: Vec<std::sync::Arc<str>> = Vec::new();
        let mut index: HashMap<std::sync::Arc<str>, usize> = HashMap::new();
        let mut files = serde_json::Map::new();
        for (path, list) in &self.definitions {
            let rows: Vec<Json> = list
                .iter()
                .map(|d| {
                    let u = *index.entry(d.target.uri.clone()).or_insert_with(|| {
                        uris.push(d.target.uri.clone());
                        uris.len() - 1
                    });
                    let [l0, c0, l1, c1] = d.target.range;
                    json!([d.start, d.end, u, l0, c0, l1, c1])
                })
                .collect();
            files.insert(path.to_string_lossy().into_owned(), Json::Array(rows));
        }
        json!({
            "manifest": self.manifest,
            "scope": self.scope,
            "diagnostics": self.diagnostics,
            "uris": uris.iter().map(|u| &**u).collect::<Vec<_>>(),
            "definitions": files,
            "hovers": self
                .hovers
                .iter()
                .map(|(path, list)| {
                    let rows: Vec<Json> = list.iter().map(|h| json!([h.start, h.end, h.ty])).collect();
                    (path.to_string_lossy().into_owned(), Json::Array(rows))
                })
                .collect::<serde_json::Map<_, _>>(),
            "unloaded": self.unloaded,
            "found": self.found.as_ref().map(crate::refs::Found::to_json),
            "signature": self.signature.as_ref().map(|forms| forms.iter().map(|f| json!([f.params, f.rest, f.results])).collect::<Vec<_>>()),
            "completions": self.completions.as_ref().map(|c| json!({
                "context": c.context,
                "incomplete": c.incomplete,
                "items": c.items.iter().map(|i| json!([i.label, i.kind, i.detail, i.rank, i.declared, i.doc])).collect::<Vec<_>>(),
            })),
        })
    }

    fn from_json(json: &Json) -> Option<Checked> {
        let uris: Vec<std::sync::Arc<str>> = json["uris"]
            .as_array()?
            .iter()
            .map(|u| u.as_str().map(Into::into))
            .collect::<Option<_>>()?;
        let mut definitions = HashMap::new();
        for (path, rows) in json["definitions"].as_object()? {
            let mut list = Vec::new();
            for row in rows.as_array()? {
                let n = |i: usize| row.get(i).and_then(Json::as_u64).map(|v| v as u32);
                list.push(Definition {
                    start: n(0)?,
                    end: n(1)?,
                    target: Target {
                        uri: uris.get(n(2)? as usize)?.clone(),
                        range: [n(3)?, n(4)?, n(5)?, n(6)?],
                    },
                });
            }
            definitions.insert(PathBuf::from(path), list);
        }
        let mut hovers = HashMap::new();
        for (path, rows) in json["hovers"].as_object().into_iter().flatten() {
            let mut list = Vec::new();
            for row in rows.as_array()? {
                list.push(Hover {
                    start: row.get(0)?.as_u64()? as u32,
                    end: row.get(1)?.as_u64()? as u32,
                    ty: row.get(2)?.as_str()?.to_string(),
                });
            }
            hovers.insert(PathBuf::from(path), list);
        }
        let diagnostics = json["diagnostics"]
            .as_object()?
            .iter()
            .map(|(p, items)| Some((PathBuf::from(p), items.as_array()?.clone())))
            .collect::<Option<_>>()?;
        Some(Checked {
            manifest: PathBuf::from(json["manifest"].as_str()?),
            scope: json["scope"].as_array().map(|s| {
                s.iter()
                    .filter_map(Json::as_str)
                    .map(PathBuf::from)
                    .collect()
            }),
            diagnostics,
            definitions,
            hovers,
            completions: completions_from(&json["completions"]),
            unloaded: json["unloaded"].as_str().map(str::to_string),
            found: crate::refs::Found::from_json(&json["found"]),
            signature: json["signature"].as_array().map(|forms| {
                forms
                    .iter()
                    .filter_map(|f| {
                        Some(SignatureForm {
                            params: f
                                .get(0)?
                                .as_array()?
                                .iter()
                                .filter_map(|p| p.as_str().map(str::to_string))
                                .collect(),
                            rest: f.get(1).and_then(Json::as_str).map(str::to_string),
                            results: f.get(2)?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            }),
        })
    }
}

/// [`Completions`] as `Checked::to_json` writes them.
fn completions_from(json: &Json) -> Option<Completions> {
    let context = match json["context"].as_str()? {
        "names" => "names",
        "members" => "members",
        "methods" => "methods",
        "types" => "types",
        "strings" => "strings",
        "decorators" => "decorators",
        _ => "none",
    };
    let items = json["items"]
        .as_array()?
        .iter()
        .map(|row| {
            Some(Completion {
                label: row.get(0)?.as_str()?.to_string(),
                kind: row.get(1)?.as_u64()? as u8,
                detail: row.get(2)?.as_str()?.to_string(),
                rank: row.get(3)?.as_u64()? as u8,
                declared: row.get(4).and_then(|d| {
                    Some((d.get(0)?.as_str()?.to_string(), d.get(1)?.as_u64()? as u32))
                }),
                doc: row.get(5).and_then(Json::as_str).map(str::to_string),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Completions {
        context,
        incomplete: json["incomplete"].as_bool().unwrap_or(false),
        items,
    })
}
