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
use wowlua_syntax::view::{BinOp, Block, ClassMemberKind, Expr, ExprKind, InterpPart, StmtKind};

/// The value is never `false`: no member of its type is `false`,
/// `boolean`, or a type that does not say (`any`, `unknown`, a type
/// parameter). `a ?? b` may be `a or b`, and `?.` may test with `and`.
pub const NEVER_FALSE: u8 = 1;
/// The value is a `string` or a `number`, never `nil`: a backtick string
/// concatenates it without `tostring`.
pub const STRING_OR_NUMBER: u8 = 2;
/// The value is a number (spec optimization §15.2.3): arithmetic and
/// comparisons with another number run no metamethod.
pub const NUMBER: u8 = 4;
/// The value is a string: `..` and comparisons with another string run no
/// metamethod.
pub const STRING: u8 = 8;
/// The value is a table, never `nil`, whose reads and writes run no code:
/// an array, a map, a record or interface value, a namespace, a widget or
/// declared class, or an instance or class table of a package class
/// without accessors or `__index` and `__newindex` metamethods. Records
/// and interfaces only while no class of the package has those (one of
/// its instances could be held as one), and no record with an `any`
/// indexer (a `setmetatable` object whose `__index` is a function).
pub const PLAIN: u8 = 16;
/// The value is a plain array (`T[]`).
pub const ARRAY: u8 = 32;
/// The checker met the expression (facts are only of checked ones).
const CHECKED: u8 = 0x80;

/// The expressions the fast forms look at, in walk order: the embedded
/// values of backtick strings, the left operands of `??`, the operands
/// before each `?.`, and the chains that contain one.
pub(crate) fn spans(parse: &wowlua_syntax::Parse) -> Vec<wowlua_span::Span> {
    let mut out = Vec::new();
    walk_with_classes(parse.view().block(), &mut |e| match e.kind() {
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
            out.push(e.span());
            optimized(e, &mut out);
        }
        _ => optimized(e, &mut out),
    });
    out
}

/// The expressions the optimizations that need types look at (spec
/// optimization §15.2.3): the objects of fields and indexes, the operands
/// of operators, and the arguments of `ipairs`, `table.insert` and
/// `tinsert`.
fn optimized<'t>(e: Expr<'t>, out: &mut Vec<wowlua_span::Span>) {
    match e.kind() {
        ExprKind::Field { obj, .. }
        | ExprKind::Index { obj, .. }
        | ExprKind::Method { obj, .. } => out.push(obj.span()),
        ExprKind::Binary { op, lhs, rhs } => {
            if !matches!(op, BinOp::And | BinOp::Or | BinOp::Coalesce) {
                out.push(lhs.span());
                out.push(rhs.span());
            }
        }
        ExprKind::Unary { operand, .. } => out.push(operand.span()),
        ExprKind::Call { callee, args } => {
            let named = match callee.kind() {
                ExprKind::Name(n) => matches!(n.text(), "ipairs" | "tinsert"),
                ExprKind::Field { obj, name } => {
                    name.text() == "insert"
                        && matches!(obj.kind(), ExprKind::Name(t) if t.text() == "table")
                }
                _ => false,
            };
            if named && let Some(first) = args.exprs().first() {
                out.push(first.span());
            }
        }
        _ => {}
    }
}

/// [`wowlua_syntax::visit::walk_block`], and the bodies and field values of
/// the classes declared in the block (which it leaves out).
fn walk_with_classes<'t>(block: Block<'t>, f: &mut impl FnMut(Expr<'t>)) {
    wowlua_syntax::visit::walk_block(block, f);
    classes_in(block, f);
}

fn classes_in<'t>(block: Block<'t>, f: &mut impl FnMut(Expr<'t>)) {
    for stmt in block.stmts() {
        class_stmt(stmt, f);
    }
}

fn class_stmt<'t>(stmt: wowlua_syntax::view::Stmt<'t>, f: &mut impl FnMut(Expr<'t>)) {
    match stmt.kind() {
        StmtKind::Class(c) => {
            for m in c.members() {
                match m.kind() {
                    ClassMemberKind::Field { value: Some(v), .. } => {
                        wowlua_syntax::visit::walk_expr(v, f)
                    }
                    ClassMemberKind::Method { func, .. }
                    | ClassMemberKind::Constructor { func, .. } => {
                        walk_with_classes(func.body(), f)
                    }
                    ClassMemberKind::Getter { body, .. } | ClassMemberKind::Setter { body, .. } => {
                        walk_with_classes(body, f)
                    }
                    _ => {}
                }
            }
        }
        StmtKind::Export(inner) => class_stmt(inner, f),
        StmtKind::Do(b) | StmtKind::While { body: b, .. } | StmtKind::Repeat { body: b, .. } => {
            classes_in(b, f)
        }
        StmtKind::NumericFor { body, .. } | StmtKind::GenericFor { body, .. } => {
            classes_in(body, f)
        }
        StmtKind::If {
            then_block,
            elseifs,
            else_block,
            ..
        } => {
            classes_in(then_block, f);
            for (_, b) in elseifs {
                classes_in(b, f);
            }
            if let Some(b) = else_block {
                classes_in(b, f);
            }
        }
        StmtKind::Function { func, .. } | StmtKind::LocalFunction { func, .. } => {
            classes_in(func.body(), f)
        }
        _ => {}
    }
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
        let found = self.fast_facts(t, 0) | self.opt_facts(t, 0) | CHECKED;
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

    /// [`NUMBER`], [`STRING`], [`PLAIN`] and [`ARRAY`] of type `t`: what
    /// every member of it is.
    fn opt_facts(&mut self, t: TypeId, depth: u32) -> u8 {
        let mut facts = NUMBER | STRING | PLAIN | ARRAY;
        for m in self.env.types.members(t) {
            facts &= self.member_opt_facts(m, depth);
            if facts == 0 {
                break;
            }
        }
        facts
    }

    /// Instances of no package class hold a function `__index` while this
    /// is false.
    fn structural_plain(&mut self) -> bool {
        if self.accessor_classes.is_none() {
            self.accessor_classes = Some(self.env.package_has_accessor_classes());
        }
        self.accessor_classes == Some(false)
    }

    /// A class (and its superclasses) without accessors and without
    /// `__index` or `__newindex` metamethods, or one of the declarations
    /// (a widget, a mixin): its instances and its table read and write
    /// raw, or through tables.
    fn plain_class(&mut self, def: super::types::DefId, depth: u32) -> bool {
        if depth > 16 {
            return false;
        }
        let (native, widget, bases, has_index) = {
            let d = self.env.def(def);
            let has_index = d.members.iter().chain(d.statics.iter()).any(|(name, m)| {
                matches!(
                    m.kind,
                    super::env::MemberKind::Getter | super::env::MemberKind::Setter
                ) || &**name == "__index"
                    || &**name == "__newindex"
            });
            (d.native, d.widget.is_some(), d.bases.clone(), has_index)
        };
        if native || widget {
            return true;
        }
        if has_index {
            return false;
        }
        bases
            .into_iter()
            .all(|b| match self.env.types.get(b).clone() {
                Ty::Named(base, _) => self.plain_class(base, depth + 1),
                _ => false,
            })
    }

    fn member_opt_facts(&mut self, m: TypeId, depth: u32) -> u8 {
        if depth > 8 {
            return 0;
        }
        match self.env.types.get(m).clone() {
            Ty::Number | Ty::Num(_) => NUMBER,
            Ty::String | Ty::Str(_) => STRING,
            Ty::Never => NUMBER | STRING | PLAIN | ARRAY,
            Ty::Array(_) => PLAIN | ARRAY,
            Ty::Map(..) => PLAIN,
            Ty::Record(r) => {
                let function_index = r
                    .indexer
                    .is_some_and(|(_, v)| matches!(self.env.types.get(v), Ty::Any));
                if !function_index && self.structural_plain() {
                    PLAIN
                } else {
                    0
                }
            }
            Ty::ClassOf(def) => {
                let kind = self.env.def(def).kind;
                match kind {
                    DefKind::Class if self.plain_class(def, 0) => PLAIN,
                    DefKind::Namespace => PLAIN,
                    _ => 0,
                }
            }
            Ty::EnumMember(def, i) => {
                match self
                    .env
                    .def(def)
                    .enum_members
                    .get(i as usize)
                    .map(|(_, t)| *t)
                {
                    Some(t) => self.opt_facts(t, depth + 1),
                    None => 0,
                }
            }
            Ty::Named(def, args) => {
                let (kind, alias, members) = {
                    let d = self.env.def(def);
                    let members: Vec<TypeId> = d.enum_members.iter().map(|(_, t)| *t).collect();
                    (d.kind, d.alias, members)
                };
                match kind {
                    DefKind::Class if self.plain_class(def, 0) => PLAIN,
                    DefKind::Interface | DefKind::Namespace if self.structural_plain() => PLAIN,
                    DefKind::Enum if !members.is_empty() => {
                        let mut facts = NUMBER | STRING;
                        for t in members {
                            facts &= self.opt_facts(t, depth + 1);
                        }
                        facts
                    }
                    DefKind::Alias if args.is_empty() => match alias {
                        Some(body) => self.opt_facts(body, depth + 1),
                        None => 0,
                    },
                    _ => 0,
                }
            }
            _ => 0,
        }
    }
}
