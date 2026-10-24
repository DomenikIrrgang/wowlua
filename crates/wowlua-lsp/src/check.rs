//! A package checked as `wowlua check` checks it (the test side too), with
//! the editor's unsaved texts over the disk's (`wowlua_driver::overlay`);
//! its diagnostics by file, as the protocol has them.

use crate::text;
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wowlua_diagnostics::{Diagnostic, Severity};
use wowlua_driver::Session;
use wowlua_driver::build::BuildRequest;
use wowlua_driver::project::LoadOptions;

/// The diagnostics of the package whose manifest is `manifest`, by file
/// (absolute paths); a diagnostic about the package as a whole goes to
/// its manifest.
pub fn check(manifest: &Path) -> BTreeMap<PathBuf, Vec<Json>> {
    let dir = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut session = Session::new();
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
    let mut out: BTreeMap<PathBuf, Vec<Json>> = BTreeMap::new();
    for d in session.diagnostics.iter() {
        if let Some((path, item)) = convert(&session, &roots, manifest, d) {
            out.entry(path).or_default().push(item);
        }
    }
    out
}

/// A file of the session as an absolute path, if it is one on disk.
fn absolute(roots: &[PathBuf], rel: &Path) -> Option<PathBuf> {
    if rel.is_absolute() {
        return rel.exists().then(|| rel.to_path_buf());
    }
    roots.iter().map(|r| r.join(rel)).find(|p| p.exists())
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
