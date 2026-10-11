//! Calls of standard-library intrinsics (spec §7.1): `wowlua:debug`
//! helpers are folded to constants, given their source location and text,
//! or removed with their arguments, depending on the profile.

use super::*;

/// `wowlua:debug` helpers that may only be call statements (E3045).
const STATEMENT_ONLY: &[&str] = &[
    "debug.assert",
    "debug.dump",
    "debug.time",
    "debug.timeEnd",
    "debug.resetStats",
    "debug.watch",
];

impl Lowerer<'_> {
    /// The `@intrinsic` id of a call's callee: an imported intrinsic
    /// (`assert(x)`) or a member of a namespace import (`debug.assert(x)`).
    pub(crate) fn call_intrinsic(&self, e: &Expr) -> Option<String> {
        let ExprKind::Call { callee, .. } = &e.kind else {
            return None;
        };
        let id = match &callee.kind {
            ExprKind::Name(n) => match self.lookup(&n.text) {
                Some(Binding::Intrinsic(id)) => id.clone(),
                _ => return None,
            },
            ExprKind::Field { obj, name } => match &obj.kind {
                ExprKind::Name(m) => match self.lookup(&m.text) {
                    Some(Binding::IntrinsicNamespace(map)) => map.get(name.text.as_ref())?.clone(),
                    _ => return None,
                },
                _ => return None,
            },
            _ => return None,
        };
        id.starts_with("debug.").then_some(id)
    }

    /// `"src/a.wowlua:12 (onLogin)"`: the source location of `span` and the
    /// enclosing named function.
    pub(crate) fn location(&self, span: Span) -> String {
        let line = self.parse.lexed.line_of(span.start);
        let mut out = format!("{}:{line}", self.ctx.file);
        if let Some(name) = self.fn_names.iter().rev().flatten().next() {
            out.push_str(&format!(" ({name})"));
        }
        out
    }

    /// The source text of an expression, on one line.
    fn source_text(&self, span: Span) -> String {
        let text = String::from_utf8_lossy(&self.src[span.start as usize..span.end as usize]);
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// A call statement of an intrinsic.
    pub(crate) fn intrinsic_stmt(&mut self, id: &str, e: &Expr, pos: Pos) -> Vec<ir::Stmt> {
        let ExprKind::Call { callee, args } = &e.kind else {
            return Vec::new();
        };
        let tools = &self.ctx.tools;
        let removed = match id {
            "debug.assert" => !tools.assertions,
            "debug.dump" | "debug.time" | "debug.timeEnd" | "debug.resetStats" => !self.ctx.debug,
            "debug.watch" => !tools.dev_console,
            _ => false,
        };
        if removed {
            return Vec::new();
        }
        self.intrinsic_slices(id);
        let call_ir = match id {
            "debug.assert" => {
                let f = self.expr(callee);
                let mut a = self.args_list(args);
                a.truncate(2);
                while a.len() < 2 {
                    a.push(lit(ir::ExprKind::Nil, pos));
                }
                a.push(self.string(&self.location(e.span), pos));
                call(f, a, pos)
            }
            "debug.dump" => {
                let f = self.expr(callee);
                let mut a = vec![self.string(&self.location(e.span), pos)];
                for x in args.exprs() {
                    a.push(self.string(&self.source_text(x.span), pos));
                    a.push(self.expr(x));
                }
                call(f, a, pos)
            }
            _ if STATEMENT_ONLY.contains(&id) => {
                let f = self.expr(callee);
                let a = self.args_list(args);
                call(f, a, pos)
            }
            _ => self.intrinsic_expr(id, callee, args, e),
        };
        // A folded constant is not a statement.
        match call_ir.kind {
            ir::ExprKind::Call { .. } | ir::ExprKind::Method { .. } => {
                vec![stmt(ir::StmtKind::Call(call_ir), pos)]
            }
            _ => Vec::new(),
        }
    }

    /// A call of an intrinsic used as a value.
    pub(crate) fn intrinsic_expr(
        &mut self,
        id: &str,
        callee: &Expr,
        args: &Args,
        e: &Expr,
    ) -> ir::Expr {
        let pos = self.span_pos(e.span);
        let exprs = args.exprs();
        match id {
            _ if STATEMENT_ONLY.contains(&id) => {
                let name = id.trim_start_matches("debug.");
                self.error(
                    "E3045",
                    e.span,
                    format!(
                        "`{name}` can only be a call statement, because the call may be removed"
                    ),
                );
                lit(ir::ExprKind::Nil, pos)
            }
            "debug.location" => self.string(&self.location(e.span), pos),
            "debug.nameof" => match exprs.first().and_then(|x| nameof(x)) {
                Some(name) => self.string(&name, pos),
                None => {
                    self.error(
                        "E7001",
                        e.span,
                        "`nameof` needs a name, a field access or an index with a string key",
                    );
                    lit(ir::ExprKind::Nil, pos)
                }
            },
            "debug.typeName" => {
                let name = exprs
                    .first()
                    .and_then(|x| self.ctx.type_names.get(&(x.span.start, x.span.end)))
                    .cloned()
                    .unwrap_or_else(|| "any".into());
                self.string(&name, pos)
            }
            "debug.todo" | "debug.unreachable" => {
                if id == "debug.todo" && !self.ctx.debug {
                    self.warnings.push(
                        Diagnostic::warning("W7001", "`todo()` in a release build")
                            .with_label(e.span, "raises \"not implemented\" when it runs"),
                    );
                }
                let f = self.expr(callee);
                let mut a = match exprs.first() {
                    Some(x) => vec![self.expr(x)],
                    None => vec![lit(ir::ExprKind::Nil, pos)],
                };
                a.push(self.string(&self.location(e.span), pos));
                call(f, a, pos)
            }
            // Without profiling, `measure(label, fn, ...)` is `fn(...)`.
            "debug.measure" if !self.ctx.debug && exprs.len() >= 2 => {
                let f = self.expr(exprs[1]);
                let rest: Vec<ir::Expr> = exprs[2..].iter().map(|x| self.expr(x)).collect();
                call(f, rest, pos)
            }
            "debug.stats" if !self.ctx.debug && !self.ctx.tools.profiling => lit(
                ir::ExprKind::Table {
                    fields: Vec::new(),
                    close: pos,
                },
                pos,
            ),
            _ => {
                self.intrinsic_slices(id);
                let f = self.expr(callee);
                let a = self.args_list(args);
                call(f, a, pos)
            }
        }
    }

    /// The runtime slices a kept helper call needs.
    fn intrinsic_slices(&mut self, id: &str) {
        let slice = match id {
            "debug.measure" | "debug.time" | "debug.timeEnd" | "debug.stats"
            | "debug.resetStats" => Slice::Prof,
            "debug.watch" => Slice::DevConsole,
            _ => return,
        };
        self.uses_runtime = true;
        self.slices.insert(slice);
    }
}

/// The final identifier of `nameof(expr)`.
fn nameof(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Name(n) => Some(n.text.to_string()),
        ExprKind::Field { name, .. } | ExprKind::OptField { name, .. } => {
            Some(name.text.to_string())
        }
        ExprKind::Index { key, .. } | ExprKind::OptIndex { key, .. } => match &key.kind {
            ExprKind::String(s) => Some(String::from_utf8_lossy(&s.value).into_owned()),
            _ => None,
        },
        ExprKind::Paren(inner) => nameof(inner),
        _ => None,
    }
}
