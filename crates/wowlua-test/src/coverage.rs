//! Coverage of a test run (spec Tooling §16.1.6, decision T188): the VM's
//! counts of the addon's own files, mapped to their sources (the `test`
//! profile keeps lines), merged over test files and flavors.
//!
//! From the compiled code: a line is covered when an instruction on it
//! ran; a branch is one outcome of a conditional jump (each arm of an `if`,
//! a loop entered or skipped, both outcomes of `and`, `or`, `??`, `?.`); a
//! condition is a branch on a line with several conditional jumps (a
//! compound test); a function is covered when it was called. Statements
//! count as lines. `-- wowlua: coverage ignore` leaves out the next line
//! with code, `-- wowlua: coverage ignore-file` the file.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use wowlua_vm::coverage::{Coverage, is_conditional};
use wowlua_vm::proto::{Op, Proto, Rk};
use wowlua_vm::{Value, Vm};

/// The conditional jumps of the compiler's runtime checks (`if not (…)
/// then __wl.check.fail(…)`, `castFail`): not the source's branches.
fn generated_checks(vm: &Vm, proto: &Proto) -> HashSet<usize> {
    let key = |op: &Op| -> Option<&[u8]> {
        match op {
            Op::GetTable { c: Rk::K(k), .. } => match proto.constants.get(*k as usize) {
                Some(Value::Str(s)) => Some(vm.str_bytes(*s)),
                _ => None,
            },
            _ => None,
        }
    };
    let mut out = HashSet::new();
    for (p, op) in proto.code.iter().enumerate() {
        let fails = matches!(key(op), Some(b"fail" | b"castFail"));
        let from_check = p > 0 && key(&proto.code[p - 1]) == Some(b"check");
        if !(fails && from_check) {
            continue;
        }
        let line = proto.lines.get(p).copied();
        for q in p.saturating_sub(24)..p {
            if proto.lines.get(q).copied() == line && is_conditional(&proto.code[q]) {
                out.insert(q);
            }
        }
    }
    out
}

/// One source file's counts.
#[derive(Clone, Debug, Default)]
pub struct FileCoverage {
    /// Line → times it ran (lines with code only).
    pub lines: BTreeMap<u32, u64>,
    /// (line, ordinal on the line) → [jumped, fell through].
    pub branches: BTreeMap<(u32, u32), [u64; 2]>,
    /// (line, ordinal) → (name, times called).
    pub functions: BTreeMap<(u32, u32), (String, u64)>,
}

impl FileCoverage {
    fn merge(&mut self, other: &FileCoverage) {
        for (l, h) in &other.lines {
            *self.lines.entry(*l).or_default() += h;
        }
        for (k, b) in &other.branches {
            let e = self.branches.entry(*k).or_default();
            e[0] += b[0];
            e[1] += b[1];
        }
        for (k, (name, h)) in &other.functions {
            let e = self
                .functions
                .entry(*k)
                .or_insert_with(|| (name.clone(), 0));
            e.1 += h;
        }
    }

    /// Lines with several conditional jumps: their branches are conditions.
    fn compound_lines(&self) -> BTreeSet<u32> {
        let mut per_line: BTreeMap<u32, u32> = BTreeMap::new();
        for (line, _) in self.branches.keys() {
            *per_line.entry(*line).or_default() += 1;
        }
        per_line
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(l, _)| l)
            .collect()
    }
}

/// Covered and total, per kind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub lines: (u64, u64),
    pub branches: (u64, u64),
    pub functions: (u64, u64),
    pub conditions: (u64, u64),
}

impl Totals {
    fn add(&mut self, other: &Totals) {
        for (a, b) in [
            (&mut self.lines, other.lines),
            (&mut self.branches, other.branches),
            (&mut self.functions, other.functions),
            (&mut self.conditions, other.conditions),
        ] {
            a.0 += b.0;
            a.1 += b.1;
        }
    }
}

pub fn percent((covered, total): (u64, u64)) -> f64 {
    if total == 0 {
        100.0
    } else {
        covered as f64 * 100.0 / total as f64
    }
}

/// A run's coverage: source path → counts.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub files: BTreeMap<String, FileCoverage>,
}

impl Report {
    pub fn merge(&mut self, other: &Report) {
        for (path, f) in &other.files {
            self.files.entry(path.clone()).or_default().merge(f);
        }
    }

    pub fn file_totals(f: &FileCoverage) -> Totals {
        let compound = f.compound_lines();
        let mut t = Totals {
            lines: (
                f.lines.values().filter(|h| **h > 0).count() as u64,
                f.lines.len() as u64,
            ),
            functions: (
                f.functions.values().filter(|(_, h)| *h > 0).count() as u64,
                f.functions.len() as u64,
            ),
            ..Totals::default()
        };
        for ((line, _), b) in &f.branches {
            let covered = b.iter().filter(|n| **n > 0).count() as u64;
            t.branches.0 += covered;
            t.branches.1 += 2;
            if compound.contains(line) {
                t.conditions.0 += covered;
                t.conditions.1 += 2;
            }
        }
        t
    }

    pub fn totals(&self) -> Totals {
        let mut t = Totals::default();
        for f in self.files.values() {
            t.add(&Report::file_totals(f));
        }
        t
    }
}

/// A function's name from its first line (`function a.b:c(`, `local
/// function f(`), else where it is.
fn function_name(source: &[&str], line: u32) -> String {
    let text = source
        .get(line.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("");
    let re = regex::Regex::new(r"function\s+([\w.:]+)\s*[<(]").expect("a valid pattern");
    match re.captures(text) {
        Some(c) => c[1].to_string(),
        None => format!("(anonymous:{line})"),
    }
}

/// Lines `-- wowlua: coverage ignore` leaves out, or `None` for an ignored
/// file.
fn ignored(source: &[&str]) -> Option<BTreeSet<u32>> {
    let mut out = BTreeSet::new();
    let mut pending = false;
    for (i, line) in source.iter().enumerate() {
        let t = line.trim();
        if t.contains("wowlua: coverage ignore-file") {
            return None;
        }
        if t.contains("wowlua: coverage ignore") {
            if t.starts_with("--") {
                pending = true;
            } else {
                out.insert(i as u32 + 1);
            }
            continue;
        }
        if pending && !t.is_empty() && !t.starts_with("--") {
            out.insert(i as u32 + 1);
            pending = false;
        }
    }
    Some(out)
}

/// The `RETURN` Lua's compiler puts after a tail call: it never runs (the
/// called function returns for it), and on a multi-line `return f(…)` it
/// sits alone on the closing line.
fn after_tail_call(proto: &Proto, pc: usize) -> bool {
    pc > 0
        && matches!(proto.code.get(pc), Some(Op::Return { .. }))
        && matches!(proto.code.get(pc - 1), Some(Op::TailCall { .. }))
}

/// A source's lines and the lines coverage ignores.
type Source = (Vec<String>, BTreeSet<u32>);

/// The counts of one VM, by source file: `source_of` maps a chunk name
/// (`src/x.lua`) to the source (`src/x.wowlua`, `None`: not counted).
pub fn collect(
    vm: &Vm,
    cov: &Coverage,
    root: &Path,
    source_of: &dyn Fn(&str) -> Option<String>,
) -> Report {
    let mut report = Report::default();
    // Each source's lines and ignored lines (`None`: an ignored file).
    let mut sources: BTreeMap<String, Option<Source>> = BTreeMap::new();
    for counts in &cov.protos {
        let chunk = String::from_utf8_lossy(&counts.proto.source).into_owned();
        let chunk = chunk.strip_prefix('@').unwrap_or(&chunk).to_string();
        let Some(path) = source_of(&chunk) else {
            continue;
        };
        let entry = sources.entry(path.clone()).or_insert_with(|| {
            let text = wowlua_host::read_to_string(root.join(&path)).unwrap_or_default();
            let lines: Vec<String> = text.lines().map(str::to_string).collect();
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            ignored(&refs).map(|ig| (lines, ig))
        });
        let Some((text, ignore)) = entry else {
            continue;
        };
        let refs: Vec<&str> = text.iter().map(String::as_str).collect();
        let file = report.files.entry(path).or_default();
        let proto = &counts.proto;
        for (pc, &line) in proto.lines.iter().enumerate() {
            if line == 0 || ignore.contains(&line) || after_tail_call(proto, pc) {
                continue;
            }
            let e = file.lines.entry(line).or_default();
            *e = (*e).max(u64::from(counts.hits.get(pc).copied().unwrap_or(0)));
        }
        let mut ordinal: BTreeMap<u32, u32> = BTreeMap::new();
        let generated = generated_checks(vm, proto);
        for (pc, op) in proto.code.iter().enumerate() {
            if !is_conditional(op) || generated.contains(&pc) {
                continue;
            }
            let line = proto.lines.get(pc).copied().unwrap_or(0);
            if line == 0 || ignore.contains(&line) {
                continue;
            }
            let n = ordinal.entry(line).or_default();
            let key = (line, *n);
            *n += 1;
            let b = counts.branches.get(&pc).copied().unwrap_or([0, 0]);
            let e = file.branches.entry(key).or_default();
            e[0] += u64::from(b[0]);
            e[1] += u64::from(b[1]);
        }
        // Functions of the source: generated ones (a class's field
        // defaults) have no `function` on their line.
        let written = refs
            .get(proto.line_defined.saturating_sub(1) as usize)
            .is_some_and(|l| l.contains("function"));
        if !counts.main && written && !ignore.contains(&proto.line_defined) {
            let line = proto.line_defined;
            let n = file.functions.keys().filter(|(l, _)| *l == line).count() as u32;
            let called = u64::from(counts.hits.first().copied().unwrap_or(0));
            file.functions
                .insert((line, n), (function_name(&refs, line), called));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_comments() {
        let src = [
            "local a = 1",
            "-- wowlua: coverage ignore",
            "",
            "print(a)",
            "x() -- wowlua: coverage ignore",
        ];
        assert_eq!(ignored(&src), Some([4, 5].into_iter().collect()));
        assert_eq!(ignored(&["-- wowlua: coverage ignore-file"]), None);
        assert_eq!(function_name(&["local function f(a)"], 1), "f");
        assert_eq!(function_name(&["function M.a:b<T>(x)"], 1), "M.a:b");
    }
}
