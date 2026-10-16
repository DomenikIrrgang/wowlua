//! Lints over the syntax tree (spec appendix B): unused locals, parameters
//! and imports (W1003) and locals that shadow a local of an enclosing scope
//! (W1004), with machine-applicable fixes. Unreachable code (W1005) is
//! reported by the type checker, which knows which calls never return.

use wowlua_diagnostics::{Diagnostic, Edit};
use wowlua_span::Span;
use wowlua_syntax::Parse;
use wowlua_syntax::ast::*;

/// The lints of one file. `text` is the file's source.
pub fn lint(parse: &Parse, text: &[u8]) -> Vec<Diagnostic> {
    let mut l = Linter {
        text,
        words: std::cell::OnceCell::new(),
        bindings: Vec::new(),
        groups: Vec::new(),
        scopes: vec![Vec::new()],
        diagnostics: Vec::new(),
    };
    let block = &parse.chunk.block;
    let module = block
        .stmts
        .iter()
        .any(|s| matches!(s.kind, StmtKind::Import(_) | StmtKind::Export(_)));
    // Top-level functions of a module are hoisted locals.
    if module {
        for s in &block.stmts {
            let (decorated, inner) = match &s.kind {
                StmtKind::Export(inner) => (true, &**inner),
                _ => (!s.decorators.is_empty(), s),
            };
            if let StmtKind::Function { name, .. } = &inner.kind
                && name.path.len() == 1
                && name.method.is_none()
            {
                let kind = if decorated || !inner.decorators.is_empty() {
                    Kind::Exempt
                } else {
                    Kind::Unused
                };
                let group = l.group(Some(l.delete(s.span)), 1);
                l.declare(&name.path[0], kind, Some(group), false);
            }
        }
    }
    for s in &block.stmts {
        l.stmt(s);
    }
    l.pop_scope();
    l.diagnostics
        .sort_by_key(|d| d.labels.first().map(|x| (x.span.start, x.span.end)));
    l.diagnostics
}

/// How an unused binding is reported.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// Never reported (exports, decorated declarations, implicit `self`).
    Exempt,
    /// A local: removed with its statement when possible, else renamed.
    Unused,
    /// A destructured field: reported without a fix (renaming would read
    /// another field).
    NoFix,
    /// A parameter or loop variable: reported only when no later one of the
    /// same list is used; renamed.
    Positional,
    /// A loop variable: like a parameter.
    Loop,
    /// An imported name: removed.
    Import,
}

struct Binding {
    name: Box<str>,
    span: Span,
    kind: Kind,
    used: bool,
    group: Option<usize>,
    /// Imports: the edit removing just this name.
    own_removal: Option<Edit>,
}

/// The names one statement declares, and the edit removing the statement
/// (when its values have no side effects).
struct Group {
    removal: Option<Edit>,
    size: usize,
}

struct Linter<'a> {
    text: &'a [u8],
    /// The identifiers that occur in `text` (maximal runs of word bytes),
    /// found the first time a rename asks.
    words: std::cell::OnceCell<rustc_hash::FxHashSet<&'a [u8]>>,
    bindings: Vec<Binding>,
    groups: Vec<Group>,
    scopes: Vec<Vec<usize>>,
    diagnostics: Vec<Diagnostic>,
}

fn exempt_name(name: &str) -> bool {
    name.starts_with('_') || name == "self"
}

/// An expression without side effects (literals, names, functions, and
/// tables of those).
fn pure(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Vararg
        | ExprKind::Name(_)
        | ExprKind::Function(_) => true,
        ExprKind::Paren(inner) => pure(inner),
        ExprKind::Cast { expr, .. } => pure(expr),
        ExprKind::Table(fields) => fields.iter().all(|f| match f {
            TableField::Keyed { key, value, .. } => pure(key) && pure(value),
            TableField::Named { value, .. } => pure(value),
            TableField::Positional(v) => pure(v),
        }),
        _ => false,
    }
}

impl Linter<'_> {
    fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    fn group(&mut self, removal: Option<Edit>, size: usize) -> usize {
        self.groups.push(Group { removal, size });
        self.groups.len() - 1
    }

    fn reportable(&self, i: usize) -> bool {
        let b = &self.bindings[i];
        !b.used && !exempt_name(&b.name) && b.kind != Kind::Exempt
    }

    /// Ends a scope and reports its unused bindings.
    fn pop_scope(&mut self) {
        let scope = self.scopes.pop().expect("a scope");
        let mut done: Vec<usize> = Vec::new();
        for &i in &scope {
            if !self.reportable(i) || done.contains(&i) {
                continue;
            }
            let b = &self.bindings[i];
            match b.kind {
                Kind::Exempt | Kind::Positional | Kind::Loop => {}
                Kind::NoFix => self.unused(i, None),
                Kind::Unused | Kind::Import => {
                    // The whole statement goes when all its names are unused.
                    let group = b.group;
                    let members: Vec<usize> = match group {
                        Some(g) => scope
                            .iter()
                            .copied()
                            .filter(|&j| self.bindings[j].group == Some(g))
                            .collect(),
                        None => vec![i],
                    };
                    let whole = group.and_then(|g| {
                        let all = members.len() == self.groups[g].size
                            && members
                                .iter()
                                .all(|&j| self.reportable(j) && self.bindings[j].kind == b.kind);
                        all.then(|| self.groups[g].removal.clone()).flatten()
                    });
                    match whole {
                        Some(edit) => {
                            let what = if b.kind == Kind::Import {
                                "remove the unused import"
                            } else {
                                "remove the unused local"
                            };
                            for (n, &j) in members.iter().enumerate() {
                                let fix = (n == 0).then(|| (what.to_string(), vec![edit.clone()]));
                                self.unused(j, fix);
                                done.push(j);
                            }
                        }
                        None if b.kind == Kind::Import => {
                            let fix = b
                                .own_removal
                                .clone()
                                .map(|e| ("remove the unused import".to_string(), vec![e]));
                            self.unused(i, fix);
                        }
                        None => self.rename(i),
                    }
                }
            }
        }
    }

    fn unused(&mut self, i: usize, fix: Option<(String, Vec<Edit>)>) {
        let b = &self.bindings[i];
        let what = match b.kind {
            Kind::Import => "import",
            Kind::Positional => "parameter",
            Kind::Loop => "loop variable",
            _ => "local",
        };
        let mut d = Diagnostic::warning("W1003", format!("unused {what} `{}`", b.name))
            .with_label(b.span, "never read");
        if let Some((message, edits)) = fix {
            d = d.with_help(message.clone()).with_fix(message, edits);
        }
        self.diagnostics.push(d);
    }

    /// No identifier `name` occurs in the file (a rename to it cannot
    /// capture or shadow anything).
    fn name_free(&self, name: &str) -> bool {
        let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        let n = name.as_bytes();
        // An occurrence bounded by non-word bytes is a maximal run of word
        // bytes when the name is made of word bytes only.
        if !n.is_empty() && n.iter().all(|&c| word(c)) {
            let words = self.words.get_or_init(|| {
                self.text
                    .split(|&c| !word(c))
                    .filter(|w| !w.is_empty())
                    .collect()
            });
            return !words.contains(n);
        }
        !self.text.windows(n.len()).enumerate().any(|(at, w)| {
            w == n
                && (at == 0 || !word(self.text[at - 1]))
                && self.text.get(at + n.len()).is_none_or(|&c| !word(c))
        })
    }

    fn rename(&mut self, i: usize) {
        let b = &self.bindings[i];
        if !self.name_free(&format!("_{}", b.name)) {
            self.unused(i, None);
            return;
        }
        let b = &self.bindings[i];
        let edit = Edit {
            span: b.span,
            replacement: format!("_{}", b.name),
        };
        let fix = format!("prefix it with `_` (`_{}`)", b.name);
        self.unused(i, Some((fix, vec![edit])));
    }

    /// Positional bindings (parameters, loop variables): the unused ones
    /// after the last used one.
    fn positional(&mut self, ids: &[usize]) {
        let last_used = ids.iter().rposition(|&i| self.bindings[i].used);
        let from = last_used.map_or(0, |p| p + 1);
        for &i in &ids[from..] {
            if self.reportable(i) {
                self.rename(i);
            }
        }
    }

    /// An edit deleting `span`, with its line when nothing else is on it.
    fn delete(&self, span: Span) -> Edit {
        let bytes = self.text;
        let (mut start, mut end) = (span.start as usize, span.end as usize);
        let mut line_start = start;
        while line_start > 0 && matches!(bytes[line_start - 1], b' ' | b'\t') {
            line_start -= 1;
        }
        let mut line_end = end;
        while line_end < bytes.len() && matches!(bytes[line_end], b' ' | b'\t' | b';') {
            line_end += 1;
        }
        let at_line_start = line_start == 0 || bytes[line_start - 1] == b'\n';
        if at_line_start && (line_end == bytes.len() || matches!(bytes[line_end], b'\n' | b'\r')) {
            start = line_start;
            end = line_end;
            if bytes.get(end) == Some(&b'\r') {
                end += 1;
            }
            if bytes.get(end) == Some(&b'\n') {
                end += 1;
            }
        }
        Edit {
            span: Span {
                file: span.file,
                start: start as u32,
                end: end as u32,
            },
            replacement: String::new(),
        }
    }

    fn lookup(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|s| s.iter().rev())
            .copied()
            .find(|&i| self.bindings[i].name.as_ref() == name)
    }

    /// Declares a binding in the current scope; `shadow`: report W1004 when
    /// it shadows a local of an enclosing scope.
    fn declare(&mut self, name: &Name, kind: Kind, group: Option<usize>, shadow: bool) -> usize {
        if shadow && kind != Kind::Exempt && !exempt_name(&name.text) {
            let (current, outer) = self.scopes.split_last().expect("a scope");
            let in_current = current.iter().any(|&i| self.bindings[i].name == name.text);
            let shadowed = outer
                .iter()
                .rev()
                .flat_map(|s| s.iter().rev())
                .copied()
                .find(|&i| self.bindings[i].name == name.text);
            if let Some(o) = shadowed
                && !in_current
            {
                let d = Diagnostic::warning(
                    "W1004",
                    format!("`{}` shadows a local of an enclosing scope", name.text),
                )
                .with_label(name.span, "declared here")
                .with_label(self.bindings[o].span, "shadowed local");
                self.diagnostics.push(d);
            }
        }
        let i = self.bindings.len();
        self.bindings.push(Binding {
            name: name.text.clone(),
            span: name.span,
            kind,
            used: false,
            group,
            own_removal: None,
        });
        self.scopes.last_mut().expect("a scope").push(i);
        i
    }

    fn use_name(&mut self, name: &str) {
        if let Some(i) = self.lookup(name) {
            self.bindings[i].used = true;
        }
    }

    // ------------------------------------------------------------ blocks --

    fn block(&mut self, b: &Block) {
        self.push_scope();
        for s in &b.stmts {
            self.stmt(s);
        }
        self.pop_scope();
    }

    fn decorators(&mut self, ds: &[Decorator]) {
        for d in ds {
            if !d.is_core() {
                self.use_name(&d.path[0].text);
            }
            if let Some(args) = &d.args {
                self.args(args);
            }
        }
    }

    /// The kind of a declared local (decorated declarations are exempt).
    fn local_kind(s: &Stmt, kind: Kind) -> Kind {
        if s.decorators.is_empty() {
            kind
        } else {
            Kind::Exempt
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        self.decorators(&s.decorators);
        match &s.kind {
            StmtKind::Export(inner) => {
                let first = self.bindings.len();
                self.stmt(inner);
                for i in first..self.bindings.len() {
                    if self.scopes.last().expect("a scope").contains(&i) {
                        self.bindings[i].kind = Kind::Exempt;
                    }
                }
            }
            StmtKind::Local { names, values } => {
                for v in values {
                    self.expr(v);
                }
                for n in names {
                    if let Some(t) = &n.ty {
                        self.ty(t);
                    }
                }
                let removal = values.iter().all(pure).then(|| self.delete(s.span));
                let group = self.group(removal, names.len());
                for n in names {
                    self.declare(
                        &n.name,
                        Self::local_kind(s, Kind::Unused),
                        Some(group),
                        true,
                    );
                }
            }
            StmtKind::LocalDestructure { fields, ty, value } => {
                if let Some(v) = value {
                    self.expr(v);
                }
                if let Some(t) = ty {
                    self.ty(t);
                }
                for f in fields {
                    if let Some(d) = &f.default {
                        self.expr(d);
                    }
                }
                for f in fields {
                    self.declare(&f.name, Self::local_kind(s, Kind::NoFix), None, true);
                }
            }
            StmtKind::Const { name, ty, value } => {
                if let Some(v) = value {
                    self.expr(v);
                }
                if let Some(t) = ty {
                    self.ty(t);
                }
                let removal = value
                    .as_deref()
                    .is_none_or(pure)
                    .then(|| self.delete(s.span));
                let group = self.group(removal, 1);
                self.declare(name, Self::local_kind(s, Kind::Unused), Some(group), true);
            }
            StmtKind::LocalFunction { name, func } => {
                let group = self.group(Some(self.delete(s.span)), 1);
                self.declare(name, Self::local_kind(s, Kind::Unused), Some(group), true);
                self.func(func, false);
            }
            StmtKind::Function { name, func } => {
                // `function a.b()` reads `a`; `function f()` assigns `f` (a
                // hoisted local in a module, else a local or a global).
                if name.path.len() > 1 || name.method.is_some() {
                    self.use_name(&name.path[0].text);
                }
                self.func(func, name.method.is_some());
            }
            StmtKind::Assign { targets, values } => {
                for v in values {
                    self.expr(v);
                }
                for t in targets {
                    self.target(t);
                }
            }
            StmtKind::Compound { target, value, .. } => {
                self.expr(value);
                self.expr(target);
            }
            StmtKind::Call(e) => self.expr(e),
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            StmtKind::Repeat { body, cond } => {
                self.push_scope();
                for s in &body.stmts {
                    self.stmt(s);
                }
                self.expr(cond);
                self.pop_scope();
            }
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                self.expr(cond);
                self.block(then_block);
                for (c, b) in elseifs {
                    self.expr(c);
                    self.block(b);
                }
                if let Some(b) = else_block {
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
                self.expr(start);
                self.expr(limit);
                if let Some(e) = step {
                    self.expr(e);
                }
                self.push_scope();
                if let Some(t) = &var.ty {
                    self.ty(t);
                }
                let i = self.declare(&var.name, Kind::Loop, None, true);
                self.block(body);
                self.positional(&[i]);
                self.pop_scope();
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                for e in exprs {
                    self.expr(e);
                }
                self.push_scope();
                let mut ids = Vec::new();
                for v in vars {
                    if let Some(t) = &v.ty {
                        self.ty(t);
                    }
                    ids.push(self.declare(&v.name, Kind::Loop, None, true));
                }
                self.block(body);
                self.positional(&ids);
                self.pop_scope();
            }
            StmtKind::Return(values) => {
                for v in values {
                    self.expr(v);
                }
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
            StmtKind::Class(c) => self.class(c),
            StmtKind::Interface(i) => self.interface(i),
            StmtKind::Enum(e) => {
                for item in &e.items {
                    if let Some(v) = &item.value {
                        self.expr(v);
                    }
                }
            }
            StmtKind::TypeAlias(a) => {
                self.type_params(&a.type_params);
                self.ty(&a.ty);
            }
            StmtKind::Declare(_) => {}
            StmtKind::Import(i) => self.import(i, s.span),
        }
    }

    fn import(&mut self, i: &Import, span: Span) {
        let from = String::from_utf8_lossy(&i.from.value).into_owned();
        // Removing every name: the statement goes when the import has no
        // load-order effect (type-only, standard library), else it becomes
        // `import _ from "…"`, which keeps the load order.
        let whole = if i.type_only || from.starts_with("wowlua:") {
            self.delete(span)
        } else {
            let raw = String::from_utf8_lossy(&i.from.raw).into_owned();
            Edit {
                span,
                replacement: format!("import _ from {raw}"),
            }
        };
        match &i.clause {
            ImportClause::SideEffect => {}
            ImportClause::Namespace(n) => {
                let group = self.group(Some(whole), 1);
                self.declare(n, Kind::Import, Some(group), false);
            }
            ImportClause::Names(specs) => {
                let bound = |s: &ImportSpec| s.alias.clone().unwrap_or_else(|| s.name.clone());
                if specs.iter().any(|s| bound(s).text.as_ref() == "_") {
                    return;
                }
                let group = self.group(Some(whole), specs.len());
                for (k, spec) in specs.iter().enumerate() {
                    let end = |s: &ImportSpec| bound(s).span.end;
                    // One name of several: delete it and its comma.
                    let own = if k + 1 < specs.len() {
                        Some((spec.name.span.start, specs[k + 1].name.span.start))
                    } else if k > 0 {
                        Some((end(&specs[k - 1]), end(spec)))
                    } else {
                        None
                    };
                    let id = self.declare(&bound(spec), Kind::Import, Some(group), false);
                    self.bindings[id].own_removal = own.map(|(start, end)| Edit {
                        span: Span {
                            file: span.file,
                            start,
                            end,
                        },
                        replacement: String::new(),
                    });
                }
            }
        }
    }

    // --------------------------------------------------------- functions --

    /// A function body; `method`: it has an implicit `self`.
    fn func(&mut self, f: &FuncBody, method: bool) {
        self.type_params(&f.type_params);
        self.push_scope();
        if method {
            let name = Name {
                text: "self".into(),
                span: f.params_span,
            };
            self.declare(&name, Kind::Exempt, None, false);
        }
        let mut ids = Vec::new();
        for p in &f.params {
            self.decorators(&p.decorators);
            if let Some(t) = &p.ty {
                self.ty(t);
            }
            if let Some(d) = &p.default {
                self.expr(d);
            }
            let kind = if p.decorators.is_empty() {
                Kind::Positional
            } else {
                Kind::Exempt
            };
            ids.push(self.declare(&p.name, kind, None, true));
        }
        if let Some(v) = &f.vararg
            && let Some(t) = &v.ty
        {
            self.vararg_ty(t);
        }
        if let Some(r) = &f.ret {
            self.ret(r);
        }
        self.block(&f.body);
        self.positional(&ids);
        self.pop_scope();
    }

    /// A signature without a body (declarations, overloads): types only.
    fn signature(&mut self, f: &FuncBody) {
        self.type_params(&f.type_params);
        for p in &f.params {
            if let Some(t) = &p.ty {
                self.ty(t);
            }
        }
        if let Some(v) = &f.vararg
            && let Some(t) = &v.ty
        {
            self.vararg_ty(t);
        }
        if let Some(r) = &f.ret {
            self.ret(r);
        }
    }

    fn class(&mut self, c: &ClassDecl) {
        self.type_params(&c.type_params);
        if let Some(t) = &c.extends {
            self.ty(t);
        }
        for t in &c.implements {
            self.ty(t);
        }
        for m in &c.members {
            self.decorators(&m.decorators);
            match &m.kind {
                ClassMemberKind::Field { ty, value, .. } => {
                    self.ty(ty);
                    if let Some(v) = value {
                        self.expr(v);
                    }
                }
                ClassMemberKind::Method { func, has_body, .. }
                | ClassMemberKind::Constructor { func, has_body } => {
                    let is_static = m.modifiers.contains(&Modifier::Static);
                    if *has_body {
                        self.func(func, !is_static);
                    } else {
                        self.signature(func);
                    }
                }
                ClassMemberKind::Getter { ty, body, .. } => {
                    if let Some(t) = ty {
                        self.ty(t);
                    }
                    self.push_scope();
                    self.implicit_self(m.span);
                    self.block(body);
                    self.pop_scope();
                }
                ClassMemberKind::Setter { param, body, .. } => {
                    if let Some(t) = &param.ty {
                        self.ty(t);
                    }
                    self.push_scope();
                    self.implicit_self(m.span);
                    let id = self.declare(&param.name, Kind::Positional, None, true);
                    self.block(body);
                    self.positional(&[id]);
                    self.pop_scope();
                }
            }
        }
    }

    fn implicit_self(&mut self, span: Span) {
        let name = Name {
            text: "self".into(),
            span,
        };
        self.declare(&name, Kind::Exempt, None, false);
    }

    fn interface(&mut self, i: &InterfaceDecl) {
        self.type_params(&i.type_params);
        for t in &i.extends {
            self.ty(t);
        }
        for m in &i.members {
            match m {
                InterfaceMember::Field { ty, .. } => self.ty(ty),
                InterfaceMember::Method { sig, .. } => self.signature(sig),
            }
        }
    }

    // ------------------------------------------------------------- types --

    fn type_params(&mut self, ps: &[TypeParam]) {
        for p in ps {
            if let Some(c) = &p.constraint {
                self.ty(c);
            }
        }
    }

    fn ty(&mut self, t: &Type) {
        match &t.kind {
            TypeKind::Ref { path, args } => {
                self.use_name(&path[0].text);
                for a in args {
                    match a {
                        TypeArg::Type(t) => self.ty(t),
                        TypeArg::Pack(TypePack::List { types, vararg, .. }) => {
                            for t in types {
                                self.ty(t);
                            }
                            if let Some(v) = vararg {
                                self.vararg_ty(v);
                            }
                        }
                        TypeArg::Pack(TypePack::Named(_)) => {}
                    }
                }
            }
            TypeKind::Typeof(e) => self.expr(e),
            TypeKind::Table(fields) => {
                for f in fields {
                    match f {
                        TableTypeField::Named { ty, .. }
                        | TableTypeField::NamedString { ty, .. }
                        | TableTypeField::Array(ty) => self.ty(ty),
                        TableTypeField::Indexer { key, value } => {
                            self.ty(key);
                            self.ty(value);
                        }
                    }
                }
            }
            TypeKind::Function(f) => {
                self.type_params(&f.type_params);
                for p in &f.params {
                    self.ty(&p.ty);
                }
                if let Some(v) = &f.vararg {
                    self.vararg_ty(v);
                }
                self.ret(&f.ret);
            }
            TypeKind::Paren(t) | TypeKind::Optional(t) | TypeKind::Array(t) => self.ty(t),
            TypeKind::Union(ts) | TypeKind::Intersection(ts) => {
                for t in ts {
                    self.ty(t);
                }
            }
            _ => {}
        }
    }

    fn vararg_ty(&mut self, v: &VarargType) {
        match v {
            VarargType::Type(t) | VarargType::Expand(t) => self.ty(t),
            VarargType::Pack(_) => {}
        }
    }

    fn ret(&mut self, r: &RetType) {
        match r {
            RetType::Single(t) | RetType::Predicate { ty: t, .. } => self.ty(t),
            RetType::List { types, vararg, .. } => {
                for t in types {
                    self.ty(t);
                }
                if let Some(v) = vararg {
                    self.vararg_ty(v);
                }
            }
            RetType::Pack(_) => {}
        }
    }

    // ------------------------------------------------------- expressions --

    /// An assignment target: a plain name is written, not read.
    fn target(&mut self, t: &Expr) {
        match &t.kind {
            ExprKind::Name(_) => {}
            _ => self.expr(t),
        }
    }

    fn args(&mut self, a: &Args) {
        for e in a.exprs() {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &Expr) {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.expr_inner(e))
    }

    fn expr_inner(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Name(n) => self.use_name(&n.text),
            ExprKind::Function(f) => self.func(f, false),
            ExprKind::Table(fields) => {
                for f in fields {
                    match f {
                        TableField::Keyed { key, value, .. } => {
                            self.expr(key);
                            self.expr(value);
                        }
                        TableField::Named { value, .. } => self.expr(value),
                        TableField::Positional(v) => self.expr(v),
                    }
                }
            }
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(e) = p {
                        self.expr(e);
                    }
                }
            }
            ExprKind::Index { obj, key } | ExprKind::OptIndex { obj, key } => {
                self.expr(obj);
                self.expr(key);
            }
            ExprKind::Field { obj, .. } | ExprKind::OptField { obj, .. } => self.expr(obj),
            ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
                self.expr(callee);
                self.args(args);
            }
            ExprKind::Method { obj, args, .. } | ExprKind::OptMethod { obj, args, .. } => {
                self.expr(obj);
                self.args(args);
            }
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Cast { expr, ty } => {
                self.expr(expr);
                self.ty(ty);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wowlua_span::SourceDb;

    fn run(src: &str) -> Vec<(String, String, Option<String>)> {
        let mut db = SourceDb::new();
        let file = db.add("t.wowlua", src.to_string());
        let parse = wowlua_syntax::parse(src.as_bytes(), file);
        assert!(!parse.has_errors(), "{:?}", parse.diagnostics);
        lint(&parse, src.as_bytes())
            .into_iter()
            .map(|d| {
                let fixed = d.fixes.first().map(|f| apply(src, &f.edits));
                (d.code.unwrap_or("-").to_string(), d.message, fixed)
            })
            .collect()
    }

    fn apply(src: &str, edits: &[Edit]) -> String {
        let mut out = src.to_string();
        let mut edits = edits.to_vec();
        edits.sort_by_key(|e| std::cmp::Reverse(e.span.start));
        for e in edits {
            out.replace_range(e.span.start as usize..e.span.end as usize, &e.replacement);
        }
        out
    }

    #[test]
    fn unused_locals() {
        let d = run("local a = 1\nlocal b = f()\nprint(c)\n");
        assert_eq!(d.len(), 2, "{d:#?}");
        assert_eq!(d[0].1, "unused local `a`");
        assert_eq!(d[0].2.as_deref(), Some("local b = f()\nprint(c)\n"));
        assert_eq!(
            d[1].2.as_deref(),
            Some("local a = 1\nlocal _b = f()\nprint(c)\n")
        );
        assert!(run("local a = 1\nprint(a)\nlocal _x = 2\n").is_empty());
        // Assigned but never read.
        assert_eq!(run("local a\na = 1\n").len(), 1);
        // All names of a statement unused and removable.
        let d = run("local a, b = 1, 2\n");
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].2.as_deref(), Some(""));
        let d = run("local a, b = f()\nprint(b)\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].2.as_deref(), Some("local _a, b = f()\nprint(b)\n"));
        assert_eq!(run("local function f() end\n")[0].2.as_deref(), Some(""));
        assert!(run("local function f() f() end\nf()\n").is_empty());
        // No rename onto a name the file uses.
        let d = run("local _b = 1\nlocal b = f()\nprint(_b)\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].2, None);
        // Only whole words count, anywhere in the text (comments too).
        let d = run("local b = f()\nprint(x_b, _bc)\n");
        assert_eq!(d[0].2.as_deref(), Some("local _b = f()\nprint(x_b, _bc)\n"));
        let d = run("local b = f()\nprint(1) -- _b\n");
        assert_eq!(d[0].2, None);
        // Several renames in one file.
        let d = run("local b = f()\nlocal c = f()\nprint(_c)\n");
        assert_eq!(d.len(), 2, "{d:#?}");
        assert_eq!(
            d[0].2.as_deref(),
            Some("local _b = f()\nlocal c = f()\nprint(_c)\n")
        );
        assert_eq!(d[1].2, None);
    }

    #[test]
    fn parameters_and_loops() {
        // Only trailing unused parameters.
        let d = run("local function f(a, b, c) return b end\nf()\n");
        assert_eq!(d.len(), 1, "{d:#?}");
        assert_eq!(d[0].1, "unused parameter `c`");
        assert!(run("local t = {}\nfunction t:m(_x) end\nprint(t)\n").is_empty());
        let d = run("for k, v in pairs({}) do print(k) end\nfor i = 1, 2 do end\n");
        assert_eq!(d.len(), 2, "{d:#?}");
        assert!(d[1].2.as_deref().unwrap().contains("for _i = 1"));
    }

    #[test]
    fn imports_and_exports() {
        let d = run(
            "import a, b from \"./m\"\nimport type T from \"./t\"\nimport x from \"wowlua:meta\"\nexport local y = a\n",
        );
        assert_eq!(d.len(), 3, "{d:#?}");
        assert_eq!(d[0].1, "unused import `b`");
        assert!(
            d[0].2
                .as_deref()
                .unwrap()
                .starts_with("import a from \"./m\"\n")
        );
        assert!(
            d[1].2
                .as_deref()
                .unwrap()
                .starts_with("import a, b from \"./m\"\nimport x")
        );
        let d = run("import a from \"./m\"\nexport local y = 1\n");
        assert!(
            d[0].2
                .as_deref()
                .unwrap()
                .starts_with("import _ from \"./m\"\n")
        );
        // Used in a type position.
        assert!(run("import type T from \"./t\"\nexport local y: T? = nil\n").is_empty());
        // Hoisted module functions.
        let d = run(
            "import a from \"./m\"\nexport function g() return a + h() end\nfunction h() return 1 end\nfunction unused() end\n",
        );
        assert_eq!(d.len(), 1, "{d:#?}");
        assert_eq!(d[0].1, "unused local `unused`");
    }

    #[test]
    fn shadowing() {
        let d = run(
            "local x = 1\ndo local x = 2 print(x) end\nlocal x = 3\nprint(x)\nlocal function f(x) return x end\nf()\n",
        );
        let shadows: Vec<_> = d.iter().filter(|d| d.0 == "W1004").collect();
        assert_eq!(shadows.len(), 2, "{d:#?}");
        assert!(
            run("local _x = 1\ndo local _x = 2 print(_x) end\n")
                .iter()
                .all(|d| d.0 != "W1004")
        );
    }
}
