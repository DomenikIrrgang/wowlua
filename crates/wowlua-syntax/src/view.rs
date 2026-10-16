//! A typed, read-only view of a syntax tree (#19).
//!
//! Every consumer outside the parser reads the tree through these handles:
//! `Copy` values (`Expr<'t>`, `Stmt<'t>`, `Name<'t>`, …) whose `kind()`
//! returns an enum shaped like the tree's, with handles for children and
//! [`List`]s for sequences. The handles say nothing about how the tree is
//! stored, so its storage can change underneath them (a flat, index-based
//! layout) without touching a consumer. Today they wrap the boxed nodes of
//! [`crate::ast`].
//!
//! Operator and modifier enums ([`BinOp`], [`UnOp`], [`CompoundOp`],
//! [`Modifier`]) and [`Directive`]s are plain data and shared with `ast`.

use crate::ast;
use std::borrow::Cow;
use wowlua_span::Span;

pub use crate::ast::{BinOp, CompoundOp, Directive, Modifier, UnOp};

// ------------------------------------------------------------------ lists --

/// A sequence of child handles: `len`, `get`, `first`, `last` and `iter`.
pub struct List<'t, A, V> {
    items: &'t [A],
    view: fn(&'t A) -> V,
}

impl<A, V> Clone for List<'_, A, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A, V> Copy for List<'_, A, V> {}

impl<'t, A, V> List<'t, A, V> {
    fn new(items: &'t [A], view: fn(&'t A) -> V) -> Self {
        List { items, view }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<V> {
        self.items.get(index).map(self.view)
    }

    /// Item `index`; panics past the end, like indexing a slice.
    pub fn at(&self, index: usize) -> V {
        (self.view)(&self.items[index])
    }

    pub fn first(&self) -> Option<V> {
        self.items.first().map(self.view)
    }

    pub fn last(&self) -> Option<V> {
        self.items.last().map(self.view)
    }

    /// The items, when there are exactly `N` (for matching like a slice
    /// pattern: `if let Some([a, b]) = list.exactly()`).
    pub fn exactly<const N: usize>(&self) -> Option<[V; N]> {
        (self.items.len() == N).then(|| std::array::from_fn(|i| self.at(i)))
    }

    pub fn iter(&self) -> std::iter::Map<std::slice::Iter<'t, A>, fn(&'t A) -> V> {
        self.items.iter().map(self.view)
    }
}

impl<'t, A, V> IntoIterator for List<'t, A, V> {
    type Item = V;
    type IntoIter = std::iter::Map<std::slice::Iter<'t, A>, fn(&'t A) -> V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'t, A, V> IntoIterator for &List<'t, A, V> {
    type Item = V;
    type IntoIter = std::iter::Map<std::slice::Iter<'t, A>, fn(&'t A) -> V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<A, V: std::fmt::Debug> std::fmt::Debug for List<'_, A, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<A, V> Default for List<'_, A, V> {
    /// No items.
    fn default() -> Self {
        List::new(&[], |_| unreachable!("an empty list has no items"))
    }
}

pub type Stmts<'t> = List<'t, ast::Stmt, Stmt<'t>>;
pub type Exprs<'t> = List<'t, ast::Expr, Expr<'t>>;
pub type Names<'t> = List<'t, ast::Name, Name<'t>>;
pub type TypedNames<'t> = List<'t, ast::TypedName, TypedName<'t>>;
pub type DestructFields<'t> = List<'t, ast::DestructField, DestructField<'t>>;
pub type ElseIfs<'t> = List<'t, (ast::Expr, ast::Block), (Expr<'t>, Block<'t>)>;
pub type Decorators<'t> = List<'t, ast::Decorator, Decorator<'t>>;
pub type Params<'t> = List<'t, ast::Param, Param<'t>>;
pub type TableFields<'t> = List<'t, ast::TableField, TableField<'t>>;
pub type InterpParts<'t> = List<'t, ast::InterpPart, InterpPart<'t>>;
pub type Types<'t> = List<'t, ast::Type, Type<'t>>;
pub type TypeArgs<'t> = List<'t, ast::TypeArg, TypeArg<'t>>;
pub type TypeParams<'t> = List<'t, ast::TypeParam, TypeParam<'t>>;
pub type TableTypeFields<'t> = List<'t, ast::TableTypeField, TableTypeField<'t>>;
pub type ParamTypes<'t> = List<'t, ast::ParamType, ParamType<'t>>;
pub type ClassMembers<'t> = List<'t, ast::ClassMember, ClassMember<'t>>;
pub type InterfaceMembers<'t> = List<'t, ast::InterfaceMember, InterfaceMember<'t>>;
pub type EnumItems<'t> = List<'t, ast::EnumItem, EnumItem<'t>>;
pub type NsMembers<'t> = List<'t, ast::NsMember, NsMember<'t>>;
pub type DeclMembers<'t> = List<'t, ast::DeclMember, DeclMember<'t>>;
pub type ImportSpecs<'t> = List<'t, ast::ImportSpec, ImportSpec<'t>>;

macro_rules! handle {
    ($(#[$doc:meta])* $name:ident, $ast:ty) => {
        $(#[$doc])*
        #[derive(Clone, Copy)]
        pub struct $name<'t>(&'t $ast);

        impl<'t> $name<'t> {
            fn new(node: &'t $ast) -> Self {
                $name(node)
            }

            /// The handle of a boxed node (for the parser; removed with
            /// the boxed tree, #19).
            #[allow(dead_code)]
            pub(crate) fn of(node: &'t $ast) -> Self {
                $name(node)
            }

            /// The boxed node (for the parser; removed with the boxed
            /// tree, #19).
            #[allow(dead_code)]
            pub(crate) fn ast(self) -> &'t $ast {
                self.0
            }
        }

        impl std::fmt::Debug for $name<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Debug::fmt(self.0, f)
            }
        }

        impl PartialEq for $name<'_> {
            /// The same node (not equal content).
            fn eq(&self, other: &Self) -> bool {
                std::ptr::eq(self.0, other.0)
            }
        }

        impl Eq for $name<'_> {}

        impl std::hash::Hash for $name<'_> {
            /// By identity, like `==`.
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                std::ptr::hash(self.0, state)
            }
        }
    };
}

// ---------------------------------------------------------- the file --

handle!(
    /// A file's tree.
    Chunk,
    ast::Chunk
);

impl<'t> Chunk<'t> {
    pub fn directives(self) -> &'t [Directive] {
        &self.0.directives
    }

    pub fn block(self) -> Block<'t> {
        Block::new(&self.0.block)
    }

    pub fn span(self) -> Span {
        self.0.span
    }
}

impl crate::Parse {
    /// The tree, as handles.
    pub fn view(&self) -> Chunk<'_> {
        Chunk::new(&self.chunk)
    }
}

handle!(Block, ast::Block);

impl<'t> Block<'t> {
    pub fn stmts(self) -> Stmts<'t> {
        List::new(&self.0.stmts, Stmt::new)
    }

    pub fn span(self) -> Span {
        self.0.span
    }
}

// ------------------------------------------------------------- statements --

handle!(Stmt, ast::Stmt);

impl<'t> Stmt<'t> {
    pub fn span(self) -> Span {
        self.0.span
    }

    pub fn decorators(self) -> Decorators<'t> {
        List::new(&self.0.decorators, Decorator::new)
    }

    pub fn kind(self) -> StmtKind<'t> {
        use ast::StmtKind as K;
        match &self.0.kind {
            K::Assign { targets, values } => StmtKind::Assign {
                targets: List::new(targets, Expr::new),
                values: List::new(values, Expr::new),
            },
            K::Compound { target, op, value } => StmtKind::Compound {
                target: Expr::new(target),
                op: *op,
                value: Expr::new(value),
            },
            K::Call(e) => StmtKind::Call(Expr::new(e)),
            K::Do(b) => StmtKind::Do(Block::new(b)),
            K::While { cond, body } => StmtKind::While {
                cond: Expr::new(cond),
                body: Block::new(body),
            },
            K::Repeat { body, cond } => StmtKind::Repeat {
                body: Block::new(body),
                cond: Expr::new(cond),
            },
            K::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => StmtKind::If {
                cond: Expr::new(cond),
                then_block: Block::new(then_block),
                elseifs: List::new(elseifs, |(c, b)| (Expr::new(c), Block::new(b))),
                else_block: else_block.as_deref().map(Block::new),
            },
            K::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => StmtKind::NumericFor {
                var: TypedName::new(var),
                start: Expr::new(start),
                limit: Expr::new(limit),
                step: step.as_deref().map(Expr::new),
                body: Block::new(body),
            },
            K::GenericFor { vars, exprs, body } => StmtKind::GenericFor {
                vars: List::new(vars, TypedName::new),
                exprs: List::new(exprs, Expr::new),
                body: Block::new(body),
            },
            K::Function { name, func } => StmtKind::Function {
                name: FuncName::new(name),
                func: Func::new(func),
            },
            K::LocalFunction { name, func } => StmtKind::LocalFunction {
                name: Name::new(name),
                func: Func::new(func),
            },
            K::Local { names, values } => StmtKind::Local {
                names: List::new(names, TypedName::new),
                values: List::new(values, Expr::new),
            },
            K::LocalDestructure { fields, ty, value } => StmtKind::LocalDestructure {
                fields: List::new(fields, DestructField::new),
                ty: ty.as_deref().map(Type::new),
                value: value.as_deref().map(Expr::new),
            },
            K::Const { name, ty, value } => StmtKind::Const {
                name: Name::new(name),
                ty: ty.as_deref().map(Type::new),
                value: value.as_deref().map(Expr::new),
            },
            K::Return(values) => StmtKind::Return(List::new(values, Expr::new)),
            K::Break => StmtKind::Break,
            K::Continue => StmtKind::Continue,
            K::Class(c) => StmtKind::Class(ClassDecl::new(c)),
            K::Interface(i) => StmtKind::Interface(InterfaceDecl::new(i)),
            K::Enum(e) => StmtKind::Enum(EnumDecl::new(e)),
            K::TypeAlias(t) => StmtKind::TypeAlias(TypeAlias::new(t)),
            K::Declare(d) => StmtKind::Declare(Declare::new(d)),
            K::Import(i) => StmtKind::Import(Import::new(i)),
            K::Export(s) => StmtKind::Export(Stmt::new(s)),
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
    fn new(t: &'t ast::TypedName) -> Self {
        TypedName {
            name: Name::new(&t.name),
            ty: t.ty.as_ref().map(Type::new),
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
    fn new(f: &'t ast::DestructField) -> Self {
        DestructField {
            name: Name::new(&f.name),
            default: f.default.as_ref().map(Expr::new),
        }
    }
}

handle!(
    /// `a.b.c:m`
    FuncName,
    ast::FuncName
);

impl<'t> FuncName<'t> {
    pub fn path(self) -> Names<'t> {
        List::new(&self.0.path, Name::new)
    }

    pub fn method(self) -> Option<Name<'t>> {
        self.0.method.as_ref().map(Name::new)
    }

    pub fn span(self) -> Span {
        self.0.span
    }
}

handle!(
    /// A function: parameters, results and body (also a declaration's
    /// signature, whose body is empty).
    Func,
    ast::FuncBody
);

impl<'t> Func<'t> {
    pub fn type_params(self) -> TypeParams<'t> {
        List::new(&self.0.type_params, TypeParam::new)
    }

    pub fn params(self) -> Params<'t> {
        List::new(&self.0.params, Param::new)
    }

    pub fn vararg(self) -> Option<Vararg<'t>> {
        self.0.vararg.as_ref().map(Vararg::new)
    }

    pub fn ret(self) -> Option<RetType<'t>> {
        self.0.ret.as_ref().map(RetType::new)
    }

    pub fn body(self) -> Block<'t> {
        Block::new(&self.0.body)
    }

    /// The parameter list, `(` to `)`.
    pub fn params_span(self) -> Span {
        self.0.params_span
    }

    /// The closing `end`.
    pub fn end_span(self) -> Span {
        self.0.end_span
    }

    pub fn span(self) -> Span {
        self.0.span
    }
}

handle!(Param, ast::Param);

impl<'t> Param<'t> {
    pub fn decorators(self) -> Decorators<'t> {
        List::new(&self.0.decorators, Decorator::new)
    }

    pub fn name(self) -> Name<'t> {
        Name::new(&self.0.name)
    }

    pub fn ty(self) -> Option<Type<'t>> {
        self.0.ty.as_ref().map(Type::new)
    }

    pub fn default(self) -> Option<Expr<'t>> {
        self.0.default.as_deref().map(Expr::new)
    }
}

/// `...`, with its type.
#[derive(Clone, Copy, Debug)]
pub struct Vararg<'t> {
    pub span: Span,
    pub ty: Option<VarargType<'t>>,
}

impl<'t> Vararg<'t> {
    fn new(v: &'t ast::Vararg) -> Self {
        Vararg {
            span: v.span,
            ty: v.ty.as_ref().map(VarargType::new),
        }
    }
}

handle!(
    /// `@a.b(x)`
    Decorator,
    ast::Decorator
);

impl<'t> Decorator<'t> {
    pub fn path(self) -> Names<'t> {
        List::new(&self.0.path, Name::new)
    }

    pub fn args(self) -> Option<Args<'t>> {
        self.0.args.as_ref().map(Args::new)
    }

    pub fn span(self) -> Span {
        self.0.span
    }

    /// The decorator as an expression: its path, called with its arguments
    /// (`@a.b(x)` is `a.b(x)`, spec §4.5).
    pub fn expr(self) -> Expr<'t> {
        Expr::new(&self.0.call)
    }

    /// A core decorator (a single name from [`ast::CORE_DECORATORS`]).
    pub fn is_core(self) -> bool {
        self.0.is_core()
    }

    /// The single name `name`.
    pub fn is_named(self, name: &str) -> bool {
        self.0.is_named(name)
    }
}

// ------------------------------------------------------------ expressions --

handle!(Expr, ast::Expr);

impl<'t> Expr<'t> {
    pub fn span(self) -> Span {
        self.0.span
    }

    /// The node's number in its file, unique per parse; 0 for nodes the
    /// compiler made (a decorator's call).
    pub fn id(self) -> u32 {
        self.0.id.0
    }

    pub fn kind(self) -> ExprKind<'t> {
        use ast::ExprKind as K;
        match &self.0.kind {
            K::Nil => ExprKind::Nil,
            K::True => ExprKind::True,
            K::False => ExprKind::False,
            K::Number(n) => ExprKind::Number(Number::new(n)),
            K::String(s) => ExprKind::String(Str::new(s)),
            K::Vararg => ExprKind::Vararg,
            K::Function(f) => ExprKind::Function(Func::new(f)),
            K::Table(fields) => ExprKind::Table(List::new(fields, TableField::new)),
            K::Interp(parts) => ExprKind::Interp(List::new(parts, InterpPart::new)),
            K::Name(n) => ExprKind::Name(Name::new(n)),
            K::Index { obj, key } => ExprKind::Index {
                obj: Expr::new(obj),
                key: Expr::new(key),
            },
            K::Field { obj, name } => ExprKind::Field {
                obj: Expr::new(obj),
                name: Name::new(name),
            },
            K::Call { callee, args } => ExprKind::Call {
                callee: Expr::new(callee),
                args: Args::new(args),
            },
            K::Method { obj, name, args } => ExprKind::Method {
                obj: Expr::new(obj),
                name: Name::new(name),
                args: Args::new(args),
            },
            K::OptField { obj, name } => ExprKind::OptField {
                obj: Expr::new(obj),
                name: Name::new(name),
            },
            K::OptIndex { obj, key } => ExprKind::OptIndex {
                obj: Expr::new(obj),
                key: Expr::new(key),
            },
            K::OptMethod { obj, name, args } => ExprKind::OptMethod {
                obj: Expr::new(obj),
                name: Name::new(name),
                args: Args::new(args),
            },
            K::OptCall { callee, args } => ExprKind::OptCall {
                callee: Expr::new(callee),
                args: Args::new(args),
            },
            K::Paren(e) => ExprKind::Paren(Expr::new(e)),
            K::Binary { op, lhs, rhs } => ExprKind::Binary {
                op: *op,
                lhs: Expr::new(lhs),
                rhs: Expr::new(rhs),
            },
            K::Unary { op, operand } => ExprKind::Unary {
                op: *op,
                operand: Expr::new(operand),
            },
            K::Cast { expr, ty } => ExprKind::Cast {
                expr: Expr::new(expr),
                ty: Type::new(ty),
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

handle!(Name, ast::Name);

impl<'t> Name<'t> {
    pub fn text(self) -> &'t str {
        &self.0.text
    }

    pub fn span(self) -> Span {
        self.0.span
    }
}

handle!(
    /// A numeral.
    Number,
    ast::NumberLit
);

impl<'t> Number<'t> {
    /// The numeral as written.
    pub fn raw(self) -> &'t str {
        &self.0.raw
    }

    /// `None` if malformed (already reported).
    pub fn value(self) -> Option<f64> {
        self.0.value
    }
}

handle!(
    /// A string literal.
    Str,
    ast::StringLit
);

impl<'t> Str<'t> {
    /// The literal as written, quotes or long brackets included.
    pub fn raw(self) -> &'t [u8] {
        &self.0.raw
    }

    /// The decoded bytes.
    pub fn value(self) -> Cow<'t, [u8]> {
        Cow::Borrowed(&self.0.value)
    }

    /// A long string (`[[…]]`).
    pub fn long(self) -> bool {
        self.0.long
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
    fn new(p: &'t ast::InterpPart) -> Self {
        match p {
            ast::InterpPart::Text { raw, value, span } => InterpPart::Text {
                raw,
                value: Cow::Borrowed(value),
                span: *span,
            },
            ast::InterpPart::Expr(e) => InterpPart::Expr(Expr::new(e)),
        }
    }
}

handle!(
    /// The arguments of a call.
    Args,
    ast::Args
);

impl<'t> Args<'t> {
    pub fn kind(self) -> ArgsKind<'t> {
        match self.0 {
            ast::Args::Parens { args, span } => ArgsKind::Parens {
                args: List::new(args, Expr::new),
                span: *span,
            },
            ast::Args::Table(e) => ArgsKind::Table(Expr::new(e)),
            ast::Args::String(e) => ArgsKind::String(Expr::new(e)),
        }
    }

    /// The argument expressions, in order.
    pub fn exprs(self) -> Exprs<'t> {
        let items: &'t [ast::Expr] = match self.0 {
            ast::Args::Parens { args, .. } => args,
            ast::Args::Table(e) | ast::Args::String(e) => std::slice::from_ref(&**e),
        };
        List::new(items, Expr::new)
    }

    /// How many arguments there are.
    pub fn len(self) -> usize {
        match self.0 {
            ast::Args::Parens { args, .. } => args.len(),
            ast::Args::Table(_) | ast::Args::String(_) => 1,
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
    fn new(f: &'t ast::TableField) -> Self {
        match f {
            ast::TableField::Keyed { key, value, span } => TableField::Keyed {
                key: Expr::new(key),
                value: Expr::new(value),
                span: *span,
            },
            ast::TableField::Named { name, value } => TableField::Named {
                name: Name::new(name),
                value: Expr::new(value),
            },
            ast::TableField::Positional(v) => TableField::Positional(Expr::new(v)),
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

handle!(Type, ast::Type);

impl<'t> Type<'t> {
    pub fn span(self) -> Span {
        self.0.span
    }

    pub fn kind(self) -> TypeKind<'t> {
        use ast::TypeKind as K;
        match &self.0.kind {
            K::Nil => TypeKind::Nil,
            K::True => TypeKind::True,
            K::False => TypeKind::False,
            K::AnyFunction => TypeKind::AnyFunction,
            K::String(s) => TypeKind::String(Str::new(s)),
            K::Number(n) => TypeKind::Number(Number::new(n)),
            K::Ref { path, args } => TypeKind::Ref {
                path: List::new(path, Name::new),
                args: List::new(args, TypeArg::new),
            },
            K::Typeof(e) => TypeKind::Typeof(Expr::new(e)),
            K::Table(fields) => TypeKind::Table(List::new(fields, TableTypeField::new)),
            K::Function(f) => TypeKind::Function(FunctionType::new(f)),
            K::Paren(t) => TypeKind::Paren(Type::new(t)),
            K::Optional(t) => TypeKind::Optional(Type::new(t)),
            K::Array(t) => TypeKind::Array(Type::new(t)),
            K::Union(ts) => TypeKind::Union(List::new(ts, Type::new)),
            K::Intersection(ts) => TypeKind::Intersection(List::new(ts, Type::new)),
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
    fn new(a: &'t ast::TypeArg) -> Self {
        match a {
            ast::TypeArg::Type(t) => TypeArg::Type(Type::new(t)),
            ast::TypeArg::Pack(p) => TypeArg::Pack(TypePack::new(p)),
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
    fn new(p: &'t ast::TypePack) -> Self {
        match p {
            ast::TypePack::Named(n) => TypePack::Named(Name::new(n)),
            ast::TypePack::List {
                types,
                vararg,
                span,
            } => TypePack::List {
                types: List::new(types, Type::new),
                vararg: vararg.as_deref().map(VarargType::new),
                span: *span,
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
    /// `["end"]: T` names a field.
    NamedString {
        key: Str<'t>,
        ty: Type<'t>,
        span: Span,
    },
    /// `[K]: V`
    Indexer { key: Type<'t>, value: Type<'t> },
    /// `{ T }` (array)
    Array(Type<'t>),
}

impl<'t> TableTypeField<'t> {
    fn new(f: &'t ast::TableTypeField) -> Self {
        match f {
            ast::TableTypeField::Named {
                name,
                optional,
                readonly,
                ty,
            } => TableTypeField::Named {
                name: Name::new(name),
                optional: *optional,
                readonly: *readonly,
                ty: Type::new(ty),
            },
            ast::TableTypeField::NamedString { key, ty, span } => TableTypeField::NamedString {
                key: Str::new(key),
                ty: Type::new(ty),
                span: *span,
            },
            ast::TableTypeField::Indexer { key, value } => TableTypeField::Indexer {
                key: Type::new(key),
                value: Type::new(value),
            },
            ast::TableTypeField::Array(t) => TableTypeField::Array(Type::new(t)),
        }
    }
}

handle!(
    /// `function(A, B): R`
    FunctionType,
    ast::FunctionType
);

impl<'t> FunctionType<'t> {
    pub fn type_params(self) -> TypeParams<'t> {
        List::new(&self.0.type_params, TypeParam::new)
    }

    pub fn params(self) -> ParamTypes<'t> {
        List::new(&self.0.params, ParamType::new)
    }

    pub fn vararg(self) -> Option<VarargType<'t>> {
        self.0.vararg.as_ref().map(VarargType::new)
    }

    pub fn ret(self) -> RetType<'t> {
        RetType::new(&self.0.ret)
    }
}

/// A parameter of a function type.
#[derive(Clone, Copy, Debug)]
pub struct ParamType<'t> {
    pub name: Option<Name<'t>>,
    pub ty: Type<'t>,
}

impl<'t> ParamType<'t> {
    fn new(p: &'t ast::ParamType) -> Self {
        ParamType {
            name: p.name.as_ref().map(Name::new),
            ty: Type::new(&p.ty),
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
    fn new(v: &'t ast::VarargType) -> Self {
        match v {
            ast::VarargType::Type(t) => VarargType::Type(Type::new(t)),
            ast::VarargType::Pack(n) => VarargType::Pack(Name::new(n)),
            ast::VarargType::Expand(t) => VarargType::Expand(Type::new(t)),
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
    fn new(r: &'t ast::RetType) -> Self {
        match r {
            ast::RetType::Single(t) => RetType::Single(Type::new(t)),
            ast::RetType::List {
                types,
                vararg,
                span,
            } => RetType::List {
                types: List::new(types, Type::new),
                vararg: vararg.as_deref().map(VarargType::new),
                span: *span,
            },
            ast::RetType::Pack(n) => RetType::Pack(Name::new(n)),
            ast::RetType::Predicate { name, ty } => RetType::Predicate {
                name: Name::new(name),
                ty: Type::new(ty),
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
    fn new(p: &'t ast::TypeParam) -> Self {
        TypeParam {
            name: Name::new(&p.name),
            constraint: p.constraint.as_ref().map(Type::new),
            pack: p.pack,
        }
    }
}

// ----------------------------------------------------------- declarations --

handle!(ClassDecl, ast::ClassDecl);

impl<'t> ClassDecl<'t> {
    pub fn is_abstract(self) -> bool {
        self.0.is_abstract
    }

    pub fn name(self) -> Name<'t> {
        Name::new(&self.0.name)
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(&self.0.type_params, TypeParam::new)
    }

    pub fn extends(self) -> Option<Type<'t>> {
        self.0.extends.as_ref().map(Type::new)
    }

    pub fn implements(self) -> Types<'t> {
        List::new(&self.0.implements, Type::new)
    }

    pub fn members(self) -> ClassMembers<'t> {
        List::new(&self.0.members, ClassMember::new)
    }

    pub fn end_span(self) -> Span {
        self.0.end_span
    }
}

handle!(ClassMember, ast::ClassMember);

impl<'t> ClassMember<'t> {
    pub fn decorators(self) -> Decorators<'t> {
        List::new(&self.0.decorators, Decorator::new)
    }

    pub fn modifiers(self) -> &'t [Modifier] {
        &self.0.modifiers
    }

    pub fn span(self) -> Span {
        self.0.span
    }

    pub fn kind(self) -> ClassMemberKind<'t> {
        use ast::ClassMemberKind as K;
        match &self.0.kind {
            K::Field {
                name,
                optional,
                ty,
                value,
            } => ClassMemberKind::Field {
                name: Name::new(name),
                optional: *optional,
                ty: Type::new(ty),
                value: value.as_ref().map(Expr::new),
            },
            K::Method {
                name,
                func,
                has_body,
            } => ClassMemberKind::Method {
                name: Name::new(name),
                func: Func::new(func),
                has_body: *has_body,
            },
            K::Getter { name, ty, body } => ClassMemberKind::Getter {
                name: Name::new(name),
                ty: ty.as_ref().map(Type::new),
                body: Block::new(body),
            },
            K::Setter { name, param, body } => ClassMemberKind::Setter {
                name: Name::new(name),
                param: TypedName::new(param),
                body: Block::new(body),
            },
            K::Constructor { func, has_body } => ClassMemberKind::Constructor {
                func: Func::new(func),
                has_body: *has_body,
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

handle!(InterfaceDecl, ast::InterfaceDecl);

impl<'t> InterfaceDecl<'t> {
    pub fn name(self) -> Name<'t> {
        Name::new(&self.0.name)
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(&self.0.type_params, TypeParam::new)
    }

    pub fn extends(self) -> Types<'t> {
        List::new(&self.0.extends, Type::new)
    }

    pub fn members(self) -> InterfaceMembers<'t> {
        List::new(&self.0.members, InterfaceMember::new)
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
    fn new(m: &'t ast::InterfaceMember) -> Self {
        match m {
            ast::InterfaceMember::Field {
                readonly,
                name,
                optional,
                ty,
            } => InterfaceMember::Field {
                readonly: *readonly,
                name: Name::new(name),
                optional: *optional,
                ty: Type::new(ty),
            },
            ast::InterfaceMember::Method { name, sig } => InterfaceMember::Method {
                name: Name::new(name),
                sig: Func::new(sig),
            },
        }
    }
}

handle!(EnumDecl, ast::EnumDecl);

impl<'t> EnumDecl<'t> {
    pub fn name(self) -> Name<'t> {
        Name::new(&self.0.name)
    }

    pub fn items(self) -> EnumItems<'t> {
        List::new(&self.0.items, EnumItem::new)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EnumItem<'t> {
    pub name: Name<'t>,
    pub value: Option<Expr<'t>>,
}

impl<'t> EnumItem<'t> {
    fn new(i: &'t ast::EnumItem) -> Self {
        EnumItem {
            name: Name::new(&i.name),
            value: i.value.as_ref().map(Expr::new),
        }
    }
}

handle!(TypeAlias, ast::TypeAlias);

impl<'t> TypeAlias<'t> {
    pub fn name(self) -> Name<'t> {
        Name::new(&self.0.name)
    }

    pub fn type_params(self) -> TypeParams<'t> {
        List::new(&self.0.type_params, TypeParam::new)
    }

    pub fn ty(self) -> Type<'t> {
        Type::new(&self.0.ty)
    }
}

handle!(
    /// A `declare …` statement.
    Declare,
    ast::Declare
);

impl<'t> Declare<'t> {
    pub fn kind(self) -> DeclareKind<'t> {
        use ast::Declare as K;
        match self.0 {
            K::Function { name, sig } => DeclareKind::Function {
                name: FuncName::new(name),
                sig: Func::new(sig),
            },
            K::Global { name, ty } => DeclareKind::Global {
                name: Name::new(name),
                ty: Type::new(ty),
            },
            K::Namespace { name, members } => DeclareKind::Namespace {
                name: Name::new(name),
                members: List::new(members, NsMember::new),
            },
            K::Class {
                name,
                type_params,
                extends,
                implements,
                members,
            } => DeclareKind::Class {
                name: Name::new(name),
                type_params: List::new(type_params, TypeParam::new),
                extends: extends.as_ref().map(Type::new),
                implements: List::new(implements, Type::new),
                members: List::new(members, DeclMember::new),
            },
            K::Interface(i) => DeclareKind::Interface(InterfaceDecl::new(i)),
            K::Event { name, params } => DeclareKind::Event {
                name: Name::new(name),
                params: Func::new(params),
            },
            K::Message { name, params } => DeclareKind::Message {
                name: Name::new(name),
                params: Func::new(params),
            },
            K::Module { path, members } => DeclareKind::Module {
                path: Str::new(path),
                members: List::new(members, NsMember::new),
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

handle!(NsMember, ast::NsMember);

impl<'t> NsMember<'t> {
    pub fn decorators(self) -> Decorators<'t> {
        List::new(&self.0.decorators, Decorator::new)
    }

    pub fn span(self) -> Span {
        self.0.span
    }

    pub fn kind(self) -> NsMemberKind<'t> {
        use ast::NsMemberKind as K;
        match &self.0.kind {
            K::Function { name, sig } => NsMemberKind::Function {
                name: Name::new(name),
                sig: Func::new(sig),
            },
            K::Value { is_const, name, ty } => NsMemberKind::Value {
                is_const: *is_const,
                name: Name::new(name),
                ty: Type::new(ty),
            },
            K::Namespace { name, members } => NsMemberKind::Namespace {
                name: Name::new(name),
                members: List::new(members, NsMember::new),
            },
            K::Class {
                name,
                type_params,
                extends,
                members,
            } => NsMemberKind::Class {
                name: Name::new(name),
                type_params: List::new(type_params, TypeParam::new),
                extends: extends.as_ref().map(Type::new),
                members: List::new(members, DeclMember::new),
            },
            K::Interface(i) => NsMemberKind::Interface(InterfaceDecl::new(i)),
            K::TypeAlias(t) => NsMemberKind::TypeAlias(TypeAlias::new(t)),
            K::Enum(e) => NsMemberKind::Enum(EnumDecl::new(e)),
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

handle!(DeclMember, ast::DeclMember);

impl<'t> DeclMember<'t> {
    pub fn decorators(self) -> Decorators<'t> {
        List::new(&self.0.decorators, Decorator::new)
    }

    pub fn is_static(self) -> bool {
        self.0.is_static
    }

    pub fn span(self) -> Span {
        self.0.span
    }

    pub fn kind(self) -> DeclMemberKind<'t> {
        match &self.0.kind {
            ast::DeclMemberKind::Function { name, sig } => DeclMemberKind::Function {
                name: Name::new(name),
                sig: Func::new(sig),
            },
            ast::DeclMemberKind::Field {
                readonly,
                name,
                optional,
                ty,
            } => DeclMemberKind::Field {
                readonly: *readonly,
                name: Name::new(name),
                optional: *optional,
                ty: Type::new(ty),
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

handle!(Import, ast::Import);

impl<'t> Import<'t> {
    pub fn clause(self) -> ImportClause<'t> {
        match &self.0.clause {
            ast::ImportClause::Namespace(n) => ImportClause::Namespace(Name::new(n)),
            ast::ImportClause::SideEffect => ImportClause::SideEffect,
            ast::ImportClause::Names(specs) => {
                ImportClause::Names(List::new(specs, ImportSpec::new))
            }
        }
    }

    pub fn type_only(self) -> bool {
        self.0.type_only
    }

    pub fn from(self) -> Str<'t> {
        Str::new(&self.0.from)
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
    fn new(s: &'t ast::ImportSpec) -> Self {
        ImportSpec {
            name: Name::new(&s.name),
            alias: s.alias.as_ref().map(Name::new),
        }
    }
}
