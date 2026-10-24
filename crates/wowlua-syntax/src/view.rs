//! A typed, read-only view of a syntax tree (#19).
//!
//! Every consumer outside the parser reads the tree through these handles:
//! `Copy` values (`Expr<'t>`, `Stmt<'t>`, `Name<'t>`, …) whose `kind()`
//! returns an enum shaped like the tree's, with handles for children and
//! [`List`]s for sequences. A handle is the [`Tree`] and an index into one
//! of its arrays; the handles say nothing about that layout, so it can
//! change without touching a consumer.
//!
//! Operator and modifier enums ([`BinOp`], [`UnOp`], [`CompoundOp`],
//! [`Modifier`]) and [`Directive`]s are plain data.

use crate::tree::{self, NONE, Range, Sp, Tree};
use std::borrow::Cow;
use wowlua_span::Span;

pub use crate::ops::{BinOp, CompoundOp, Directive, Modifier, UnOp};

fn span(t: &Tree, s: Sp) -> Span {
    Span {
        file: t.file,
        start: s.start,
        end: s.end,
    }
}

fn opt<V>(i: u32, f: impl FnOnce(u32) -> V) -> Option<V> {
    (i != NONE).then(|| f(i))
}

// ------------------------------------------------------------------ lists --

/// A sequence of child handles: `len`, `get`, `first`, `last` and `iter`.
pub struct List<'t, V> {
    t: Option<&'t Tree>,
    start: u32,
    len: u32,
    view: fn(&'t Tree, u32) -> V,
}

impl<V> Clone for List<'_, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for List<'_, V> {}

impl<'t, V> List<'t, V> {
    fn new(t: &'t Tree, r: Range, view: fn(&'t Tree, u32) -> V) -> Self {
        List {
            t: Some(t),
            start: r.start,
            len: r.len,
            view,
        }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<V> {
        (index < self.len()).then(|| self.at(index))
    }

    /// Item `index`; panics past the end, like indexing a slice.
    pub fn at(&self, index: usize) -> V {
        assert!(
            index < self.len(),
            "index {index} out of a list of {}",
            self.len
        );
        (self.view)(
            self.t.expect("a list with items"),
            self.start + index as u32,
        )
    }

    pub fn first(&self) -> Option<V> {
        self.get(0)
    }

    pub fn last(&self) -> Option<V> {
        self.len().checked_sub(1).map(|i| self.at(i))
    }

    /// The items, when there are exactly `N` (for matching like a slice
    /// pattern: `if let Some([a, b]) = list.exactly()`).
    pub fn exactly<const N: usize>(&self) -> Option<[V; N]> {
        (self.len() == N).then(|| std::array::from_fn(|i| self.at(i)))
    }

    pub fn iter(&self) -> Iter<'t, V> {
        Iter {
            t: self.t,
            next: self.start,
            end: self.start + self.len,
            view: self.view,
        }
    }
}

/// The items of a [`List`], in order.
pub struct Iter<'t, V> {
    t: Option<&'t Tree>,
    next: u32,
    end: u32,
    view: fn(&'t Tree, u32) -> V,
}

impl<V> Iterator for Iter<'_, V> {
    type Item = V;

    fn next(&mut self) -> Option<V> {
        if self.next == self.end {
            return None;
        }
        self.next += 1;
        Some((self.view)(
            self.t.expect("a list with items"),
            self.next - 1,
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = (self.end - self.next) as usize;
        (n, Some(n))
    }
}

impl<V> DoubleEndedIterator for Iter<'_, V> {
    fn next_back(&mut self) -> Option<V> {
        if self.next == self.end {
            return None;
        }
        self.end -= 1;
        Some((self.view)(self.t.expect("a list with items"), self.end))
    }
}

impl<V> ExactSizeIterator for Iter<'_, V> {}

impl<'t, V> IntoIterator for List<'t, V> {
    type Item = V;
    type IntoIter = Iter<'t, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'t, V> IntoIterator for &List<'t, V> {
    type Item = V;
    type IntoIter = Iter<'t, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<V: std::fmt::Debug> std::fmt::Debug for List<'_, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<V> Default for List<'_, V> {
    /// No items.
    fn default() -> Self {
        List {
            t: None,
            start: 0,
            len: 0,
            view: |_, _| unreachable!("an empty list has no items"),
        }
    }
}

pub type Stmts<'t> = List<'t, Stmt<'t>>;
pub type Exprs<'t> = List<'t, Expr<'t>>;
pub type Names<'t> = List<'t, Name<'t>>;
pub type TypedNames<'t> = List<'t, TypedName<'t>>;
pub type DestructFields<'t> = List<'t, DestructField<'t>>;
pub type ElseIfs<'t> = List<'t, (Expr<'t>, Block<'t>)>;
pub type Decorators<'t> = List<'t, Decorator<'t>>;
pub type Params<'t> = List<'t, Param<'t>>;
pub type TableFields<'t> = List<'t, TableField<'t>>;
pub type InterpParts<'t> = List<'t, InterpPart<'t>>;
pub type Types<'t> = List<'t, Type<'t>>;
pub type TypeArgs<'t> = List<'t, TypeArg<'t>>;
pub type TypeParams<'t> = List<'t, TypeParam<'t>>;
pub type TableTypeFields<'t> = List<'t, TableTypeField<'t>>;
pub type ParamTypes<'t> = List<'t, ParamType<'t>>;
pub type ClassMembers<'t> = List<'t, ClassMember<'t>>;
pub type InterfaceMembers<'t> = List<'t, InterfaceMember<'t>>;
pub type EnumItems<'t> = List<'t, EnumItem<'t>>;
pub type NsMembers<'t> = List<'t, NsMember<'t>>;
pub type DeclMembers<'t> = List<'t, DeclMember<'t>>;
pub type ImportSpecs<'t> = List<'t, ImportSpec<'t>>;

macro_rules! handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy)]
        pub struct $name<'t> {
            t: &'t Tree,
            i: u32,
        }

        impl<'t> $name<'t> {
            fn new(t: &'t Tree, i: u32) -> Self {
                $name { t, i }
            }
        }

        impl PartialEq for $name<'_> {
            /// The same node (not equal content).
            fn eq(&self, other: &Self) -> bool {
                std::ptr::eq(self.t, other.t) && self.i == other.i
            }
        }

        impl Eq for $name<'_> {}

        impl std::hash::Hash for $name<'_> {
            /// By identity, like `==`.
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                std::ptr::hash(self.t, state);
                self.i.hash(state);
            }
        }
    };
}

/// A `Debug` that shows the node's content (never its index), so two
/// trees with the same content print the same.
macro_rules! debug_fields {
    ($name:ident { $($field:ident),* }) => {
        impl std::fmt::Debug for $name<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name))
                    $(.field(stringify!($field), &self.$field()))*
                    .finish()
            }
        }
    };
}

/// The handle of expression `i` of `t` (for the parser, which walks what
/// it built).
pub(crate) fn expr_at(t: &Tree, i: u32) -> Expr<'_> {
    Expr::new(t, i)
}

// ---------------------------------------------------------- the file --

handle!(
    /// A file's tree.
    Chunk
);
debug_fields!(Chunk {
    directives,
    block,
    span
});

impl<'t> Chunk<'t> {
    pub fn directives(self) -> &'t [Directive] {
        &self.t.directives
    }

    pub fn block(self) -> Block<'t> {
        Block::new(self.t, self.t.root)
    }

    pub fn span(self) -> Span {
        span(self.t, self.t.span)
    }
}

impl crate::Parse {
    /// The tree, as handles.
    pub fn view(&self) -> Chunk<'_> {
        Chunk::new(&self.tree, 0)
    }
}

handle!(Block);
debug_fields!(Block { span, stmts });

impl<'t> Block<'t> {
    fn node(self) -> &'t tree::BlockNode {
        &self.t.blocks[self.i as usize]
    }

    pub fn stmts(self) -> Stmts<'t> {
        List::new(self.t, self.node().stmts, Stmt::new)
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }
}

// ------------------------------------------------------------- statements --

handle!(Stmt);
debug_fields!(Stmt {
    span,
    decorators,
    kind
});

impl<'t> Stmt<'t> {
    fn node(self) -> &'t tree::StmtNode {
        &self.t.stmts[self.i as usize]
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }

    pub fn decorators(self) -> Decorators<'t> {
        List::new(self.t, self.node().decorators, Decorator::new)
    }

    pub fn kind(self) -> StmtKind<'t> {
        use tree::StmtData as K;
        let t = self.t;
        let exprs = |r| List::new(t, r, Expr::new);
        let expr = |i| Expr::new(t, i);
        let block = |i| Block::new(t, i);
        match self.node().data {
            K::Assign { targets, values } => StmtKind::Assign {
                targets: exprs(targets),
                values: exprs(values),
            },
            K::Compound { target, op, value } => StmtKind::Compound {
                target: expr(target),
                op,
                value: expr(value),
            },
            K::Call(e) => StmtKind::Call(expr(e)),
            K::Do(b) => StmtKind::Do(block(b)),
            K::While { cond, body } => StmtKind::While {
                cond: expr(cond),
                body: block(body),
            },
            K::Repeat { body, cond } => StmtKind::Repeat {
                body: block(body),
                cond: expr(cond),
            },
            K::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => StmtKind::If {
                cond: expr(cond),
                then_block: block(then_block),
                elseifs: List::new(t, elseifs, |t, i| {
                    let (c, b) = t.elseifs[i as usize];
                    (Expr::new(t, c), Block::new(t, b))
                }),
                else_block: opt(else_block, block),
            },
            K::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => StmtKind::NumericFor {
                var: TypedName::at(t, var),
                start: expr(start),
                limit: expr(limit),
                step: opt(step, expr),
                body: block(body),
            },
            K::GenericFor {
                vars,
                exprs: es,
                body,
            } => StmtKind::GenericFor {
                vars: List::new(t, vars, TypedName::at),
                exprs: exprs(es),
                body: block(body),
            },
            K::Function { name, func } => StmtKind::Function {
                name: FuncName::new(t, name),
                func: Func::new(t, func),
            },
            K::LocalFunction { name, func } => StmtKind::LocalFunction {
                name: Name::new(t, name),
                func: Func::new(t, func),
            },
            K::Overload { name, func } => StmtKind::Overload {
                name: Name::new(t, name),
                func: Func::new(t, func),
            },
            K::Local { names, values } => StmtKind::Local {
                names: List::new(t, names, TypedName::at),
                values: exprs(values),
            },
            K::LocalDestructure { fields, ty, value } => StmtKind::LocalDestructure {
                fields: List::new(t, fields, DestructField::at),
                ty: opt(ty, |i| Type::new(t, i)),
                value: opt(value, expr),
            },
            K::Const { name, ty, value } => StmtKind::Const {
                name: Name::new(t, name),
                ty: opt(ty, |i| Type::new(t, i)),
                value: opt(value, expr),
            },
            K::Return(values) => StmtKind::Return(exprs(values)),
            K::Break => StmtKind::Break,
            K::Continue => StmtKind::Continue,
            K::Class(c) => StmtKind::Class(ClassDecl::new(t, c)),
            K::Interface(i) => StmtKind::Interface(InterfaceDecl::new(t, i)),
            K::Enum(e) => StmtKind::Enum(EnumDecl::new(t, e)),
            K::TypeAlias(a) => StmtKind::TypeAlias(TypeAlias::new(t, a)),
            K::Declare(d) => StmtKind::Declare(Declare::new(t, d)),
            K::Import(i) => StmtKind::Import(Import::new(t, i)),
            K::Export(s) => StmtKind::Export(Stmt::new(t, s)),
            K::Error => StmtKind::Error,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum StmtKind<'t> {
    /// `a, b.c = x, y`
    Assign {
        targets: Exprs<'t>,
        values: Exprs<'t>,
    },
    /// `a += e`
    Compound {
        target: Expr<'t>,
        op: CompoundOp,
        value: Expr<'t>,
    },
    Call(Expr<'t>),
    Do(Block<'t>),
    While {
        cond: Expr<'t>,
        body: Block<'t>,
    },
    Repeat {
        body: Block<'t>,
        cond: Expr<'t>,
    },
    If {
        cond: Expr<'t>,
        then_block: Block<'t>,
        elseifs: ElseIfs<'t>,
        else_block: Option<Block<'t>>,
    },
    NumericFor {
        var: TypedName<'t>,
        start: Expr<'t>,
        limit: Expr<'t>,
        step: Option<Expr<'t>>,
        body: Block<'t>,
    },
    GenericFor {
        vars: TypedNames<'t>,
        exprs: Exprs<'t>,
        body: Block<'t>,
    },
    Function {
        name: FuncName<'t>,
        func: Func<'t>,
    },
    LocalFunction {
        name: Name<'t>,
        func: Func<'t>,
    },
    /// `overload function name(…): …`: a signature without body of the
    /// function implemented next (spec §5.1.1).
    Overload {
        name: Name<'t>,
        func: Func<'t>,
    },
    Local {
        names: TypedNames<'t>,
        values: Exprs<'t>,
    },
    LocalDestructure {
        fields: DestructFields<'t>,
        ty: Option<Type<'t>>,
        value: Option<Expr<'t>>,
    },
    Const {
        name: Name<'t>,
        ty: Option<Type<'t>>,
        value: Option<Expr<'t>>,
    },
    Return(Exprs<'t>),
    Break,
    Continue,
    Class(ClassDecl<'t>),
    Interface(InterfaceDecl<'t>),
    Enum(EnumDecl<'t>),
    TypeAlias(TypeAlias<'t>),
    Declare(Declare<'t>),
    Import(Import<'t>),
    Export(Stmt<'t>),
    Error,
}

/// A name with an optional type annotation (`local a: T`).
#[derive(Clone, Copy, Debug)]
pub struct TypedName<'t> {
    pub name: Name<'t>,
    pub ty: Option<Type<'t>>,
}

impl<'t> TypedName<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        let n = t.typed_names[i as usize];
        TypedName {
            name: Name::new(t, n.name),
            ty: opt(n.ty, |i| Type::new(t, i)),
        }
    }
}

/// A field of `local { x, y = 1 } = e`.
#[derive(Clone, Copy, Debug)]
pub struct DestructField<'t> {
    pub name: Name<'t>,
    pub default: Option<Expr<'t>>,
}

impl<'t> DestructField<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        let f = t.destructs[i as usize];
        DestructField {
            name: Name::new(t, f.name),
            default: opt(f.default, |i| Expr::new(t, i)),
        }
    }
}

handle!(
    /// `a.b.c:m`
    FuncName
);
debug_fields!(FuncName { path, method, span });

impl<'t> FuncName<'t> {
    fn node(self) -> &'t tree::FuncNameNode {
        &self.t.func_names[self.i as usize]
    }

    pub fn path(self) -> Names<'t> {
        List::new(self.t, self.node().path, Name::listed)
    }

    pub fn method(self) -> Option<Name<'t>> {
        self.node().method.map(|m| Name::new(self.t, m))
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }
}

handle!(
    /// A function: parameters, results and body (also a declaration's
    /// signature, whose body is empty).
    Func
);
debug_fields!(Func {
    type_params,
    params,
    vararg,
    ret,
    body,
    params_span,
    end_span,
    span
});

impl<'t> Func<'t> {
    fn node(self) -> &'t tree::FuncNode {
        &self.t.funcs[self.i as usize]
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(self.t, self.node().type_params, TypeParam::at)
    }

    pub fn params(self) -> Params<'t> {
        List::new(self.t, self.node().params, Param::new)
    }

    pub fn vararg(self) -> Option<Vararg<'t>> {
        let t = self.t;
        self.node().vararg.map(|(s, ty)| Vararg {
            span: span(t, s),
            ty: opt(ty, |i| VarargType::at(t, i)),
        })
    }

    pub fn ret(self) -> Option<RetType<'t>> {
        opt(self.node().ret, |i| RetType::at(self.t, i))
    }

    pub fn body(self) -> Block<'t> {
        Block::new(self.t, self.node().body)
    }

    /// The parameter list, `(` to `)`.
    pub fn params_span(self) -> Span {
        span(self.t, self.node().params_span)
    }

    /// The closing `end`.
    pub fn end_span(self) -> Span {
        span(self.t, self.node().end_span)
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }
}

handle!(Param);
debug_fields!(Param {
    decorators,
    modifiers,
    name,
    ty,
    default
});

impl<'t> Param<'t> {
    fn node(self) -> &'t tree::ParamNode {
        &self.t.params[self.i as usize]
    }

    pub fn decorators(self) -> Decorators<'t> {
        let n = self.node();
        let range = Range {
            start: n.decorators_start,
            len: u32::from(n.decorators_len),
        };
        List::new(self.t, range, Decorator::new)
    }

    /// A constructor's parameter property: `public`, `protected`,
    /// `private`, `readonly`, in that order (empty for a plain parameter;
    /// decision L35).
    pub fn modifiers(self) -> Vec<Modifier> {
        let bits = self.node().property;
        [
            (tree::PROPERTY_PUBLIC, Modifier::Public),
            (tree::PROPERTY_PROTECTED, Modifier::Protected),
            (tree::PROPERTY_PRIVATE, Modifier::Private),
            (tree::PROPERTY_READONLY, Modifier::Readonly),
        ]
        .into_iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, m)| m)
        .collect()
    }

    /// Whether it is a parameter property.
    pub fn is_property(self) -> bool {
        self.node().property != 0
    }

    pub fn name(self) -> Name<'t> {
        Name::new(self.t, self.node().name)
    }

    pub fn ty(self) -> Option<Type<'t>> {
        opt(self.node().ty, |i| Type::new(self.t, i))
    }

    pub fn default(self) -> Option<Expr<'t>> {
        opt(self.node().default, |i| Expr::new(self.t, i))
    }
}

/// `...`, with its type.
#[derive(Clone, Copy, Debug)]
pub struct Vararg<'t> {
    pub span: Span,
    pub ty: Option<VarargType<'t>>,
}

handle!(
    /// `@a.b(x)`
    Decorator
);
debug_fields!(Decorator {
    path,
    args,
    span,
    expr
});

impl<'t> Decorator<'t> {
    fn node(self) -> &'t tree::DecoratorNode {
        &self.t.decorators[self.i as usize]
    }

    pub fn path(self) -> Names<'t> {
        List::new(self.t, self.node().path, Name::listed)
    }

    pub fn args(self) -> Option<Args<'t>> {
        opt(self.node().args, |i| Args::new(self.t, i))
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }

    /// The decorator as an expression: its path, called with its arguments
    /// (`@a.b(x)` is `a.b(x)`, spec §4.5).
    pub fn expr(self) -> Expr<'t> {
        Expr::new(self.t, self.node().call)
    }

    /// A core decorator (a single name from [`crate::ops::CORE_DECORATORS`]).
    pub fn is_core(self) -> bool {
        let path = self.path();
        path.len() == 1 && crate::ops::CORE_DECORATORS.contains(&path.at(0).text())
    }

    /// The single name `name`.
    pub fn is_named(self, name: &str) -> bool {
        let path = self.path();
        path.len() == 1 && path.at(0).text() == name
    }
}

// ------------------------------------------------------------ expressions --

handle!(Expr);
debug_fields!(Expr { span, kind });

impl<'t> Expr<'t> {
    fn node(self) -> tree::ExprNode {
        self.t.expr(self.i)
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }

    /// The node's number in its file, unique per parse (never 0).
    pub fn id(self) -> u32 {
        self.i + 1
    }

    pub fn kind(self) -> ExprKind<'t> {
        use tree::ExprData as K;
        let t = self.t;
        let expr = |i| Expr::new(t, i);
        let name = |s| Name::new(t, s);
        let method = |i: u32| {
            let m = t.methods[i as usize];
            (Name::new(t, m.name), Args::new(t, m.args))
        };
        match self.node().data {
            K::Nil => ExprKind::Nil,
            K::True => ExprKind::True,
            K::False => ExprKind::False,
            K::Number(s) => ExprKind::Number(Number { t, sp: s }),
            K::String(s) => ExprKind::String(Str::new(t, s)),
            K::Vararg => ExprKind::Vararg,
            K::Function(f) => ExprKind::Function(Func::new(t, f)),
            K::Table(fields) => ExprKind::Table(List::new(t, fields, TableField::at)),
            K::Interp(parts) => ExprKind::Interp(List::new(t, parts, InterpPart::at)),
            K::Name(s) => ExprKind::Name(name(s)),
            K::Index { obj, key } => ExprKind::Index {
                obj: expr(obj),
                key: expr(key),
            },
            K::Field { obj, name: n } => ExprKind::Field {
                obj: expr(obj),
                name: name(n),
            },
            K::Call { callee, args } => ExprKind::Call {
                callee: expr(callee),
                args: Args::new(t, args),
            },
            K::Method { obj, method: m } => {
                let (name, args) = method(m);
                ExprKind::Method {
                    obj: expr(obj),
                    name,
                    args,
                }
            }
            K::OptField { obj, name: n } => ExprKind::OptField {
                obj: expr(obj),
                name: name(n),
            },
            K::OptIndex { obj, key } => ExprKind::OptIndex {
                obj: expr(obj),
                key: expr(key),
            },
            K::OptMethod { obj, method: m } => {
                let (name, args) = method(m);
                ExprKind::OptMethod {
                    obj: expr(obj),
                    name,
                    args,
                }
            }
            K::OptCall { callee, args } => ExprKind::OptCall {
                callee: expr(callee),
                args: Args::new(t, args),
            },
            K::Paren(e) => ExprKind::Paren(expr(e)),
            K::Binary { op, lhs, rhs } => ExprKind::Binary {
                op,
                lhs: expr(lhs),
                rhs: expr(rhs),
            },
            K::Unary { op, operand } => ExprKind::Unary {
                op,
                operand: expr(operand),
            },
            K::Cast { expr: e, ty } => ExprKind::Cast {
                expr: expr(e),
                ty: Type::new(t, ty),
            },
            K::Error => ExprKind::Error,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ExprKind<'t> {
    Nil,
    True,
    False,
    Number(Number<'t>),
    String(Str<'t>),
    Vararg,
    Function(Func<'t>),
    Table(TableFields<'t>),
    Interp(InterpParts<'t>),
    Name(Name<'t>),
    Index {
        obj: Expr<'t>,
        key: Expr<'t>,
    },
    Field {
        obj: Expr<'t>,
        name: Name<'t>,
    },
    Call {
        callee: Expr<'t>,
        args: Args<'t>,
    },
    Method {
        obj: Expr<'t>,
        name: Name<'t>,
        args: Args<'t>,
    },
    OptField {
        obj: Expr<'t>,
        name: Name<'t>,
    },
    OptIndex {
        obj: Expr<'t>,
        key: Expr<'t>,
    },
    OptMethod {
        obj: Expr<'t>,
        name: Name<'t>,
        args: Args<'t>,
    },
    OptCall {
        callee: Expr<'t>,
        args: Args<'t>,
    },
    Paren(Expr<'t>),
    Binary {
        op: BinOp,
        lhs: Expr<'t>,
        rhs: Expr<'t>,
    },
    Unary {
        op: UnOp,
        operand: Expr<'t>,
    },
    /// `e :: T`
    Cast {
        expr: Expr<'t>,
        ty: Type<'t>,
    },
    Error,
}

/// A name: its text is its source.
#[derive(Clone, Copy)]
pub struct Name<'t> {
    t: &'t Tree,
    sp: Sp,
}

impl<'t> Name<'t> {
    fn new(t: &'t Tree, sp: Sp) -> Self {
        Name { t, sp }
    }

    /// Item `i` of the tree's name lists.
    fn listed(t: &'t Tree, i: u32) -> Self {
        Name::new(t, t.names[i as usize])
    }

    pub fn text(self) -> &'t str {
        self.t.src.str(self.sp)
    }

    pub fn span(self) -> Span {
        span(self.t, self.sp)
    }
}

impl PartialEq for Name<'_> {
    /// The same name (the same place in the same tree).
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.t, other.t) && self.sp == other.sp
    }
}

impl Eq for Name<'_> {}

impl std::hash::Hash for Name<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.t, state);
        self.sp.hash(state);
    }
}

impl std::fmt::Debug for Name<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Name")
            .field(&self.text())
            .field(&self.span())
            .finish()
    }
}

/// A numeral.
#[derive(Clone, Copy)]
pub struct Number<'t> {
    t: &'t Tree,
    sp: Sp,
}

impl<'t> Number<'t> {
    /// The numeral as written.
    pub fn raw(self) -> &'t str {
        self.t.src.str(self.sp)
    }

    /// `None` if malformed (already reported).
    pub fn value(self) -> Option<f64> {
        crate::lexer::number_value(self.raw().as_bytes()).ok()
    }
}

impl PartialEq for Number<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.t, other.t) && self.sp == other.sp
    }
}

impl std::fmt::Debug for Number<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Number")
            .field(&self.raw())
            .field(&self.value())
            .finish()
    }
}

handle!(
    /// A string literal.
    Str
);

impl<'t> Str<'t> {
    fn node(self) -> &'t tree::StrNode {
        &self.t.strs[self.i as usize]
    }

    /// The literal as written, quotes or long brackets included.
    pub fn raw(self) -> &'t [u8] {
        let r = self.node().raw;
        &self.t.src.bytes()[r.start as usize..r.end as usize]
    }

    /// The decoded bytes.
    pub fn value(self) -> Cow<'t, [u8]> {
        let n = self.node();
        Cow::Borrowed(self.t.str_value(n.value, n.in_buf))
    }

    /// A long string (`[[…]]`).
    pub fn long(self) -> bool {
        self.node().long
    }
}

impl std::fmt::Debug for Str<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Str")
            .field("raw", &self.raw().escape_ascii().to_string())
            .field("value", &self.value().escape_ascii().to_string())
            .field("long", &self.long())
            .finish()
    }
}

/// A part of a backtick string.
#[derive(Clone, Debug)]
pub enum InterpPart<'t> {
    /// Raw text between braces (escapes not decoded), its value and span.
    Text {
        raw: &'t [u8],
        value: Cow<'t, [u8]>,
        span: Span,
    },
    Expr(Expr<'t>),
}

impl<'t> InterpPart<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.interps[i as usize] {
            tree::InterpData::Text {
                raw,
                value,
                in_buf,
                span: s,
            } => InterpPart::Text {
                raw: &t.src.bytes()[raw.start as usize..raw.end as usize],
                value: Cow::Borrowed(t.str_value(value, in_buf)),
                span: span(t, s),
            },
            tree::InterpData::Expr(e) => InterpPart::Expr(Expr::new(t, e)),
        }
    }
}

handle!(
    /// The arguments of a call.
    Args
);
debug_fields!(Args { kind });

impl<'t> Args<'t> {
    fn data(self) -> tree::ArgsData {
        self.t.args[self.i as usize]
    }

    pub fn kind(self) -> ArgsKind<'t> {
        let t = self.t;
        match self.data() {
            tree::ArgsData::Parens { args, span: s } => ArgsKind::Parens {
                args: List::new(t, args, Expr::new),
                span: span(t, s),
            },
            tree::ArgsData::Table(e) => ArgsKind::Table(Expr::new(t, e)),
            tree::ArgsData::String(e) => ArgsKind::String(Expr::new(t, e)),
        }
    }

    /// The argument expressions, in order.
    pub fn exprs(self) -> Exprs<'t> {
        let r = match self.data() {
            tree::ArgsData::Parens { args, .. } => args,
            tree::ArgsData::Table(e) | tree::ArgsData::String(e) => Range { start: e, len: 1 },
        };
        List::new(self.t, r, Expr::new)
    }

    /// How many arguments there are.
    pub fn len(self) -> usize {
        match self.data() {
            tree::ArgsData::Parens { args, .. } => args.len as usize,
            tree::ArgsData::Table(_) | tree::ArgsData::String(_) => 1,
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ArgsKind<'t> {
    /// `f(a, b)`; the span covers the parentheses.
    Parens { args: Exprs<'t>, span: Span },
    /// `f{…}`
    Table(Expr<'t>),
    /// `f"…"`
    String(Expr<'t>),
}

/// A field of a table constructor.
#[derive(Clone, Copy, Debug)]
pub enum TableField<'t> {
    /// `[k] = v`
    Keyed {
        key: Expr<'t>,
        value: Expr<'t>,
        span: Span,
    },
    /// `name = v`
    Named { name: Name<'t>, value: Expr<'t> },
    /// `v`
    Positional(Expr<'t>),
}

impl<'t> TableField<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.field(i) {
            tree::FieldData::Keyed {
                key,
                value,
                span: s,
            } => TableField::Keyed {
                key: Expr::new(t, key),
                value: Expr::new(t, value),
                span: span(t, t.spans[s as usize]),
            },
            tree::FieldData::Named { name, value } => TableField::Named {
                name: Name::new(t, name),
                value: Expr::new(t, value),
            },
            tree::FieldData::Positional(v) => TableField::Positional(Expr::new(t, v)),
        }
    }

    /// From the key (or name) to the end of the value.
    pub fn span(self) -> Span {
        match self {
            TableField::Keyed { span, .. } => span,
            TableField::Named { name, value } => Span {
                file: name.span().file,
                start: name.span().start,
                end: value.span().end,
            },
            TableField::Positional(value) => value.span(),
        }
    }
}

// ------------------------------------------------------------------ types --

handle!(Type);
debug_fields!(Type { span, kind });

impl<'t> Type<'t> {
    fn node(self) -> &'t tree::TypeNode {
        &self.t.types[self.i as usize]
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }

    pub fn kind(self) -> TypeKind<'t> {
        use tree::TypeData as K;
        let t = self.t;
        let ty = |i| Type::new(t, i);
        match self.node().data {
            K::Nil => TypeKind::Nil,
            K::True => TypeKind::True,
            K::False => TypeKind::False,
            K::AnyFunction => TypeKind::AnyFunction,
            K::String(s) => TypeKind::String(Str::new(t, s)),
            K::Number(s) => TypeKind::Number(Number { t, sp: s }),
            K::Ref { path, args } => TypeKind::Ref {
                path: List::new(t, path, Name::listed),
                args: List::new(t, args, TypeArg::at),
            },
            K::Typeof(e) => TypeKind::Typeof(Expr::new(t, e)),
            K::Table(fields) => TypeKind::Table(List::new(t, fields, TableTypeField::at)),
            K::Function(f) => TypeKind::Function(FunctionType::new(t, f)),
            K::Paren(i) => TypeKind::Paren(ty(i)),
            K::Optional(i) => TypeKind::Optional(ty(i)),
            K::Array(i) => TypeKind::Array(ty(i)),
            K::KeyOf(i) => TypeKind::KeyOf(ty(i)),
            K::Index { object, key } => TypeKind::Index {
                object: ty(object),
                key: ty(key),
            },
            K::Union(ts) => TypeKind::Union(List::new(t, ts, Type::new)),
            K::Intersection(ts) => TypeKind::Intersection(List::new(t, ts, Type::new)),
            K::Error => TypeKind::Error,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TypeKind<'t> {
    Nil,
    True,
    False,
    /// The bare `function` type.
    AnyFunction,
    String(Str<'t>),
    Number(Number<'t>),
    /// `a.b.C<T, U>`
    Ref {
        path: Names<'t>,
        args: TypeArgs<'t>,
    },
    Typeof(Expr<'t>),
    Table(TableTypeFields<'t>),
    Function(FunctionType<'t>),
    Paren(Type<'t>),
    Optional(Type<'t>),
    Array(Type<'t>),
    /// `keyof T`: the names of `T`'s fields (decision L45).
    KeyOf(Type<'t>),
    /// `T[K]`: the type of `T`'s field `K` (decision L45).
    Index {
        object: Type<'t>,
        key: Type<'t>,
    },
    Union(Types<'t>),
    Intersection(Types<'t>),
    Error,
}

#[derive(Clone, Copy, Debug)]
pub enum TypeArg<'t> {
    Type(Type<'t>),
    Pack(TypePack<'t>),
}

impl<'t> TypeArg<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.type_args[i as usize] {
            tree::TypeArgData::Type(ty) => TypeArg::Type(Type::new(t, ty)),
            tree::TypeArgData::Pack(p) => TypeArg::Pack(TypePack::at(t, p)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TypePack<'t> {
    /// `T...`
    Named(Name<'t>),
    /// `(A, B, ...C)`
    List {
        types: Types<'t>,
        vararg: Option<VarargType<'t>>,
        span: Span,
    },
}

impl<'t> TypePack<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.type_packs[i as usize] {
            tree::TypePackData::Named(n) => TypePack::Named(Name::new(t, n)),
            tree::TypePackData::List {
                types,
                vararg,
                span: s,
            } => TypePack::List {
                types: List::new(t, types, Type::new),
                vararg: opt(vararg, |i| VarargType::at(t, i)),
                span: span(t, s),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TableTypeField<'t> {
    Named {
        name: Name<'t>,
        optional: bool,
        readonly: bool,
        ty: Type<'t>,
    },
    /// `["end"]: T` names a field (`["end"]?: T`, an optional one;
    /// `readonly ["end"]: T`, a read-only one).
    NamedString {
        key: Str<'t>,
        optional: bool,
        readonly: bool,
        ty: Type<'t>,
        span: Span,
    },
    /// `[K]: V`
    Indexer { key: Type<'t>, value: Type<'t> },
    /// `{ T }` (array)
    Array(Type<'t>),
}

impl<'t> TableTypeField<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.table_type_fields[i as usize] {
            tree::TableTypeFieldData::Named {
                name,
                optional,
                readonly,
                ty,
            } => TableTypeField::Named {
                name: Name::new(t, name),
                optional,
                readonly,
                ty: Type::new(t, ty),
            },
            tree::TableTypeFieldData::NamedString {
                key,
                optional,
                readonly,
                ty,
                span: s,
            } => TableTypeField::NamedString {
                key: Str::new(t, key),
                optional,
                readonly,
                ty: Type::new(t, ty),
                span: span(t, s),
            },
            tree::TableTypeFieldData::Indexer { key, value } => TableTypeField::Indexer {
                key: Type::new(t, key),
                value: Type::new(t, value),
            },
            tree::TableTypeFieldData::Array(ty) => TableTypeField::Array(Type::new(t, ty)),
        }
    }
}

handle!(
    /// `function(A, B): R`
    FunctionType
);
debug_fields!(FunctionType {
    type_params,
    params,
    vararg,
    ret
});

impl<'t> FunctionType<'t> {
    fn node(self) -> &'t tree::FnTypeNode {
        &self.t.fn_types[self.i as usize]
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(self.t, self.node().type_params, TypeParam::at)
    }

    pub fn params(self) -> ParamTypes<'t> {
        List::new(self.t, self.node().params, ParamType::at)
    }

    pub fn vararg(self) -> Option<VarargType<'t>> {
        opt(self.node().vararg, |i| VarargType::at(self.t, i))
    }

    pub fn ret(self) -> RetType<'t> {
        RetType::at(self.t, self.node().ret)
    }
}

/// A parameter of a function type.
#[derive(Clone, Copy, Debug)]
pub struct ParamType<'t> {
    pub name: Option<Name<'t>>,
    pub ty: Type<'t>,
}

impl<'t> ParamType<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        let p = t.param_types[i as usize];
        ParamType {
            name: p.name.map(|n| Name::new(t, n)),
            ty: Type::new(t, p.ty),
        }
    }
}

/// `...: T`, `...: T...`, or a pack expansion as the last element of a list.
#[derive(Clone, Copy, Debug)]
pub enum VarargType<'t> {
    Type(Type<'t>),
    Pack(Name<'t>),
    Expand(Type<'t>),
}

impl<'t> VarargType<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.vararg_types[i as usize] {
            tree::VarargTypeData::Type(ty) => VarargType::Type(Type::new(t, ty)),
            tree::VarargTypeData::Pack(n) => VarargType::Pack(Name::new(t, n)),
            tree::VarargTypeData::Expand(ty) => VarargType::Expand(Type::new(t, ty)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RetType<'t> {
    Single(Type<'t>),
    /// `()`, `(A, B)`, `(A, ...B)`
    List {
        types: Types<'t>,
        vararg: Option<VarargType<'t>>,
        span: Span,
    },
    /// `T...`
    Pack(Name<'t>),
    /// `x is T`
    Predicate {
        name: Name<'t>,
        ty: Type<'t>,
    },
}

impl<'t> RetType<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.rets[i as usize] {
            tree::RetData::Single(ty) => RetType::Single(Type::new(t, ty)),
            tree::RetData::List {
                types,
                vararg,
                span: s,
            } => RetType::List {
                types: List::new(t, types, Type::new),
                vararg: opt(vararg, |i| VarargType::at(t, i)),
                span: span(t, s),
            },
            tree::RetData::Pack(n) => RetType::Pack(Name::new(t, n)),
            tree::RetData::Predicate { name, ty } => RetType::Predicate {
                name: Name::new(t, name),
                ty: Type::new(t, ty),
            },
        }
    }
}

/// `T: Constraint`, or a pack `T...`.
#[derive(Clone, Copy, Debug)]
pub struct TypeParam<'t> {
    pub name: Name<'t>,
    pub constraint: Option<Type<'t>>,
    pub pack: bool,
}

impl<'t> TypeParam<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        let p = t.type_params[i as usize];
        TypeParam {
            name: Name::new(t, p.name),
            constraint: opt(p.constraint, |i| Type::new(t, i)),
            pack: p.pack,
        }
    }
}

// ----------------------------------------------------------- declarations --

handle!(ClassDecl);
debug_fields!(ClassDecl {
    is_abstract,
    name,
    type_params,
    extends,
    implements,
    members,
    end_span
});

impl<'t> ClassDecl<'t> {
    fn node(self) -> &'t tree::ClassNode {
        &self.t.classes[self.i as usize]
    }

    pub fn is_abstract(self) -> bool {
        self.node().is_abstract
    }

    pub fn name(self) -> Name<'t> {
        Name::new(self.t, self.node().name)
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(self.t, self.node().type_params, TypeParam::at)
    }

    pub fn extends(self) -> Option<Type<'t>> {
        opt(self.node().extends, |i| Type::new(self.t, i))
    }

    pub fn implements(self) -> Types<'t> {
        List::new(self.t, self.node().implements, Type::new)
    }

    pub fn members(self) -> ClassMembers<'t> {
        List::new(self.t, self.node().members, ClassMember::new)
    }

    pub fn end_span(self) -> Span {
        span(self.t, self.node().end_span)
    }
}

handle!(ClassMember);
debug_fields!(ClassMember {
    decorators,
    modifiers,
    span,
    kind
});

impl<'t> ClassMember<'t> {
    fn node(self) -> &'t tree::ClassMemberNode {
        &self.t.class_members[self.i as usize]
    }

    pub fn decorators(self) -> Decorators<'t> {
        List::new(self.t, self.node().decorators, Decorator::new)
    }

    pub fn modifiers(self) -> &'t [Modifier] {
        let r = self.node().modifiers;
        &self.t.modifiers[r.start as usize..(r.start + r.len) as usize]
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }

    pub fn kind(self) -> ClassMemberKind<'t> {
        use tree::ClassMemberData as K;
        let t = self.t;
        match self.node().data {
            K::Field {
                name,
                optional,
                ty,
                value,
            } => ClassMemberKind::Field {
                name: Name::new(t, name),
                optional,
                ty: Type::new(t, ty),
                value: opt(value, |i| Expr::new(t, i)),
            },
            K::Method {
                name,
                func,
                has_body,
            } => ClassMemberKind::Method {
                name: Name::new(t, name),
                func: Func::new(t, func),
                has_body,
            },
            K::Getter { name, ty, body } => ClassMemberKind::Getter {
                name: Name::new(t, name),
                ty: opt(ty, |i| Type::new(t, i)),
                body: Block::new(t, body),
            },
            K::Setter { name, param, body } => ClassMemberKind::Setter {
                name: Name::new(t, name),
                param: TypedName::at(t, param),
                body: Block::new(t, body),
            },
            K::Constructor { func, has_body } => ClassMemberKind::Constructor {
                func: Func::new(t, func),
                has_body,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ClassMemberKind<'t> {
    Field {
        name: Name<'t>,
        optional: bool,
        ty: Type<'t>,
        value: Option<Expr<'t>>,
    },
    /// A method; for abstract methods and overload signatures the body is
    /// empty and `has_body` false.
    Method {
        name: Name<'t>,
        func: Func<'t>,
        has_body: bool,
    },
    Getter {
        name: Name<'t>,
        ty: Option<Type<'t>>,
        body: Block<'t>,
    },
    Setter {
        name: Name<'t>,
        param: TypedName<'t>,
        body: Block<'t>,
    },
    Constructor {
        func: Func<'t>,
        has_body: bool,
    },
}

handle!(InterfaceDecl);
debug_fields!(InterfaceDecl {
    name,
    type_params,
    extends,
    members
});

impl<'t> InterfaceDecl<'t> {
    fn node(self) -> &'t tree::InterfaceNode {
        &self.t.interfaces[self.i as usize]
    }

    pub fn name(self) -> Name<'t> {
        Name::new(self.t, self.node().name)
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(self.t, self.node().type_params, TypeParam::at)
    }

    pub fn extends(self) -> Types<'t> {
        List::new(self.t, self.node().extends, Type::new)
    }

    pub fn members(self) -> InterfaceMembers<'t> {
        List::new(self.t, self.node().members, InterfaceMember::at)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum InterfaceMember<'t> {
    Field {
        readonly: bool,
        name: Name<'t>,
        optional: bool,
        ty: Type<'t>,
    },
    Method {
        name: Name<'t>,
        sig: Func<'t>,
    },
}

impl<'t> InterfaceMember<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        match t.interface_members[i as usize] {
            tree::InterfaceMemberData::Field {
                readonly,
                name,
                optional,
                ty,
            } => InterfaceMember::Field {
                readonly,
                name: Name::new(t, name),
                optional,
                ty: Type::new(t, ty),
            },
            tree::InterfaceMemberData::Method { name, sig } => InterfaceMember::Method {
                name: Name::new(t, name),
                sig: Func::new(t, sig),
            },
        }
    }
}

handle!(EnumDecl);
debug_fields!(EnumDecl { name, items });

impl<'t> EnumDecl<'t> {
    fn node(self) -> &'t tree::EnumNode {
        &self.t.enums[self.i as usize]
    }

    pub fn name(self) -> Name<'t> {
        Name::new(self.t, self.node().name)
    }

    pub fn items(self) -> EnumItems<'t> {
        List::new(self.t, self.node().items, EnumItem::at)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EnumItem<'t> {
    pub name: Name<'t>,
    pub value: Option<Expr<'t>>,
}

impl<'t> EnumItem<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        let item = t.enum_items[i as usize];
        EnumItem {
            name: Name::new(t, item.name),
            value: opt(item.value, |i| Expr::new(t, i)),
        }
    }
}

handle!(TypeAlias);
debug_fields!(TypeAlias {
    name,
    type_params,
    ty
});

impl<'t> TypeAlias<'t> {
    fn node(self) -> &'t tree::AliasNode {
        &self.t.aliases[self.i as usize]
    }

    pub fn name(self) -> Name<'t> {
        Name::new(self.t, self.node().name)
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(self.t, self.node().type_params, TypeParam::at)
    }

    pub fn ty(self) -> Type<'t> {
        Type::new(self.t, self.node().ty)
    }
}

handle!(
    /// A `declare …` statement.
    Declare
);
debug_fields!(Declare { kind });

impl<'t> Declare<'t> {
    pub fn kind(self) -> DeclareKind<'t> {
        use tree::DeclareData as K;
        let t = self.t;
        match t.declares[self.i as usize] {
            K::Function { name, sig } => DeclareKind::Function {
                name: FuncName::new(t, name),
                sig: Func::new(t, sig),
            },
            K::Global { name, ty } => DeclareKind::Global {
                name: Name::new(t, name),
                ty: Type::new(t, ty),
            },
            K::Namespace { name, members } => DeclareKind::Namespace {
                name: Name::new(t, name),
                members: List::new(t, members, NsMember::new),
            },
            K::Class {
                name,
                type_params,
                extends,
                implements,
                members,
            } => DeclareKind::Class {
                name: Name::new(t, name),
                type_params: List::new(t, type_params, TypeParam::at),
                extends: opt(extends, |i| Type::new(t, i)),
                implements: List::new(t, implements, Type::new),
                members: List::new(t, members, DeclMember::new),
            },
            K::Interface(i) => DeclareKind::Interface(InterfaceDecl::new(t, i)),
            K::Event { name, params } => DeclareKind::Event {
                name: Name::new(t, name),
                params: Func::new(t, params),
            },
            K::Message { name, params } => DeclareKind::Message {
                name: Name::new(t, name),
                params: Func::new(t, params),
            },
            K::Module { path, members } => DeclareKind::Module {
                path: Str::new(t, path),
                members: List::new(t, members, NsMember::new),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum DeclareKind<'t> {
    Function {
        name: FuncName<'t>,
        sig: Func<'t>,
    },
    Global {
        name: Name<'t>,
        ty: Type<'t>,
    },
    Namespace {
        name: Name<'t>,
        members: NsMembers<'t>,
    },
    Class {
        name: Name<'t>,
        type_params: TypeParams<'t>,
        extends: Option<Type<'t>>,
        implements: Types<'t>,
        members: DeclMembers<'t>,
    },
    Interface(InterfaceDecl<'t>),
    Event {
        name: Name<'t>,
        params: Func<'t>,
    },
    Message {
        name: Name<'t>,
        params: Func<'t>,
    },
    Module {
        path: Str<'t>,
        members: NsMembers<'t>,
    },
}

handle!(NsMember);
debug_fields!(NsMember {
    decorators,
    span,
    kind
});

impl<'t> NsMember<'t> {
    fn node(self) -> &'t tree::NsMemberNode {
        &self.t.ns_members[self.i as usize]
    }

    pub fn decorators(self) -> Decorators<'t> {
        List::new(self.t, self.node().decorators, Decorator::new)
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }

    pub fn kind(self) -> NsMemberKind<'t> {
        use tree::NsMemberData as K;
        let t = self.t;
        match self.node().data {
            K::Function { name, sig } => NsMemberKind::Function {
                name: Name::new(t, name),
                sig: Func::new(t, sig),
            },
            K::Value { is_const, name, ty } => NsMemberKind::Value {
                is_const,
                name: Name::new(t, name),
                ty: Type::new(t, ty),
            },
            K::Namespace { name, members } => NsMemberKind::Namespace {
                name: Name::new(t, name),
                members: List::new(t, members, NsMember::new),
            },
            K::Class {
                name,
                type_params,
                extends,
                members,
            } => NsMemberKind::Class {
                name: Name::new(t, name),
                type_params: List::new(t, type_params, TypeParam::at),
                extends: opt(extends, |i| Type::new(t, i)),
                members: List::new(t, members, DeclMember::new),
            },
            K::Interface(i) => NsMemberKind::Interface(InterfaceDecl::new(t, i)),
            K::TypeAlias(a) => NsMemberKind::TypeAlias(TypeAlias::new(t, a)),
            K::Enum(e) => NsMemberKind::Enum(EnumDecl::new(t, e)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum NsMemberKind<'t> {
    Function {
        name: Name<'t>,
        sig: Func<'t>,
    },
    Value {
        is_const: bool,
        name: Name<'t>,
        ty: Type<'t>,
    },
    Namespace {
        name: Name<'t>,
        members: NsMembers<'t>,
    },
    Class {
        name: Name<'t>,
        type_params: TypeParams<'t>,
        extends: Option<Type<'t>>,
        members: DeclMembers<'t>,
    },
    Interface(InterfaceDecl<'t>),
    TypeAlias(TypeAlias<'t>),
    Enum(EnumDecl<'t>),
}

handle!(DeclMember);
debug_fields!(DeclMember {
    decorators,
    is_static,
    span,
    kind
});

impl<'t> DeclMember<'t> {
    fn node(self) -> &'t tree::DeclMemberNode {
        &self.t.decl_members[self.i as usize]
    }

    pub fn decorators(self) -> Decorators<'t> {
        List::new(self.t, self.node().decorators, Decorator::new)
    }

    pub fn is_static(self) -> bool {
        self.node().is_static
    }

    pub fn span(self) -> Span {
        span(self.t, self.node().span)
    }

    pub fn kind(self) -> DeclMemberKind<'t> {
        let t = self.t;
        match self.node().data {
            tree::DeclMemberData::Function { name, sig } => DeclMemberKind::Function {
                name: Name::new(t, name),
                sig: Func::new(t, sig),
            },
            tree::DeclMemberData::Field {
                readonly,
                name,
                optional,
                ty,
            } => DeclMemberKind::Field {
                readonly,
                name: Name::new(t, name),
                optional,
                ty: Type::new(t, ty),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum DeclMemberKind<'t> {
    Function {
        name: Name<'t>,
        sig: Func<'t>,
    },
    Field {
        readonly: bool,
        name: Name<'t>,
        optional: bool,
        ty: Type<'t>,
    },
}

handle!(Import);
debug_fields!(Import {
    clause,
    type_only,
    internal,
    from
});

impl<'t> Import<'t> {
    fn node(self) -> &'t tree::ImportNode {
        &self.t.imports[self.i as usize]
    }

    pub fn clause(self) -> ImportClause<'t> {
        let t = self.t;
        match self.node().clause {
            tree::ImportClauseData::Namespace(n) => ImportClause::Namespace(Name::new(t, n)),
            tree::ImportClauseData::SideEffect => ImportClause::SideEffect,
            tree::ImportClauseData::Names(specs) => {
                ImportClause::Names(List::new(t, specs, ImportSpec::at))
            }
        }
    }

    pub fn type_only(self) -> bool {
        self.node().type_only
    }

    /// `import internal a from "…"`: a module's file-level locals (test
    /// files only, spec Tooling §16.1.3.1).
    pub fn internal(self) -> bool {
        self.node().internal
    }

    pub fn from(self) -> Str<'t> {
        Str::new(self.t, self.node().from)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ImportClause<'t> {
    /// `* as name`
    Namespace(Name<'t>),
    /// `_`
    SideEffect,
    Names(ImportSpecs<'t>),
}

#[derive(Clone, Copy, Debug)]
pub struct ImportSpec<'t> {
    pub name: Name<'t>,
    pub alias: Option<Name<'t>>,
}

impl<'t> ImportSpec<'t> {
    fn at(t: &'t Tree, i: u32) -> Self {
        let s = t.import_specs[i as usize];
        ImportSpec {
            name: Name::new(t, s.name),
            alias: s.alias.map(|a| Name::new(t, a)),
        }
    }
}
