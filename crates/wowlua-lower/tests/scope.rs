//! Scope resolution: `resolve_with_conflicts` gives what the resolver gave
//! before it recorded each pair of locals once, and `resolve` the same
//! without the conflicts.

use std::collections::BTreeSet;
use wowlua_luair::ir::{Chunk, Ident};
use wowlua_luair::scope::{
    Binding, FunctionInfo, Ref, Resolution, Use, Visitor, resolve, resolve_with_conflicts, walk,
};
use wowlua_span::SourceDb;

/// The resolver as it was, recording conflicts on every use.
#[derive(Default)]
struct Old {
    res: Resolution,
    stack: Vec<usize>,
    blocks: Vec<usize>,
    functions: Vec<usize>,
    active: Vec<usize>,
}

impl Old {
    fn declare(&mut self, name: &str, implicit: bool) -> usize {
        let id = self.res.bindings.len();
        let function = *self.functions.last().expect("inside a function");
        self.res.bindings.push(Binding {
            name: name.to_string(),
            implicit,
            function,
            uses: 0,
            writes: 0,
        });
        self.res.conflicts.push(BTreeSet::new());
        self.stack.push(id);
        if let Some(a) = self.active.last_mut() {
            *a += 1;
            let info = &mut self.res.functions[function];
            info.max_locals = info.max_locals.max(*a);
        }
        id
    }
}

impl Visitor for Old {
    fn ident(&mut self, id: &mut Ident, use_: Use) {
        if use_ == Use::Declare {
            let b = self.declare(&id.name, false);
            self.res.occurrences.push(Ref::Local(b));
            return;
        }
        let found = self
            .stack
            .iter()
            .rposition(|&b| self.res.bindings[b].name == id.name);
        let Some(index) = found else {
            self.res.globals.insert(id.name.clone());
            self.res.occurrences.push(Ref::Global);
            return;
        };
        let b = self.stack[index];
        let binding = &mut self.res.bindings[b];
        binding.uses += 1;
        if use_ == Use::Write {
            binding.writes += 1;
        }
        let owner = binding.function;
        for &f in self.functions.iter().rev() {
            if f == owner {
                break;
            }
            self.res.functions[f].upvalues.insert(b);
        }
        for &later in &self.stack[index + 1..] {
            self.res.conflicts[b].insert(later);
            self.res.conflicts[later].insert(b);
        }
        self.res.occurrences.push(Ref::Local(b));
    }

    fn implicit_self(&mut self) {
        self.declare("self", true);
    }

    fn enter_block(&mut self) {
        self.blocks.push(self.stack.len());
    }

    fn leave_block(&mut self) {
        let len = self.blocks.pop().expect("balanced blocks");
        let left = self.stack.len() - len;
        self.stack.truncate(len);
        if let Some(a) = self.active.last_mut() {
            *a -= left;
        }
    }

    fn enter_function(&mut self) {
        let id = self.res.functions.len();
        self.res.functions.push(FunctionInfo {
            parent: self.functions.last().copied(),
            ..FunctionInfo::default()
        });
        self.functions.push(id);
        self.active.push(0);
    }

    fn leave_function(&mut self) {
        self.functions.pop();
        self.active.pop();
    }
}

fn lowered(src: &[u8]) -> Option<Chunk> {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", String::from_utf8_lossy(src).into_owned());
    let parsed = wowlua_syntax::parse(src, file);
    if parsed.has_errors() {
        return None;
    }
    wowlua_lower::lower(&parsed, src).ok()
}

#[track_caller]
fn same(src: &[u8]) {
    let Some(mut chunk) = lowered(src) else {
        return;
    };
    let mut old = Old::default();
    walk(&mut chunk, &mut old);
    let old = old.res;
    let new = resolve_with_conflicts(&mut chunk);
    assert_eq!(format!("{new:?}"), format!("{old:?}"));
    let without = resolve(&mut chunk);
    assert!(without.conflicts.is_empty());
    assert_eq!(
        format!(
            "{:?}",
            (
                &without.bindings,
                &without.occurrences,
                &without.globals,
                &without.functions
            )
        ),
        format!(
            "{:?}",
            (
                &old.bindings,
                &old.occurrences,
                &old.globals,
                &old.functions
            )
        )
    );
}

#[test]
fn conflicts_are_the_same() {
    for src in [
        "local a = 1 local b = 2 print(a, b) print(a) local c = 3 print(a, c) print(b)",
        "local a do local b = 1 print(a) local c = 2 print(a, b) end local d print(a, d)",
        "local function f(x) local y = x return function(z) return x + y + z end end print(f)",
        "for i = 1, 10 do local a = i for j = 1, i do print(a, i, j) end print(a) end",
        "local t = {} function t:m(a) local b = self print(a, b, self) end G = t",
        "local a while a do local b = a repeat local c = b print(a) until c end",
        "local a, b, c = 1, 2, 3 local a = a + b print(a, c) x, y = a, b",
        "local function rec(n) if n > 0 then return rec(n - 1) end return n end print(rec(3))",
    ] {
        same(src.as_bytes());
    }
}

/// A small deterministic generator (xorshift), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Random nestings of blocks, functions, declarations and uses.
#[test]
fn random_scopes_have_the_same_conflicts() {
    const NAMES: &[&str] = &["a", "b", "c", "d", "e"];
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for _ in 0..2000 {
        let mut src = String::new();
        let mut open = Vec::new();
        for _ in 0..rng.next() % 40 {
            let name = NAMES[(rng.next() % 5) as usize];
            match rng.next() % 9 {
                0 | 1 => src.push_str(&format!("local {name} = 1 ")),
                2 | 3 => src.push_str(&format!("print({name}) ")),
                4 => src.push_str(&format!("{name} = {name} ")),
                5 => {
                    src.push_str("do ");
                    open.push("end ");
                }
                6 => {
                    src.push_str(&format!("local function f({name}) "));
                    open.push("end ");
                }
                7 => {
                    src.push_str(&format!("for {name} = 1, 2 do "));
                    open.push("end ");
                }
                _ => {
                    if let Some(close) = open.pop() {
                        src.push_str(close);
                    }
                }
            }
        }
        while let Some(close) = open.pop() {
            src.push_str(close);
        }
        same(src.as_bytes());
    }
}

/// Every `.lua` file below `$WOWLUA_CORPUS` (run with `cargo test --release
/// -p wowlua-lower --test scope corpus -- --ignored`).
#[test]
#[ignore]
fn corpus() {
    let Ok(root) = std::env::var("WOWLUA_CORPUS") else {
        return;
    };
    let mut stack = vec![std::path::PathBuf::from(root)];
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "lua") {
                same(&std::fs::read(&path).unwrap());
                files += 1;
            }
        }
    }
    eprintln!("{files} files resolve the same");
}
