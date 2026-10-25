//! Applying machine-applicable fixes (`wowlua fix`, `wowlua lint --fix`).

use std::collections::BTreeMap;
use std::path::Path;
use wowlua_diagnostics::{Diagnostic, Edit, Fix};
use wowlua_span::{FileId, SourceDb};

/// The result of applying fixes.
#[derive(Debug, Default)]
pub struct Applied {
    pub fixes: usize,
    pub files: Vec<std::path::PathBuf>,
}

/// Applies the first fix of each diagnostic to the files under `root`.
/// A fix whose edits overlap an earlier one is left for the next round.
pub fn apply(db: &SourceDb, diagnostics: &[&Diagnostic], root: &Path) -> std::io::Result<Applied> {
    let mut per_file: BTreeMap<FileId, Vec<&Fix>> = BTreeMap::new();
    for d in diagnostics {
        let Some(fix) = d.fixes.first() else { continue };
        let Some(file) = fix.edits.first().map(|e| e.span.file) else {
            continue;
        };
        if fix.edits.iter().all(|e| e.span.file == file) {
            per_file.entry(file).or_default().push(fix);
        }
    }
    let mut out = Applied::default();
    for (file, fixes) in per_file {
        let path = root.join(db.get(file).path());
        if !path.starts_with(root) || !crate::host::is_file(&path) {
            continue;
        }
        let mut taken: Vec<&Edit> = Vec::new();
        let mut count = 0;
        for fix in fixes {
            let overlaps = fix.edits.iter().any(|e| {
                taken.iter().any(|t| {
                    e.span.start < t.span.end && t.span.start < e.span.end
                        || e.span.start == t.span.start
                })
            });
            if !overlaps {
                taken.extend(&fix.edits);
                count += 1;
            }
        }
        if taken.is_empty() {
            continue;
        }
        taken.sort_by_key(|e| std::cmp::Reverse(e.span.start));
        // Spans are byte offsets into the file as read.
        let mut bytes = crate::host::read(&path)?;
        for e in taken {
            let range = e.span.start as usize..e.span.end as usize;
            if range.end > bytes.len() {
                continue;
            }
            bytes.splice(range, e.replacement.bytes());
        }
        crate::host::write(&path, bytes)?;
        out.fixes += count;
        out.files.push(path);
    }
    Ok(out)
}
