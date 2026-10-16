//! `cargo xtask parse-stats <dir>` (#19, phase 0): parses every `.lua` and
//! `.wowlua` file below a directory, keeps every tree alive, and reports what
//! the trees hold: tokens, trivia and nodes by kind, list lengths, names,
//! numerals and strings, and the heap they take, by category (from the
//! types' sizes and every list's capacity) and in total (from a counting
//! allocator, which also counts the allocations).

use anyhow::{Result, bail};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use wowlua_syntax::ast::*;

/// Counts live bytes and allocations; everything else is the system's.
pub struct Counting;

static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call forwards to `System` with the same arguments; the
// counters are plain atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let live =
                LIVE.fetch_add(layout.size() as u64, Ordering::Relaxed) + layout.size() as u64;
            PEAK.fetch_max(live, Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            let live = LIVE.fetch_add(new_size as u64, Ordering::Relaxed) + new_size as u64
                - layout.size() as u64;
            LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
            PEAK.fetch_max(live, Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
}

#[derive(Default)]
struct Stats {
    /// Node kinds and other counts.
    count: BTreeMap<&'static str, u64>,
    /// Heap bytes held, by category (requested sizes, not allocator classes).
    bytes: BTreeMap<&'static str, u64>,
    /// Heap blocks held, by category.
    blocks: BTreeMap<&'static str, u64>,
    /// List lengths: (list kind, bucket) → count.
    lens: BTreeMap<(&'static str, &'static str), u64>,
}

fn bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3..=4 => "3-4",
        5..=8 => "5-8",
        9..=16 => "9-16",
        17..=64 => "17-64",
        65..=1024 => "65-1024",
        _ => ">1024",
    }
}

impl Stats {
    fn n(&mut self, what: &'static str) {
        *self.count.entry(what).or_default() += 1;
    }
    fn heap(&mut self, what: &'static str, bytes: usize) {
        if bytes > 0 {
            *self.bytes.entry(what).or_default() += bytes as u64;
            *self.blocks.entry(what).or_default() += 1;
        }
    }
    fn vec<T>(&mut self, what: &'static str, v: &Vec<T>) {
        self.heap(what, v.capacity() * std::mem::size_of::<T>());
    }
    fn boxed<T>(&mut self, what: &'static str, _: &T) {
        self.heap(what, std::mem::size_of::<T>());
    }
    fn len(&mut self, what: &'static str, n: usize) {
        *self.lens.entry((what, bucket(n))).or_default() += 1;
    }

    fn name(&mut self, n: &Name) {
        self.n("name");
        self.heap("name text", n.text.len());
    }
    fn string(&mut self, s: &StringLit) {
        self.n("string");
        self.heap("string raw", s.raw.len());
        self.heap("string value", s.value.len());
        let inner = (!s.long && s.raw.len() >= 2).then(|| &s.raw[1..s.raw.len() - 1]);
        if inner == Some(&s.value[..]) {
            self.n("string without escapes");
        }
    }
    fn number(&mut self, n: &NumberLit) {
        self.n("numeral");
        if let NumText::Heap(t) = &n.raw {
            self.heap("numeral text", t.len());
        }
    }

    fn block(&mut self, b: &Block) {
        self.n("block");
        self.vec("statement lists", &b.stmts);
        self.len("block statements", b.stmts.len());
        for s in &b.stmts {
            self.stmt(s);
        }
    }

    fn decorators(&mut self, d: &Vec<Decorator>) {
        self.vec("decorator lists", d);
        for d in d {
            self.n("decorator");
            self.vec("name lists", &d.path);
            for n in &d.path {
                self.name(n);
            }
            if let Some(a) = &d.args {
                self.args(a);
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        self.decorators(&s.decorators);
        match &s.kind {
            StmtKind::Assign { targets, values } => {
                self.n("stmt Assign");
                self.vec("expression lists", targets);
                self.vec("expression lists", values);
                targets.iter().chain(values).for_each(|e| self.expr(e));
            }
            StmtKind::Compound { target, value, .. } => {
                self.n("stmt Compound");
                self.boxed("expression boxes", &**target);
                self.boxed("expression boxes", &**value);
                self.expr(target);
                self.expr(value);
            }
            StmtKind::Call(e) => {
                self.n("stmt Call");
                self.expr(e);
            }
            StmtKind::Do(b) => {
                self.n("stmt Do");
                self.block(b);
            }
            StmtKind::While { cond, body } | StmtKind::Repeat { body, cond } => {
                self.n("stmt While/Repeat");
                self.boxed("expression boxes", &**cond);
                self.expr(cond);
                self.block(body);
            }
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                self.n("stmt If");
                self.boxed("expression boxes", &**cond);
                self.expr(cond);
                self.block(then_block);
                self.vec("elseif lists", elseifs);
                for (c, b) in elseifs {
                    self.expr(c);
                    self.block(b);
                }
                if let Some(b) = else_block {
                    self.boxed("block boxes", &**b);
                    self.block(b);
                }
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => {
                self.n("stmt NumericFor");
                self.boxed("other boxes", &**var);
                self.typed_name(var);
                for e in [Some(start), Some(limit), step.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    self.boxed("expression boxes", &**e);
                    self.expr(e);
                }
                self.block(body);
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                self.n("stmt GenericFor");
                self.vec("other lists", vars);
                vars.iter().for_each(|v| self.typed_name(v));
                self.vec("expression lists", exprs);
                exprs.iter().for_each(|e| self.expr(e));
                self.block(body);
            }
            StmtKind::Function { name, func } => {
                self.n("stmt Function");
                self.vec("name lists", &name.path);
                name.path.iter().for_each(|n| self.name(n));
                if let Some(m) = &name.method {
                    self.name(m);
                }
                self.func(func);
            }
            StmtKind::LocalFunction { name, func } => {
                self.n("stmt LocalFunction");
                self.name(name);
                self.func(func);
            }
            StmtKind::Local { names, values } => {
                self.n("stmt Local");
                self.vec("other lists", names);
                names.iter().for_each(|v| self.typed_name(v));
                self.vec("expression lists", values);
                values.iter().for_each(|e| self.expr(e));
            }
            StmtKind::LocalDestructure { fields, ty, value } => {
                self.n("stmt LocalDestructure");
                self.vec("other lists", fields);
                for f in fields {
                    self.name(&f.name);
                    if let Some(d) = &f.default {
                        self.expr(d);
                    }
                }
                if let Some(t) = ty {
                    self.boxed("type boxes", &**t);
                    self.ty(t);
                }
                if let Some(v) = value {
                    self.boxed("expression boxes", &**v);
                    self.expr(v);
                }
            }
            StmtKind::Const { name, ty, value } => {
                self.n("stmt Const");
                self.name(name);
                if let Some(t) = ty {
                    self.boxed("type boxes", &**t);
                    self.ty(t);
                }
                if let Some(v) = value {
                    self.boxed("expression boxes", &**v);
                    self.expr(v);
                }
            }
            StmtKind::Return(v) => {
                self.n("stmt Return");
                self.vec("expression lists", v);
                v.iter().for_each(|e| self.expr(e));
            }
            StmtKind::Break | StmtKind::Continue => self.n("stmt Break/Continue"),
            StmtKind::Class(c) => {
                self.n("stmt Class");
                self.boxed("declaration boxes", &**c);
                self.decl_rest("class");
            }
            StmtKind::Interface(i) => {
                self.n("stmt Interface");
                self.boxed("declaration boxes", &**i);
                self.decl_rest("interface");
            }
            StmtKind::Enum(e) => {
                self.n("stmt Enum");
                self.boxed("declaration boxes", &**e);
                self.decl_rest("enum");
            }
            StmtKind::TypeAlias(t) => {
                self.n("stmt TypeAlias");
                self.boxed("declaration boxes", &**t);
                self.name(&t.name);
                self.ty(&t.ty);
            }
            StmtKind::Declare(d) => {
                self.n("stmt Declare");
                self.boxed("declaration boxes", &**d);
                self.declare(d);
            }
            StmtKind::Import(i) => {
                self.n("stmt Import");
                self.boxed("declaration boxes", &**i);
                self.string(&i.from);
            }
            StmtKind::Export(s) => {
                self.n("stmt Export");
                self.boxed("statement boxes", &**s);
                self.stmt(s);
            }
            StmtKind::Error => self.n("stmt Error"),
        }
    }

    /// Declarations other than `declare` (rare outside `.wowlua` modules):
    /// counted, not walked.
    fn decl_rest(&mut self, _: &'static str) {
        self.n("declaration (not walked)");
    }

    fn declare(&mut self, d: &Declare) {
        match d {
            Declare::Global { name, ty } => {
                self.n("declare Global");
                self.name(name);
                self.ty(ty);
            }
            Declare::Function { name, sig } => {
                self.n("declare Function");
                self.vec("name lists", &name.path);
                name.path.iter().for_each(|n| self.name(n));
                if let Some(m) = &name.method {
                    self.name(m);
                }
                self.func(sig);
            }
            _ => self.n("declare other (not walked)"),
        }
    }

    fn typed_name(&mut self, t: &TypedName) {
        self.name(&t.name);
        if let Some(ty) = &t.ty {
            self.ty(ty);
        }
    }

    fn func(&mut self, f: &FuncBody) {
        self.n("function body");
        self.boxed("function bodies", f);
        self.vec("other lists", &f.type_params);
        self.vec("parameter lists", &f.params);
        self.len("parameters", f.params.len());
        for p in &f.params {
            self.n("parameter");
            self.decorators(&p.decorators);
            self.name(&p.name);
            if let Some(t) = &p.ty {
                self.ty(t);
            }
            if let Some(d) = &p.default {
                self.boxed("expression boxes", &**d);
                self.expr(d);
            }
        }
        if let Some(r) = &f.ret {
            self.ret(r);
        }
        self.block(&f.body);
    }

    fn ret(&mut self, r: &RetType) {
        match r {
            RetType::Single(t) => self.ty(t),
            RetType::List { types, .. } => {
                self.vec("type lists", types);
                types.iter().for_each(|t| self.ty(t));
            }
            RetType::Pack(n) => self.name(n),
            RetType::Predicate { name, ty } => {
                self.name(name);
                self.ty(ty);
            }
        }
    }

    fn ty(&mut self, t: &Type) {
        self.n("type");
        match &t.kind {
            TypeKind::Ref { path, args } => {
                self.vec("name lists", path);
                path.iter().for_each(|n| self.name(n));
                self.vec("type lists", args);
                for a in args {
                    if let TypeArg::Type(t) = a {
                        self.ty(t);
                    }
                }
            }
            TypeKind::String(s) => self.string(s),
            TypeKind::Number(n) => self.number(n),
            TypeKind::Union(ts) | TypeKind::Intersection(ts) => {
                self.vec("type lists", ts);
                ts.iter().for_each(|t| self.ty(t));
            }
            TypeKind::Optional(t) | TypeKind::Array(t) | TypeKind::Paren(t) => {
                self.boxed("type boxes", &**t);
                self.ty(t);
            }
            TypeKind::Table(fields) => {
                self.vec("type lists", fields);
                for f in fields {
                    match f {
                        TableTypeField::Named { name, ty, .. } => {
                            self.name(name);
                            self.ty(ty);
                        }
                        TableTypeField::NamedString { key, ty, .. } => {
                            self.string(key);
                            self.ty(ty);
                        }
                        TableTypeField::Indexer { key, value } => {
                            self.ty(key);
                            self.ty(value);
                        }
                        TableTypeField::Array(t) => self.ty(t),
                    }
                }
            }
            TypeKind::Function(f) => {
                self.boxed("type boxes", &**f);
                self.vec("type lists", &f.params);
                for p in &f.params {
                    if let Some(n) = &p.name {
                        self.name(n);
                    }
                    self.ty(&p.ty);
                }
                self.ret(&f.ret);
            }
            TypeKind::Typeof(e) => {
                self.boxed("expression boxes", &**e);
                self.expr(e);
            }
            _ => {}
        }
    }

    fn args(&mut self, a: &Args) {
        match a {
            Args::Parens { args, .. } => {
                self.n("call args ()");
                self.heap("argument lists", args.len() * std::mem::size_of::<Expr>());
                self.len("call arguments", args.len());
                args.iter().for_each(|e| self.expr(e));
            }
            Args::Table(e) => {
                self.n("call args {}");
                self.boxed("expression boxes", &**e);
                self.expr(e);
            }
            Args::String(e) => {
                self.n("call args string");
                self.boxed("expression boxes", &**e);
                self.expr(e);
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        self.n("expression");
        let obj = |s: &mut Self, what: &'static str, o: &Expr| {
            s.n(what);
            s.boxed("expression boxes", o);
            s.expr(o);
        };
        match &e.kind {
            ExprKind::Nil | ExprKind::True | ExprKind::False | ExprKind::Vararg => {
                self.n("expr literal (nil/bool/...)")
            }
            ExprKind::Number(n) => {
                self.n("expr Number");
                self.number(n);
            }
            ExprKind::String(s) => {
                self.n("expr String");
                self.string(s);
            }
            ExprKind::Function(f) => {
                self.n("expr Function");
                self.func(f);
            }
            ExprKind::Table(fields) => {
                self.n("expr Table");
                self.vec("table field lists", fields);
                self.len("table fields", fields.len());
                for f in fields {
                    match f {
                        TableField::Keyed { key, value, .. } => {
                            self.n("field Keyed");
                            self.boxed("expression boxes", &**key);
                            self.expr(key);
                            self.expr(value);
                        }
                        TableField::Named { name, value } => {
                            self.n("field Named");
                            self.name(name);
                            self.expr(value);
                        }
                        TableField::Positional(v) => {
                            self.n("field Positional");
                            self.expr(v);
                        }
                    }
                }
            }
            ExprKind::Interp(parts) => {
                self.n("expr Interp");
                self.vec("other lists", parts);
                for p in parts {
                    match p {
                        InterpPart::Text { raw, value, .. } => {
                            self.heap("string raw", raw.len());
                            self.heap("string value", value.len());
                        }
                        InterpPart::Expr(e) => self.expr(e),
                    }
                }
            }
            ExprKind::Name(n) => {
                self.n("expr Name");
                self.name(n);
            }
            ExprKind::Index { obj: o, key } | ExprKind::OptIndex { obj: o, key } => {
                obj(self, "expr Index", o);
                self.boxed("expression boxes", &**key);
                self.expr(key);
            }
            ExprKind::Field { obj: o, name } | ExprKind::OptField { obj: o, name } => {
                obj(self, "expr Field", o);
                self.name(name);
            }
            ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
                obj(self, "expr Call", callee);
                self.args(args);
            }
            ExprKind::Method { obj: o, name, args }
            | ExprKind::OptMethod { obj: o, name, args } => {
                obj(self, "expr Method", o);
                self.boxed("other boxes", &**name);
                self.name(name);
                self.boxed("other boxes", &**args);
                self.args(args);
            }
            ExprKind::Paren(o) => obj(self, "expr Paren", o),
            ExprKind::Binary { lhs, rhs, .. } => {
                obj(self, "expr Binary", lhs);
                self.boxed("expression boxes", &**rhs);
                self.expr(rhs);
            }
            ExprKind::Unary { operand, .. } => obj(self, "expr Unary", operand),
            ExprKind::Cast { expr, ty } => {
                obj(self, "expr Cast", expr);
                self.boxed("type boxes", &**ty);
                self.ty(ty);
            }
            ExprKind::Error => self.n("expr Error"),
        }
    }
}

pub fn run(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let mut files: Vec<_> = walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("lua" | "wowlua")
            ) && !p
                .components()
                .any(|c| matches!(c.as_os_str().to_str(), Some("dist" | "target")))
        })
        .collect();
    files.sort();
    let sources: Vec<Vec<u8>> = files.iter().map(std::fs::read).collect::<Result<_, _>>()?;
    let source_bytes: u64 = sources.iter().map(|s| s.len() as u64).sum();
    let mut db = wowlua_span::SourceDb::new();
    let ids: Vec<_> = files.iter().map(|p| db.add(p.as_path(), "")).collect();

    // Parse with a large stack (deep chains), keeping every tree.
    let (parses, live, allocs, peak, secs) = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let live0 = LIVE.load(Ordering::Relaxed);
            let allocs0 = ALLOCS.load(Ordering::Relaxed);
            PEAK.store(live0, Ordering::Relaxed);
            let started = std::time::Instant::now();
            let parses: Vec<wowlua_syntax::Parse> = sources
                .iter()
                .zip(&ids)
                .map(|(src, file)| wowlua_syntax::parse(src, *file))
                .collect();
            let secs = started.elapsed().as_secs_f64();
            let live = LIVE.load(Ordering::Relaxed) - live0;
            let allocs = ALLOCS.load(Ordering::Relaxed) - allocs0;
            let peak = PEAK.load(Ordering::Relaxed) - live0;
            (parses, live, allocs, peak, secs)
        })?
        .join()
        .expect("parsing");

    let mut st = Stats::default();
    let mut tokens: BTreeMap<String, u64> = BTreeMap::new();
    let mut trivia: BTreeMap<String, u64> = BTreeMap::new();
    let mut errors = 0;
    for p in &parses {
        errors += usize::from(p.has_errors());
        st.heap("tokens", p.lexed.tokens.heap_bytes());
        st.vec("trivia", &p.lexed.trivia);
        st.vec("line starts", &p.lexed.line_starts);
        st.vec("diagnostics", &p.diagnostics);
        for t in p.lexed.tokens.iter() {
            *tokens.entry(format!("{:?}", t.kind)).or_default() += 1;
        }
        for t in &p.lexed.trivia {
            *trivia.entry(format!("{:?}", t.kind)).or_default() += 1;
        }
        st.vec("directive lists", &p.chunk.directives);
    }
    let walked = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            for p in &parses {
                st.block(&p.chunk.block);
            }
            (st, parses)
        })?
        .join()
        .expect("walking");
    let (st, parses) = walked;

    let mb = |b: u64| b as f64 / 1_048_576.0;
    println!(
        "{} files, {:.1} MB of source, {} with errors; parsed in {:.2} s",
        parses.len(),
        mb(source_bytes),
        errors,
        secs
    );
    println!(
        "heap held by the trees (counting allocator): {:.1} MB in {} allocations ({:.1} bytes per source byte); peak while parsing {:.1} MB",
        mb(live),
        allocs,
        live as f64 / source_bytes.max(1) as f64,
        mb(peak)
    );
    println!(
        "\nSIZES (bytes): Token {}, Trivia {}, Stmt {}, Expr {}, TableField {}, Name {}, Type {}, FuncBody {}, NumberLit {}, StringLit {}",
        std::mem::size_of::<wowlua_syntax::token::Token>(),
        std::mem::size_of::<wowlua_syntax::token::Trivia>(),
        std::mem::size_of::<Stmt>(),
        std::mem::size_of::<Expr>(),
        std::mem::size_of::<TableField>(),
        std::mem::size_of::<Name>(),
        std::mem::size_of::<Type>(),
        std::mem::size_of::<FuncBody>(),
        std::mem::size_of::<NumberLit>(),
        std::mem::size_of::<StringLit>()
    );
    let total: u64 = st.bytes.values().sum();
    println!(
        "\nHEAP BY CATEGORY (requested bytes, from sizes and capacities): {:.1} MB",
        mb(total)
    );
    let mut cats: Vec<_> = st.bytes.iter().collect();
    cats.sort_by_key(|(_, b)| std::cmp::Reverse(**b));
    for (k, b) in cats {
        println!(
            "  {:>9.1} MB {:5.1}%  {:>11} blocks  {k}",
            mb(*b),
            100.0 * *b as f64 / total.max(1) as f64,
            st.blocks.get(k).copied().unwrap_or(0)
        );
    }
    println!("\nCOUNTS");
    for (k, v) in &st.count {
        println!("  {v:>12}  {k}");
    }
    println!("\nTOKENS BY KIND ({} in all)", tokens.values().sum::<u64>());
    let mut t: Vec<_> = tokens.into_iter().collect();
    t.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
    for (k, v) in t.iter().take(20) {
        println!("  {v:>12}  {k}");
    }
    println!("\nTRIVIA BY KIND");
    for (k, v) in &trivia {
        println!("  {v:>12}  {k}");
    }
    println!("\nLIST LENGTHS");
    let mut last = "";
    for ((list, b), v) in &st.lens {
        if *list != last {
            println!("  {list}:");
            last = list;
        }
        println!("    {b:>8}: {v}");
    }
    Ok(())
}
