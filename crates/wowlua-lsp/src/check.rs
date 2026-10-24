//! A package checked as `wowlua check` checks it (the test side too), with
//! the editor's unsaved texts over the disk's (`wowlua_driver::overlay`):
//! its diagnostics by file, as the protocol has them, and what each name
//! refers to (go to definition).

use crate::text;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::{Diagnostic, Severity};
use wowlua_driver::Session;
use wowlua_driver::build::BuildRequest;
use wowlua_driver::project::LoadOptions;

/// A name of a file, by its bytes, and where what it refers to is (the
/// protocol's `Location`).
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub start: u32,
    pub end: u32,
    pub target: Json,
}

/// What a check found, by file (canonical absolute paths).
#[derive(Default)]
pub struct Checked {
    /// Diagnostics; one about the package as a whole goes to its manifest.
    pub diagnostics: BTreeMap<PathBuf, Vec<Json>>,
    /// Each file's names that refer to something, by start.
    pub definitions: HashMap<PathBuf, Vec<Definition>>,
}

/// Checks the package whose manifest is `manifest`.
pub fn check(manifest: &Path) -> Checked {
    let dir = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut session = Session::new();
    session.definitions = Some(Vec::new());
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
    if let Some(projects) = wowlua_driver::load_projects(&mut session, &options) {
        for project in projects {
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
        }
    }
    let mut out = Checked::default();
    for d in session.diagnostics.iter() {
        if let Some((path, item)) = convert(&session, &roots, manifest, d) {
            out.diagnostics.entry(path).or_default().push(item);
        }
    }
    out.definitions = definitions(&session, &roots);
    out
}

/// The session's (name, definition) spans as each file's [`Definition`]s.
fn definitions(session: &Session, roots: &[PathBuf]) -> HashMap<PathBuf, Vec<Definition>> {
    let mut out: HashMap<PathBuf, Vec<Definition>> = HashMap::new();
    let Some(found) = session.definitions.as_ref() else {
        return out;
    };
    // Each file's path (or none) and line index, once: a file on disk for
    // names, also one written for the compiler's own as a target.
    let mut paths: HashMap<wowlua_span::FileId, Option<PathBuf>> = HashMap::new();
    let mut targets: HashMap<wowlua_span::FileId, Option<PathBuf>> = HashMap::new();
    let mut lines: HashMap<wowlua_span::FileId, text::Lines<'_>> = HashMap::new();
    for (at, to) in found {
        let Some(path) = paths
            .entry(at.file)
            .or_insert_with(|| absolute(roots, session.db.get(at.file).path()))
            .clone()
        else {
            continue;
        };
        let target_path = targets
            .entry(to.file)
            .or_insert_with(|| {
                let file = session.db.get(to.file);
                absolute(roots, file.path()).or_else(|| materialize(file.path(), file.text()))
            })
            .clone();
        let Some(target_path) = target_path else {
            continue;
        };
        let range = lines
            .entry(to.file)
            .or_insert_with(|| text::Lines::new(session.db.get(to.file).text()))
            .range(to.start as usize, to.end as usize);
        out.entry(path).or_default().push(Definition {
            start: at.start,
            end: at.end,
            target: json!({ "uri": text::uri_of(&target_path), "range": range }),
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
    Some(std::fs::canonicalize(&found).unwrap_or(found))
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
