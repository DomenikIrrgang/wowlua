//! Runtime type checks (spec §7.5): at function entry for annotated
//! parameters, and on `::` assertions, from the checks the type checker
//! found for each static type.

use super::*;
use wowlua_sema::check::runtime::{RuntimeCheck, ValueCheck};

impl<'a> Lowerer<'a> {
    /// Whether a function with these marks gets entry checks.
    pub(crate) fn checks_function(&self, marks: &FnMarks) -> bool {
        if marks.unchecked || self.ctx.compiler_known || self.ctx.dependency {
            return false;
        }
        match self.ctx.tools.runtime_checks {
            0 => marks.checked,
            1 => marks.checked || marks.exported || marks.global,
            _ => true,
        }
    }

    /// The entry checks of a function's annotated parameters.
    pub(crate) fn entry_checks(&mut self, f: &FnShape<'a>, label: &str) -> Vec<ir::Stmt> {
        let pos = self.span_pos(f.params_span);
        let location = format!("{}:{}", self.ctx.file, pos.line);
        let mut out = Vec::new();
        // Arguments count from the first declared parameter (`self` of a
        // method is not one).
        for (i, p) in f.params.iter().enumerate() {
            // A later parameter of the same name (`_: number, _: any`)
            // hides this one: its name reads the later one's value, and
            // the body can't read this one at all (decision T262).
            if f.params[i + 1..]
                .iter()
                .any(|q| q.name.text() == p.name.text())
            {
                continue;
            }
            let Some(check) = self
                .ctx
                .value_checks
                .get(&(p.name.span().start, p.name.span().end))
                .cloned()
            else {
                continue;
            };
            let value = name_expr(self.ident(p.name));
            let cond = self.check_cond(&check, value.clone(), pos);
            let fail = self.check_fail(i + 1, p.name.text(), label, &location, &check, value, pos);
            out.push(if_stmt(not(cond, pos), vec![fail], pos));
        }
        // `...: T`: value by value.
        if let Some(v) = &f.vararg
            && let Some(check) = self
                .ctx
                .value_checks
                .get(&(v.span.start, v.span.end))
                .cloned()
        {
            let i = self.temp("i");
            let value = self.temp("v");
            let select = self.global("select", None, pos);
            let count = call(
                select.clone(),
                vec![self.string("#", pos), lit(ir::ExprKind::Vararg, pos)],
                pos,
            );
            let v_expr = name_expr(generated(&value, pos));
            let cond = self.check_cond(&check, v_expr.clone(), pos);
            let index = binary(
                ir::BinOp::Add,
                name_expr(generated(&i, pos)),
                lit(ir::ExprKind::Number(f.params.len().to_string().into()), pos),
                pos,
            );
            let fail_call = self.fail_call(index, "...", label, &location, &check, v_expr, pos);
            let body = vec![
                local(
                    generated(&value, pos),
                    call(
                        select,
                        vec![
                            name_expr(generated(&i, pos)),
                            lit(ir::ExprKind::Vararg, pos),
                        ],
                        pos,
                    ),
                    pos,
                ),
                if_stmt(
                    not(cond, pos),
                    vec![stmt(ir::StmtKind::Call(fail_call), pos)],
                    pos,
                ),
            ];
            out.push(stmt(
                ir::StmtKind::NumericFor {
                    var: generated(&i, pos),
                    start: Box::new(lit(ir::ExprKind::Number("1".into()), pos)),
                    limit: Box::new(count),
                    step: None,
                    do_pos: pos,
                    body: ir::Block {
                        stmts: body,
                        end: pos,
                    },
                },
                pos,
            ));
        }
        if !out.is_empty() {
            self.uses_runtime = true;
            self.slices.insert(Slice::Check);
        }
        out
    }

    /// `x :: T` in profiles that check everything: the value, checked
    /// through a temporary.
    pub(crate) fn checked_cast(&mut self, e: Expr<'a>, value: ir::Expr) -> ir::Expr {
        let Some(check) = self
            .ctx
            .value_checks
            .get(&(e.span().start, e.span().end))
            .cloned()
            .filter(|_| {
                self.ctx.tools.runtime_checks == 2
                    && !self.ctx.compiler_known
                    && !self.ctx.dependency
            })
        else {
            return value;
        };
        let pos = self.span_pos(e.span());
        let t = self.temp("c");
        self.prelude.push(local(generated(&t, pos), value, pos));
        let v = name_expr(generated(&t, pos));
        let cond = self.check_cond(&check, v.clone(), pos);
        let wl = self.runtime_ref(pos);
        let fail = call(
            field(field(wl, "check", pos), "castFail", pos),
            vec![
                self.string(&check.expected, pos),
                v.clone(),
                self.string(&self.location(e.span()), pos),
            ],
            pos,
        );
        self.prelude.push(if_stmt(
            not(cond, pos),
            vec![stmt(ir::StmtKind::Call(fail), pos)],
            pos,
        ));
        self.slices.insert(Slice::Check);
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn check_fail(
        &mut self,
        index: usize,
        name: &str,
        label: &str,
        location: &str,
        check: &ValueCheck,
        value: ir::Expr,
        pos: Pos,
    ) -> ir::Stmt {
        let index = lit(ir::ExprKind::Number(index.to_string().into()), pos);
        let c = self.fail_call(index, name, label, location, check, value, pos);
        stmt(ir::StmtKind::Call(c), pos)
    }

    /// `__wl.check.fail(index, name, function, location, expected, value)`.
    #[allow(clippy::too_many_arguments)]
    fn fail_call(
        &mut self,
        index: ir::Expr,
        name: &str,
        label: &str,
        location: &str,
        check: &ValueCheck,
        value: ir::Expr,
        pos: Pos,
    ) -> ir::Expr {
        let wl = self.runtime_ref(pos);
        call(
            field(field(wl, "check", pos), "fail", pos),
            vec![
                index,
                self.string(name, pos),
                self.string(label, pos),
                self.string(location, pos),
                self.string(&check.expected, pos),
                value,
            ],
            pos,
        )
    }

    /// The condition that `value` passes `check`.
    fn check_cond(&mut self, check: &ValueCheck, value: ir::Expr, pos: Pos) -> ir::Expr {
        let c = self.cond(&check.check, value.clone(), pos);
        if check.optional {
            let is_nil = binary(ir::BinOp::Eq, value, lit(ir::ExprKind::Nil, pos), pos);
            or(is_nil, paren(c), pos)
        } else {
            c
        }
    }

    fn cond(&mut self, check: &RuntimeCheck, value: ir::Expr, pos: Pos) -> ir::Expr {
        match check {
            RuntimeCheck::Types(types) => {
                let parts: Vec<ir::Expr> = types
                    .iter()
                    .map(|t| self.type_is(value.clone(), t, pos))
                    .collect();
                any(parts, pos)
            }
            // A large union (`EventName`): one lookup in a set made once,
            // not a comparison per member (Lua refuses an expression that
            // deep).
            RuntimeCheck::Literals(literals) if literals.len() > SET_MIN => {
                let name = match self.literal_sets.iter().find(|(_, l)| l == literals) {
                    Some((name, _)) => name.clone(),
                    None => {
                        let name = format!("__wl_set{}", self.literal_sets.len() + 1);
                        self.literal_sets.push((name.clone(), literals.clone()));
                        name
                    }
                };
                let member = ir::Expr::new(
                    ir::ExprKind::Index {
                        facts: Default::default(),
                        obj: Box::new(name_expr(generated(&name, pos))),
                        key: Box::new(value),
                    },
                    pos,
                );
                binary(ir::BinOp::Eq, member, lit(ir::ExprKind::True, pos), pos)
            }
            RuntimeCheck::Literals(literals) => {
                let parts: Vec<ir::Expr> = literals
                    .iter()
                    .map(|l| binary(ir::BinOp::Eq, value.clone(), literal(l, pos), pos))
                    .collect();
                any(parts, pos)
            }
            RuntimeCheck::Instance(class) => {
                if matches!(
                    self.lookup(class),
                    Some(Binding::Class { .. } | Binding::ImportedClass(_))
                ) {
                    let wl = self.runtime_ref(pos);
                    self.slices.insert(Slice::Class);
                    call(
                        field(wl, "isInstance", pos),
                        vec![value, name_expr(ident(class, pos))],
                        pos,
                    )
                } else {
                    self.type_is(value, "table", pos)
                }
            }
            RuntimeCheck::Table(fields) => {
                let is_table = self.type_is(value.clone(), "table", pos);
                let mut c: Option<ir::Expr> = None;
                for (name, ty) in fields {
                    let key = self.string(name, pos);
                    let member = ir::Expr::new(
                        ir::ExprKind::Index {
                            facts: Default::default(),
                            obj: Box::new(value.clone()),
                            key: Box::new(key),
                        },
                        pos,
                    );
                    let part = match ty {
                        Some(t) => self.type_is(member, t, pos),
                        None => binary(ir::BinOp::Ne, member, lit(ir::ExprKind::Nil, pos), pos),
                    };
                    c = Some(match c {
                        Some(c) => binary(ir::BinOp::And, c, part, pos),
                        None => part,
                    });
                }
                // A test's `mock.partial` fake has only the fields the test
                // needs, and passes (spec §16, decision S42): asked only
                // when a field is missing, so real values pay nothing.
                match c {
                    Some(c) => {
                        let wl = self.runtime_ref(pos);
                        let partial = call(
                            field(field(wl, "check", pos), "partial", pos),
                            vec![value],
                            pos,
                        );
                        binary(
                            ir::BinOp::And,
                            is_table,
                            paren(or(paren(c), partial, pos)),
                            pos,
                        )
                    }
                    None => is_table,
                }
            }
            RuntimeCheck::AnyOf(parts) => {
                let parts: Vec<ir::Expr> = parts
                    .iter()
                    .map(|p| paren(self.cond(p, value.clone(), pos)))
                    .collect();
                any(parts, pos)
            }
        }
    }

    /// `type(value) == "t"`.
    fn type_is(&mut self, value: ir::Expr, t: &str, pos: Pos) -> ir::Expr {
        let type_fn = self.global("type", None, pos);
        binary(
            ir::BinOp::Eq,
            call(type_fn, vec![value], pos),
            self.string(t, pos),
            pos,
        )
    }
}

fn not(e: ir::Expr, pos: Pos) -> ir::Expr {
    ir::Expr::new(
        ir::ExprKind::Unary {
            facts: ir::Facts::NO_META,
            op: ir::UnOp::Not,
            operand: Box::new(paren(e)),
        },
        pos,
    )
}

fn or(a: ir::Expr, b: ir::Expr, pos: Pos) -> ir::Expr {
    binary(ir::BinOp::Or, a, b, pos)
}

fn any(parts: Vec<ir::Expr>, pos: Pos) -> ir::Expr {
    let mut it = parts.into_iter();
    let first = it.next().unwrap_or_else(|| lit(ir::ExprKind::True, pos));
    it.fold(first, |acc, p| or(acc, p, pos))
}

/// A Lua literal from its source text (`"LEFT"`, `2`, `true`).
/// Unions of more literals than this are checked against a set.
const SET_MIN: usize = 8;

/// `{ [lit] = true, … }`: the set of a large union's literals.
pub(crate) fn literal_set(literals: &[String], pos: Pos) -> ir::Expr {
    let fields = literals
        .iter()
        .map(|l| ir::Field::Keyed {
            open: pos,
            key: Box::new(literal(l, pos)),
            value: lit(ir::ExprKind::True, pos),
        })
        .collect();
    ir::Expr::new(ir::ExprKind::Table { fields, close: pos }, pos)
}

fn literal(text: &str, pos: Pos) -> ir::Expr {
    match text {
        "true" => lit(ir::ExprKind::True, pos),
        "false" => lit(ir::ExprKind::False, pos),
        t if t.starts_with('"') => lit(ir::ExprKind::String(t.as_bytes().into()), pos),
        t => lit(ir::ExprKind::Number(t.into()), pos),
    }
}
