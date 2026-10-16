//! Snapshots (spec Tooling §16.1.3, decision T218): `toMatchSnapshot`
//! compares a value's text with the one stored in
//! `__snapshots__/<test file>.snap` next to the test file. A missing one is
//! written; `wowlua test -u` rewrites them (and drops those no test
//! matched, when every test of the file ran).
//!
//! The file: each snapshot is a `# <key>` line, then its text, each line
//! after `| ` (an empty line is `|`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

#[derive(Default)]
struct File {
    path: PathBuf,
    /// Key → text, in the file's order (then new ones).
    entries: Vec<(String, String)>,
    touched: HashSet<String>,
    changed: bool,
    update: bool,
    written: usize,
    updated: usize,
}

thread_local! {
    static FILE: RefCell<File> = RefCell::default();
}

/// What a test file's run did to its snapshots.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub written: usize,
    pub updated: usize,
    pub removed: usize,
}

pub fn path_for(test_file: &Path) -> PathBuf {
    let name = test_file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    test_file
        .parent()
        .unwrap_or(Path::new(""))
        .join("__snapshots__")
        .join(format!("{name}.snap"))
}

fn parse(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in text.lines() {
        if let Some(key) = line.strip_prefix("# ") {
            if let Some((k, lines)) = current.take() {
                out.push((k, lines.join("\n")));
            }
            current = Some((key.to_string(), Vec::new()));
        } else if let Some(rest) = line.strip_prefix('|')
            && let Some((_, lines)) = current.as_mut()
        {
            lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }
    if let Some((k, lines)) = current {
        out.push((k, lines.join("\n")));
    }
    out
}

fn render(entries: &[(String, String)]) -> String {
    let mut out = String::from("-- wowlua snapshots (wowlua test -u updates them)\n");
    for (k, text) in entries {
        out.push_str(&format!("\n# {k}\n"));
        for line in text.split('\n') {
            if line.is_empty() {
                out.push_str("|\n");
            } else {
                out.push_str(&format!("| {line}\n"));
            }
        }
    }
    out
}

/// Starts a test file's snapshots.
pub fn begin(test_file: &Path, update: bool) {
    let path = path_for(test_file);
    let entries = std::fs::read_to_string(&path)
        .map(|t| parse(&t))
        .unwrap_or_default();
    FILE.with(|f| {
        *f.borrow_mut() = File {
            path,
            entries,
            update,
            ..File::default()
        }
    });
}

/// Writes what changed; `complete`: every test of the file ran (with
/// `-u`, snapshots no test matched are dropped).
pub fn finish(complete: bool) -> Result<Summary, String> {
    FILE.with(|f| {
        let mut f = f.borrow_mut();
        let mut summary = Summary {
            written: f.written,
            updated: f.updated,
            removed: 0,
        };
        if f.update && complete {
            let before = f.entries.len();
            let touched = f.touched.clone();
            f.entries.retain(|(k, _)| touched.contains(k));
            summary.removed = before - f.entries.len();
            f.changed |= summary.removed > 0;
        }
        if !f.changed {
            return Ok(summary);
        }
        if f.entries.is_empty() {
            let _ = std::fs::remove_file(&f.path);
            return Ok(summary);
        }
        if let Some(dir) = f.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&f.path, render(&f.entries)).map_err(|e| e.to_string())?;
        Ok(summary)
    })
}

/// A line diff of `old` and `new` (`-` stored, `+` now).
fn diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.split('\n').collect();
    let b: Vec<&str> = new.split('\n').collect();
    // Longest common subsequence, then walk it.
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            out.push(format!("+ {}", b[j]));
            j += 1;
        } else {
            out.push(format!("- {}", a[i]));
            i += 1;
        }
    }
    out.join("\n")
}

/// `snapshot(key, text)`: `true`, or `false` and the diff.
pub fn snapshot_native(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = match c.arg(1) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return Err(vm.error("snapshot: a key expected")),
    };
    let text = match c.arg(2) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return Err(vm.error("snapshot: a text expected")),
    };
    let result = FILE.with(|f| {
        let mut f = f.borrow_mut();
        if !f.touched.insert(key.clone()) {
            return Err(format!("two snapshots are named \"{key}\""));
        }
        let at = f.entries.iter().position(|(k, _)| *k == key);
        match at {
            Some(i) if f.entries[i].1 == text => Ok(None),
            Some(i) if f.update => {
                f.entries[i].1 = text.clone();
                f.changed = true;
                f.updated += 1;
                Ok(None)
            }
            Some(i) => Ok(Some(diff(&f.entries[i].1, &text))),
            None => {
                f.entries.push((key.clone(), text.clone()));
                f.changed = true;
                f.written += 1;
                Ok(None)
            }
        }
    });
    match result {
        Ok(None) => Ok(vec![Value::Bool(true)]),
        Ok(Some(d)) => Ok(vec![Value::Bool(false), vm.str(&d)]),
        Err(e) => Err(vm.error(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip_and_diff() {
        let entries = vec![
            ("a › 1".to_string(), "{\n  x = 1,\n\n}".to_string()),
            ("b".to_string(), "\"text\"".to_string()),
        ];
        assert_eq!(parse(&render(&entries)), entries);
        assert_eq!(diff("a\nb\nc", "a\nc\nd"), "  a\n- b\n  c\n+ d");
    }
}
