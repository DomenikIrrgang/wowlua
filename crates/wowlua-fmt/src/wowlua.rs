//! wowlua's own syntax (spec tooling §16.3, milestone M9 S6): types and
//! annotations, generics, decorators, classes, interfaces, enums, type
//! aliases, `declare` statements, imports and exports, `const`,
//! `continue`, compound assignment, destructuring, optional chaining, `??`,
//! casts and interpolated strings (which keep their text).

use crate::doc::Doc;
use crate::format::Formatter;
use wowlua_syntax::view::*;

fn txt(s: &str) -> Doc {
    Doc::text(s)
}

fn hard() -> Doc {
    Doc::HardLine
}

pub(crate) fn compound_op(op: CompoundOp) -> &'static str {
    match op {
        CompoundOp::Add => "+=",
        CompoundOp::Sub => "-=",
        CompoundOp::Mul => "*=",
        CompoundOp::Div => "/=",
        CompoundOp::IDiv => "//=",
        CompoundOp::Mod => "%=",
        CompoundOp::Pow => "^=",
        CompoundOp::Concat => "..=",
        CompoundOp::BAnd => "&=",
        CompoundOp::BOr => "|=",
        CompoundOp::Shl => "<<=",
        CompoundOp::Shr => ">>=",
        CompoundOp::Coalesce => "??=",
    }
}

fn modifier(m: Modifier) -> &'static str {
    match m {
        Modifier::Public => "public",
        Modifier::Private => "private",
        Modifier::Protected => "protected",
        Modifier::Static => "static",
        Modifier::Readonly => "readonly",
        Modifier::Override => "override",
        Modifier::Abstract => "abstract",
        Modifier::Overload => "overload",
    }
}

impl Formatter<'_> {
    // ------------------------------------------------------------- types --

    pub(crate) fn ty(&mut self, t: Type<'_>) -> Doc {
        let start = t.span().start;
        match t.kind() {
            TypeKind::Nil => self.tok("nil", start),
            TypeKind::True => self.tok("true", start),
            TypeKind::False => self.tok("false", start),
            TypeKind::AnyFunction => self.tok("function", start),
            TypeKind::String(s) => {
                let text = self.string(s);
                self.tok(&text, start)
            }
            TypeKind::Number(n) => self.tok(n.raw().as_bytes(), start),
            TypeKind::Ref { path, args } => {
                let mut parts = Vec::new();
                for (i, n) in path.iter().enumerate() {
                    if i > 0 {
                        parts.push(txt("."));
                    }
                    parts.push(self.name(n));
                }
                if !args.is_empty() {
                    parts.push(txt("<"));
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            parts.push(txt(", "));
                        }
                        parts.push(self.type_arg(a));
                    }
                    parts.push(txt(">"));
                }
                Doc::concat(parts)
            }
            TypeKind::Typeof(e) => {
                let k = self.tok("typeof", start);
                Doc::concat(vec![k, txt("("), self.expr(e), txt(")")])
            }
            TypeKind::Table(fields) => self.table_type(fields, t.span()),
            TypeKind::Function(f) => self.function_type(f, start),
            TypeKind::Paren(inner) => {
                let open = self.tok("(", start);
                Doc::concat(vec![open, self.ty(inner), txt(")")])
            }
            TypeKind::Optional(inner) => Doc::concat(vec![self.ty(inner), txt("?")]),
            TypeKind::Array(inner) => Doc::concat(vec![self.ty(inner), txt("[]")]),
            TypeKind::Union(members) | TypeKind::Intersection(members) => {
                let sep = if matches!(t.kind(), TypeKind::Union(_)) {
                    "|"
                } else {
                    "&"
                };
                let mut parts = Vec::new();
                for (i, m) in members.iter().enumerate() {
                    if i > 0 {
                        parts.push(Doc::Line);
                        parts.push(txt(sep));
                        parts.push(txt(" "));
                    }
                    parts.push(self.ty(m));
                }
                let first = parts.remove(0);
                Doc::group(Doc::concat(vec![first, Doc::indent(Doc::concat(parts))]))
            }
            TypeKind::Error => txt(""),
        }
    }

    fn type_arg(&mut self, a: TypeArg<'_>) -> Doc {
        match a {
            TypeArg::Type(t) => self.ty(t),
            TypeArg::Pack(p) => self.type_pack(p),
        }
    }

    fn type_pack(&mut self, p: TypePack<'_>) -> Doc {
        match p {
            TypePack::Named(n) => Doc::concat(vec![self.name(n), txt("...")]),
            TypePack::List {
                types,
                vararg,
                span,
            } => {
                let open = self.tok("(", span.start);
                let mut parts = vec![open];
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        parts.push(txt(", "));
                    }
                    parts.push(self.ty(t));
                }
                if let Some(v) = vararg {
                    if !types.is_empty() {
                        parts.push(txt(", "));
                    }
                    parts.push(self.vararg_type(v, false));
                }
                parts.push(txt(")"));
                Doc::concat(parts)
            }
        }
    }

    /// `...: T`, `...: T...` (a parameter's), or `T...` / `...T` in a list.
    fn vararg_type(&mut self, v: VarargType<'_>, param: bool) -> Doc {
        match v {
            VarargType::Type(t) if param => Doc::concat(vec![txt("...: "), self.ty(t)]),
            VarargType::Type(t) => Doc::concat(vec![txt("..."), self.ty(t)]),
            VarargType::Pack(n) if param => {
                Doc::concat(vec![txt("...: "), self.name(n), txt("...")])
            }
            VarargType::Pack(n) => Doc::concat(vec![self.name(n), txt("...")]),
            VarargType::Expand(t) => Doc::concat(vec![self.ty(t), txt("...")]),
        }
    }

    fn table_type(&mut self, fields: TableTypeFields<'_>, span: wowlua_span::Span) -> Doc {
        let open = self.tok("{", span.start);
        let close_offset = span.end.saturating_sub(1);
        if fields.is_empty() {
            let close = self.tok("}", close_offset);
            return Doc::concat(vec![open, close]);
        }
        let mut parts = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                parts.push(txt(","));
                parts.push(Doc::Line);
            }
            let doc = match f {
                TableTypeField::Named {
                    name,
                    optional,
                    readonly,
                    ty,
                } => {
                    let mut p = Vec::new();
                    if readonly {
                        p.push(self.tok("readonly", self.prev_token(name.span().start)));
                        p.push(txt(" "));
                    }
                    p.push(self.name(name));
                    p.push(txt(if optional { "?: " } else { ": " }));
                    p.push(self.ty(ty));
                    Doc::concat(p)
                }
                TableTypeField::NamedString {
                    key,
                    optional,
                    readonly,
                    ty,
                    span,
                } => {
                    let open_b = if readonly {
                        let word = self.tok("readonly", span.start);
                        Doc::concat(vec![word, txt(" [")])
                    } else {
                        self.tok("[", span.start)
                    };
                    let k = self.string(key);
                    let colon = if optional { "]?: " } else { "]: " };
                    Doc::concat(vec![open_b, Doc::text(k), txt(colon), self.ty(ty)])
                }
                TableTypeField::Indexer { key, value } => {
                    let open_b = self.tok("[", self.prev_token(key.span().start));
                    Doc::concat(vec![open_b, self.ty(key), txt("]: "), self.ty(value)])
                }
                TableTypeField::Array(t) => self.ty(t),
            };
            parts.push(doc);
        }
        let close = self.tok("}", close_offset);
        Doc::group(Doc::concat(vec![
            open,
            Doc::indent(Doc::concat(vec![
                Doc::Line,
                Doc::concat(parts),
                Doc::if_break(txt(","), txt("")),
            ])),
            Doc::Line,
            close,
        ]))
    }

    fn function_type(&mut self, f: FunctionType<'_>, start: u32) -> Doc {
        let mut parts = Vec::new();
        if !f.type_params().is_empty() {
            parts.push(self.type_params(f.type_params()));
        }
        parts.push(self.tok(
            "(",
            if f.type_params().is_empty() {
                start
            } else {
                self.after_type_params(start)
            },
        ));
        for (i, p) in f.params().iter().enumerate() {
            if i > 0 {
                parts.push(txt(", "));
            }
            if let Some(n) = &p.name {
                parts.push(self.name(*n));
                parts.push(txt(": "));
            }
            parts.push(self.ty(p.ty));
        }
        if let Some(v) = &f.vararg() {
            if !f.params().is_empty() {
                parts.push(txt(", "));
            }
            // A function type's rest is `...T` (as written in Luau).
            parts.push(self.vararg_type(*v, false));
        }
        parts.push(txt(") -> "));
        parts.push(self.ret_type(f.ret(), true));
        Doc::concat(parts)
    }

    /// The `(` after a function type's type parameters.
    fn after_type_params(&self, start: u32) -> u32 {
        let tokens = &self.parse.lexed.tokens;
        let i = tokens.first_from(start);
        let mut depth = 0i32;
        for t in tokens.iter().skip(i) {
            match &self.src[t.start as usize..t.end as usize] {
                b"<" => depth += 1,
                b">" => {
                    depth -= 1;
                    if depth == 0 {
                        return self.next_token(t.end);
                    }
                }
                _ => {}
            }
        }
        start
    }

    /// A return type; `arrow`: after `->` (a function type's), where a
    /// single type needs no parentheses either.
    fn ret_type(&mut self, r: RetType<'_>, _arrow: bool) -> Doc {
        match r {
            RetType::Single(t) => self.ty(t),
            // `...T` alone needs no parentheses.
            RetType::List {
                types,
                vararg: Some(v),
                ..
            } if types.is_empty() => self.vararg_type(v, false),
            RetType::List {
                types,
                vararg,
                span,
            } => {
                let open = self.tok("(", span.start);
                let mut parts = vec![open];
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        parts.push(txt(", "));
                    }
                    parts.push(self.ty(t));
                }
                if let Some(v) = vararg {
                    if !types.is_empty() {
                        parts.push(txt(", "));
                    }
                    parts.push(self.vararg_type(v, false));
                }
                parts.push(txt(")"));
                Doc::concat(parts)
            }
            RetType::Pack(n) => Doc::concat(vec![self.name(n), txt("...")]),
            RetType::Predicate { name, ty } => {
                Doc::concat(vec![self.name(name), txt(" is "), self.ty(ty)])
            }
        }
    }

    pub(crate) fn type_params(&mut self, ps: TypeParams<'_>) -> Doc {
        if ps.is_empty() {
            return txt("");
        }
        let mut parts = vec![txt("<")];
        for (i, p) in ps.iter().enumerate() {
            if i > 0 {
                parts.push(txt(", "));
            }
            parts.push(self.name(p.name));
            if p.pack {
                parts.push(txt("..."));
            }
            if let Some(c) = &p.constraint {
                parts.push(txt(" extends "));
                parts.push(self.ty(*c));
            }
        }
        parts.push(txt(">"));
        Doc::concat(parts)
    }

    /// `name` or `name: T`.
    pub(crate) fn typed_name(&mut self, n: TypedName<'_>) -> Doc {
        let name = self.name(n.name);
        match &n.ty {
            Some(t) => Doc::concat(vec![name, txt(": "), self.ty(*t)]),
            None => name,
        }
    }

    /// A function's signature: type parameters, parameters (with types,
    /// defaults and decorators), `...` and the return type.
    pub(crate) fn signature(&mut self, f: Func<'_>) -> Doc {
        let mut parts = vec![self.type_params(f.type_params())];
        parts.push(self.tok("(", f.params_span().start));
        for (i, p) in f.params().iter().enumerate() {
            if i > 0 {
                parts.push(txt(", "));
            }
            for d in &p.decorators() {
                parts.push(self.decorator(d));
                parts.push(txt(" "));
            }
            for m in p.modifiers() {
                parts.push(txt(modifier(m)));
                parts.push(txt(" "));
            }
            parts.push(self.name(p.name()));
            if let Some(t) = &p.ty() {
                parts.push(txt(": "));
                parts.push(self.ty(*t));
            }
            if let Some(d) = &p.default() {
                parts.push(txt(" = "));
                parts.push(self.expr(*d));
            }
        }
        if let Some(v) = &f.vararg() {
            if !f.params().is_empty() {
                parts.push(txt(", "));
            }
            parts.push(self.tok("...", v.span.start));
            match &v.ty {
                Some(VarargType::Type(t)) => {
                    parts.push(txt(": "));
                    parts.push(self.ty(*t));
                }
                Some(VarargType::Pack(n)) => {
                    parts.push(txt(": "));
                    parts.push(self.name(*n));
                    parts.push(txt("..."));
                }
                Some(VarargType::Expand(t)) => {
                    parts.push(txt(": "));
                    parts.push(self.ty(*t));
                    parts.push(txt("..."));
                }
                None => {}
            }
        }
        parts.push(self.tok(")", f.params_span().end.saturating_sub(1)));
        if let Some(r) = &f.ret() {
            parts.push(txt(": "));
            parts.push(self.ret_type(*r, false));
        }
        Doc::concat(parts)
    }

    // -------------------------------------------------------- decorators --

    pub(crate) fn decorator(&mut self, d: Decorator<'_>) -> Doc {
        let at = self.tok("@", d.span().start);
        let mut parts = vec![at];
        for (i, n) in d.path().iter().enumerate() {
            if i > 0 {
                parts.push(txt("."));
            }
            parts.push(self.name(n));
        }
        if let Some(a) = &d.args() {
            parts.push(self.args(*a));
        }
        Doc::concat(parts)
    }

    /// A statement's decorators, each on its own line.
    pub(crate) fn decorators(&mut self, ds: Decorators<'_>) -> Doc {
        let mut parts = Vec::new();
        for d in ds {
            parts.push(self.decorator(d));
            parts.push(hard());
        }
        Doc::concat(parts)
    }

    // ------------------------------------------------------- statements --

    /// The statements only wowlua has.
    pub(crate) fn wowlua_stmt(&mut self, s: Stmt<'_>) -> Doc {
        let start = s.span().start;
        match s.kind() {
            StmtKind::Compound { target, op, value } => {
                let t = self.expr(target);
                let o = self.tok(compound_op(op), self.next_token(target.span().end));
                Doc::concat(vec![t, txt(" "), o, txt(" "), self.expr(value)])
            }
            StmtKind::Continue => self.tok("continue", start),
            StmtKind::Const { name, ty, value } => {
                let mut parts = vec![self.tok("const", start), txt(" "), self.name(name)];
                if let Some(t) = ty {
                    parts.push(txt(": "));
                    parts.push(self.ty(t));
                }
                if let Some(v) = value {
                    parts.push(txt(" = "));
                    parts.push(self.expr(v));
                }
                Doc::concat(parts)
            }
            StmtKind::LocalDestructure { fields, ty, value } => {
                let mut parts = vec![self.tok("local", start), txt(" { ")];
                for (i, f) in fields.iter().enumerate() {
                    if i > 0 {
                        parts.push(txt(", "));
                    }
                    parts.push(self.name(f.name));
                    if let Some(d) = &f.default {
                        parts.push(txt(" = "));
                        parts.push(self.expr(*d));
                    }
                }
                parts.push(txt(" }"));
                if let Some(t) = ty {
                    parts.push(txt(": "));
                    parts.push(self.ty(t));
                }
                if let Some(v) = value {
                    parts.push(txt(" = "));
                    parts.push(self.expr(v));
                }
                Doc::concat(parts)
            }
            StmtKind::Class(c) => self.class(c, start),
            StmtKind::Interface(i) => {
                let k = self.tok("interface", start);
                Doc::concat(vec![k, self.interface_body(i, s.span().end)])
            }
            StmtKind::Enum(e) => self.enum_decl(e, start),
            StmtKind::TypeAlias(a) => {
                let k = self.tok("type", start);
                let n = self.name(a.name());
                let tp = self.type_params(a.type_params());
                Doc::concat(vec![k, txt(" "), n, tp, txt(" = "), self.ty(a.ty())])
            }
            StmtKind::Import(i) => self.import(i, start),
            StmtKind::Export(inner) => {
                let k = self.tok("export", start);
                Doc::concat(vec![k, txt(" "), self.stmt(inner)])
            }
            StmtKind::Declare(d) => {
                // From `declare` (the decorators are printed already).
                let from = match s.decorators().last() {
                    Some(d) => self.next_token(d.span().end),
                    None => start,
                };
                self.declare(d, from, s.span().end)
            }
            _ => txt(""),
        }
    }

    fn import(&mut self, i: Import<'_>, start: u32) -> Doc {
        let mut parts = vec![self.tok("import", start), txt(" ")];
        if i.type_only() {
            parts.push(txt("type "));
        }
        if i.internal() {
            parts.push(txt("internal "));
        }
        match &i.clause() {
            ImportClause::Namespace(n) => {
                parts.push(txt("* as "));
                parts.push(self.name(*n));
            }
            ImportClause::SideEffect => parts.push(txt("_")),
            ImportClause::Names(specs) => {
                for (k, spec) in specs.iter().enumerate() {
                    if k > 0 {
                        parts.push(txt(", "));
                    }
                    parts.push(self.name(spec.name));
                    if let Some(a) = &spec.alias {
                        parts.push(txt(" as "));
                        parts.push(self.name(*a));
                    }
                }
            }
        }
        parts.push(txt(" from "));
        let text = self.string(i.from());
        parts.push(Doc::text(text));
        Doc::concat(parts)
    }

    fn enum_decl(&mut self, e: EnumDecl<'_>, start: u32) -> Doc {
        let k = self.tok("enum", start);
        let n = self.name(e.name());
        let open = self.tok("{", self.next_token(e.name().span().end));
        if e.items().is_empty() {
            return Doc::concat(vec![k, txt(" "), n, txt(" "), open, txt("}")]);
        }
        let mut parts = Vec::new();
        for (i, item) in e.items().iter().enumerate() {
            if i > 0 {
                parts.push(txt(","));
                parts.push(self.trailing_before(item.name.span().start));
                parts.push(Doc::Line);
            }
            parts.push(self.name(item.name));
            if let Some(v) = &item.value {
                parts.push(txt(" = "));
                parts.push(self.expr(*v));
            }
        }
        let last_end = e
            .items()
            .last()
            .map(|it| {
                it.value
                    .as_ref()
                    .map_or(it.name.span().end, |v| v.span().end)
            })
            .unwrap_or(start);
        let close_offset = self.close_brace_after(last_end);
        let inner = self.inner_comments(close_offset);
        let close = self.tok("}", close_offset);
        Doc::concat(vec![
            k,
            txt(" "),
            n,
            txt(" "),
            Doc::group(Doc::concat(vec![
                open,
                Doc::indent(Doc::concat(vec![
                    Doc::Line,
                    Doc::concat(parts),
                    Doc::if_break(txt(","), txt("")),
                    inner.unwrap_or_else(|| txt("")),
                ])),
                Doc::Line,
                close,
            ])),
        ])
    }

    /// The first `}` at or after `offset`.
    fn close_brace_after(&self, offset: u32) -> u32 {
        let tokens = &self.parse.lexed.tokens;
        let i = tokens.first_from(offset);
        tokens
            .iter()
            .skip(i)
            .find(|t| &self.src[t.start as usize..t.end as usize] == b"}")
            .map_or(offset, |t| t.start)
    }

    /// `name<T> extends A, B` and the members, then `end` (at the last
    /// token before `end_offset`).
    fn interface_body(&mut self, i: InterfaceDecl<'_>, end_offset: u32) -> Doc {
        let mut parts = vec![
            txt(" "),
            self.name(i.name()),
            self.type_params(i.type_params()),
        ];
        if !i.extends().is_empty() {
            parts.push(txt(" extends "));
            for (k, t) in i.extends().iter().enumerate() {
                if k > 0 {
                    parts.push(txt(", "));
                }
                parts.push(self.ty(t));
            }
        }
        let mut body = Vec::new();
        let mut prev_end: Option<u32> = None;
        for m in &i.members() {
            let (start, end) = match m {
                InterfaceMember::Field {
                    name, ty, readonly, ..
                } => (
                    if readonly {
                        self.prev_token(name.span().start)
                    } else {
                        name.span().start
                    },
                    ty.span().end,
                ),
                InterfaceMember::Method { name, sig } => (
                    self.prev_token(name.span().start),
                    sig.params_span().end.max(sig.span().end),
                ),
            };
            if let Some(p) = prev_end {
                body.push(self.trailing_before(start));
                body.push(hard());
                if self.blank_line_between(p, start) {
                    body.push(hard());
                }
            } else {
                body.push(hard());
            }
            let doc = match m {
                InterfaceMember::Field {
                    readonly,
                    name,
                    optional,
                    ty,
                } => {
                    let mut p = Vec::new();
                    if readonly {
                        p.push(self.tok("readonly", start));
                        p.push(txt(" "));
                    }
                    p.push(self.name(name));
                    p.push(txt(if optional { "?: " } else { ": " }));
                    p.push(self.ty(ty));
                    Doc::concat(p)
                }
                InterfaceMember::Method { name, sig } => {
                    let f = self.tok("function", start);
                    let n = self.name(name);
                    Doc::concat(vec![f, txt(" "), n, self.signature(sig)])
                }
            };
            body.push(doc);
            prev_end = Some(end);
        }
        let end = self.prev_token(end_offset);
        let trailing = self.trailing_before(end);
        let inner = self.inner_comments(end);
        parts.push(Doc::indent(Doc::concat(vec![
            Doc::concat(body),
            trailing,
            inner.unwrap_or_else(|| txt("")),
        ])));
        parts.push(hard());
        parts.push(self.tok("end", end));
        Doc::concat(parts)
    }

    fn class(&mut self, c: ClassDecl<'_>, start: u32) -> Doc {
        let mut parts = Vec::new();
        if c.is_abstract() {
            parts.push(self.tok("abstract", start));
            parts.push(txt(" "));
            parts.push(self.tok("class", self.next_token(start + 1)));
        } else {
            parts.push(self.tok("class", start));
        }
        parts.push(txt(" "));
        parts.push(self.name(c.name()));
        parts.push(self.type_params(c.type_params()));
        if let Some(e) = &c.extends() {
            parts.push(txt(" extends "));
            parts.push(self.ty(*e));
        }
        if !c.implements().is_empty() {
            parts.push(txt(" implements "));
            for (k, t) in c.implements().iter().enumerate() {
                if k > 0 {
                    parts.push(txt(", "));
                }
                parts.push(self.ty(t));
            }
        }
        let mut body = Vec::new();
        for (k, m) in c.members().iter().enumerate() {
            if k > 0 {
                body.push(self.trailing_before(m.span().start));
                body.push(hard());
                if self.blank_line_between(c.members().at(k - 1).span().end, m.span().start) {
                    body.push(hard());
                }
            } else {
                body.push(self.trailing_before(m.span().start));
                body.push(hard());
            }
            body.push(self.class_member(m));
        }
        let end = c.end_span().start;
        let trailing = self.trailing_before(end);
        let inner = self.inner_comments(end);
        parts.push(Doc::indent(Doc::concat(vec![
            Doc::concat(body),
            trailing,
            inner.unwrap_or_else(|| txt("")),
        ])));
        parts.push(hard());
        parts.push(self.tok("end", end));
        Doc::concat(parts)
    }

    fn class_member(&mut self, m: ClassMember<'_>) -> Doc {
        let mut parts = vec![self.decorators(m.decorators())];
        // The modifiers' tokens come before the member's own.
        let first = match m.kind() {
            ClassMemberKind::Field { name, .. } => name.span().start,
            _ => m.span().start,
        };
        let mut offset = if m.decorators().is_empty() {
            m.span().start
        } else {
            self.next_token(
                m.decorators()
                    .last()
                    .map_or(m.span().start, |d| d.span().end),
            )
        };
        for md in m.modifiers() {
            parts.push(self.tok(modifier(*md), offset));
            parts.push(txt(" "));
            offset = self.next_token(offset + 1);
        }
        let _ = first;
        match m.kind() {
            ClassMemberKind::Field {
                name,
                optional,
                ty,
                value,
            } => {
                parts.push(self.name(name));
                parts.push(txt(if optional { "?: " } else { ": " }));
                parts.push(self.ty(ty));
                if let Some(v) = value {
                    parts.push(txt(" = "));
                    parts.push(self.expr(v));
                }
            }
            ClassMemberKind::Method {
                name,
                func,
                has_body,
            } => {
                parts.push(self.tok("function", offset));
                parts.push(txt(" "));
                parts.push(self.name(name));
                if has_body {
                    parts.push(self.func_body(func));
                } else {
                    parts.push(self.signature(func));
                }
            }
            ClassMemberKind::Constructor { func, has_body } => {
                parts.push(self.tok("constructor", offset));
                if has_body {
                    parts.push(self.func_body(func));
                } else {
                    parts.push(self.signature(func));
                }
            }
            ClassMemberKind::Getter { name, ty, body } => {
                parts.push(self.tok("get", offset));
                parts.push(txt(" "));
                parts.push(self.name(name));
                parts.push(txt("()"));
                if let Some(t) = ty {
                    parts.push(txt(": "));
                    parts.push(self.ty(t));
                }
                let end = self.prev_token(m.span().end);
                parts.push(self.block(body, "end", end));
            }
            ClassMemberKind::Setter { name, param, body } => {
                parts.push(self.tok("set", offset));
                parts.push(txt(" "));
                parts.push(self.name(name));
                parts.push(txt("("));
                parts.push(self.typed_name(param));
                parts.push(txt(")"));
                let end = self.prev_token(m.span().end);
                parts.push(self.block(body, "end", end));
            }
        }
        Doc::concat(parts)
    }

    /// `declare …` from the `declare` at `start`; `end`: the statement's.
    fn declare(&mut self, d: Declare<'_>, start: u32, end: u32) -> Doc {
        let k = self.tok("declare", start);
        let word = self.next_token(start + 1);
        let close = self.prev_token(end);
        match d.kind() {
            DeclareKind::Function { name, sig } => {
                let f = self.tok("function", word);
                let n = self.func_name(name);
                Doc::concat(vec![k, txt(" "), f, txt(" "), n, self.signature(sig)])
            }
            DeclareKind::Global { name, ty } => {
                let g = self.tok("global", word);
                let n = self.name(name);
                Doc::concat(vec![k, txt(" "), g, txt(" "), n, txt(": "), self.ty(ty)])
            }
            DeclareKind::Namespace { name, members } => {
                let w = self.tok("namespace", word);
                let n = self.name(name);
                let body = self.ns_members(members, close);
                Doc::concat(vec![k, txt(" "), w, txt(" "), n, body])
            }
            DeclareKind::Class {
                name,
                type_params,
                extends,
                implements,
                members,
            } => {
                let w = self.tok("class", word);
                let head = self.class_head(name, type_params, extends, implements);
                let body = self.decl_members(members, close);
                Doc::concat(vec![k, txt(" "), w, head, body])
            }
            DeclareKind::Interface(i) => {
                let w = self.tok("interface", word);
                Doc::concat(vec![k, txt(" "), w, self.interface_body(i, end)])
            }
            DeclareKind::Event { name, params } | DeclareKind::Message { name, params } => {
                let kind = if matches!(d.kind(), DeclareKind::Event { .. }) {
                    "event"
                } else {
                    "message"
                };
                let w = self.tok(kind, word);
                let n = self.name(name);
                Doc::concat(vec![k, txt(" "), w, txt(" "), n, self.signature(params)])
            }
            DeclareKind::Module { path, members } => {
                let w = self.tok("module", word);
                let p = self.string(path);
                let body = self.ns_members(members, close);
                Doc::concat(vec![k, txt(" "), w, txt(" "), Doc::text(p), body])
            }
        }
    }

    /// ` Name<T> extends A implements B, C`.
    fn class_head(
        &mut self,
        name: Name<'_>,
        type_params: TypeParams<'_>,
        extends: Option<Type<'_>>,
        implements: Types<'_>,
    ) -> Doc {
        let mut parts = vec![txt(" "), self.name(name), self.type_params(type_params)];
        if let Some(e) = extends {
            parts.push(txt(" extends "));
            parts.push(self.ty(e));
        }
        if !implements.is_empty() {
            parts.push(txt(" implements "));
            for (i, t) in implements.iter().enumerate() {
                if i > 0 {
                    parts.push(txt(", "));
                }
                parts.push(self.ty(t));
            }
        }
        Doc::concat(parts)
    }

    /// Members, one per line with their blank lines, then `end` at `close`.
    fn members<M: Copy>(
        &mut self,
        members: &[M],
        span: impl Fn(M) -> wowlua_span::Span,
        mut print: impl FnMut(&mut Self, M) -> Doc,
        close: u32,
    ) -> Doc {
        let mut body = Vec::new();
        for (i, &m) in members.iter().enumerate() {
            let sp = span(m);
            body.push(self.trailing_before(sp.start));
            body.push(hard());
            if i > 0 && self.blank_line_between(span(members[i - 1]).end, sp.start) {
                body.push(hard());
            }
            body.push(print(self, m));
        }
        let trailing = self.trailing_before(close);
        let inner = self.inner_comments(close);
        Doc::concat(vec![
            Doc::indent(Doc::concat(vec![
                Doc::concat(body),
                trailing,
                inner.unwrap_or_else(|| txt("")),
            ])),
            hard(),
            self.tok("end", close),
        ])
    }

    fn ns_members(&mut self, members: NsMembers<'_>, close: u32) -> Doc {
        let members: Vec<_> = members.iter().collect();
        self.members(&members, |m| m.span(), |f, m| f.ns_member(m), close)
    }

    fn ns_member(&mut self, m: NsMember<'_>) -> Doc {
        let mut parts = vec![self.decorators(m.decorators())];
        let kw = match m.decorators().last() {
            Some(d) => self.next_token(d.span().end),
            None => m.span().start,
        };
        let close = self.prev_token(m.span().end);
        match m.kind() {
            NsMemberKind::Function { name, sig } => {
                parts.push(self.tok("function", kw));
                parts.push(txt(" "));
                parts.push(self.name(name));
                parts.push(self.signature(sig));
            }
            NsMemberKind::Value { is_const, name, ty } => {
                if is_const {
                    parts.push(self.tok("const", kw));
                    parts.push(txt(" "));
                }
                parts.push(self.name(name));
                parts.push(txt(": "));
                parts.push(self.ty(ty));
            }
            NsMemberKind::Namespace { name, members } => {
                parts.push(self.tok("namespace", kw));
                parts.push(txt(" "));
                parts.push(self.name(name));
                parts.push(self.ns_members(members, close));
            }
            NsMemberKind::Class {
                name,
                type_params,
                extends,
                members,
            } => {
                parts.push(self.tok("class", kw));
                parts.push(self.class_head(name, type_params, extends, Types::default()));
                parts.push(self.decl_members(members, close));
            }
            NsMemberKind::Interface(i) => {
                parts.push(self.tok("interface", kw));
                parts.push(self.interface_body(i, m.span().end));
            }
            NsMemberKind::TypeAlias(a) => {
                parts.push(self.tok("type", kw));
                parts.push(txt(" "));
                parts.push(self.name(a.name()));
                parts.push(self.type_params(a.type_params()));
                parts.push(txt(" = "));
                parts.push(self.ty(a.ty()));
            }
            NsMemberKind::Enum(e) => parts.push(self.enum_decl(e, kw)),
        }
        Doc::concat(parts)
    }

    fn decl_members(&mut self, members: DeclMembers<'_>, close: u32) -> Doc {
        let members: Vec<_> = members.iter().collect();
        self.members(&members, |m| m.span(), |f, m| f.decl_member(m), close)
    }

    fn decl_member(&mut self, m: DeclMember<'_>) -> Doc {
        let mut parts = vec![self.decorators(m.decorators())];
        let mut kw = match m.decorators().last() {
            Some(d) => self.next_token(d.span().end),
            None => m.span().start,
        };
        if m.is_static() {
            parts.push(self.tok("static", kw));
            parts.push(txt(" "));
            kw = self.next_token(kw + 1);
        }
        match m.kind() {
            DeclMemberKind::Function { name, sig } => {
                parts.push(self.tok("function", kw));
                parts.push(txt(" "));
                parts.push(self.name(name));
                parts.push(self.signature(sig));
            }
            DeclMemberKind::Field {
                readonly,
                name,
                optional,
                ty,
            } => {
                if readonly {
                    parts.push(self.tok("readonly", kw));
                    parts.push(txt(" "));
                }
                parts.push(self.name(name));
                parts.push(txt(if optional { "?: " } else { ": " }));
                parts.push(self.ty(ty));
            }
        }
        Doc::concat(parts)
    }

    /// The source text of `span` as it is, with the comments in it.
    pub(crate) fn verbatim(&mut self, span: wowlua_span::Span) -> Doc {
        let mut parts = self.comments_before(span.start);
        parts.push(Doc::text(&self.src[span.start as usize..span.end as usize]));
        self.skip_comments_before(span.end);
        parts.push(Doc::BreakParent);
        Doc::concat(parts)
    }

    // ------------------------------------------------------ expressions --

    /// The expressions only wowlua has.
    pub(crate) fn wowlua_expr(&mut self, e: Expr<'_>) -> Doc {
        let start = e.span().start;
        match e.kind() {
            // Interpolated strings keep their text.
            ExprKind::Interp(_) => self.verbatim(e.span()),
            ExprKind::OptField { obj, name } => {
                let o = self.expr(obj);
                let q = self.tok("?.", self.next_token(obj.span().end));
                Doc::concat(vec![o, q, self.name(name)])
            }
            ExprKind::OptIndex { obj, key } => {
                let o = self.expr(obj);
                let q = self.tok("?.[", self.next_token(obj.span().end));
                let k = self.expr(key);
                let close = self.tok("]", self.next_token(key.span().end));
                Doc::concat(vec![o, q, k, close])
            }
            ExprKind::OptMethod { obj, name, args } => {
                let o = self.expr(obj);
                let q = self.tok("?:", self.next_token(obj.span().end));
                let n = self.name(name);
                Doc::concat(vec![o, q, n, self.args(args)])
            }
            ExprKind::OptCall { callee, args } => {
                let c = self.expr(callee);
                let q = self.tok("?.", self.next_token(callee.span().end));
                // `f?.(x)`: the arguments' parentheses follow.
                Doc::concat(vec![c, q, self.args(args)])
            }
            ExprKind::Cast { expr, ty } => {
                let x = self.expr(expr);
                let c = self.tok("::", self.next_token(expr.span().end));
                Doc::concat(vec![x, txt(" "), c, txt(" "), self.ty(ty)])
            }
            _ => self.tok("", start),
        }
    }
}
