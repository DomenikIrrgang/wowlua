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
pub fn check(
    manifest: &Path,
    whole: bool,
    open: &HashSet<PathBuf>,
    focus: Option<&HashSet<PathBuf>>,
) -> Vec<Checked> {
    check_and(manifest, whole, open, focus, drop)
}

/// [`check`], handing the session to `done` (a process about to end
/// leaves it unfreed: a large package's take tens of milliseconds).
fn check_and(
    manifest: &Path,
    whole: bool,
    open: &HashSet<PathBuf>,
    focus: Option<&HashSet<PathBuf>>,
    done: fn(Session),
) -> Vec<Checked> {
    let dir = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut session = Session::new();
    session.definitions = Some(Vec::new());
    session.focus = focus.cloned();
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
    };
    let mut roots = vec![dir.clone()];
    // (diagnostics and definitions by package, as each build makes them)
    let mut parts: Vec<(PathBuf, Vec<wowlua_diagnostics::Diagnostic>, Vec<Spans>)> = Vec::new();
    let loaded = wowlua_driver::load_projects(&mut session, &options);
    parts.push((
        manifest.to_path_buf(),
        session.take_diagnostics().into_iter().collect(),
        Vec::new(),
    ));
    if let Some(projects) = loaded {
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
            let _ = wowlua_driver::build(&mut session, &project, &request);
            project.apply_lints(&mut session);
            roots.push(project.root().to_path_buf());
            let defs = session.definitions.replace(Vec::new()).unwrap_or_default();
            parts.push((
                project.manifest.path.clone(),
                session.take_diagnostics().into_iter().collect(),
                defs,
            ));
        }
    }
    let mut out: Vec<Checked> = Vec::new();
    for (package, diagnostics, defs) in parts {
        let package = text::canonical(&package);
        let mut checked = Checked {
            definitions: definitions(&session, &roots, &defs, open),
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
        }
        // (a package loaded and built: one part)
        match out.iter_mut().find(|c| c.manifest == package) {
            Some(c) => {
                for (path, items) in checked.diagnostics {
                    c.diagnostics.entry(path).or_default().extend(items);
                }
                c.definitions.extend(checked.definitions);
            }
            None => {
                checked.manifest = package;
                out.push(checked);
            }
        }
    }
    done(session);
    out
}

/// A name's span and its definition's.
type Spans = (wowlua_span::Span, wowlua_span::Span);

/// (name, definition) spans as each file's [`Definition`]s.
fn definitions(
    session: &Session,
    roots: &[PathBuf],
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

/// A file the compiler has built in (the game's API, the standard
/// library's declarations) written where the editor can open it: the
/// system's temporary directory, one folder per wowlua version.
fn materialize(rel: &Path, contents: &str) -> Option<PathBuf> {
    let name: String = rel
        .to_string_lossy()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '@' | '*' | '?' | '"' | '|' => '_',
            c => c,
        })
        .collect();
    let path = std::env::temp_dir()
        .join(format!("wowlua-{}", env!("CARGO_PKG_VERSION")))
        .join(name.trim_start_matches(['/', '\\']));
    let fresh = std::fs::metadata(&path).is_ok_and(|m| m.len() == contents.len() as u64);
    if !fresh {
        std::fs::create_dir_all(path.parent()?).ok()?;
        // written whole, then renamed: a reader never sees half a file
        let partial = path.with_extension("partial");
        std::fs::write(&partial, contents).ok()?;
        std::fs::rename(&partial, &path).ok()?;
    }
    Some(path)
}

/// A file of the session as a canonical absolute path, if it is one on
/// disk.
fn absolute(roots: &[PathBuf], rel: &Path) -> Option<PathBuf> {
    let found = if rel.is_absolute() {
        rel.exists().then(|| rel.to_path_buf())
    } else {
        roots.iter().map(|r| r.join(rel)).find(|p| p.exists())
    }?;
    Some(text::canonical(&found))
}

fn convert(
    session: &Session,
    roots: &[PathBuf],
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
) -> Json {
    json!({
        "manifest": manifest,
        "whole": whole,
        "focus": focus,
        "docs": docs.iter().map(|(p, t)| json!({ "path": p, "text": t })).collect::<Vec<_>>(),
    })
}

/// Runs a [`request`] in this process: the answer [`answer`] reads.
pub fn child(request: &Json) -> Json {
    let manifest = PathBuf::from(request["manifest"].as_str().unwrap_or_default());
    let whole = request["whole"].as_bool().unwrap_or(false);
    let mut open = HashSet::new();
    for doc in request["docs"].as_array().into_iter().flatten() {
        let (Some(path), Some(text)) = (doc["path"].as_str(), doc["text"].as_str()) else {
            continue;
        };
        let path = PathBuf::from(path);
        wowlua_driver::overlay::set(&path, text.as_bytes().to_vec());
        open.insert(text::canonical(&path));
    }
    let focus: Option<HashSet<PathBuf>> = request["focus"].as_array().map(|f| {
        f.iter()
            .filter_map(Json::as_str)
            .map(PathBuf::from)
            .collect()
    });
    // (this process ends after answering: its memory is not freed first)
    let packages = check_and(&manifest, whole, &open, focus.as_ref(), std::mem::forget);
    Json::Array(packages.iter().map(Checked::to_json).collect())
}

/// What a child's [`child`] wrote, read back.
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
        })
    }
}
