//! Flow-sensitive narrowing (spec §2.5): the types of locals along control
//! flow, refined by conditions.

use super::body::{Checker, VarId};
use super::types::*;
use std::collections::HashMap;
use wowlua_syntax::ast::*;

/// The narrowed types of locals at one point of a function.
pub type Flow = HashMap<VarId, TypeId>;

impl Checker<'_> {
    /// Merges the flows of two paths that meet.
    pub fn join(&mut self, a: &Flow, b: &Flow, a_live: bool, b_live: bool) -> Flow {
        match (a_live, b_live) {
            (true, false) => return a.clone(),
            (false, true) => return b.clone(),
            (false, false) => return a.clone(),
            _ => {}
        }
        let mut out = Flow::new();
        for (v, &ta) in a {
            let tb = b.get(v).copied().unwrap_or_else(|| self.var_base(*v));
            out.insert(*v, self.env.types.union(&[ta, tb]));
        }
        for (v, &tb) in b {
            if !out.contains_key(v) {
                let ta = self.var_base(*v);
                out.insert(*v, self.env.types.union(&[ta, tb]));
            }
        }
        out
    }

    /// `t` where it is truthy: without `nil` and `false`.
    pub fn truthy(&mut self, t: TypeId) -> TypeId {
        let members: Vec<TypeId> = self
            .env
            .types
            .members(t)
            .into_iter()
            .filter_map(|m| match self.env.types.get(m) {
                Ty::Nil | Ty::Bool(false) => None,
                Ty::Boolean => Some(Types::TRUE),
                _ => Some(m),
            })
            .collect();
        self.env.types.union(&members)
    }

    /// `t` where it is falsy: `nil` or `false` (other members cannot be).
    pub fn falsy(&mut self, t: TypeId) -> TypeId {
        let members: Vec<TypeId> = self
            .env
            .types
            .members(t)
            .into_iter()
            .filter_map(|m| match self.env.types.get(m) {
                Ty::Nil | Ty::Bool(false) => Some(m),
                Ty::Boolean => Some(Types::FALSE),
                Ty::Any | Ty::Unknown | Ty::Error => Some(m),
                _ => None,
            })
            .collect();
        self.env.types.union(&members)
    }

    /// The local a condition operand refers to (`x`, `(x)`).
    fn narrowable(&self, e: &Expr) -> Option<VarId> {
        match &e.kind {
            ExprKind::Name(n) => self.lookup_var(&n.text),
            ExprKind::Paren(inner) => self.narrowable(inner),
            _ => None,
        }
    }

    /// The flows where `cond` is true and where it is false. The condition
    /// itself must already have been checked.
    pub fn narrow(&mut self, cond: &Expr, flow: &Flow) -> (Flow, Flow) {
        let mut t_flow = flow.clone();
        let mut f_flow = flow.clone();
        match &cond.kind {
            ExprKind::Paren(inner) => return self.narrow(inner, flow),
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => {
                let (t, f) = self.narrow(operand, flow);
                return (f, t);
            }
            ExprKind::Binary {
                op: BinOp::And,
                lhs,
                rhs,
            } => {
                let (lt, lf) = self.narrow(lhs, flow);
                let (rt, rf) = self.narrow(rhs, &lt);
                let f = self.join(&lf, &rf, true, true);
                return (rt, f);
            }
            ExprKind::Binary {
                op: BinOp::Or,
                lhs,
                rhs,
            } => {
                let (lt, lf) = self.narrow(lhs, flow);
                let (rt, rf) = self.narrow(rhs, &lf);
                let t = self.join(&lt, &rt, true, true);
                return (t, rf);
            }
            ExprKind::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                lhs,
                rhs,
            } => {
                let (eq_t, eq_f) = self.narrow_eq(lhs, rhs, flow);
                return if *op == BinOp::Eq {
                    (eq_t, eq_f)
                } else {
                    (eq_f, eq_t)
                };
            }
            ExprKind::Call { callee, args } => {
                self.narrow_predicate(callee, args, flow, &mut t_flow, &mut f_flow);
                return (t_flow, f_flow);
            }
            _ => {}
        }
        if let Some(v) = self.narrowable(cond) {
            let current = self.var_type_in(v, flow);
            let t = self.truthy(current);
            let f = self.falsy(current);
            t_flow.insert(v, t);
            f_flow.insert(v, f);
        }
        (t_flow, f_flow)
    }

    /// `x == nil`, `type(x) == "string"`, `x == "literal"`.
    fn narrow_eq(&mut self, lhs: &Expr, rhs: &Expr, flow: &Flow) -> (Flow, Flow) {
        let mut t_flow = flow.clone();
        let mut f_flow = flow.clone();
        let (subject, other) = if self.narrowable(lhs).is_some() || type_call(lhs).is_some() {
            (lhs, rhs)
        } else {
            (rhs, lhs)
        };
        // type(x) == "name"
        if let Some(arg) = type_call(subject)
            && let Some(v) = self.narrowable(arg)
            && let ExprKind::String(s) = &other.kind
        {
            let name = String::from_utf8_lossy(&s.value).into_owned();
            let current = self.var_type_in(v, flow);
            let (yes, no) = self.split_by_type_name(current, &name);
            t_flow.insert(v, yes);
            f_flow.insert(v, no);
            return (t_flow, f_flow);
        }
        let Some(v) = self.narrowable(subject) else {
            return (t_flow, f_flow);
        };
        let current = self.var_type_in(v, flow);
        let literal = match &other.kind {
            ExprKind::Nil => Some(Types::NIL),
            ExprKind::True => Some(Types::TRUE),
            ExprKind::False => Some(Types::FALSE),
            ExprKind::String(s) => Some(self.env.types.str(&String::from_utf8_lossy(&s.value))),
            ExprKind::Number(n) => n.value.map(|x| self.env.types.num(x)),
            _ => None,
        };
        if let Some(lit) = literal {
            let members = self.env.types.members(current);
            // Equal: the members that can hold the literal.
            let yes: Vec<TypeId> = members
                .iter()
                .copied()
                .filter(|&m| {
                    m == lit
                        || matches!(self.env.types.get(m), Ty::Any | Ty::Unknown | Ty::Error)
                        || self.literal_fits(lit, m)
                })
                .map(|m| {
                    if matches!(self.env.types.get(m), Ty::Any | Ty::Unknown | Ty::Error) {
                        m
                    } else {
                        lit
                    }
                })
                .collect();
            // Not equal: drop the member only if it is exactly the literal.
            let no: Vec<TypeId> = members.iter().copied().filter(|&m| m != lit).collect();
            let yes = self.env.types.union(&yes);
            let no = self.env.types.union(&no);
            t_flow.insert(v, yes);
            f_flow.insert(v, no);
        }
        (t_flow, f_flow)
    }

    fn literal_fits(&self, lit: TypeId, member: TypeId) -> bool {
        matches!(
            (self.env.types.get(lit), self.env.types.get(member)),
            (Ty::Str(_), Ty::String) | (Ty::Num(_), Ty::Number) | (Ty::Bool(_), Ty::Boolean)
        )
    }

    /// The members of `t` with Lua type `name`, and the others.
    pub fn split_by_type_name(&mut self, t: TypeId, name: &str) -> (TypeId, TypeId) {
        let target = match name {
            "nil" => Types::NIL,
            "boolean" => Types::BOOLEAN,
            "number" => Types::NUMBER,
            "string" => Types::STRING,
            "table" => Types::TABLE,
            "function" => Types::FUNCTION,
            "thread" => Types::THREAD,
            "userdata" => Types::USERDATA,
            _ => return (t, t),
        };
        let mut yes = Vec::new();
        let mut no = Vec::new();
        for m in self.env.types.members(t) {
            match self.lua_type_of(m) {
                Some(n) if n == name => yes.push(m),
                Some(_) => no.push(m),
                None => {
                    // `any`, `unknown`: narrowed to the named type.
                    yes.push(target);
                    no.push(m);
                }
            }
        }
        (self.env.types.union(&yes), self.env.types.union(&no))
    }

    /// The `type()` name of values of `t`, if fixed.
    fn lua_type_of(&self, t: TypeId) -> Option<&'static str> {
        Some(match self.env.types.get(t) {
            Ty::Nil => "nil",
            Ty::Boolean | Ty::Bool(_) => "boolean",
            Ty::Number | Ty::Num(_) => "number",
            Ty::String | Ty::Str(_) => "string",
            Ty::Table
            | Ty::Array(_)
            | Ty::Map(..)
            | Ty::Record(_)
            | Ty::ClassOf(_)
            | Ty::Module(_) => "table",
            Ty::Named(d, _) => match self.env.def(*d).kind {
                super::env::DefKind::Enum => return None,
                _ if self.env.def(*d).widget.is_some() => "table",
                _ => "table",
            },
            Ty::EnumMember(..) => return None,
            Ty::Function | Ty::Func(_) | Ty::Intersection(_) => "function",
            Ty::Thread => "thread",
            Ty::Userdata => "userdata",
            _ => return None,
        })
    }

    /// A call of a type predicate (`isButton(x)`, spec §2.2.8).
    fn narrow_predicate(
        &mut self,
        callee: &Expr,
        args: &Args,
        flow: &Flow,
        t_flow: &mut Flow,
        f_flow: &mut Flow,
    ) {
        let callee_ty = self.peek_type(callee);
        let Ty::Func(f) = self.env.types.get(callee_ty).clone() else {
            return;
        };
        let Some((index, target)) = f.predicate else {
            return;
        };
        let args = args.exprs();
        let Some(arg) = args.get(index) else {
            return;
        };
        let Some(v) = self.narrowable(arg) else {
            return;
        };
        let current = self.var_type_in(v, flow);
        let mut yes = Vec::new();
        let mut no = Vec::new();
        for m in self.env.types.members(current) {
            if self.env.assignable(m, target, super::query::Mode::Strict) {
                yes.push(m);
            } else {
                no.push(m);
            }
        }
        let yes = if yes.is_empty() {
            target
        } else {
            self.env.types.union(&yes)
        };
        let no = self.env.types.union(&no);
        t_flow.insert(v, yes);
        f_flow.insert(v, if no == Types::NEVER { current } else { no });
    }
}

/// `type(x)`: its argument.
fn type_call(e: &Expr) -> Option<&Expr> {
    let ExprKind::Call { callee, args } = &e.kind else {
        return None;
    };
    let ExprKind::Name(n) = &callee.kind else {
        return None;
    };
    if n.text.as_ref() != "type" {
        return None;
    }
    match args.exprs().as_slice() {
        [only] => Some(only),
        _ => None,
    }
}
