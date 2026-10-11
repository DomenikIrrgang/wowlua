//! Profiling and tracing wrappers (spec §7.3, §7.4): after lowering, every
//! named function definition the profile instruments is followed, on its
//! `end` line, by `f = __wl.prof.wrap(f, "name", "src/a.wowlua:3")` (and
//! `__wl.trace.wrap` for `@trace`), so line numbers stay as they were.

use super::*;

/// What the source says about one function: its name and the profiling
/// and tracing decorators that apply to it.
#[derive(Clone, Debug, Default)]
pub(crate) struct FnMarks {
    /// The display name (`Tracker:record`), when the function has one.
    pub label: Option<String>,
    /// `@profile(args)`.
    pub profile: Option<Vec<ir::Expr>>,
    /// `@noprofile`.
    pub noprofile: bool,
    /// `@trace(args)`.
    pub trace: Option<Vec<ir::Expr>>,
    /// An exported function or a public method of an exported class.
    pub exported: bool,
    /// `@global` (runtime checks in `exports` profiles).
    pub global: bool,
    /// `@checked` / `@unchecked` (spec §7.5).
    pub checked: bool,
    pub unchecked: bool,
    /// `@reflect` (spec §7.8.2), and the metadata to register.
    pub reflect: bool,
    pub reflect_meta: Option<ir::Expr>,
}

impl Lowerer<'_> {
    /// Adds the wrappers to the lowered statements of a file.
    pub(crate) fn instrument(&mut self, stmts: &mut Vec<ir::Stmt>) {
        let tools = &self.ctx.tools;
        // The standard library is never instrumented.
        if self.ctx.compiler_known || (tools.instrument == 0 && tools.log_level > 0) {
            return;
        }
        self.instrument_block(stmts);
    }

    fn instrument_block(&mut self, stmts: &mut Vec<ir::Stmt>) {
        let mut i = 0;
        while i < stmts.len() {
            self.instrument_nested(&mut stmts[i]);
            let wraps = self.wraps_for(&stmts[i]);
            let n = wraps.len();
            for (k, w) in wraps.into_iter().enumerate() {
                stmts.insert(i + 1 + k, w);
            }
            i += 1 + n;
        }
    }

    /// Named functions inside nested blocks and function bodies.
    fn instrument_nested(&mut self, s: &mut ir::Stmt) {
        use ir::StmtKind as K;
        match &mut s.kind {
            K::Local { values, .. } => values.iter_mut().for_each(|e| self.instrument_expr(e)),
            K::Assign { targets, values } => {
                targets.iter_mut().for_each(|e| self.instrument_expr(e));
                values.iter_mut().for_each(|e| self.instrument_expr(e));
            }
            K::Call(e) => self.instrument_expr(e),
            K::Return(es) => es.iter_mut().for_each(|e| self.instrument_expr(e)),
            K::Do(b) => self.instrument_block(&mut b.stmts),
            K::While { cond, body, .. } | K::Repeat { body, cond } => {
                self.instrument_expr(cond);
                self.instrument_block(&mut body.stmts);
            }
            K::If {
                clauses,
                else_block,
            } => {
                for c in clauses {
                    self.instrument_expr(&mut c.cond);
                    self.instrument_block(&mut c.body.stmts);
                }
                if let Some(b) = else_block {
                    self.instrument_block(&mut b.stmts);
                }
            }
            K::NumericFor {
                start,
                limit,
                step,
                body,
                ..
            } => {
                self.instrument_expr(start);
                self.instrument_expr(limit);
                if let Some(e) = step {
                    self.instrument_expr(e);
                }
                self.instrument_block(&mut body.stmts);
            }
            K::GenericFor { exprs, body, .. } => {
                exprs.iter_mut().for_each(|e| self.instrument_expr(e));
                self.instrument_block(&mut body.stmts);
            }
            K::Function { func, .. } | K::LocalFunction { func, .. } => {
                self.instrument_block(&mut func.body.stmts)
            }
            K::Break => {}
        }
    }

    /// The bodies of function expressions.
    fn instrument_expr(&mut self, e: &mut ir::Expr) {
        use ir::ExprKind as K;
        match &mut e.kind {
            K::Function(f) => self.instrument_block(&mut f.body.stmts),
            K::Table { fields, .. } => {
                for f in fields {
                    match f {
                        ir::Field::Keyed { key, value, .. } => {
                            self.instrument_expr(key);
                            self.instrument_expr(value);
                        }
                        ir::Field::Named { value, .. } | ir::Field::Positional(value) => {
                            self.instrument_expr(value)
                        }
                    }
                }
            }
            K::Index { obj, key } => {
                self.instrument_expr(obj);
                self.instrument_expr(key);
            }
            K::Field { obj, .. } | K::Paren { inner: obj, .. } | K::Unary { operand: obj, .. } => {
                self.instrument_expr(obj)
            }
            K::Call { callee: obj, args } | K::Method { obj, args, .. } => {
                self.instrument_expr(obj);
                match args {
                    ir::Args::Parens { args, .. } => {
                        args.iter_mut().for_each(|a| self.instrument_expr(a))
                    }
                    ir::Args::Table(a) | ir::Args::String(a) => self.instrument_expr(a),
                }
            }
            K::Binary { lhs, rhs, .. } => {
                self.instrument_expr(lhs);
                self.instrument_expr(rhs);
            }
            _ => {}
        }
    }

    /// The wrapper statements after a named function definition.
    fn wraps_for(&mut self, s: &ir::Stmt) -> Vec<ir::Stmt> {
        use ir::StmtKind as K;
        let (target, func, fallback) = match &s.kind {
            K::Function { path, method, func } => {
                let mut target = name_expr(path[0].clone());
                for p in &path[1..] {
                    target = field(target, &p.name, p.pos);
                }
                let mut label = path
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                if let Some(m) = method {
                    target = field(target, &m.name, m.pos);
                    label = match m.name.as_str() {
                        // A class's constructor.
                        "__init" | "__ctor" => format!("{label}.new"),
                        name => format!("{label}:{name}"),
                    };
                }
                (target, func, label)
            }
            K::LocalFunction { name, func } => (name_expr(name.clone()), func, name.name.clone()),
            K::Local { names, values } if names.len() == 1 && values.len() == 1 => {
                match &values[0].kind {
                    ir::ExprKind::Function(f) => {
                        (name_expr(names[0].clone()), &**f, names[0].name.clone())
                    }
                    _ => return Vec::new(),
                }
            }
            K::Assign { targets, values } if targets.len() == 1 && values.len() == 1 => {
                match (&values[0].kind, path_label(&targets[0])) {
                    (ir::ExprKind::Function(f), Some(label)) => (targets[0].clone(), &**f, label),
                    _ => return Vec::new(),
                }
            }
            _ => return Vec::new(),
        };
        let marks = self.fn_marks.get(&func.pos).cloned().unwrap_or_default();
        // Generated helpers (`__wl…`) are never instrumented.
        if fallback.starts_with("__wl") {
            return Vec::new();
        }
        let tools = &self.ctx.tools;
        let profile = tools.instrument > 0
            && !marks.noprofile
            && (marks.profile.is_some()
                || tools.instrument == 3
                || (tools.instrument == 2 && marks.exported));
        let trace = tools.log_level == 0 && marks.trace.is_some();
        if !profile && !trace && marks.reflect_meta.is_none() {
            return Vec::new();
        }
        let label = marks.label.clone().unwrap_or(fallback);
        let location = format!("{}:{}", self.ctx.file, func.pos.line);
        let end = func.body.end;
        let mut out = Vec::new();
        if trace {
            self.slices.insert(Slice::Trace);
            let opts = marks.trace.clone().unwrap_or_default();
            out.push(self.wrap_stmt("trace", &target, &label, &location, opts, end));
        }
        if profile {
            self.slices.insert(Slice::Prof);
            let opts = marks.profile.clone().unwrap_or_default();
            out.push(self.wrap_stmt("prof", &target, &label, &location, opts, end));
        }
        // `@reflect`: the final value is registered, after the wrappers.
        if let Some(meta) = marks.reflect_meta.clone() {
            self.slices.insert(Slice::Reflect);
            let call_ir = self.wl_call("reflectFn", vec![target.clone(), meta], end);
            out.push(stmt(ir::StmtKind::Call(call_ir), end));
        }
        self.uses_runtime = true;
        out
    }

    /// `target = __wl.<slice>.wrap(target, "label", "location", opts…)`.
    fn wrap_stmt(
        &mut self,
        slice: &str,
        target: &ir::Expr,
        label: &str,
        location: &str,
        opts: Vec<ir::Expr>,
        pos: Pos,
    ) -> ir::Stmt {
        let wl = self.runtime_ref(pos);
        let wrap = field(field(wl, slice, pos), "wrap", pos);
        let mut args = vec![
            target.clone(),
            self.string(label, pos),
            self.string(location, pos),
        ];
        args.extend(opts.into_iter().take(1));
        assign(target.clone(), call(wrap, args, pos), pos)
    }
}

/// The display name of an assignment target (`M.f`, `f`), when it is a
/// plain path.
fn path_label(e: &ir::Expr) -> Option<String> {
    match &e.kind {
        ir::ExprKind::Name(n) => Some(n.name.clone()),
        ir::ExprKind::Field { obj, name } => Some(format!("{}.{}", path_label(obj)?, name.name)),
        _ => None,
    }
}
