//! Classes, widget classes and enums (spec chapter 3): hoisting, the class
//! table and its members, constructors (`__init`, or `__ctor` for widget
//! classes), `super`, accessors, finalization, static initializers, and
//! enum tables with inlined members.

use super::decorate::{Decorations, Target};
use super::*;
use wowlua_sema::classes::{self as sema_classes, Base};

/// The class whose body is being lowered.
pub(crate) struct ClassCtx {
    pub name: String,
    /// The superclass value (`None` for base classes and root widget
    /// classes).
    pub base: Option<ir::Expr>,
    /// The class extends a native widget type directly.
    pub widget_root: bool,
    /// The member being lowered is static.
    pub is_static: bool,
}

/// `e` placed at `pos`: the expression and its leftmost operands (whose
/// first token decides the line in preserve mode).
fn at(mut e: ir::Expr, pos: Pos) -> ir::Expr {
    fn leftmost(e: &mut ir::Expr, pos: Pos) {
        e.pos = pos;
        match &mut e.kind {
            ir::ExprKind::Field { obj, .. }
            | ir::ExprKind::Index { obj, .. }
            | ir::ExprKind::Method { obj, .. } => leftmost(obj, pos),
            ir::ExprKind::Call { callee, .. } => leftmost(callee, pos),
            ir::ExprKind::Name(id) => id.pos = pos,
            _ => {}
        }
    }
    leftmost(&mut e, pos);
    e
}

fn has(m: ClassMember<'_>, modifier: Modifier) -> bool {
    m.modifiers().contains(&modifier)
}

impl<'a> Lowerer<'a> {
    /// Pre-declares the hoisted top-level declarations and returns the
    /// names of the prologue's `local` (spec §3.1.9, decisions L24, T76).
    pub(crate) fn hoist(&mut self) -> Vec<String> {
        let parse = self.parse;
        let used = enum_value_uses(parse);
        let mut hoisted = Vec::new();
        for s in parse.view().block().stmts() {
            let (inner, exported) = match s.kind() {
                StmtKind::Export(inner) => (inner, true),
                _ => (s, false),
            };
            match inner.kind() {
                StmtKind::Function { name, .. } if exported && self.ctx.module.is_some() => {
                    let n = name.path().at(0).text().to_string();
                    self.declare(&n, Binding::Export);
                    self.pending.insert(n.clone());
                    hoisted.push(n);
                }
                StmtKind::Class(c) => {
                    let n = c.name().text().to_string();
                    let info = self.class_info(c);
                    self.declare(&n, Binding::Class { info, exported });
                    self.pending.insert(n.clone());
                    hoisted.push(n);
                }
                StmtKind::Enum(e) => {
                    let n = e.name().text().to_string();
                    let info = self.enum_info(e);
                    let global = s.decorators().iter().any(|d| d.is_named("global"));
                    let table = exported || global || used.contains(&n);
                    self.declare(&n, Binding::Enum { info, table });
                    if table {
                        self.pending.insert(n.clone());
                        hoisted.push(n);
                    }
                }
                _ => {}
            }
        }
        hoisted
    }

    /// The class from the package analysis, or from the declaration alone
    /// (without package context).
    fn class_info(&self, c: ClassDecl<'a>) -> Arc<ClassInfo> {
        if let Some(info) = self.ctx.classes.get(c.name().text()) {
            return info.clone();
        }
        Arc::new(ClassInfo {
            name: c.name().text().to_string(),
            is_abstract: c.is_abstract(),
            base: if c.extends().is_some() {
                Base::Unknown
            } else {
                Base::None
            },
            widget: None,
            members: sema_classes::members(c.ast()),
        })
    }

    fn enum_info(&mut self, e: EnumDecl<'a>) -> Arc<EnumInfo> {
        if let Some(info) = self.ctx.enums.get(e.name().text()) {
            return info.clone();
        }
        let mut diagnostics = Vec::new();
        let info = sema_classes::enum_info(e.ast(), &|_| None, &mut diagnostics);
        self.errors.extend(diagnostics);
        Arc::new(info)
    }

    pub(crate) fn wl_call(&mut self, function: &str, args: Vec<ir::Expr>, pos: Pos) -> ir::Expr {
        let wl = self.runtime_ref(pos);
        call(field(wl, function, pos), args, pos)
    }

    pub(crate) fn string(&self, s: &str, pos: Pos) -> ir::Expr {
        lit(ir::ExprKind::String(quote(s.as_bytes())), pos)
    }

    /// The value a type path refers to (`extends a.b.C`).
    fn path_value(&mut self, t: Type<'a>, pos: Pos) -> ir::Expr {
        let Some(path) = sema_classes::type_path(t.ast()) else {
            return lit(ir::ExprKind::Nil, pos);
        };
        let mut value = match self.lookup(&path[0]).cloned() {
            Some(Binding::LazyImport { table, name }) => {
                field(name_expr(generated(&table, pos)), &name, pos)
            }
            _ => name_expr(ident(&path[0], pos)),
        };
        for p in &path[1..] {
            value = field(value, p, pos);
        }
        value
    }

    /// `_G.name = value` (`@global`, spec §4.3).
    pub(crate) fn global_write(&mut self, name: &str, value: ir::Expr, pos: Pos) -> ir::Stmt {
        let g = self.global("_G", None, pos);
        let target = if is_identifier(name) {
            field(g, name, pos)
        } else {
            ir::Expr::new(
                ir::ExprKind::Index {
                    obj: Box::new(g),
                    key: Box::new(self.string(name, pos)),
                },
                pos,
            )
        };
        assign(target, value, pos)
    }

    /// A class declaration (spec §3.1.9).
    /// The marks of a class member's function: its own decorators, else the
    /// class's (spec §7.3).
    fn member_marks(&mut self, class: &FnMarks, exported: bool) {
        let marks = &mut self.next_fn_marks;
        if marks.profile.is_none() && !marks.noprofile {
            marks.profile = class.profile.clone();
            marks.noprofile = class.noprofile;
        }
        marks.exported = exported;
    }

    pub(crate) fn class_decl(
        &mut self,
        c: ClassDecl<'a>,
        deco: &Decorations<'a>,
        exported: bool,
    ) -> Vec<ir::Stmt> {
        let name = c.name().text().to_string();
        // `@profile` / `@noprofile` on the class apply to its methods.
        let class_marks = self.class_marks.take().unwrap_or_default();
        // On the `class` line, not on a decorator's.
        let pos = self.span_pos(c.name().span());
        let end = self.span_pos(c.end_span());
        let info = match self.lookup(&name).cloned() {
            Some(Binding::Class { info, .. }) => info,
            _ => self.class_info(c),
        };
        let widget = info.widget.clone();
        self.slices.insert(Slice::Class);
        if widget.is_some() {
            self.slices.insert(Slice::Widget);
        }
        let widget_root = matches!(info.base, Base::Widget { .. });
        let base = match (&info.base, &c.extends()) {
            (Base::None | Base::Widget { .. }, _) | (_, None) => None,
            (_, Some(t)) => Some(self.path_value(*t, pos)),
        };
        let cls = name_expr(ident(&name, pos));
        let mut out = Vec::new();
        let mut args = vec![self.string(&name, pos)];
        args.extend(base.clone());
        let create = self.wl_call(
            if widget.is_some() {
                "widgetClass"
            } else {
                "class"
            },
            args,
            pos,
        );
        out.push(assign(cls.clone(), create, pos));
        self.pending.remove(&name);
        self.classes.push(ClassCtx {
            name: name.clone(),
            base: base.clone(),
            widget_root,
            is_static: false,
        });
        let mut field_inits: Vec<ClassMember<'a>> = Vec::new();
        let mut statics: Vec<ClassMember<'a>> = Vec::new();
        let mut has_ctor = false;
        for m in &c.members() {
            if !m.decorators().is_empty() && !self.kept_by_if_decorators(m.decorators(), false) {
                continue;
            }
            let is_static = has(m, Modifier::Static);
            self.classes.last_mut().expect("class").is_static = is_static;
            let mpos = self.span_pos(m.span());
            match m.kind() {
                ClassMemberKind::Field {
                    name: fname, value, ..
                } => {
                    let d = self.decorations(m.decorators(), Target::Field);
                    if value.is_some() {
                        if is_static {
                            statics.push(m);
                        } else {
                            field_inits.push(m);
                        }
                    }
                    if !d.user.is_empty() {
                        let ctx = self.deco_ctx(
                            "field",
                            fname.text(),
                            is_static,
                            Some(cls.clone()),
                            mpos,
                        );
                        let mut args = vec![ctx];
                        args.extend(self.decorator_values(&d.user));
                        let apply = self.wl_call("decorateField", args, mpos);
                        self.slices.insert(Slice::Decorate);
                        out.push(stmt(ir::StmtKind::Call(apply), mpos));
                    }
                }
                ClassMemberKind::Method {
                    name: mname,
                    func,
                    has_body,
                } => {
                    if has(m, Modifier::Overload) {
                        continue;
                    }
                    let d = self.decorations(m.decorators(), Target::Method);
                    if !has_body {
                        if has(m, Modifier::Abstract) && self.ctx.debug {
                            let args = vec![cls.clone(), self.string(mname.text(), mpos)];
                            let stub = self.wl_call("abstract", args, mpos);
                            out.push(stmt(ir::StmtKind::Call(stub), mpos));
                        }
                        continue;
                    }
                    let removed = d.debug_only && !self.ctx.debug;
                    if d.debug_only {
                        self.check_debug_only(func);
                    }
                    let lowered = if removed {
                        // Calls through `obj:m()` cannot be found without
                        // types: the method stays as a no-op (decision T77).
                        ir::Func {
                            pos: mpos,
                            params: Vec::new(),
                            vararg: true,
                            body: ir::Block {
                                stmts: Vec::new(),
                                end: self.span_pos(func.end_span()),
                            },
                        }
                    } else {
                        let sep = if is_static { "." } else { ":" };
                        self.next_fn_name = Some(format!("{name}{sep}{}", mname.text()));
                        let public = !has(m, Modifier::Private) && !has(m, Modifier::Protected);
                        self.member_marks(&class_marks, exported && public);
                        self.func(func, !is_static)
                    };
                    let (path, method) = if is_static {
                        (vec![ident(&name, mpos), self.ident(mname)], None)
                    } else {
                        (vec![ident(&name, mpos)], Some(self.ident(mname)))
                    };
                    out.push(stmt(
                        ir::StmtKind::Function {
                            path,
                            method,
                            func: lowered,
                        },
                        mpos,
                    ));
                    let fend = self.span_pos(func.end_span());
                    let target = field(cls.clone(), mname.text(), fend);
                    if !d.user.is_empty() && !removed {
                        let s = self.apply_decorators(
                            target.clone(),
                            &d.user,
                            "method",
                            mname.text(),
                            is_static,
                            Some(cls.clone()),
                            fend,
                        );
                        out.push(s);
                    }
                    if let (Some(message), true) = (&d.deprecated, self.ctx.debug && !removed) {
                        out.push(self.deprecated_wrap(
                            target.clone(),
                            &format!("{name}:{}", mname.text()),
                            message.clone(),
                            fend,
                        ));
                    }
                    if !d.scripts.is_empty() && !removed {
                        let scripts = d.scripts.clone();
                        out.extend(self.script_handlers(
                            &cls,
                            &scripts,
                            mname.text(),
                            widget.is_some() && !is_static,
                            fend,
                        ));
                    }
                    if !d.register.is_empty() && !removed {
                        let s = self.apply_registering(
                            target,
                            &d.register,
                            "method",
                            mname.text(),
                            is_static,
                            Some(cls.clone()),
                            fend,
                        );
                        out.push(s);
                    }
                }
                ClassMemberKind::Getter {
                    name: gname, body, ..
                } => {
                    let d = self.decorations(m.decorators(), Target::Accessor);
                    let f = self.accessor_func(None, body, !is_static, mpos);
                    let kind = if is_static { "sget" } else { "get" };
                    let s = self.accessor(&cls, kind, gname.text(), f, &d, is_static, mpos);
                    out.push(s);
                }
                ClassMemberKind::Setter {
                    name: sname,
                    param,
                    body,
                } => {
                    let d = self.decorations(m.decorators(), Target::Accessor);
                    let f = self.accessor_func(Some(param.name), body, !is_static, mpos);
                    let kind = if is_static { "sset" } else { "set" };
                    let s = self.accessor(&cls, kind, sname.text(), f, &d, is_static, mpos);
                    out.push(s);
                }
                ClassMemberKind::Constructor { func, has_body } => {
                    if !has_body || has(m, Modifier::Overload) {
                        continue;
                    }
                    has_ctor = true;
                    let d = self.decorations(m.decorators(), Target::Constructor);
                    self.next_fn_name = Some(format!("{name}.new"));
                    self.member_marks(&class_marks, exported);
                    let own_inits = widget.is_some() && !field_inits_of(c).is_empty();
                    let s = match &widget {
                        Some(w) => self.widget_ctor(
                            func,
                            &name,
                            w,
                            widget_root,
                            base.clone(),
                            own_inits,
                            mpos,
                        ),
                        None => {
                            let inits = field_inits_of(c);
                            self.ctor(func, &name, base.clone(), &inits, mpos)
                        }
                    };
                    out.push(s);
                    if !d.user.is_empty() {
                        let fend = self.span_pos(func.end_span());
                        let key = if widget.is_some() { "__ctor" } else { "__init" };
                        let target = field(cls.clone(), key, fend);
                        let s = self.apply_decorators(
                            target,
                            &d.user,
                            "constructor",
                            &name,
                            false,
                            Some(cls.clone()),
                            fend,
                        );
                        out.push(s);
                    }
                }
            }
        }
        self.classes.last_mut().expect("class").is_static = false;
        let field_inits: Vec<ClassMember<'a>> = field_inits_of(c);
        if !has_ctor {
            out.extend(self.implicit_ctor(
                &name,
                widget.as_deref(),
                widget_root,
                base.clone(),
                &field_inits,
                end,
            ));
        }
        if widget.is_some() && !field_inits.is_empty() {
            out.push(self.init_fields(&name, &field_inits, end));
        }
        let finalize = self.wl_call(
            if widget.is_some() {
                "finalizeWidget"
            } else {
                "finalize"
            },
            vec![cls.clone()],
            end,
        );
        out.push(stmt(ir::StmtKind::Call(finalize), end));
        // Static field initializers, after finalization (spec §3.1.9).
        self.classes.last_mut().expect("class").is_static = true;
        for m in statics {
            if let ClassMemberKind::Field {
                name: fname,
                value: Some(value),
                ..
            } = m.kind()
            {
                let target = field(cls.clone(), fname.text(), end);
                out.extend(self.wrapped(end, |l| {
                    let v = l.expr(value);
                    vec![assign(target, v, end)]
                }));
            }
        }
        self.classes.pop();
        if !deco.user.is_empty() {
            let s =
                self.apply_decorators(cls.clone(), &deco.user, "class", &name, false, None, end);
            out.push(s);
        }
        // Full reflection metadata (spec §7.8.2).
        let reflection = self.ctx.tools.reflection;
        if reflection == 2 || (reflection == 1 && deco.reflect) {
            let mut decorators: Vec<Decorator<'a>> = deco.user.clone();
            decorators.extend(deco.register.iter().copied());
            let s = self.class_meta(c, &decorators, cls.clone(), end);
            out.push(s);
        }
        if deco.has_global {
            let value = if widget.is_some() {
                self.wl_call("xmlMixin", vec![cls.clone()], end)
            } else {
                cls.clone()
            };
            let global = deco.global.clone().unwrap_or_else(|| name.clone());
            out.push(self.global_write(&global, value, end));
        }
        if !deco.register.is_empty() {
            let s = self.apply_registering(
                cls.clone(),
                &deco.register,
                "class",
                &name,
                false,
                None,
                end,
            );
            out.push(s);
        }
        if let Some(message) = &deco.deprecated {
            self.mark_deprecated(&name, message.clone());
        }
        if exported {
            out.push(self.export_assign(&name, cls, end));
        }
        out
    }

    /// `@script("OnClick")` / `@script("OnShow", { hook = true })`: the
    /// method handles a script of every frame of the class (spec §3.2.1).
    fn script_handlers(
        &mut self,
        cls: &ir::Expr,
        scripts: &[Decorator<'a>],
        method: &str,
        allowed: bool,
        pos: Pos,
    ) -> Vec<ir::Stmt> {
        let mut out = Vec::new();
        for d in scripts {
            if !allowed {
                self.error(
                    "E3021",
                    d.span(),
                    "`@script` applies to instance methods of widget classes",
                );
                continue;
            }
            let args = d.args().map(|a| a.exprs());
            let name = match args.and_then(|a| a.first()).map(|e| e.kind()) {
                Some(ExprKind::String(s)) => String::from_utf8_lossy(&s.value()).into_owned(),
                _ => {
                    self.error(
                        "E3021",
                        d.span(),
                        "`@script` takes a script name: `@script(\"OnClick\")`",
                    );
                    continue;
                }
            };
            let hook = match args.and_then(|a| a.get(1)).map(|e| e.kind()) {
                None => false,
                Some(ExprKind::Table(fields)) => {
                    let mut hook = None;
                    for f in fields {
                        match f {
                            TableField::Named { name, value, .. } if name.text() == "hook" => {
                                hook = match value.kind() {
                                    ExprKind::True => Some(true),
                                    ExprKind::False => Some(false),
                                    _ => None,
                                }
                            }
                            _ => hook = None,
                        }
                    }
                    match hook {
                        Some(h) => h,
                        None => {
                            self.error(
                                "E3021",
                                d.span(),
                                "`@script` options are `{ hook = true }` or `{ hook = false }`",
                            );
                            continue;
                        }
                    }
                }
                Some(_) => {
                    self.error(
                        "E3021",
                        d.span(),
                        "`@script` options are `{ hook = true }` or `{ hook = false }`",
                    );
                    continue;
                }
            };
            let mut values = vec![
                cls.clone(),
                self.string(&name, pos),
                self.string(method, pos),
            ];
            if hook {
                values.push(lit(ir::ExprKind::True, pos));
            }
            let c = self.wl_call("script", values, pos);
            out.push(stmt(ir::StmtKind::Call(c), pos));
        }
        out
    }

    /// `self.x = <initializer>` for instance fields, in the current scope
    /// (where `self` is bound).
    fn field_init_stmts(&mut self, inits: &[ClassMember<'a>], pos: Pos) -> Vec<ir::Stmt> {
        let mut out = Vec::new();
        for m in inits {
            if let ClassMemberKind::Field {
                name,
                value: Some(value),
                ..
            } = m.kind()
            {
                let target = field(name_expr(ident("self", pos)), name.text(), pos);
                out.extend(self.wrapped(pos, |l| {
                    let v = l.expr(value);
                    vec![assign(target, v, pos)]
                }));
            }
        }
        out
    }

    /// Opens a function scope for a generated or constructor function.
    fn enter_function(&mut self) -> (Vec<LoopInfo>, Vec<ir::Stmt>) {
        self.func_depth += 1;
        self.push_scope();
        (
            std::mem::take(&mut self.loops),
            std::mem::take(&mut self.prelude),
        )
    }

    fn leave_function(&mut self, saved: (Vec<LoopInfo>, Vec<ir::Stmt>)) {
        self.pop_scope();
        self.loops = saved.0;
        self.prelude = saved.1;
        self.func_depth -= 1;
    }

    /// The constructor of a class: `function C:__init(…)`; field
    /// initializers run first (base class) or right after `super(…)`.
    fn ctor(
        &mut self,
        f: Func<'a>,
        name: &str,
        base: Option<ir::Expr>,
        inits: &[ClassMember<'a>],
        pos: Pos,
    ) -> ir::Stmt {
        let saved = self.enter_function();
        self.declare("self", Binding::Local);
        for p in &f.params() {
            self.declare(p.name().text(), Binding::Local);
        }
        let mut stmts = self.default_params(&FnShape::of(f));
        if base.is_none() {
            stmts.extend(self.field_init_stmts(inits, pos));
        }
        for s in &f.body().stmts() {
            match (super_call_args(s), &base) {
                (Some(args), Some(b)) => {
                    let spos = self.span_pos(s.span());
                    let call_stmt = self.wrapped(spos, |l| {
                        let mut values = vec![name_expr(ident("self", spos))];
                        values.extend(l.exprs(args));
                        vec![stmt(
                            ir::StmtKind::Call(call(
                                field(at(b.clone(), spos), "__init", spos),
                                values,
                                spos,
                            )),
                            spos,
                        )]
                    });
                    stmts.extend(call_stmt);
                    stmts.extend(self.field_init_stmts(inits, spos));
                }
                _ => stmts.extend(self.stmt(s)),
            }
        }
        let func = ir::Func {
            pos: self.span_pos(f.params_span()),
            params: f.params().iter().map(|p| self.ident(p.name())).collect(),
            vararg: f.vararg().is_some(),
            body: ir::Block {
                stmts,
                end: self.span_pos(f.end_span()),
            },
        };
        self.leave_function(saved);
        stmt(
            ir::StmtKind::Function {
                path: vec![ident(name, pos)],
                method: Some(ident("__init", pos)),
                func,
            },
            pos,
        )
    }

    /// The frame-creating value of a widget class's `super(…)`.
    fn widget_super(
        &mut self,
        widget: &str,
        root: bool,
        base: Option<ir::Expr>,
        args: Vec<ir::Expr>,
        pos: Pos,
    ) -> ir::Expr {
        let cls_arg = name_expr(generated("__wl_cls", pos));
        match base {
            Some(base) if !root => {
                let mut values = vec![cls_arg];
                values.extend(args);
                call(field(at(base, pos), "__ctor", pos), values, pos)
            }
            _ => {
                let mut create_args = vec![self.string(widget, pos)];
                create_args.extend(args);
                let create = self.global("CreateFrame", None, pos);
                let frame = call(create, create_args, pos);
                self.wl_call("mixin", vec![frame, cls_arg], pos)
            }
        }
    }

    /// The constructor of a widget class: `function C.__ctor(cls, …)`
    /// creates the frame through `super(…)` and returns it (spec §3.2).
    #[allow(clippy::too_many_arguments)]
    fn widget_ctor(
        &mut self,
        f: Func<'a>,
        name: &str,
        widget: &str,
        root: bool,
        base: Option<ir::Expr>,
        own_inits: bool,
        pos: Pos,
    ) -> ir::Stmt {
        let saved = self.enter_function();
        for p in &f.params() {
            self.declare(p.name().text(), Binding::Local);
        }
        let saved_ctor = self.widget_ctor.replace(self.func_depth);
        let mut stmts = self.default_params(&FnShape::of(f));
        let mut ends_with_return = false;
        for s in &f.body().stmts() {
            ends_with_return = matches!(s.kind(), StmtKind::Return(_));
            match super_call_args(s) {
                Some(args) => {
                    let spos = self.span_pos(s.span());
                    let base = base.clone();
                    let created = self.wrapped(spos, |l| {
                        let values = l.exprs(args);
                        let frame = l.widget_super(widget, root, base, values, spos);
                        vec![local(ident("self", spos), frame, spos)]
                    });
                    stmts.extend(created);
                    self.declare("self", Binding::Local);
                    if own_inits {
                        let init = call(
                            field(name_expr(ident(name, spos)), "__initFields", spos),
                            vec![name_expr(ident("self", spos))],
                            spos,
                        );
                        stmts.push(stmt(ir::StmtKind::Call(init), spos));
                    }
                }
                None => stmts.extend(self.stmt(s)),
            }
        }
        let end = self.span_pos(f.end_span());
        if !ends_with_return {
            stmts.push(stmt(
                ir::StmtKind::Return(vec![name_expr(ident("self", end))]),
                end,
            ));
        }
        self.widget_ctor = saved_ctor;
        let mut params = vec![generated("__wl_cls", pos)];
        params.extend(f.params().iter().map(|p| self.ident(p.name())));
        let func = ir::Func {
            pos: self.span_pos(f.params_span()),
            params,
            vararg: f.vararg().is_some(),
            body: ir::Block { stmts, end },
        };
        self.leave_function(saved);
        stmt(
            ir::StmtKind::Function {
                path: vec![ident(name, pos), ident("__ctor", pos)],
                method: None,
                func,
            },
            pos,
        )
    }

    /// The constructor of a class that declares none (spec §3.1.2).
    fn implicit_ctor(
        &mut self,
        name: &str,
        widget: Option<&str>,
        root: bool,
        base: Option<ir::Expr>,
        inits: &[ClassMember<'a>],
        pos: Pos,
    ) -> Vec<ir::Stmt> {
        let saved = self.enter_function();
        self.declare("self", Binding::Local);
        let result = match widget {
            Some(w) => {
                // Root: `(cls, name, parent, template, id)` → CreateFrame;
                // derived: forwards everything to the superclass.
                let params: Vec<&str> = if root || base.is_none() {
                    vec!["name", "parent", "template", "id"]
                } else {
                    Vec::new()
                };
                let args: Vec<ir::Expr> = if params.is_empty() {
                    vec![lit(ir::ExprKind::Vararg, pos)]
                } else {
                    params
                        .iter()
                        .map(|p| name_expr(generated(&format!("__wl_{p}"), pos)))
                        .collect()
                };
                let frame = self.widget_super(w, root, base, args, pos);
                let mut stmts = vec![local(ident("self", pos), frame, pos)];
                if !inits.is_empty() {
                    let init = call(
                        field(name_expr(ident(name, pos)), "__initFields", pos),
                        vec![name_expr(ident("self", pos))],
                        pos,
                    );
                    stmts.push(stmt(ir::StmtKind::Call(init), pos));
                }
                stmts.push(stmt(
                    ir::StmtKind::Return(vec![name_expr(ident("self", pos))]),
                    pos,
                ));
                let mut fparams = vec![generated("__wl_cls", pos)];
                fparams.extend(params.iter().map(|p| generated(&format!("__wl_{p}"), pos)));
                vec![stmt(
                    ir::StmtKind::Function {
                        path: vec![ident(name, pos), ident("__ctor", pos)],
                        method: None,
                        func: ir::Func {
                            pos,
                            params: fparams,
                            vararg: params.is_empty(),
                            body: ir::Block { stmts, end: pos },
                        },
                    },
                    pos,
                )]
            }
            None if inits.is_empty() => Vec::new(),
            None => {
                let mut stmts = Vec::new();
                if let Some(b) = &base {
                    let values = vec![
                        name_expr(ident("self", pos)),
                        lit(ir::ExprKind::Vararg, pos),
                    ];
                    stmts.push(stmt(
                        ir::StmtKind::Call(call(
                            field(at(b.clone(), pos), "__init", pos),
                            values,
                            pos,
                        )),
                        pos,
                    ));
                }
                stmts.extend(self.field_init_stmts(inits, pos));
                vec![stmt(
                    ir::StmtKind::Function {
                        path: vec![ident(name, pos)],
                        method: Some(ident("__init", pos)),
                        func: ir::Func {
                            pos,
                            params: Vec::new(),
                            vararg: base.is_some(),
                            body: ir::Block { stmts, end: pos },
                        },
                    },
                    pos,
                )]
            }
        };
        self.leave_function(saved);
        result
    }

    /// `function C.__initFields(self) … end`: a widget class's own field
    /// initializers (for `__ctor` and XML-created frames).
    fn init_fields(&mut self, name: &str, inits: &[ClassMember<'a>], pos: Pos) -> ir::Stmt {
        let saved = self.enter_function();
        self.declare("self", Binding::Local);
        let stmts = self.field_init_stmts(inits, pos);
        self.leave_function(saved);
        stmt(
            ir::StmtKind::Function {
                path: vec![ident(name, pos), ident("__initFields", pos)],
                method: None,
                func: ir::Func {
                    pos,
                    params: vec![ident("self", pos)],
                    vararg: false,
                    body: ir::Block { stmts, end: pos },
                },
            },
            pos,
        )
    }

    /// The function of an accessor (`self` unless static; a setter's
    /// parameter).
    fn accessor_func(
        &mut self,
        param: Option<Name<'a>>,
        body: Block<'a>,
        method: bool,
        pos: Pos,
    ) -> ir::Func {
        let span = body.span();
        let mut f = self.func_shape(&FnShape::accessor(param, body), method);
        f.pos = pos;
        f.body.end = self.next_token_pos(span.end);
        if method {
            f.params.insert(0, ident("self", pos));
        }
        f
    }

    #[allow(clippy::too_many_arguments)]
    fn accessor(
        &mut self,
        cls: &ir::Expr,
        kind: &str,
        name: &str,
        f: ir::Func,
        d: &Decorations<'a>,
        is_static: bool,
        pos: Pos,
    ) -> ir::Stmt {
        let mut value = ir::Expr::new(ir::ExprKind::Function(Box::new(f)), pos);
        if !d.user.is_empty() {
            let ctx_kind = if kind.ends_with("get") {
                "getter"
            } else {
                "setter"
            };
            let ctx = self.deco_ctx(ctx_kind, name, is_static, Some(cls.clone()), pos);
            let mut args = vec![value, ctx];
            args.extend(self.decorator_values(&d.user));
            self.slices.insert(Slice::Decorate);
            value = self.wl_call("decorate", args, pos);
        }
        let args = vec![
            cls.clone(),
            self.string(kind, pos),
            self.string(name, pos),
            value,
        ];
        let c = self.wl_call("accessor", args, pos);
        stmt(ir::StmtKind::Call(c), pos)
    }

    /// `super` inside a class body: the superclass value (spec §3.1.4).
    pub(crate) fn super_value(&mut self, span: Span, pos: Pos) -> ir::Expr {
        let Some(ctx) = self.classes.last() else {
            return name_expr(ident("super", pos));
        };
        if let Some(b) = &ctx.base {
            return at(b.clone(), pos);
        }
        if ctx.widget_root && !ctx.is_static {
            // The native widget methods: the frame's own metatable.
            let gm = self.global("getmetatable", None, pos);
            let mt = call(gm, vec![name_expr(ident("self", pos))], pos);
            return field(mt, "__index", pos);
        }
        let name = ctx.name.clone();
        self.error("E3001", span, format!("class `{name}` has no superclass"));
        lit(ir::ExprKind::Nil, pos)
    }

    /// `super:m(args)`: `Base.m(self, args)` (no `self` in static methods).
    pub(crate) fn super_method(
        &mut self,
        obj: Expr<'a>,
        name: Name<'a>,
        args: Args<'a>,
        pos: Pos,
    ) -> ir::Expr {
        let base = self.super_value(obj.span(), pos);
        let is_static = self.classes.last().is_some_and(|c| c.is_static);
        let mut values = if is_static {
            Vec::new()
        } else {
            vec![name_expr(ident("self", pos))]
        };
        values.extend(self.args_list(args));
        call(field(base, name.text(), pos), values, pos)
    }

    /// `super` refers to the superclass here.
    pub(crate) fn is_super(&self, e: Expr<'a>) -> bool {
        matches!(e.kind(), ExprKind::Name(n) if n.text() == "super")
            && !self.classes.is_empty()
            && self.lookup("super").is_none()
    }

    /// An enum declaration: its table when the enum is used as a value
    /// (spec §3.4.3).
    pub(crate) fn enum_decl(
        &mut self,
        e: EnumDecl<'a>,
        deco: &Decorations<'a>,
        exported: bool,
    ) -> Vec<ir::Stmt> {
        let name = e.name().text().to_string();
        if let Some(message) = &deco.deprecated {
            self.mark_deprecated(&name, message.clone());
        }
        let Some(Binding::Enum {
            info, table: true, ..
        }) = self.lookup(&name).cloned()
        else {
            return Vec::new();
        };
        let pos = self.span_pos(e.name().span());
        let fields = info
            .members
            .iter()
            .map(|(member, value)| ir::Field::Named {
                name: ident(member, pos),
                value: value.to_expr(pos),
            })
            .collect();
        let mut value = lit(
            ir::ExprKind::Table {
                fields,
                close: Pos::NONE,
            },
            pos,
        );
        if self.ctx.debug {
            self.slices.insert(Slice::Enum);
            let label = self.string(&name, pos);
            value = self.wl_call("enum", vec![value, label], pos);
        }
        let target = name_expr(ident(&name, pos));
        let mut out = vec![assign(target.clone(), value, pos)];
        if self.ctx.tools.reflect_used {
            let names: Vec<String> = info.members.iter().map(|(m, _)| m.to_string()).collect();
            out.push(self.enum_order(&names, target.clone(), pos));
        }
        self.pending.remove(&name);
        if deco.has_global {
            let global = deco.global.clone().unwrap_or_else(|| name.clone());
            out.push(self.global_write(&global, target.clone(), pos));
        }
        if exported {
            out.push(self.export_assign(&name, target, pos));
        }
        out
    }

    /// `obj` names an enum.
    pub(crate) fn is_enum_object(&self, obj: Expr<'a>) -> bool {
        if !self.scopes.seen().enums {
            return false;
        }
        matches!(obj.kind(), ExprKind::Name(n) if matches!(self.lookup(n.text()), Some(Binding::Enum { .. } | Binding::ImportedEnum(_))))
    }

    /// The value of an enum member, if `obj` names an enum that has it.
    pub(crate) fn enum_value(&self, obj: Expr<'a>, key: &str) -> Option<Const> {
        if !self.scopes.seen().enums {
            return None;
        }
        let ExprKind::Name(n) = obj.kind() else {
            return None;
        };
        match self.lookup(n.text())? {
            Binding::Enum { info, .. } | Binding::ImportedEnum(info) => info.value(key).cloned(),
            _ => None,
        }
    }

    /// An enum member read with a constant key: its value (spec §3.4.3).
    pub(crate) fn enum_member(
        &mut self,
        obj: Expr<'a>,
        key: &str,
        key_span: Span,
        pos: Pos,
    ) -> Option<ir::Expr> {
        if !self.scopes.seen().enums {
            return None;
        }
        let ExprKind::Name(n) = obj.kind() else {
            return None;
        };
        let info = match self.lookup(n.text())? {
            Binding::Enum { info, .. } | Binding::ImportedEnum(info) => info.clone(),
            _ => return None,
        };
        match info.value(key) {
            Some(v) => Some(v.to_expr(pos)),
            None => {
                self.error(
                    "E2003",
                    key_span,
                    format!("enum `{}` has no member `{key}`", info.name),
                );
                Some(lit(ir::ExprKind::Nil, pos))
            }
        }
    }

    /// Instantiating an abstract class (E3002).
    pub(crate) fn check_instantiation(&mut self, callee: Expr<'a>) {
        if !self.scopes.seen().classes {
            return;
        }
        let target = match callee.kind() {
            ExprKind::Name(n) => Some(n),
            ExprKind::Field { obj, name } if name.text() == "new" => match obj.kind() {
                ExprKind::Name(n) => Some(n),
                _ => None,
            },
            _ => None,
        };
        let Some(n) = target else { return };
        if let Some(Binding::Class { info, .. } | Binding::ImportedClass(info)) =
            self.lookup(n.text())
            && info.is_abstract
        {
            let name = info.name.clone();
            self.error(
                "E3002",
                callee.span(),
                format!("cannot instantiate the abstract class `{name}`"),
            );
        }
    }
}

/// The arguments of a top-level `super(…)` statement.
fn super_call_args(s: Stmt<'_>) -> Option<Exprs<'_>> {
    let StmtKind::Call(e) = s.kind() else {
        return None;
    };
    let ExprKind::Call { callee, args } = e.kind() else {
        return None;
    };
    match (callee.kind(), args.kind()) {
        (ExprKind::Name(n), ArgsKind::Parens { args, .. }) if n.text() == "super" => Some(args),
        _ => None,
    }
}

fn field_inits_of(c: ClassDecl<'_>) -> Vec<ClassMember<'_>> {
    c.members()
        .iter()
        .filter(|m| {
            !has(*m, Modifier::Static)
                && matches!(m.kind(), ClassMemberKind::Field { value: Some(_), .. })
        })
        .collect()
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !LUA_KEYWORDS.contains(&s)
}

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Enum names used as values anywhere in the file (not only as `E.Member`).
fn enum_value_uses(parse: &Parse) -> HashSet<String> {
    let mut enums = HashSet::new();
    for s in parse.view().block().stmts() {
        let kind = match s.kind() {
            StmtKind::Export(inner) => inner.kind(),
            k => k,
        };
        if let StmtKind::Enum(e) = kind {
            enums.insert(e.name().text().to_string());
        }
    }
    if enums.is_empty() {
        return enums;
    }
    let mut member_objects: HashSet<u32> = HashSet::new();
    let mut names: Vec<(String, u32)> = Vec::new();
    for_each_expr(parse, &mut |e| match e.kind() {
        ExprKind::Field { obj, .. } => {
            if let ExprKind::Name(n) = obj.kind() {
                member_objects.insert(n.span().start);
            }
        }
        ExprKind::Index { obj, key } if matches!(key.kind(), ExprKind::String(_)) => {
            if let ExprKind::Name(n) = obj.kind() {
                member_objects.insert(n.span().start);
            }
        }
        ExprKind::Name(n) if enums.contains(n.text()) => {
            names.push((n.text().to_string(), n.span().start))
        }
        _ => {}
    });
    names
        .into_iter()
        .filter(|(_, start)| !member_objects.contains(start))
        .map(|(n, _)| n)
        .collect()
}

/// Calls `f` on every expression of the file, class members and decorator
/// arguments included.
pub(crate) fn for_each_expr<'p>(parse: &'p Parse, f: &mut impl FnMut(Expr<'p>)) {
    let mut decorators: Vec<Decorator<'p>> = Vec::new();
    let root = parse.view().block();
    let mut blocks: Vec<Block<'p>> = vec![root];
    let mut exprs: Vec<Expr<'p>> = Vec::new();
    wowlua_syntax::visit::walk_stmts(root, &mut |s| {
        decorators.extend(s.decorators());
        let kind = match s.kind() {
            StmtKind::Export(inner) => inner.kind(),
            k => k,
        };
        if let StmtKind::Class(c) = kind {
            for m in c.members() {
                decorators.extend(m.decorators());
                match m.kind() {
                    ClassMemberKind::Field { value: Some(v), .. } => exprs.push(v),
                    ClassMemberKind::Method { func, .. }
                    | ClassMemberKind::Constructor { func, .. } => blocks.push(func.body()),
                    ClassMemberKind::Getter { body, .. } | ClassMemberKind::Setter { body, .. } => {
                        blocks.push(body)
                    }
                    _ => {}
                }
            }
        }
    });
    for b in blocks {
        wowlua_syntax::visit::walk_block(b, f);
    }
    for e in exprs {
        wowlua_syntax::visit::walk_expr(e, f);
    }
    for d in decorators {
        if let Some(args) = d.args() {
            for e in args.exprs() {
                wowlua_syntax::visit::walk_expr(e, f);
            }
        }
    }
}
