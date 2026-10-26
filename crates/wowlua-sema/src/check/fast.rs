//! What the strict lowering's fast forms rely on (spec §1.2.4, §1.4.1,
//! §1.4.2; decision L28: type-based fast forms only where types are
//! trustworthy). In strict mode the checker records, for the expressions
//! the forms look at, what every check of the expression found its type to
//! allow; the lowering reads them by span ([`CheckOutput::fast_forms`]).
//!
//! [`CheckOutput::fast_forms`]: super::CheckOutput::fast_forms

use super::body::Checker;
use super::env::DefKind;
use super::types::{Ty, TypeId};
use wowlua_syntax::view::{BinOp, Expr, ExprKind, InterpPart};

/// The value is never `false`: no member of its type is `false`,
/// `boolean`, or a type that does not say (`any`, `unknown`, a type
/// parameter). `a ?? b` may be `a or b`, and `?.` may test with `and`.
pub const NEVER_FALSE: u8 = 1;
/// The value is a `string` or a `number`, never `nil`: a backtick string
/// concatenates it without `tostring`.
pub const STRING_OR_NUMBER: u8 = 2;
/// The checker met the expression (facts are only of checked ones).
const CHECKED: u8 = 0x80;

/// The expressions the fast forms look at, in walk order: the embedded
/// values of backtick strings, the left operands of `??`, the operands
/// before each `?.`, and the chains that contain one.
pub(crate) fn spans(parse: &wowlua_syntax::Parse) -> Vec<wowlua_span::Span> {
    let mut out = Vec::new();
    wowlua_syntax::visit::walk_block(parse.view().block(), &mut |e| match e.kind() {
        ExprKind::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    out.push(x.span());
                }
            }
        }
        ExprKind::Binary {
            op: BinOp::Coalesce,
            lhs,
            ..
        } => out.push(lhs.span()),
        ExprKind::OptField { obj, .. }
        | ExprKind::OptIndex { obj, .. }
        | ExprKind::OptMethod { obj, .. } => {
            out.push(obj.span());
            out.push(e.span());
        }
        ExprKind::OptCall { callee, .. } => {
            out.push(callee.span());
            out.push(e.span());
        }
        ExprKind::Field { .. }
        | ExprKind::Index { .. }
        | ExprKind::Method { .. }
        | ExprKind::Call { .. }
            if has_optional(e) =>
        {
            out.push(e.span())
        }
        _ => {}
    });
    out
}

/// `e` is a chain with a `?.` link.
fn has_optional(e: Expr<'_>) -> bool {
    match e.kind() {
        ExprKind::OptField { .. }
        | ExprKind::OptIndex { .. }
        | ExprKind::OptMethod { .. }
        | ExprKind::OptCall { .. } => true,
        ExprKind::Field { obj, .. }
        | ExprKind::Index { obj, .. }
        | ExprKind::Method { obj, .. } => has_optional(obj),
        ExprKind::Call { callee, .. } => has_optional(callee),
        _ => false,
    }
}

impl Checker<'_, '_> {
    /// Records what type `t` of the expression at `span` allows, if a fast
    /// form looks at it: what every check found.
    pub(crate) fn note_fast(&mut self, span: (u32, u32), t: TypeId) {
        if self.fast.is_empty() || !self.fast.contains_key(&span) {
            return;
        }
        let found = self.fast_facts(t, 0) | CHECKED;
        if let Some(f) = self.fast.get_mut(&span) {
            *f = if *f & CHECKED == 0 { found } else { *f & found };
        }
    }

    /// The facts of the checked expressions, by span.
    pub(crate) fn fast_forms(&self) -> impl Iterator<Item = ((u32, u32), u8)> + '_ {
        self.fast
            .iter()
            .filter(|(_, f)| **f & CHECKED != 0 && **f & !CHECKED != 0)
            .map(|(&s, &f)| (s, f & !CHECKED))
    }

    fn fast_facts(&mut self, t: TypeId, depth: u32) -> u8 {
        let members = self.env.types.members(t);
        let mut facts = NEVER_FALSE | STRING_OR_NUMBER;
        for m in members {
            facts &= self.member_facts(m, depth);
            if facts == 0 {
                break;
            }
        }
        facts
    }

    fn member_facts(&mut self, m: TypeId, depth: u32) -> u8 {
        match self.env.types.get(m).clone() {
            Ty::Number | Ty::String | Ty::Num(_) | Ty::Str(_) => NEVER_FALSE | STRING_OR_NUMBER,
            Ty::Never => NEVER_FALSE | STRING_OR_NUMBER,
            Ty::Nil
            | Ty::Bool(true)
            | Ty::Table
            | Ty::Function
            | Ty::Thread
            | Ty::Userdata
            | Ty::Array(_)
            | Ty::Map(..)
            | Ty::Record(_)
            | Ty::Func(_)
            | Ty::Intersection(_)
            | Ty::ClassOf(_)
            | Ty::Module(_) => NEVER_FALSE,
            Ty::EnumMember(def, i) if depth < 8 => {
                match self
                    .env
                    .def(def)
                    .enum_members
                    .get(i as usize)
                    .map(|(_, t)| *t)
                {
                    Some(t) => self.fast_facts(t, depth + 1),
                    None => 0,
                }
            }
            Ty::Named(def, args) if depth < 8 => {
                let (kind, alias, members) = {
                    let d = self.env.def(def);
                    let members: Vec<TypeId> = d.enum_members.iter().map(|(_, t)| *t).collect();
                    (d.kind, d.alias, members)
                };
                match kind {
                    DefKind::Class | DefKind::Interface | DefKind::Namespace => NEVER_FALSE,
                    DefKind::Enum if !members.is_empty() => {
                        let mut facts = NEVER_FALSE | STRING_OR_NUMBER;
                        for t in members {
                            facts &= self.fast_facts(t, depth + 1);
                        }
                        facts
                    }
                    DefKind::Alias if args.is_empty() => match alias {
                        Some(body) => self.fast_facts(body, depth + 1),
                        None => 0,
                    },
                    _ => 0,
                }
            }
            // `false`, `boolean`, and types that do not say what the value is.
            _ => 0,
        }
    }
}
