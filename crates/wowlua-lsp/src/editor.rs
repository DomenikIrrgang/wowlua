//! What an editor draws from a file's syntax and its package's last check
//! (spec §16.2.9): folding ranges, semantic tokens, inlay hints, quick
//! fixes and the references lenses.

use std::collections::HashMap;

use serde_json::{Value as Json, json};
use wowlua_span::{SourceDb, Span};
use wowlua_syntax::token::{TokenKind, TriviaKind};
use wowlua_syntax::view::*;
use wowlua_syntax::{Parse, visit};

use crate::check;

/// The semantic token types, in the legend's order.
pub const TOKEN_TYPES: &[&str] = &[
    "keyword",
    "type",
    "class",
    "function",
    "method",
    "parameter",
    "property",
    "decorator",
    "enumMember",
    "namespace",
];

/// The capabilities the editor's features add to `initialize`'s reply.
pub fn capabilities() -> Json {
    json!({
        "foldingRangeProvider": true,
        "semanticTokensProvider": {
            "legend": { "tokenTypes": TOKEN_TYPES, "tokenModifiers": [] },
            "full": true,
        },
        "inlayHintProvider": true,
        "codeActionProvider": { "codeActionKinds": ["quickfix"] },
        "codeLensProvider": { "resolveProvider": false },
    })
}

fn parse(text: &str) -> Parse {
    let mut db = SourceDb::new();
    let file = db.add("editor", String::new());
    wowlua_syntax::parse(text.as_bytes(), file)
}

/// Lines and UTF-16 columns of byte offsets.
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { text, starts }
    }

    fn line(&self, offset: usize) -> usize {
        self.starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1)
    }

    /// The line and the UTF-16 column of `offset`.
    fn at(&self, offset: usize) -> (usize, usize) {
        let line = self.line(offset);
        let start = self.starts[line];
        let end = offset.min(self.text.len());
        let column = self
            .text
            .get(start..end)
            .map_or(0, |s| s.encode_utf16().count());
        (line, column)
    }

    fn position(&self, offset: usize) -> Json {
        let (line, character) = self.at(offset);
        json!({ "line": line, "character": character })
    }
}

// ------------------------------------------------------------- folding --

/// The file's folding ranges: statements with bodies, functions, classes
/// and the like, table constructors over several lines, runs of comments
/// and of imports.
pub fn folding_ranges(text: &str) -> Vec<Json> {
    let parsed = parse(text);
    let lines = Lines::new(text);
    let mut out = Vec::new();
    let mut range = |start: u32, end: u32, kind: Option<&str>| {
        let (a, b) = (lines.line(start as usize), lines.line(end as usize));
        // The closing line (`end`, `}`) stays in sight.
        let last = if kind.is_none() {
            b.saturating_sub(1)
        } else {
            b
        };
        if last > a {
            let mut r = json!({ "startLine": a, "endLine": last });
            if let Some(kind) = kind {
                r["kind"] = json!(kind);
            }
            out.push(r);
        }
    };
    let block = parsed.view().block();
    visit::walk_stmts(block, &mut |s| {
        let span = s.span();
        match s.kind() {
            StmtKind::Do(_)
            | StmtKind::While { .. }
            | StmtKind::Repeat { .. }
            | StmtKind::If { .. }
            | StmtKind::NumericFor { .. }
            | StmtKind::GenericFor { .. }
            | StmtKind::Function { .. }
            | StmtKind::LocalFunction { .. }
            | StmtKind::Class(_)
            | StmtKind::Interface(_)
            | StmtKind::Enum(_)
            | StmtKind::Declare(_) => range(span.start, span.end, None),
            _ => {}
        }
    });
    // Function expressions and table constructors.
    visit::walk_block(block, &mut |e| {
        let span = e.span();
        match e.kind() {
            ExprKind::Function(_) => range(span.start, span.end, None),
            ExprKind::Table(fields) if !fields.is_empty() => range(span.start, span.end, None),
            _ => {}
        }
    });
    // Comments: a block comment, or two line comments or more in a row.
    let trivia = &parsed.lexed.trivia;
    let mut run: Option<(u32, u32, usize)> = None;
    for t in trivia {
        match t.kind {
            TriviaKind::BlockComment => range(t.start, t.end, Some("comment")),
            TriviaKind::LineComment => {
                let line = lines.line(t.start as usize);
                run = match run {
                    Some((start, _, last)) if line == last + 1 => Some((start, t.end, line)),
                    Some((start, end, _)) => {
                        range(start, end, Some("comment"));
                        Some((t.start, t.end, line))
                    }
                    None => Some((t.start, t.end, line)),
                };
            }
            TriviaKind::Newline => {}
        }
    }
    if let Some((start, end, _)) = run {
        range(start, end, Some("comment"));
    }
    // Imports in a row.
    let mut imports: Option<(u32, u32)> = None;
    for s in block.stmts() {
        if matches!(s.kind(), StmtKind::Import(_)) {
            let span = s.span();
            imports = Some(imports.map_or((span.start, span.end), |(a, _)| (a, span.end)));
        } else if let Some((a, b)) = imports.take() {
            range(a, b, Some("imports"));
        }
    }
    if let Some((a, b)) = imports {
        range(a, b, Some("imports"));
    }
    out
}

// ----------------------------------------------------- semantic tokens --

/// What a name is, by priority: a later role does not replace an earlier
/// one that is more telling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Variable,
    Namespace,
    Property,
    Parameter,
    Method,
    Function,
    EnumMember,
    Decorator,
    Type,
    Class,
}

impl Role {
    fn token(self) -> Option<u32> {
        let name = match self {
            Role::Variable => return None,
            Role::Namespace => "namespace",
            Role::Property => "property",
            Role::Parameter => "parameter",
            Role::Method => "method",
            Role::Function => "function",
            Role::EnumMember => "enumMember",
            Role::Decorator => "decorator",
            Role::Type => "type",
            Role::Class => "class",
        };
        TOKEN_TYPES
            .iter()
            .position(|t| *t == name)
            .map(|i| i as u32)
    }
}

/// The words that are keywords only where wowlua's syntax puts them
/// (spec §1.2.5); elsewhere they are names.
const CONTEXTUAL: &[&str] = &[
    "class",
    "abstract",
    "interface",
    "enum",
    "type",
    "const",
    "declare",
    "import",
    "export",
    "from",
    "as",
    "extends",
    "implements",
    "continue",
    "static",
    "readonly",
    "private",
    "protected",
    "public",
    "override",
    "overload",
    "constructor",
    "get",
    "set",
    "super",
    "typeof",
    "keyof",
    "is",
    "global",
    "namespace",
    "event",
    "message",
    "module",
    "internal",
];

/// Every name of a file and what it is.
#[derive(Default)]
struct Names {
    roles: HashMap<u32, Role>,
}

impl Names {
    fn set(&mut self, span: Span, role: Role) {
        let slot = self.roles.entry(span.start).or_insert(role);
        if role > *slot {
            *slot = role;
        }
    }

    fn block(&mut self, b: Block<'_>) {
        for s in b.stmts() {
            self.stmt(s);
        }
    }

    fn decorators(&mut self, list: Decorators<'_>) {
        for d in list {
            for n in d.path() {
                self.set(n.span(), Role::Decorator);
            }
            if let Some(args) = d.args() {
                for e in args.exprs() {
                    self.expr(e);
                }
            }
        }
    }

    fn stmt(&mut self, s: Stmt<'_>) {
        self.decorators(s.decorators());
        match s.kind() {
            StmtKind::Assign { targets, values } => {
                targets.iter().chain(values).for_each(|e| self.expr(e))
            }
            StmtKind::Compound { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            StmtKind::Call(e) => self.expr(e),
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } | StmtKind::Repeat { body, cond } => {
                self.expr(cond);
                self.block(body);
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
                self.typed(var, Role::Variable);
                [start, limit]
                    .into_iter()
                    .chain(step)
                    .for_each(|e| self.expr(e));
                self.block(body);
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                vars.iter().for_each(|v| self.typed(v, Role::Variable));
                exprs.iter().for_each(|e| self.expr(e));
                self.block(body);
            }
            StmtKind::Function { name, func } => {
                let path: Vec<Name<'_>> = name.path().iter().collect();
                match name.method() {
                    Some(m) => {
                        path.iter().for_each(|n| self.set(n.span(), Role::Variable));
                        self.set(m.span(), Role::Method);
                    }
                    None => {
                        for (i, n) in path.iter().enumerate() {
                            let role = if i + 1 == path.len() {
                                Role::Function
                            } else {
                                Role::Variable
                            };
                            self.set(n.span(), role);
                        }
                    }
                }
                self.func(func);
            }
            StmtKind::LocalFunction { name, func } | StmtKind::Overload { name, func } => {
                self.set(name.span(), Role::Function);
                self.func(func);
            }
            StmtKind::Local { names, values } => {
                names.iter().for_each(|n| self.typed(n, Role::Variable));
                values.iter().for_each(|e| self.expr(e));
            }
            StmtKind::LocalDestructure { fields, ty, value } => {
                for f in fields {
                    self.set(f.name.span(), Role::Variable);
                    if let Some(d) = f.default {
                        self.expr(d);
                    }
                }
                ty.into_iter().for_each(|t| self.ty(t));
                value.into_iter().for_each(|e| self.expr(e));
            }
            StmtKind::Const { name, ty, value } => {
                self.set(name.span(), Role::Variable);
                ty.into_iter().for_each(|t| self.ty(t));
                value.into_iter().for_each(|e| self.expr(e));
            }
            StmtKind::Return(values) => values.iter().for_each(|e| self.expr(e)),
            StmtKind::Class(c) => self.class(c),
            StmtKind::Interface(i) => self.interface(i),
            StmtKind::Enum(e) => self.enumeration(e),
            StmtKind::TypeAlias(a) => self.alias(a),
            StmtKind::Declare(d) => self.declare(d),
            StmtKind::Import(i) => match i.clause() {
                ImportClause::Namespace(n) => self.set(n.span(), Role::Namespace),
                ImportClause::Names(specs) => {
                    for spec in specs {
                        let role = if i.type_only() {
                            Role::Type
                        } else {
                            Role::Variable
                        };
                        self.set(spec.name.span(), role);
                        if let Some(a) = spec.alias {
                            self.set(a.span(), role);
                        }
                    }
                }
                ImportClause::SideEffect => {}
            },
            StmtKind::Export(inner) => self.stmt(inner),
            StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
        }
    }

    fn typed(&mut self, n: TypedName<'_>, role: Role) {
        self.set(n.name.span(), role);
        if let Some(t) = n.ty {
            self.ty(t);
        }
    }

    fn type_params(&mut self, params: TypeParams<'_>) {
        for p in params {
            self.set(p.name.span(), Role::Type);
            if let Some(c) = p.constraint {
                self.ty(c);
            }
        }
    }

    fn func(&mut self, f: Func<'_>) {
        self.type_params(f.type_params());
        for p in f.params() {
            self.decorators(p.decorators());
            self.set(p.name().span(), Role::Parameter);
            if let Some(t) = p.ty() {
                self.ty(t);
            }
            if let Some(d) = p.default() {
                self.expr(d);
            }
        }
        if let Some(v) = f.vararg().and_then(|v| v.ty) {
            self.vararg(v);
        }
        if let Some(r) = f.ret() {
            self.ret(r);
        }
        self.block(f.body());
    }

    fn vararg(&mut self, v: VarargType<'_>) {
        match v {
            VarargType::Type(t) | VarargType::Expand(t) => self.ty(t),
            VarargType::Pack(n) => self.set(n.span(), Role::Type),
        }
    }

    fn ret(&mut self, r: RetType<'_>) {
        match r {
            RetType::Single(t) => self.ty(t),
            RetType::List { types, vararg, .. } => {
                types.iter().for_each(|t| self.ty(t));
                vararg.into_iter().for_each(|v| self.vararg(v));
            }
            RetType::Pack(n) => self.set(n.span(), Role::Type),
            RetType::Predicate { name, ty } => {
                self.set(name.span(), Role::Parameter);
                self.ty(ty);
            }
        }
    }

    fn class(&mut self, c: ClassDecl<'_>) {
        self.set(c.name().span(), Role::Class);
        self.type_params(c.type_params());
        c.extends().into_iter().for_each(|t| self.ty(t));
        c.implements().iter().for_each(|t| self.ty(t));
        for m in c.members() {
            self.decorators(m.decorators());
            match m.kind() {
                ClassMemberKind::Field {
                    name, ty, value, ..
                } => {
                    self.set(name.span(), Role::Property);
                    self.ty(ty);
                    value.into_iter().for_each(|e| self.expr(e));
                }
                ClassMemberKind::Method { name, func, .. } => {
                    self.set(name.span(), Role::Method);
                    self.func(func);
                }
                ClassMemberKind::Getter { name, ty, body } => {
                    self.set(name.span(), Role::Property);
                    ty.into_iter().for_each(|t| self.ty(t));
                    self.block(body);
                }
                ClassMemberKind::Setter { name, param, body } => {
                    self.set(name.span(), Role::Property);
                    self.typed(param, Role::Parameter);
                    self.block(body);
                }
                ClassMemberKind::Constructor { func, .. } => self.func(func),
            }
        }
    }

    fn interface(&mut self, i: InterfaceDecl<'_>) {
        self.set(i.name().span(), Role::Class);
        self.type_params(i.type_params());
        i.extends().iter().for_each(|t| self.ty(t));
        for m in i.members() {
            match m {
                InterfaceMember::Field { name, ty, .. } => {
                    self.set(name.span(), Role::Property);
                    self.ty(ty);
                }
                InterfaceMember::Method { name, sig } => {
                    self.set(name.span(), Role::Method);
                    self.func(sig);
                }
            }
        }
    }

    fn enumeration(&mut self, e: EnumDecl<'_>) {
        self.set(e.name().span(), Role::Class);
        for item in e.items() {
            self.set(item.name.span(), Role::EnumMember);
            item.value.into_iter().for_each(|v| self.expr(v));
        }
    }

    fn alias(&mut self, a: TypeAlias<'_>) {
        self.set(a.name().span(), Role::Type);
        self.type_params(a.type_params());
        self.ty(a.ty());
    }

    fn declare(&mut self, d: Declare<'_>) {
        match d.kind() {
            DeclareKind::Function { name, sig } => {
                let path: Vec<Name<'_>> = name.path().iter().collect();
                for (i, n) in path.iter().enumerate() {
                    let last = i + 1 == path.len() && name.method().is_none();
                    self.set(
                        n.span(),
                        if last {
                            Role::Function
                        } else {
                            Role::Namespace
                        },
                    );
                }
                if let Some(m) = name.method() {
                    self.set(m.span(), Role::Method);
                }
                self.func(sig);
            }
            DeclareKind::Global { name, ty } => {
                self.set(name.span(), Role::Variable);
                self.ty(ty);
            }
            DeclareKind::Namespace { name, members } => {
                self.set(name.span(), Role::Namespace);
                members.iter().for_each(|m| self.ns_member(m));
            }
            DeclareKind::Class {
                name,
                type_params,
                extends,
                implements,
                members,
            } => {
                self.set(name.span(), Role::Class);
                self.type_params(type_params);
                extends.into_iter().for_each(|t| self.ty(t));
                implements.iter().for_each(|t| self.ty(t));
                members.iter().for_each(|m| self.decl_member(m));
            }
            DeclareKind::Interface(i) => self.interface(i),
            DeclareKind::Event { name, params } | DeclareKind::Message { name, params } => {
                self.set(name.span(), Role::Function);
                self.func(params);
            }
            DeclareKind::Module { members, .. } => members.iter().for_each(|m| self.ns_member(m)),
        }
    }

    fn ns_member(&mut self, m: NsMember<'_>) {
        self.decorators(m.decorators());
        match m.kind() {
            NsMemberKind::Function { name, sig } => {
                self.set(name.span(), Role::Function);
                self.func(sig);
            }
            NsMemberKind::Value { name, ty, .. } => {
                self.set(name.span(), Role::Variable);
                self.ty(ty);
            }
            NsMemberKind::Namespace { name, members } => {
                self.set(name.span(), Role::Namespace);
                members.iter().for_each(|m| self.ns_member(m));
            }
            NsMemberKind::Class {
                name,
                type_params,
                extends,
                members,
            } => {
                self.set(name.span(), Role::Class);
                self.type_params(type_params);
                extends.into_iter().for_each(|t| self.ty(t));
                members.iter().for_each(|m| self.decl_member(m));
            }
            NsMemberKind::Interface(i) => self.interface(i),
            NsMemberKind::TypeAlias(a) => self.alias(a),
            NsMemberKind::Enum(e) => self.enumeration(e),
        }
    }

    fn decl_member(&mut self, m: DeclMember<'_>) {
        self.decorators(m.decorators());
        match m.kind() {
            DeclMemberKind::Function { name, sig } => {
                self.set(name.span(), Role::Method);
                self.func(sig);
            }
            DeclMemberKind::Field { name, ty, .. } => {
                self.set(name.span(), Role::Property);
                self.ty(ty);
            }
        }
    }

    fn ty(&mut self, t: Type<'_>) {
        match t.kind() {
            TypeKind::Ref { path, args } => {
                let names: Vec<Name<'_>> = path.iter().collect();
                for (i, n) in names.iter().enumerate() {
                    let role = if i + 1 == names.len() {
                        Role::Type
                    } else {
                        Role::Namespace
                    };
                    self.set(n.span(), role);
                }
                for a in args {
                    match a {
                        TypeArg::Type(t) => self.ty(t),
                        TypeArg::Pack(TypePack::Named(n)) => self.set(n.span(), Role::Type),
                        TypeArg::Pack(TypePack::List { types, vararg, .. }) => {
                            types.iter().for_each(|t| self.ty(t));
                            vararg.into_iter().for_each(|v| self.vararg(v));
                        }
                    }
                }
            }
            TypeKind::Typeof(e) => self.expr(e),
            TypeKind::Table(fields) => {
                for f in fields {
                    match f {
                        TableTypeField::Named { name, ty, .. } => {
                            self.set(name.span(), Role::Property);
                            self.ty(ty);
                        }
                        TableTypeField::NamedString { ty, .. } | TableTypeField::Array(ty) => {
                            self.ty(ty)
                        }
                        TableTypeField::Indexer { key, value } => {
                            self.ty(key);
                            self.ty(value);
                        }
                    }
                }
            }
            TypeKind::Function(f) => {
                self.type_params(f.type_params());
                for p in f.params() {
                    if let Some(n) = p.name {
                        self.set(n.span(), Role::Parameter);
                    }
                    self.ty(p.ty);
                }
                f.vararg().into_iter().for_each(|v| self.vararg(v));
                self.ret(f.ret());
            }
            TypeKind::Paren(t)
            | TypeKind::Optional(t)
            | TypeKind::Array(t)
            | TypeKind::KeyOf(t) => self.ty(t),
            TypeKind::Index { object, key } => {
                self.ty(object);
                self.ty(key);
            }
            TypeKind::Union(types) | TypeKind::Intersection(types) => {
                types.iter().for_each(|t| self.ty(t))
            }
            TypeKind::Nil
            | TypeKind::True
            | TypeKind::False
            | TypeKind::AnyFunction
            | TypeKind::String(_)
            | TypeKind::Number(_)
            | TypeKind::Error => {}
        }
    }

    fn expr(&mut self, e: Expr<'_>) {
        visit::walk_expr_shallow(e, &mut |x| match x.kind() {
            ExprKind::Name(n) => self.set(n.span(), Role::Variable),
            ExprKind::Field { name, .. } | ExprKind::OptField { name, .. } => {
                self.set(name.span(), Role::Property)
            }
            ExprKind::Method { name, .. } | ExprKind::OptMethod { name, .. } => {
                self.set(name.span(), Role::Method)
            }
            ExprKind::Call { callee, .. } | ExprKind::OptCall { callee, .. } => match callee.kind()
            {
                ExprKind::Name(n) => self.set(n.span(), Role::Function),
                ExprKind::Field { name, .. } | ExprKind::OptField { name, .. } => {
                    self.set(name.span(), Role::Function)
                }
                _ => {}
            },
            ExprKind::Table(fields) => {
                for f in fields {
                    if let TableField::Named { name, .. } = f {
                        self.set(name.span(), Role::Property);
                    }
                }
            }
            ExprKind::Cast { ty, .. } => self.ty(ty),
            ExprKind::Function(f) => self.func(f),
            _ => {}
        });
    }
}

/// The file's semantic tokens, encoded as the protocol's relative
/// quintuples: types, functions, parameters, fields, decorators, and the
/// contextual keywords where wowlua's syntax reads them as keywords.
pub fn semantic_tokens(text: &str) -> Vec<u32> {
    let parsed = parse(text);
    let mut names = Names::default();
    names.block(parsed.view().block());
    let keyword = TOKEN_TYPES
        .iter()
        .position(|t| *t == "keyword")
        .unwrap_or(0) as u32;
    let lines = Lines::new(text);
    let mut data = Vec::new();
    let (mut last_line, mut last_col) = (0usize, 0usize);
    let tokens = &parsed.lexed.tokens;
    for i in 0..tokens.len() {
        let t = tokens.token(i);
        if t.kind != TokenKind::Name {
            continue;
        }
        let word = &text[t.start as usize..t.end as usize];
        let kind = match names.roles.get(&t.start) {
            Some(role) => role.token(),
            None if CONTEXTUAL.contains(&word) => Some(keyword),
            None => None,
        };
        let Some(kind) = kind else {
            continue;
        };
        let (line, col) = lines.at(t.start as usize);
        let length = word.encode_utf16().count();
        let delta_col = if line == last_line {
            col - last_col
        } else {
            col
        };
        data.extend([
            (line - last_line) as u32,
            delta_col as u32,
            length as u32,
            kind,
            0,
        ]);
        (last_line, last_col) = (line, col);
    }
    data
}

// --------------------------------------------------------- inlay hints --

/// The types of locals declared without one, from the last check's
/// hovers: after the name, `: T`. Literals and functions get none (their
/// type is plain to see), nor types too long to read at a glance.
pub fn inlay_hints(text: &str, hovers: &[check::Hover], first: usize, last: usize) -> Vec<Json> {
    let parsed = parse(text);
    let lines = Lines::new(text);
    let by_start: HashMap<(u32, u32), &str> = hovers
        .iter()
        .map(|h| ((h.start, h.end), h.ty.as_str()))
        .collect();
    let mut out = Vec::new();
    let mut hint = |name: Name<'_>| {
        let span = name.span();
        let line = lines.line(span.end as usize);
        if line < first || line > last {
            return;
        }
        let Some(ty) = by_start.get(&(span.start, span.end)) else {
            return;
        };
        if ty.is_empty() || ty.len() > 60 || *ty == "any" {
            return;
        }
        out.push(json!({
            "position": lines.position(span.end as usize),
            "label": format!(": {ty}"),
            "kind": 1,
        }));
    };
    let plain = |e: Expr<'_>| {
        matches!(
            e.kind(),
            ExprKind::Nil
                | ExprKind::True
                | ExprKind::False
                | ExprKind::Number(_)
                | ExprKind::String(_)
                | ExprKind::Interp(_)
                | ExprKind::Function(_)
        )
    };
    visit::walk_stmts(parsed.view().block(), &mut |s| match s.kind() {
        StmtKind::Local { names, values } => {
            for (i, n) in names.iter().enumerate() {
                let literal = values.get(i).is_some_and(plain);
                if n.ty.is_none() && !literal {
                    hint(n.name);
                }
            }
        }
        StmtKind::GenericFor { vars, .. } => {
            for v in vars.iter().filter(|v| v.ty.is_none()) {
                hint(v.name);
            }
        }
        _ => {}
    });
    out
}

// ------------------------------------------------------------- lenses --

/// A lens over each function the file declares: how many places refer to
/// it, counted in the last check's definitions (every checked file's,
/// which name the file `target`). Its command shows them, at the file the
/// editor calls `uri`.
pub fn reference_lenses<'a>(
    text: &str,
    uri: &str,
    target: &str,
    definitions: impl Iterator<Item = &'a check::Definition>,
) -> Vec<Json> {
    let parsed = parse(text);
    let lines = Lines::new(text);
    // Each declaration's place, as the definitions name it.
    let mut counts: HashMap<[u32; 2], usize> = HashMap::new();
    for d in definitions.filter(|d| &*d.target.uri == target) {
        *counts
            .entry([d.target.range[0], d.target.range[1]])
            .or_default() += 1;
    }
    let mut out = Vec::new();
    let mut lens = |name: Name<'_>| {
        let span = name.span();
        let (line, col) = lines.at(span.start as usize);
        let n = counts.get(&[line as u32, col as u32]).copied().unwrap_or(0);
        let title = if n == 1 {
            "1 reference".to_owned()
        } else {
            format!("{n} references")
        };
        let position = lines.position(span.start as usize);
        out.push(json!({
            "range": { "start": position, "end": lines.position(span.end as usize) },
            "command": {
                "title": title,
                "command": "wowlua.showReferences",
                "arguments": [uri, position],
            },
        }));
    };
    visit::walk_stmts(parsed.view().block(), &mut |s| {
        let s = match s.kind() {
            StmtKind::Export(inner) => inner,
            _ => s,
        };
        match s.kind() {
            StmtKind::Function { name, .. } => {
                if let Some(n) = name.method().or_else(|| name.path().last()) {
                    lens(n);
                }
            }
            StmtKind::LocalFunction { name, .. } => lens(name),
            StmtKind::Class(c) => {
                for m in c.members() {
                    if let ClassMemberKind::Method {
                        name,
                        has_body: true,
                        ..
                    } = m.kind()
                    {
                        lens(name);
                    }
                }
            }
            _ => {}
        }
    });
    out
}

// --------------------------------------------------------- quick fixes --

/// The quick fixes of the diagnostics the editor sends back: the fixes
/// each carries in its `data` (`check::convert` puts them there).
pub fn code_actions(diagnostics: &[Json]) -> Vec<Json> {
    let mut out = Vec::new();
    for d in diagnostics {
        for (i, fix) in d["data"]["fixes"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let mut changes: serde_json::Map<String, Json> = serde_json::Map::new();
            for edit in fix["edits"].as_array().into_iter().flatten() {
                let Some(uri) = edit["uri"].as_str() else {
                    continue;
                };
                let list = changes.entry(uri.to_owned()).or_insert_with(|| json!([]));
                if let Some(list) = list.as_array_mut() {
                    list.push(json!({ "range": edit["range"], "newText": edit["newText"] }));
                }
            }
            out.push(json!({
                "title": fix["title"],
                "kind": "quickfix",
                "diagnostics": [d],
                "isPreferred": i == 0,
                "edit": { "changes": changes },
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens as (line, column, length, type name).
    fn decoded(text: &str) -> Vec<(u32, u32, u32, &'static str)> {
        let data = semantic_tokens(text);
        let (mut line, mut col) = (0, 0);
        data.chunks(5)
            .map(|q| {
                if q[0] > 0 {
                    col = 0;
                }
                line += q[0];
                col += q[1];
                (line, col, q[2], TOKEN_TYPES[q[3] as usize])
            })
            .collect()
    }

    #[test]
    fn contextual_keywords_are_keywords_only_where_they_are() {
        let text = "import tokens as T from \"/Core/Tokens\"\nlocal type = 1\nprint(type)\n";
        let tokens = decoded(text);
        assert!(tokens.contains(&(0, 0, 6, "keyword")), "{tokens:?}");
        assert!(tokens.contains(&(0, 14, 2, "keyword")), "{tokens:?}");
        assert!(tokens.contains(&(0, 19, 4, "keyword")), "{tokens:?}");
        // `type` as a local's name is a name.
        assert!(!tokens.iter().any(|t| t.0 == 1), "{tokens:?}");
        // `print` is called: a function.
        assert!(tokens.contains(&(2, 0, 5, "function")), "{tokens:?}");
    }

    #[test]
    fn types_parameters_and_fields() {
        let text = "local function f(entry: ThreatEntry): string?\n  return entry.name\nend\n";
        let tokens = decoded(text);
        assert!(tokens.contains(&(0, 15, 1, "function")), "{tokens:?}");
        assert!(tokens.contains(&(0, 17, 5, "parameter")), "{tokens:?}");
        assert!(tokens.contains(&(0, 24, 11, "type")), "{tokens:?}");
        assert!(tokens.contains(&(0, 38, 6, "type")), "{tokens:?}");
        assert!(tokens.contains(&(1, 15, 4, "property")), "{tokens:?}");
    }

    #[test]
    fn classes_and_their_members() {
        let text =
            "class Bar extends Frame\n  count: number = 0\n  function Show(self)\n  end\nend\n";
        let tokens = decoded(text);
        assert!(tokens.contains(&(0, 0, 5, "keyword")), "{tokens:?}");
        assert!(tokens.contains(&(0, 6, 3, "class")), "{tokens:?}");
        assert!(tokens.contains(&(0, 10, 7, "keyword")), "{tokens:?}");
        assert!(tokens.contains(&(0, 18, 5, "type")), "{tokens:?}");
        assert!(tokens.contains(&(1, 2, 5, "property")), "{tokens:?}");
    }

    #[test]
    fn folding_keeps_the_end_in_sight() {
        let text = "-- a\n-- b\nlocal function f()\n  return 1\nend\nlocal t = {\n  1,\n}\n";
        let ranges = folding_ranges(text);
        assert!(
            ranges.contains(&json!({ "startLine": 0, "endLine": 1, "kind": "comment" })),
            "{ranges:?}"
        );
        assert!(
            ranges.contains(&json!({ "startLine": 2, "endLine": 3 })),
            "{ranges:?}"
        );
        assert!(
            ranges.contains(&json!({ "startLine": 5, "endLine": 6 })),
            "{ranges:?}"
        );
    }

    #[test]
    fn hints_show_the_types_of_locals_without_one() {
        let text = "local s = r.status\nlocal n = 1\nlocal t: number = 2\n";
        let hovers = vec![
            check::Hover {
                start: 6,
                end: 7,
                ty: "number?".into(),
            },
            check::Hover {
                start: 25,
                end: 26,
                ty: "number".into(),
            },
        ];
        let hints = inlay_hints(text, &hovers, 0, 10);
        assert_eq!(hints.len(), 1, "{hints:?}");
        assert_eq!(hints[0]["label"], ": number?");
        assert_eq!(hints[0]["position"], json!({ "line": 0, "character": 7 }));
    }

    #[test]
    fn lenses_count_the_references() {
        let text = "local function f()\nend\nf()\nf()\n";
        let uri = "file:///a.wowlua";
        let target = |line: u32, col: u32| check::Definition {
            start: 0,
            end: 0,
            target: check::Target {
                uri: std::sync::Arc::from(uri),
                range: [line, col, line, col + 1],
            },
        };
        let defs = [target(0, 15), target(0, 15)];
        let lenses = reference_lenses(text, uri, uri, defs.iter());
        assert_eq!(lenses[0]["command"]["title"], "2 references");
    }

    /// Every example of the repository: tokens inside the text, in order,
    /// only contextual keywords as keywords, and folds that fit.
    #[test]
    #[allow(clippy::disallowed_methods)] // (the examples are files)
    fn the_examples_are_tokenized_and_folded() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut stack = vec![root];
        let mut seen = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "wowlua") {
                    seen += 1;
                    let text = std::fs::read_to_string(&path).unwrap();
                    let lines: Vec<&str> = text.split('\n').collect();
                    for (line, col, len, kind) in decoded(&text) {
                        let row = lines[line as usize];
                        assert!(
                            (col + len) as usize <= row.encode_utf16().count(),
                            "{}:{line}: a token past the line",
                            path.display()
                        );
                        if kind == "keyword" {
                            let word: String =
                                row.chars().skip(col as usize).take(len as usize).collect();
                            assert!(
                                CONTEXTUAL.contains(&word.as_str()),
                                "{}:{line}: `{word}`",
                                path.display()
                            );
                        }
                    }
                    for r in folding_ranges(&text) {
                        assert!(
                            r["endLine"].as_u64().unwrap() < lines.len() as u64,
                            "{}",
                            path.display()
                        );
                    }
                }
            }
        }
        assert!(seen > 0, "no examples found");
    }

    #[test]
    fn fixes_become_code_actions() {
        let d = json!({
            "message": "x",
            "data": { "fixes": [{ "title": "change to role", "edits": [{
                "uri": "file:///a.wowlua",
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } },
                "newText": "role",
            }] }] },
        });
        let actions = code_actions(&[d]);
        assert_eq!(actions[0]["title"], "change to role");
        assert_eq!(
            actions[0]["edit"]["changes"]["file:///a.wowlua"][0]["newText"],
            "role"
        );
    }
}
