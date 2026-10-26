//! Decorators (spec chapter 4): core decorators (`@global`, `@deprecated`,
//! `@debugOnly`, `@inline`, `@nocheck`, …), user decorators with their
//! context tables, and the statements they decorate.

use super::*;

/// What a decorator is attached to (spec §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Function,
    Method,
    Constructor,
    Accessor,
    Field,
    Class,
    /// `local` and `const` declarations.
    Local,
    Enum,
    Import,
    Declare,
    /// Interfaces and type aliases.
    Type,
    Other,
}

/// Compiler behavior of framework decorators, by `@intrinsic` id (spec
/// §4.4). Later milestones add the standard library's decorators here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intrinsic {
    /// Replaces the function, like a user decorator.
    Wrapping,
    /// Receives the final value after every transformation.
    Registering,
}

pub(crate) fn intrinsic(id: &str) -> Option<Intrinsic> {
    match id {
        // Infrastructure checks of the standard library itself.
        "internal.wrap" => Some(Intrinsic::Wrapping),
        "internal.register" => Some(Intrinsic::Registering),
        // wowlua:events
        "events.event" | "events.unitEvent" | "events.bucket" => Some(Intrinsic::Registering),
        // wowlua:timer, wowlua:combat
        "timer.throttle" | "timer.debounce" | "timer.nextFrame" | "combat.outOfCombat" => {
            Some(Intrinsic::Wrapping)
        }
        // wowlua:hooks, wowlua:slash
        "hooks.hook" | "hooks.hookScript" | "slash.slash" => Some(Intrinsic::Registering),
        // wowlua:bindings, wowlua:tooltip
        "bindings.binding" | "tooltip.onTooltip" => Some(Intrinsic::Registering),
        // wowlua:lifecycle
        "lifecycle.onLoad"
        | "lifecycle.onLogin"
        | "lifecycle.onEnterWorld"
        | "lifecycle.onLogout" => Some(Intrinsic::Registering),
        // wowlua:test
        "test.test" | "test.skip" | "test.only" | "test.flavors" | "test.cases"
        | "test.expectError" | "test.beforeEach" | "test.afterEach" | "test.beforeAll"
        | "test.afterAll" | "test.bench" => Some(Intrinsic::Registering),
        _ => None,
    }
}

/// The decorators of one declaration, classified.
#[derive(Default)]
pub(crate) struct Decorations<'d> {
    /// User decorators and wrapping framework decorators, top to bottom.
    pub user: Vec<Decorator<'d>>,
    /// Registering framework decorators, top to bottom.
    pub register: Vec<Decorator<'d>>,
    pub has_global: bool,
    /// `@global(name)`: the name (default: the declaration's name).
    pub global: Option<String>,
    /// `@deprecated(msg?)`.
    pub deprecated: Option<Option<String>>,
    /// `@script(name, opts?)` on widget class methods.
    pub scripts: Vec<Decorator<'d>>,
    pub debug_only: bool,
    /// `@reflect` (spec §7.8.2).
    pub reflect: bool,
}

fn string_arg(d: Decorator<'_>) -> Option<String> {
    match d.args()?.exprs().first().map(|e| e.kind()) {
        Some(ExprKind::String(s)) => Some(String::from_utf8_lossy(&s.value()).into_owned()),
        _ => None,
    }
}

fn target_name(t: Target) -> &'static str {
    match t {
        Target::Function => "functions",
        Target::Method => "methods",
        Target::Constructor => "constructors",
        Target::Accessor => "accessors",
        Target::Field => "fields",
        Target::Class => "classes",
        Target::Local => "`local` and `const` declarations",
        Target::Enum => "enums",
        Target::Import => "imports",
        Target::Declare => "`declare` statements",
        Target::Type => "interfaces and type aliases",
        Target::Other => "this statement",
    }
}

impl<'a> Lowerer<'a> {
    /// Classifies and checks the decorators of a declaration (spec §4.2,
    /// §4.3): E3020, E3021, E3033, and the milestone of M5 decorators.
    pub(crate) fn decorations(
        &mut self,
        decorators: Decorators<'a>,
        target: Target,
    ) -> Decorations<'a> {
        use Target as T;
        let mut out = Decorations::default();
        for d in decorators {
            if !d.is_core() {
                if matches!(
                    target,
                    T::Function | T::Method | T::Constructor | T::Accessor | T::Field | T::Class
                ) {
                    let first = d.path().at(0).text();
                    if let Some(Binding::Intrinsic(id)) = self.lookup(first).cloned() {
                        match intrinsic(&id) {
                            Some(Intrinsic::Wrapping) => out.user.push(d),
                            Some(Intrinsic::Registering) => out.register.push(d),
                            None => self.error(
                                "E3033",
                                d.span(),
                                format!("unknown intrinsic `{id}` (the standard library needs a newer compiler)"),
                            ),
                        }
                    } else if first == "_G" || self.lookup(first).is_some() {
                        out.user.push(d);
                    } else {
                        self.error(
                            "E3020",
                            d.span(),
                            format!(
                                "unknown decorator `@{}`; decorators are core decorators, local or imported functions, or `@_G.…`",
                                d.path().iter().map(|n| n.text()).collect::<Vec<_>>().join(".")
                            ),
                        );
                    }
                } else {
                    self.error(
                        "E3021",
                        d.span(),
                        format!(
                            "user decorators cannot be applied to {}",
                            target_name(target)
                        ),
                    );
                }
                continue;
            }
            let name = d.path().at(0).text();
            let allowed = match name {
                "if" => continue,
                "global" => matches!(target, T::Function | T::Class | T::Local | T::Enum),
                "deprecated" => true,
                "inline" | "debugOnly" => matches!(target, T::Function | T::Method),
                "script" => target == T::Method,
                "nocheck" => matches!(
                    target,
                    T::Function | T::Method | T::Constructor | T::Accessor | T::Class
                ),
                "widget" | "intrinsic" | "deferred" | "onDemand" => target == T::Declare,
                "profile" | "noprofile" => {
                    matches!(target, T::Function | T::Method | T::Constructor | T::Class)
                }
                "trace" | "checked" | "unchecked" => {
                    matches!(target, T::Function | T::Method | T::Constructor)
                }
                "reflect" => matches!(target, T::Function | T::Class | T::Enum),
                _ => true,
            };
            if !allowed {
                self.error(
                    "E3021",
                    d.span(),
                    format!("`@{name}` cannot be applied to {}", target_name(target)),
                );
                continue;
            }
            match name {
                "global" => {
                    out.has_global = true;
                    out.global = string_arg(d);
                    if target == T::Function {
                        self.next_fn_marks.global = true;
                    }
                }
                "checked" => self.next_fn_marks.checked = true,
                "reflect" => {
                    out.reflect = true;
                    if target == T::Function {
                        self.next_fn_marks.reflect = true;
                    }
                }
                "unchecked" => self.next_fn_marks.unchecked = true,
                "deprecated" => out.deprecated = Some(string_arg(d)),
                "debugOnly" => out.debug_only = true,
                "script" => out.scripts.push(d),
                // Profiling and tracing (spec §7.3, §7.4): marks for the
                // function lowered next, or for every method of a class.
                "profile" | "noprofile" | "trace" => {
                    let args = d
                        .args()
                        .as_ref()
                        .map(|a| self.args_list(*a))
                        .unwrap_or_default();
                    let marks = if target == T::Class {
                        self.class_marks.get_or_insert_with(FnMarks::default)
                    } else {
                        &mut self.next_fn_marks
                    };
                    match name {
                        "profile" => marks.profile = Some(args),
                        "noprofile" => marks.noprofile = true,
                        _ => marks.trace = Some(args),
                    }
                }
                "intrinsic" if !self.ctx.compiler_known => self.error(
                    "E3033",
                    d.span(),
                    "`@intrinsic` is only for compiler-known modules (the standard library)",
                ),
                _ => {}
            }
        }
        out
    }

    /// The values of user decorators, in source order: `@a.b(args)` is
    /// `a.b(args)` (spec §4.5).
    pub(crate) fn decorator_values(&mut self, user: &[Decorator<'a>]) -> Vec<ir::Expr> {
        let exprs: Vec<Expr<'a>> = user.iter().map(|d| d.expr()).collect();
        self.seq(&exprs)
    }

    /// The context table of a decorated declaration (spec §4.5).
    pub(crate) fn deco_ctx(
        &self,
        kind: &str,
        name: &str,
        is_static: bool,
        class: Option<ir::Expr>,
        pos: Pos,
    ) -> ir::Expr {
        let named = |n: &str, value: ir::Expr| ir::Field::Named {
            name: ident(n, pos),
            value,
        };
        let s = |v: &str| lit(ir::ExprKind::String(quote(v.as_bytes()).into()), pos);
        let mut fields = vec![
            named("kind", s(kind)),
            named("name", s(name)),
            named(
                "static",
                lit(
                    if is_static {
                        ir::ExprKind::True
                    } else {
                        ir::ExprKind::False
                    },
                    pos,
                ),
            ),
        ];
        if let Some(c) = class {
            fields.push(named("class", c));
        }
        fields.push(named("file", s(&self.ctx.file)));
        fields.push(named(
            "line",
            lit(ir::ExprKind::Number(pos.line.to_string().into()), pos),
        ));
        lit(
            ir::ExprKind::Table {
                fields,
                close: Pos::NONE,
            },
            pos,
        )
    }

    /// `target = __wl.decorate(target, ctx, d1, d2, …)`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_decorators(
        &mut self,
        target: ir::Expr,
        user: &[Decorator<'a>],
        kind: &str,
        name: &str,
        is_static: bool,
        class: Option<ir::Expr>,
        pos: Pos,
    ) -> ir::Stmt {
        self.slices.insert(Slice::Decorate);
        let ctx = self.deco_ctx(kind, name, is_static, class, pos);
        let mut args = vec![target.clone(), ctx];
        args.extend(self.decorator_values(user));
        let wl = self.runtime_ref(pos);
        let value = call(field(wl, "decorate", pos), args, pos);
        assign(target, value, pos)
    }

    /// `__wl.register(final, ctx, r1, r2, …)` for registering framework
    /// decorators.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_registering(
        &mut self,
        target: ir::Expr,
        register: &[Decorator<'a>],
        kind: &str,
        name: &str,
        is_static: bool,
        class: Option<ir::Expr>,
        pos: Pos,
    ) -> ir::Stmt {
        self.slices.insert(Slice::Decorate);
        let ctx = self.deco_ctx(kind, name, is_static, class, pos);
        let mut args = vec![target, ctx];
        args.extend(self.decorator_values(register));
        let wl = self.runtime_ref(pos);
        stmt(
            ir::StmtKind::Call(call(field(wl, "register", pos), args, pos)),
            pos,
        )
    }

    /// Debug builds: `target = __wl.deprecated(target, "name", msg)`.
    pub(crate) fn deprecated_wrap(
        &mut self,
        target: ir::Expr,
        name: &str,
        message: Option<String>,
        pos: Pos,
    ) -> ir::Stmt {
        self.slices.insert(Slice::Decorate);
        let mut args = vec![
            target.clone(),
            lit(ir::ExprKind::String(quote(name.as_bytes()).into()), pos),
        ];
        if let Some(m) = message {
            args.push(lit(ir::ExprKind::String(quote(m.as_bytes()).into()), pos));
        }
        let wl = self.runtime_ref(pos);
        assign(target, call(field(wl, "deprecated", pos), args, pos), pos)
    }

    /// Remembers a `@deprecated` binding of the current scope (W3030 at its
    /// uses).
    pub(crate) fn mark_deprecated(&mut self, name: &str, message: Option<String>) {
        self.scopes.mark_deprecated(name, message);
    }

    /// The `@deprecated` message of the binding `name` refers to, if any.
    pub(crate) fn deprecated_of(&self, name: &str) -> Option<Option<String>> {
        self.scopes.deprecated_of(name)
    }

    /// A `@debugOnly` function must return nothing (E3032).
    pub(crate) fn check_debug_only(&mut self, f: Func<'a>) {
        if let Some(span) = value_return(f.body()) {
            self.error(
                "E3032",
                span,
                "a `@debugOnly` function cannot return values (its calls are removed from release builds)",
            );
        }
    }

    /// A statement with decorators other than `@if` (spec §4.1).
    pub(crate) fn decorated_stmt(&mut self, s: Stmt<'a>) -> Vec<ir::Stmt> {
        let (inner, exported) = match s.kind() {
            StmtKind::Export(inner) => (inner, true),
            _ => (s, false),
        };
        let target = match inner.kind() {
            StmtKind::Function { .. } | StmtKind::LocalFunction { .. } => Target::Function,
            StmtKind::Class(_) => Target::Class,
            StmtKind::Enum(_) => Target::Enum,
            StmtKind::Local { .. } | StmtKind::Const { .. } | StmtKind::LocalDestructure { .. } => {
                Target::Local
            }
            StmtKind::Import(_) => Target::Import,
            StmtKind::Declare(_) => Target::Declare,
            StmtKind::Interface(_) | StmtKind::TypeAlias(_) => Target::Type,
            _ => Target::Other,
        };
        let deco = self.decorations(s.decorators(), target);
        let pos = self.span_pos(s.span());
        match inner.kind() {
            StmtKind::Class(c) => self.class_decl(c, &deco, exported),
            StmtKind::Enum(e) => self.enum_decl(e, &deco, exported),
            StmtKind::Function { name, func } => {
                let label = name
                    .method()
                    .unwrap_or_else(|| name.path().last().expect("a name"));
                if deco.debug_only {
                    self.check_debug_only(func);
                    if !self.ctx.debug {
                        if name.path().len() == 1 && name.method().is_none() {
                            self.declare(name.path().at(0).text(), Binding::DebugOnlyRemoved);
                            self.pending.remove(name.path().at(0).text());
                        }
                        return Vec::new();
                    }
                }
                let mut out = if exported {
                    self.export(inner, pos)
                } else {
                    self.stmt_inner(inner)
                };
                // The export assignment (if any) receives the final value.
                let export = if exported { out.pop() } else { None };
                let end = self.span_pos(func.end_span());
                let mut target = name_expr(self.ident(name.path().at(0)));
                for n in name.path().iter().skip(1).chain(name.method()) {
                    target = field(target, n.text(), end);
                }
                target.pos = end;
                self.finish_function(&mut out, target, label.text(), &deco, end);
                if name.path().len() == 1
                    && name.method().is_none()
                    && let Some(message) = &deco.deprecated
                {
                    self.mark_deprecated(name.path().at(0).text(), message.clone());
                }
                if deco.debug_only && name.path().len() == 1 && name.method().is_none() {
                    self.scopes.mark_debug_only(name.path().at(0).text());
                }
                out.extend(export);
                out
            }
            StmtKind::LocalFunction { name, func } => {
                if deco.debug_only {
                    self.check_debug_only(func);
                    if !self.ctx.debug {
                        self.declare(name.text(), Binding::DebugOnlyRemoved);
                        return Vec::new();
                    }
                }
                let end = self.span_pos(func.end_span());
                // On the `local function` line, not on a decorator's.
                let pos = self.span_pos(name.span());
                let mut out = if deco.user.is_empty() {
                    self.stmt_inner(inner)
                } else {
                    // `local f; f = function … end`: recursive calls go
                    // through the decorated value (spec §4.5.3).
                    self.declare(name.text(), Binding::Local);
                    self.next_fn_name = Some(name.text().to_string());
                    let f = self.func(func, false);
                    vec![
                        stmt(
                            ir::StmtKind::Local {
                                names: vec![self.ident(name)],
                                values: Vec::new(),
                            },
                            pos,
                        ),
                        assign(
                            name_expr(self.ident(name)),
                            ir::Expr::new(ir::ExprKind::Function(Box::new(f)), pos),
                            pos,
                        ),
                    ]
                };
                let target = name_expr(ident(name.text(), end));
                self.finish_function(&mut out, target, name.text(), &deco, end);
                if let Some(message) = &deco.deprecated {
                    self.mark_deprecated(name.text(), message.clone());
                }
                if deco.debug_only {
                    self.scopes.mark_debug_only(name.text());
                }
                out
            }
            StmtKind::Local { names, .. } => {
                let mut out = if exported {
                    self.export(inner, pos)
                } else {
                    self.stmt_inner(inner)
                };
                let exports = if exported {
                    out.split_off(out.len().saturating_sub(names.len()))
                } else {
                    Vec::new()
                };
                for n in names {
                    if deco.has_global {
                        let global = deco
                            .global
                            .clone()
                            .unwrap_or_else(|| n.name.text().to_string());
                        let value = name_expr(self.ident(n.name));
                        out.push(self.global_write(&global, value, pos));
                    }
                    if let Some(message) = &deco.deprecated {
                        self.mark_deprecated(n.name.text(), message.clone());
                    }
                }
                out.extend(exports);
                out
            }
            StmtKind::Const { name, .. } => {
                let mut out = if exported {
                    self.export(inner, pos)
                } else {
                    self.stmt_inner(inner)
                };
                let export = if exported { out.pop() } else { None };
                if deco.has_global {
                    let value = match self.lookup(name.text()).cloned() {
                        Some(Binding::Const(c)) => c.to_expr(pos),
                        _ => name_expr(self.ident(name)),
                    };
                    let global = deco
                        .global
                        .clone()
                        .unwrap_or_else(|| name.text().to_string());
                    out.push(self.global_write(&global, value, pos));
                }
                if let Some(message) = &deco.deprecated {
                    self.mark_deprecated(name.text(), message.clone());
                }
                out.extend(export);
                out
            }
            _ => {
                if exported {
                    self.export(inner, pos)
                } else {
                    self.stmt_inner(inner)
                }
            }
        }
    }

    /// Applies user decorators, `@deprecated` (debug builds) and `@global`
    /// to a declared function, on its `end` line.
    fn finish_function(
        &mut self,
        out: &mut Vec<ir::Stmt>,
        target: ir::Expr,
        name: &str,
        deco: &Decorations<'a>,
        end: Pos,
    ) {
        if !deco.user.is_empty() {
            let s = self.apply_decorators(
                target.clone(),
                &deco.user,
                "function",
                name,
                false,
                None,
                end,
            );
            out.push(s);
        }
        if let (Some(message), true) = (&deco.deprecated, self.ctx.debug) {
            out.push(self.deprecated_wrap(target.clone(), name, message.clone(), end));
        }
        if deco.has_global {
            let global = deco.global.clone().unwrap_or_else(|| name.to_string());
            out.push(self.global_write(&global, target.clone(), end));
        }
        if !deco.register.is_empty() {
            let s =
                self.apply_registering(target, &deco.register, "function", name, false, None, end);
            out.push(s);
        }
    }
}

/// A `return` with values in a function body (not in nested functions).
fn value_return(b: Block<'_>) -> Option<Span> {
    for s in &b.stmts() {
        let found = match s.kind() {
            StmtKind::Return(values) if !values.is_empty() => Some(s.span()),
            StmtKind::Do(b)
            | StmtKind::While { body: b, .. }
            | StmtKind::Repeat { body: b, .. } => value_return(b),
            StmtKind::NumericFor { body, .. } | StmtKind::GenericFor { body, .. } => {
                value_return(body)
            }
            StmtKind::If {
                then_block,
                elseifs,
                else_block,
                ..
            } => value_return(then_block)
                .or_else(|| elseifs.iter().find_map(|(_, b)| value_return(b)))
                .or_else(|| else_block.and_then(value_return)),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}
