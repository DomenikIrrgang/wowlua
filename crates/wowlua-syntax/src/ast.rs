//! The syntax tree (spec compiler §13.4). Every node has a [`NodeId`] and a
//! [`Span`]. The tree keeps everything needed to reproduce the source's
//! meaning exactly: parentheses (they truncate multiple values in Lua),
//! the form of call arguments, and literal text.

use wowlua_span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Name {
    pub text: Box<str>,
    pub span: Span,
}

/// A directive comment before the first token (`--!strict`, spec §1.1.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Directive {
    pub name: Box<str>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub directives: Vec<Directive>,
    pub block: Block,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub id: NodeId,
    pub span: Span,
    pub decorators: Vec<Decorator>,
    pub kind: StmtKind,
}

// Statements are built once per parse and walked in place; boxing the larger
// variants would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// `a, b.c = x, y`
    Assign {
        targets: Vec<Expr>,
        values: Vec<Expr>,
    },
    /// `a += e` (single target, spec §1.3.4)
    Compound {
        target: Expr,
        op: CompoundOp,
        value: Expr,
    },
    /// A function call used as a statement.
    Call(Expr),
    Do(Block),
    While {
        cond: Expr,
        body: Block,
    },
    Repeat {
        body: Block,
        cond: Expr,
    },
    If {
        cond: Expr,
        then_block: Block,
        elseifs: Vec<(Expr, Block)>,
        else_block: Option<Block>,
    },
    NumericFor {
        var: TypedName,
        start: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Block,
    },
    GenericFor {
        vars: Vec<TypedName>,
        exprs: Vec<Expr>,
        body: Block,
    },
    /// `function a.b:c() … end`
    Function {
        name: FuncName,
        func: Box<FuncBody>,
    },
    LocalFunction {
        name: Name,
        func: Box<FuncBody>,
    },
    /// `local a: T, b = x, y`
    Local {
        names: Vec<TypedName>,
        values: Vec<Expr>,
    },
    /// `local { x, y = 1 }: T = e` (spec §1.3.3)
    LocalDestructure {
        fields: Vec<DestructField>,
        ty: Option<Type>,
        value: Option<Expr>,
    },
    /// `const NAME: T = e`
    Const {
        name: Name,
        ty: Option<Type>,
        value: Option<Expr>,
    },
    Return(Vec<Expr>),
    Break,
    Continue,
    Class(Box<ClassDecl>),
    Interface(Box<InterfaceDecl>),
    Enum(Box<EnumDecl>),
    TypeAlias(Box<TypeAlias>),
    Declare(Box<Declare>),
    Import(Box<Import>),
    /// `export <declaration>`
    Export(Box<Stmt>),
    /// A statement that failed to parse.
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompoundOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    Concat,
    BAnd,
    BOr,
    Shl,
    Shr,
    Coalesce,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedName {
    pub name: Name,
    pub ty: Option<Type>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DestructField {
    pub name: Name,
    pub default: Option<Expr>,
}

/// `a.b.c:m`
#[derive(Clone, Debug, PartialEq)]
pub struct FuncName {
    pub path: Vec<Name>,
    pub method: Option<Name>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FuncBody {
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub vararg: Option<Vararg>,
    pub ret: Option<RetType>,
    pub body: Block,
    /// The parameter list, `(` to `)`.
    pub params_span: Span,
    /// The closing `end`.
    pub end_span: Span,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// Only in declarations (`@deferred`, spec appendix A.3).
    pub decorators: Vec<Decorator>,
    pub name: Name,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Vararg {
    pub span: Span,
    pub ty: Option<VarargType>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decorator {
    pub path: Vec<Name>,
    pub args: Option<Args>,
    pub span: Span,
}

/// Core decorators (spec §4.3): always in scope, they win over lexical
/// bindings of the same name.
pub const CORE_DECORATORS: &[&str] = &[
    "if",
    "global",
    "deprecated",
    "inline",
    "debugOnly",
    "nocheck",
    "script",
    "widget",
    "intrinsic",
    "deferred",
    "onDemand",
    "profile",
    "noprofile",
    "trace",
    "reflect",
    "checked",
    "unchecked",
];

impl Decorator {
    /// A core decorator (a single name from [`CORE_DECORATORS`]).
    pub fn is_core(&self) -> bool {
        self.path.len() == 1 && CORE_DECORATORS.contains(&self.path[0].text.as_ref())
    }

    /// A core decorator name (`@if`, `@global`, …) used without a path.
    pub fn is_named(&self, name: &str) -> bool {
        self.path.len() == 1 && self.path[0].text.as_ref() == name
    }

    /// The decorator as an expression: its path, called with its arguments
    /// (`@a.b(x)` is `a.b(x)`, spec §4.5).
    pub fn expr(&self) -> Expr {
        let mk = |span: Span, kind: ExprKind| Expr {
            id: NodeId(0),
            span,
            kind,
        };
        let mut e = mk(self.path[0].span, ExprKind::Name(self.path[0].clone()));
        for n in &self.path[1..] {
            let span = Span {
                file: n.span.file,
                start: self.path[0].span.start,
                end: n.span.end,
            };
            e = mk(
                span,
                ExprKind::Field {
                    obj: Box::new(e),
                    name: n.clone(),
                },
            );
        }
        match &self.args {
            Some(args) => mk(
                self.span,
                ExprKind::Call {
                    callee: Box::new(e),
                    args: args.clone(),
                },
            ),
            None => e,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub id: NodeId,
    pub span: Span,
    pub kind: ExprKind,
}

// Every file's tree stays alive through the build, and expressions are its
// most common node. Method arguments and cast types are boxed so that the
// other expressions stay small.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Nil,
    True,
    False,
    Number(NumberLit),
    String(StringLit),
    Vararg,
    Function(Box<FuncBody>),
    Table(Vec<TableField>),
    Interp(Vec<InterpPart>),
    Name(Name),
    Index {
        obj: Box<Expr>,
        key: Box<Expr>,
    },
    Field {
        obj: Box<Expr>,
        name: Name,
    },
    Call {
        callee: Box<Expr>,
        args: Args,
    },
    Method {
        obj: Box<Expr>,
        name: Name,
        args: Box<Args>,
    },
    OptField {
        obj: Box<Expr>,
        name: Name,
    },
    OptIndex {
        obj: Box<Expr>,
        key: Box<Expr>,
    },
    OptMethod {
        obj: Box<Expr>,
        name: Name,
        args: Box<Args>,
    },
    OptCall {
        callee: Box<Expr>,
        args: Args,
    },
    Paren(Box<Expr>),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    /// `e :: T`
    Cast {
        expr: Box<Expr>,
        ty: Box<Type>,
    },
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumberLit {
    /// The numeral as written.
    pub raw: NumText,
    /// `None` if malformed (already reported).
    pub value: Option<f64>,
}

/// The text of a numeral. Short ones, nearly all, are kept inline: a data
/// addon has millions of numerals, and a heap block for each one's few bytes
/// adds up to a large part of its syntax tree. Reads as a `&str`.
#[derive(Clone)]
pub enum NumText {
    Inline {
        len: u8,
        bytes: [u8; NumText::INLINE],
    },
    Heap(Box<str>),
}

impl NumText {
    /// The longest text kept inline: `NumberLit` stays small enough that
    /// `ExprKind` does not grow.
    const INLINE: usize = 22;

    pub fn as_str(&self) -> &str {
        match self {
            NumText::Inline { len, bytes } => {
                std::str::from_utf8(&bytes[..*len as usize]).expect("inline text is a whole str")
            }
            NumText::Heap(text) => text,
        }
    }
}

impl From<&str> for NumText {
    fn from(text: &str) -> Self {
        if text.len() <= NumText::INLINE {
            let mut bytes = [0; NumText::INLINE];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            NumText::Inline {
                len: text.len() as u8,
                bytes,
            }
        } else {
            NumText::Heap(text.into())
        }
    }
}

impl std::ops::Deref for NumText {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

// Compared and shown as the text alone, the way the `Box<str>` it replaces
// was.
impl PartialEq for NumText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl std::fmt::Debug for NumText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl std::fmt::Display for NumText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.as_str(), f)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StringLit {
    /// The literal as written, including quotes or long brackets.
    pub raw: Box<[u8]>,
    pub value: Box<[u8]>,
    pub long: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InterpPart {
    /// Raw text between braces (escapes not yet decoded) and its value.
    Text {
        raw: Box<[u8]>,
        value: Box<[u8]>,
        span: Span,
    },
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Args {
    /// `f(a, b)`; the span covers the parentheses.
    Parens { args: Vec<Expr>, span: Span },
    /// `f{…}`
    Table(Box<Expr>),
    /// `f"…"`
    String(Box<Expr>),
}

impl Args {
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Args::Parens { args, .. } => args.iter().collect(),
            Args::Table(e) | Args::String(e) => vec![e],
        }
    }
}

// The key is boxed for the same reason: most fields are named or positional.
#[derive(Clone, Debug, PartialEq)]
pub enum TableField {
    /// `[k] = v`
    Keyed {
        key: Box<Expr>,
        value: Expr,
        span: Span,
    },
    /// `name = v`; its span runs from the name to the end of the value
    /// ([`TableField::span`]).
    Named { name: Name, value: Expr },
    /// `v`
    Positional(Expr),
}

impl TableField {
    pub fn span(&self) -> Span {
        match self {
            TableField::Keyed { span, .. } => *span,
            TableField::Named { name, value } => Span {
                file: name.span.file,
                start: name.span.start,
                end: value.span.end,
            },
            TableField::Positional(value) => value.span,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
    BAnd,
    BOr,
    BXor,
    Shl,
    Shr,
}

impl BinOp {
    /// Left and right binding power (spec §1.4.6; Lua 5.3's table, with
    /// `??` at the level of `or`).
    pub fn priority(self) -> (u8, u8) {
        use BinOp::*;
        match self {
            Or | Coalesce => (1, 1),
            And => (2, 2),
            Eq | Ne | Lt | Le | Gt | Ge => (3, 3),
            BOr => (4, 4),
            BXor => (5, 5),
            BAnd => (6, 6),
            Shl | Shr => (7, 7),
            Concat => (9, 8),
            Add | Sub => (10, 10),
            Mul | Div | IDiv | Mod => (11, 11),
            Pow => (14, 13),
        }
    }

    pub fn as_str(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            IDiv => "//",
            Mod => "%",
            Pow => "^",
            Concat => "..",
            Eq => "==",
            Ne => "~=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            And => "and",
            Or => "or",
            Coalesce => "??",
            BAnd => "&",
            BOr => "|",
            BXor => "~",
            Shl => "<<",
            Shr => ">>",
        }
    }

    /// Exists in Lua 5.1.
    pub fn is_lua(self) -> bool {
        use BinOp::*;
        !matches!(self, IDiv | Coalesce | BAnd | BOr | BXor | Shl | Shr)
    }
}

/// Binding power of unary operators (spec §1.4.6).
pub const UNARY_PRIORITY: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Len,
    BNot,
}

impl UnOp {
    pub fn as_str(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "not",
            UnOp::Len => "#",
            UnOp::BNot => "~",
        }
    }
}

// ---------------------------------------------------------------- types --

#[derive(Clone, Debug, PartialEq)]
pub struct Type {
    pub id: NodeId,
    pub span: Span,
    pub kind: TypeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    Nil,
    True,
    False,
    /// The bare `function` type.
    AnyFunction,
    String(StringLit),
    Number(NumberLit),
    /// `a.b.C<T, U>`
    Ref {
        path: Vec<Name>,
        args: Vec<TypeArg>,
    },
    Typeof(Box<Expr>),
    Table(Vec<TableTypeField>),
    Function(Box<FunctionType>),
    Paren(Box<Type>),
    Optional(Box<Type>),
    Array(Box<Type>),
    Union(Vec<Type>),
    Intersection(Vec<Type>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeArg {
    Type(Type),
    Pack(TypePack),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypePack {
    /// `T...`
    Named(Name),
    /// `(A, B, ...C)`
    List {
        types: Vec<Type>,
        vararg: Option<Box<VarargType>>,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TableTypeField {
    Named {
        name: Name,
        optional: bool,
        readonly: bool,
        ty: Type,
    },
    /// `["end"]: T` names a field.
    NamedString {
        key: StringLit,
        ty: Type,
        span: Span,
    },
    /// `[K]: V`
    Indexer { key: Type, value: Type },
    /// `{ T }` (array)
    Array(Type),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionType {
    pub type_params: Vec<TypeParam>,
    pub params: Vec<ParamType>,
    pub vararg: Option<VarargType>,
    pub ret: RetType,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamType {
    pub name: Option<Name>,
    pub ty: Type,
}

/// `...: T`, `...: T...`, or a pack expansion `T...` / `Args<E>...` as the
/// last element of a parameter or result list.
#[derive(Clone, Debug, PartialEq)]
pub enum VarargType {
    Type(Type),
    Pack(Name),
    Expand(Type),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RetType {
    Single(Type),
    /// `()`, `(A, B)`, `(A, ...B)`
    List {
        types: Vec<Type>,
        vararg: Option<Box<VarargType>>,
        span: Span,
    },
    /// `T...`
    Pack(Name),
    /// `x is T`
    Predicate {
        name: Name,
        ty: Type,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeParam {
    pub name: Name,
    pub constraint: Option<Type>,
    pub pack: bool,
}

// --------------------------------------------------------- declarations --

#[derive(Clone, Debug, PartialEq)]
pub struct ClassDecl {
    pub is_abstract: bool,
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub extends: Option<Type>,
    pub implements: Vec<Type>,
    pub members: Vec<ClassMember>,
    pub end_span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Public,
    Private,
    Protected,
    Static,
    Readonly,
    Override,
    Abstract,
    Overload,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassMember {
    pub decorators: Vec<Decorator>,
    pub modifiers: Vec<Modifier>,
    pub span: Span,
    pub kind: ClassMemberKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClassMemberKind {
    Field {
        name: Name,
        optional: bool,
        ty: Type,
        value: Option<Expr>,
    },
    /// A method; `func.body` is empty and `has_body` false for abstract
    /// methods and overload signatures.
    Method {
        name: Name,
        func: Box<FuncBody>,
        has_body: bool,
    },
    Getter {
        name: Name,
        ty: Option<Type>,
        body: Block,
    },
    Setter {
        name: Name,
        param: TypedName,
        body: Block,
    },
    Constructor {
        func: Box<FuncBody>,
        has_body: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceDecl {
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub extends: Vec<Type>,
    pub members: Vec<InterfaceMember>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InterfaceMember {
    Field {
        readonly: bool,
        name: Name,
        optional: bool,
        ty: Type,
    },
    Method {
        name: Name,
        sig: Box<FuncBody>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumDecl {
    pub name: Name,
    pub items: Vec<EnumItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumItem {
    pub name: Name,
    pub value: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeAlias {
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Declare {
    Function {
        name: FuncName,
        sig: Box<FuncBody>,
    },
    Global {
        name: Name,
        ty: Type,
    },
    Namespace {
        name: Name,
        members: Vec<NsMember>,
    },
    Class {
        name: Name,
        type_params: Vec<TypeParam>,
        extends: Option<Type>,
        implements: Vec<Type>,
        members: Vec<DeclMember>,
    },
    Interface(InterfaceDecl),
    Event {
        name: Name,
        params: Box<FuncBody>,
    },
    Message {
        name: Name,
        params: Box<FuncBody>,
    },
    Module {
        path: StringLit,
        members: Vec<NsMember>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct NsMember {
    pub decorators: Vec<Decorator>,
    pub span: Span,
    pub kind: NsMemberKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NsMemberKind {
    Function {
        name: Name,
        sig: Box<FuncBody>,
    },
    Value {
        is_const: bool,
        name: Name,
        ty: Type,
    },
    Namespace {
        name: Name,
        members: Vec<NsMember>,
    },
    Class {
        name: Name,
        type_params: Vec<TypeParam>,
        extends: Option<Type>,
        members: Vec<DeclMember>,
    },
    Interface(InterfaceDecl),
    TypeAlias(TypeAlias),
    Enum(EnumDecl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeclMember {
    pub decorators: Vec<Decorator>,
    pub is_static: bool,
    pub span: Span,
    pub kind: DeclMemberKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeclMemberKind {
    Function {
        name: Name,
        sig: Box<FuncBody>,
    },
    Field {
        readonly: bool,
        name: Name,
        optional: bool,
        ty: Type,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub clause: ImportClause,
    pub type_only: bool,
    pub from: StringLit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImportClause {
    /// `* as name`
    Namespace(Name),
    /// `_`
    SideEffect,
    Names(Vec<ImportSpec>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportSpec {
    pub name: Name,
    pub alias: Option<Name>,
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;

    // A large addon holds millions of expressions and table fields, so a
    // variant that grows them raises the build's peak memory.
    #[test]
    fn expressions_and_table_fields_stay_small() {
        assert!(std::mem::size_of::<Expr>() <= 72);
        assert!(std::mem::size_of::<TableField>() <= 104);
        // Numerals keep their text inline without growing `ExprKind`.
        assert!(std::mem::size_of::<NumberLit>() <= 40);
        assert!(std::mem::size_of::<ExprKind>() <= 56);
    }

    // `NumText` reads, compares and prints like the `Box<str>` it replaced,
    // whether the text is inline or not.
    #[test]
    fn numeral_text_reads_like_a_string() {
        for text in [
            "",
            "1",
            "0x1F",
            "1_000.25",
            "1234567890123456789012",
            "12345678901234567890123",
        ] {
            let num = NumText::from(text);
            assert_eq!(
                num.len() <= NumText::INLINE,
                matches!(num, NumText::Inline { .. })
            );
            assert_eq!(&*num, text);
            assert_eq!(num.to_string(), text);
            assert_eq!(format!("{num:?}"), format!("{:?}", Box::<str>::from(text)));
            assert_eq!(num, NumText::Heap(text.into()));
        }
    }
}
