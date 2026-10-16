//! The flat syntax tree (#19): a file's nodes in a few arrays, children
//! referred to by `u32` index, lists as contiguous ranges of an array.
//! Names and numerals are ranges of the source, which the tree keeps;
//! strings keep a decoded value only where it differs from their source
//! bytes. Read it through the [views](crate::view).

use crate::ast::{BinOp, CompoundOp, Directive, Modifier, UnOp};
use wowlua_span::FileId;

/// No node (an absent optional child).
pub(crate) const NONE: u32 = u32::MAX;

/// A byte range of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Sp {
    pub start: u32,
    pub end: u32,
}

/// Items `start..start + len` of an array.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Range {
    pub start: u32,
    pub len: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockNode {
    pub stmts: Range,
    pub span: Sp,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StmtNode {
    pub span: Sp,
    pub decorators: Range,
    pub data: StmtData,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum StmtData {
    Assign {
        targets: Range,
        values: Range,
    },
    Compound {
        target: u32,
        op: CompoundOp,
        value: u32,
    },
    Call(u32),
    Do(u32),
    While {
        cond: u32,
        body: u32,
    },
    Repeat {
        body: u32,
        cond: u32,
    },
    If {
        cond: u32,
        then_block: u32,
        elseifs: Range,
        else_block: u32,
    },
    NumericFor {
        var: u32,
        start: u32,
        limit: u32,
        step: u32,
        body: u32,
    },
    GenericFor {
        vars: Range,
        exprs: Range,
        body: u32,
    },
    Function {
        name: u32,
        func: u32,
    },
    LocalFunction {
        name: Sp,
        func: u32,
    },
    Local {
        names: Range,
        values: Range,
    },
    LocalDestructure {
        fields: Range,
        ty: u32,
        value: u32,
    },
    Const {
        name: Sp,
        ty: u32,
        value: u32,
    },
    Return(Range),
    Break,
    Continue,
    Class(u32),
    Interface(u32),
    Enum(u32),
    TypeAlias(u32),
    Declare(u32),
    Import(u32),
    Export(u32),
    Error,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TypedNameNode {
    pub name: Sp,
    pub ty: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DestructNode {
    pub name: Sp,
    pub default: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FuncNameNode {
    pub path: Range,
    pub method: Option<Sp>,
    pub span: Sp,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FuncNode {
    pub type_params: Range,
    pub params: Range,
    /// `...` and its type (`vararg_types`, or `NONE`).
    pub vararg: Option<(Sp, u32)>,
    pub ret: u32,
    pub body: u32,
    pub params_span: Sp,
    pub end_span: Sp,
    pub span: Sp,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParamNode {
    pub decorators: Range,
    pub name: Sp,
    pub ty: u32,
    pub default: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DecoratorNode {
    pub path: Range,
    pub args: u32,
    pub span: Sp,
    pub call: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExprNode {
    pub span: Sp,
    pub data: ExprData,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ExprData {
    Nil,
    True,
    False,
    /// The numeral's text.
    Number(Sp),
    String(u32),
    Vararg,
    Function(u32),
    Table(Range),
    Interp(Range),
    Name(Sp),
    Index {
        obj: u32,
        key: u32,
    },
    Field {
        obj: u32,
        name: Sp,
    },
    Call {
        callee: u32,
        args: u32,
    },
    /// `methods`: the name and the arguments.
    Method {
        obj: u32,
        method: u32,
    },
    OptField {
        obj: u32,
        name: Sp,
    },
    OptIndex {
        obj: u32,
        key: u32,
    },
    OptMethod {
        obj: u32,
        method: u32,
    },
    OptCall {
        callee: u32,
        args: u32,
    },
    Paren(u32),
    Binary {
        op: BinOp,
        lhs: u32,
        rhs: u32,
    },
    Unary {
        op: UnOp,
        operand: u32,
    },
    Cast {
        expr: u32,
        ty: u32,
    },
    Error,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MethodNode {
    pub name: Sp,
    pub args: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StrNode {
    pub raw: Sp,
    /// The decoded bytes: in the source (`in_buf` false) or in `strbuf`.
    pub value: Sp,
    pub in_buf: bool,
    pub long: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum InterpData {
    Text {
        raw: Sp,
        value: Sp,
        in_buf: bool,
        span: Sp,
    },
    Expr(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ArgsData {
    Parens { args: Range, span: Sp },
    Table(u32),
    String(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum FieldData {
    /// `spans`: the field's span.
    Keyed {
        key: u32,
        value: u32,
        span: u32,
    },
    Named {
        name: Sp,
        value: u32,
    },
    Positional(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TypeNode {
    pub span: Sp,
    pub data: TypeData,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TypeData {
    Nil,
    True,
    False,
    AnyFunction,
    String(u32),
    Number(Sp),
    Ref { path: Range, args: Range },
    Typeof(u32),
    Table(Range),
    Function(u32),
    Paren(u32),
    Optional(u32),
    Array(u32),
    Union(Range),
    Intersection(Range),
    Error,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TypeArgData {
    Type(u32),
    Pack(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TypePackData {
    Named(Sp),
    List { types: Range, vararg: u32, span: Sp },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TableTypeFieldData {
    Named {
        name: Sp,
        optional: bool,
        readonly: bool,
        ty: u32,
    },
    NamedString {
        key: u32,
        ty: u32,
        span: Sp,
    },
    Indexer {
        key: u32,
        value: u32,
    },
    Array(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FnTypeNode {
    pub type_params: Range,
    pub params: Range,
    pub vararg: u32,
    pub ret: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParamTypeNode {
    pub name: Option<Sp>,
    pub ty: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum VarargTypeData {
    Type(u32),
    Pack(Sp),
    Expand(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum RetData {
    Single(u32),
    List { types: Range, vararg: u32, span: Sp },
    Pack(Sp),
    Predicate { name: Sp, ty: u32 },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TypeParamNode {
    pub name: Sp,
    pub constraint: u32,
    pub pack: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClassNode {
    pub is_abstract: bool,
    pub name: Sp,
    pub type_params: Range,
    pub extends: u32,
    pub implements: Range,
    pub members: Range,
    pub end_span: Sp,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClassMemberNode {
    pub decorators: Range,
    pub modifiers: Range,
    pub span: Sp,
    pub data: ClassMemberData,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ClassMemberData {
    Field {
        name: Sp,
        optional: bool,
        ty: u32,
        value: u32,
    },
    Method {
        name: Sp,
        func: u32,
        has_body: bool,
    },
    Getter {
        name: Sp,
        ty: u32,
        body: u32,
    },
    Setter {
        name: Sp,
        param: u32,
        body: u32,
    },
    Constructor {
        func: u32,
        has_body: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InterfaceNode {
    pub name: Sp,
    pub type_params: Range,
    pub extends: Range,
    pub members: Range,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum InterfaceMemberData {
    Field {
        readonly: bool,
        name: Sp,
        optional: bool,
        ty: u32,
    },
    Method {
        name: Sp,
        sig: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EnumNode {
    pub name: Sp,
    pub items: Range,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EnumItemNode {
    pub name: Sp,
    pub value: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AliasNode {
    pub name: Sp,
    pub type_params: Range,
    pub ty: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DeclareData {
    Function {
        name: u32,
        sig: u32,
    },
    Global {
        name: Sp,
        ty: u32,
    },
    Namespace {
        name: Sp,
        members: Range,
    },
    Class {
        name: Sp,
        type_params: Range,
        extends: u32,
        implements: Range,
        members: Range,
    },
    Interface(u32),
    Event {
        name: Sp,
        params: u32,
    },
    Message {
        name: Sp,
        params: u32,
    },
    Module {
        path: u32,
        members: Range,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NsMemberNode {
    pub decorators: Range,
    pub span: Sp,
    pub data: NsMemberData,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum NsMemberData {
    Function {
        name: Sp,
        sig: u32,
    },
    Value {
        is_const: bool,
        name: Sp,
        ty: u32,
    },
    Namespace {
        name: Sp,
        members: Range,
    },
    Class {
        name: Sp,
        type_params: Range,
        extends: u32,
        members: Range,
    },
    Interface(u32),
    TypeAlias(u32),
    Enum(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DeclMemberNode {
    pub decorators: Range,
    pub is_static: bool,
    pub span: Sp,
    pub data: DeclMemberData,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DeclMemberData {
    Function {
        name: Sp,
        sig: u32,
    },
    Field {
        readonly: bool,
        name: Sp,
        optional: bool,
        ty: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ImportNode {
    pub clause: ImportClauseData,
    pub type_only: bool,
    pub from: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ImportClauseData {
    Namespace(Sp),
    SideEffect,
    Names(Range),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ImportSpecNode {
    pub name: Sp,
    pub alias: Option<Sp>,
}

/// The source text, kept for names, numerals and literals: as a `str` when
/// it is valid UTF-8 (nearly always), so their text needs no check.
#[derive(Debug)]
pub(crate) enum SourceText {
    Str(Box<str>),
    Bytes(Box<[u8]>),
}

impl SourceText {
    fn new(src: &[u8]) -> SourceText {
        match std::str::from_utf8(src) {
            Ok(s) => SourceText::Str(s.into()),
            Err(_) => SourceText::Bytes(src.into()),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            SourceText::Str(s) => s.as_bytes(),
            SourceText::Bytes(b) => b,
        }
    }

    /// The text of a name or numeral (ASCII by construction).
    pub fn str(&self, sp: Sp) -> &str {
        let range = sp.start as usize..sp.end as usize;
        match self {
            SourceText::Str(s) => &s[range],
            SourceText::Bytes(b) => std::str::from_utf8(&b[range]).expect("names are ASCII"),
        }
    }
}

/// One file's syntax tree.
#[derive(Debug)]
pub struct Tree {
    pub(crate) file: FileId,
    pub(crate) src: SourceText,
    pub(crate) span: Sp,
    pub(crate) directives: Vec<Directive>,
    pub(crate) root: u32,
    pub(crate) blocks: Vec<BlockNode>,
    pub(crate) stmts: Vec<StmtNode>,
    pub(crate) elseifs: Vec<(u32, u32)>,
    pub(crate) typed_names: Vec<TypedNameNode>,
    pub(crate) destructs: Vec<DestructNode>,
    pub(crate) func_names: Vec<FuncNameNode>,
    pub(crate) funcs: Vec<FuncNode>,
    pub(crate) params: Vec<ParamNode>,
    pub(crate) decorators: Vec<DecoratorNode>,
    pub(crate) names: Vec<Sp>,
    pub(crate) exprs: Vec<ExprNode>,
    pub(crate) methods: Vec<MethodNode>,
    pub(crate) strs: Vec<StrNode>,
    pub(crate) strbuf: Vec<u8>,
    pub(crate) interps: Vec<InterpData>,
    pub(crate) args: Vec<ArgsData>,
    pub(crate) fields: Vec<FieldData>,
    pub(crate) spans: Vec<Sp>,
    pub(crate) types: Vec<TypeNode>,
    pub(crate) type_args: Vec<TypeArgData>,
    pub(crate) type_packs: Vec<TypePackData>,
    pub(crate) table_type_fields: Vec<TableTypeFieldData>,
    pub(crate) fn_types: Vec<FnTypeNode>,
    pub(crate) param_types: Vec<ParamTypeNode>,
    pub(crate) vararg_types: Vec<VarargTypeData>,
    pub(crate) rets: Vec<RetData>,
    pub(crate) type_params: Vec<TypeParamNode>,
    pub(crate) classes: Vec<ClassNode>,
    pub(crate) class_members: Vec<ClassMemberNode>,
    pub(crate) modifiers: Vec<Modifier>,
    pub(crate) interfaces: Vec<InterfaceNode>,
    pub(crate) interface_members: Vec<InterfaceMemberData>,
    pub(crate) enums: Vec<EnumNode>,
    pub(crate) enum_items: Vec<EnumItemNode>,
    pub(crate) aliases: Vec<AliasNode>,
    pub(crate) declares: Vec<DeclareData>,
    pub(crate) ns_members: Vec<NsMemberNode>,
    pub(crate) decl_members: Vec<DeclMemberNode>,
    pub(crate) imports: Vec<ImportNode>,
    pub(crate) import_specs: Vec<ImportSpecNode>,
}

macro_rules! arrays {
    ($m:ident) => {
        $m!(
            blocks,
            stmts,
            elseifs,
            typed_names,
            destructs,
            func_names,
            funcs,
            params,
            decorators,
            names,
            exprs,
            methods,
            strs,
            strbuf,
            interps,
            args,
            fields,
            spans,
            types,
            type_args,
            type_packs,
            table_type_fields,
            fn_types,
            param_types,
            vararg_types,
            rets,
            type_params,
            classes,
            class_members,
            modifiers,
            interfaces,
            interface_members,
            enums,
            enum_items,
            aliases,
            declares,
            ns_members,
            decl_members,
            imports,
            import_specs
        )
    };
}

impl Tree {
    fn empty(src: &[u8], file: FileId) -> Tree {
        macro_rules! new {
            ($($a:ident),*) => {
                Tree {
                    file,
                    src: SourceText::new(src),
                    span: Sp { start: 0, end: 0 },
                    directives: Vec::new(),
                    root: 0,
                    $($a: Vec::new(),)*
                }
            };
        }
        arrays!(new)
    }

    /// Releases the arrays' unused capacity.
    fn shrink(&mut self) {
        macro_rules! shrink {
            ($($a:ident),*) => {
                $(self.$a.shrink_to_fit();)*
            };
        }
        arrays!(shrink);
    }

    /// The heap the tree holds, by array: name, bytes, items. The source
    /// text counts as `source`.
    pub fn heap(&self) -> Vec<(&'static str, usize, usize)> {
        fn of<T>(v: &[T]) -> (usize, usize) {
            (std::mem::size_of_val(v), v.len())
        }
        let mut out = vec![("source", self.src.bytes().len(), 1)];
        macro_rules! heap {
            ($($a:ident),*) => {
                $({
                    let (bytes, n) = of(&self.$a);
                    out.push((stringify!($a), bytes, n));
                })*
            };
        }
        arrays!(heap);
        out
    }

    pub(crate) fn str_value(&self, value: Sp, in_buf: bool) -> &[u8] {
        let range = value.start as usize..value.end as usize;
        if in_buf {
            &self.strbuf[range]
        } else {
            &self.src.bytes()[range]
        }
    }
}

// ------------------------------------------------- from the boxed tree --

use crate::ast;

/// Builds the flat tree of a boxed one, node by node (lists end up
/// contiguous: a list's items are converted, then pushed together).
pub(crate) fn from_ast(chunk: &ast::Chunk, src: &[u8], file: FileId) -> Tree {
    let mut b = Builder {
        t: Tree::empty(src, file),
    };
    b.t.span = sp(chunk.span);
    b.t.directives = chunk.directives.clone();
    // One deep chain's worth of stack for the whole walk (`deep::run` at
    // every level would make a stack per level).
    b.t.root = crate::deep::run(|| b.block(&chunk.block));
    b.t.shrink();
    b.t
}

fn sp(s: wowlua_span::Span) -> Sp {
    Sp {
        start: s.start,
        end: s.end,
    }
}

fn opt<T>(x: Option<T>, f: impl FnOnce(T) -> u32) -> u32 {
    x.map_or(NONE, f)
}

struct Builder {
    t: Tree,
}

macro_rules! push_all {
    ($self:ident, $arr:ident, $items:expr) => {{
        let items: Vec<_> = $items;
        let start = $self.t.$arr.len() as u32;
        let len = items.len() as u32;
        $self.t.$arr.extend(items);
        Range { start, len }
    }};
}

impl Builder {
    fn name(&self, n: &ast::Name) -> Sp {
        let s = sp(n.span);
        debug_assert_eq!(
            self.t.src.bytes().get(s.start as usize..s.end as usize),
            Some(n.text.as_bytes()),
            "a name is its source text"
        );
        s
    }

    fn names(&mut self, ns: &[ast::Name]) -> Range {
        let items: Vec<Sp> = ns.iter().map(|n| self.name(n)).collect();
        push_all!(self, names, items)
    }

    fn block(&mut self, b: &ast::Block) -> u32 {
        let stmts = self.stmts(&b.stmts);
        self.t.blocks.push(BlockNode {
            stmts,
            span: sp(b.span),
        });
        self.t.blocks.len() as u32 - 1
    }

    fn stmts(&mut self, ss: &[ast::Stmt]) -> Range {
        let items: Vec<StmtNode> = ss.iter().map(|s| self.stmt_node(s)).collect();
        push_all!(self, stmts, items)
    }

    fn stmt(&mut self, s: &ast::Stmt) -> u32 {
        let node = self.stmt_node(s);
        self.t.stmts.push(node);
        self.t.stmts.len() as u32 - 1
    }

    fn stmt_node(&mut self, s: &ast::Stmt) -> StmtNode {
        use ast::StmtKind as K;
        let decorators = self.decorators(&s.decorators);
        let data = match &s.kind {
            K::Assign { targets, values } => StmtData::Assign {
                targets: self.exprs(targets),
                values: self.exprs(values),
            },
            K::Compound { target, op, value } => StmtData::Compound {
                target: self.expr(target),
                op: *op,
                value: self.expr(value),
            },
            K::Call(e) => StmtData::Call(self.expr(e)),
            K::Do(b) => StmtData::Do(self.block(b)),
            K::While { cond, body } => StmtData::While {
                cond: self.expr(cond),
                body: self.block(body),
            },
            K::Repeat { body, cond } => {
                let body = self.block(body);
                StmtData::Repeat {
                    body,
                    cond: self.expr(cond),
                }
            }
            K::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let cond = self.expr(cond);
                let then_block = self.block(then_block);
                let items: Vec<(u32, u32)> = elseifs
                    .iter()
                    .map(|(c, b)| (self.expr(c), self.block(b)))
                    .collect();
                let elseifs = push_all!(self, elseifs, items);
                let else_block = opt(else_block.as_deref(), |b| self.block(b));
                StmtData::If {
                    cond,
                    then_block,
                    elseifs,
                    else_block,
                }
            }
            K::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => {
                let var = self.typed_name(var);
                let var = {
                    self.t.typed_names.push(var);
                    self.t.typed_names.len() as u32 - 1
                };
                StmtData::NumericFor {
                    var,
                    start: self.expr(start),
                    limit: self.expr(limit),
                    step: opt(step.as_deref(), |e| self.expr(e)),
                    body: self.block(body),
                }
            }
            K::GenericFor { vars, exprs, body } => StmtData::GenericFor {
                vars: self.typed_names(vars),
                exprs: self.exprs(exprs),
                body: self.block(body),
            },
            K::Function { name, func } => {
                let name = self.func_name(name);
                StmtData::Function {
                    name,
                    func: self.func(func),
                }
            }
            K::LocalFunction { name, func } => StmtData::LocalFunction {
                name: self.name(name),
                func: self.func(func),
            },
            K::Local { names, values } => StmtData::Local {
                names: self.typed_names(names),
                values: self.exprs(values),
            },
            K::LocalDestructure { fields, ty, value } => {
                let items: Vec<DestructNode> = fields
                    .iter()
                    .map(|f| DestructNode {
                        name: self.name(&f.name),
                        default: opt(f.default.as_ref(), |e| self.expr(e)),
                    })
                    .collect();
                let fields = push_all!(self, destructs, items);
                StmtData::LocalDestructure {
                    fields,
                    ty: opt(ty.as_deref(), |t| self.ty(t)),
                    value: opt(value.as_deref(), |e| self.expr(e)),
                }
            }
            K::Const { name, ty, value } => StmtData::Const {
                name: self.name(name),
                ty: opt(ty.as_deref(), |t| self.ty(t)),
                value: opt(value.as_deref(), |e| self.expr(e)),
            },
            K::Return(values) => StmtData::Return(self.exprs(values)),
            K::Break => StmtData::Break,
            K::Continue => StmtData::Continue,
            K::Class(c) => StmtData::Class(self.class(c)),
            K::Interface(i) => StmtData::Interface(self.interface(i)),
            K::Enum(e) => StmtData::Enum(self.enum_decl(e)),
            K::TypeAlias(a) => StmtData::TypeAlias(self.alias(a)),
            K::Declare(d) => StmtData::Declare(self.declare(d)),
            K::Import(i) => StmtData::Import(self.import(i)),
            K::Export(s) => StmtData::Export(self.stmt(s)),
            K::Error => StmtData::Error,
        };
        StmtNode {
            span: sp(s.span),
            decorators,
            data,
        }
    }

    fn typed_name(&mut self, t: &ast::TypedName) -> TypedNameNode {
        TypedNameNode {
            name: self.name(&t.name),
            ty: opt(t.ty.as_ref(), |t| self.ty(t)),
        }
    }

    fn typed_names(&mut self, ts: &[ast::TypedName]) -> Range {
        let items: Vec<TypedNameNode> = ts.iter().map(|t| self.typed_name(t)).collect();
        push_all!(self, typed_names, items)
    }

    fn func_name(&mut self, n: &ast::FuncName) -> u32 {
        let path = self.names(&n.path);
        let node = FuncNameNode {
            path,
            method: n.method.as_ref().map(|m| self.name(m)),
            span: sp(n.span),
        };
        self.t.func_names.push(node);
        self.t.func_names.len() as u32 - 1
    }

    fn func(&mut self, f: &ast::FuncBody) -> u32 {
        let type_params = self.type_params(&f.type_params);
        let items: Vec<ParamNode> = f
            .params
            .iter()
            .map(|p| ParamNode {
                decorators: self.decorators(&p.decorators),
                name: self.name(&p.name),
                ty: opt(p.ty.as_ref(), |t| self.ty(t)),
                default: opt(p.default.as_deref(), |e| self.expr(e)),
            })
            .collect();
        let params = push_all!(self, params, items);
        let vararg = f
            .vararg
            .as_ref()
            .map(|v| (sp(v.span), opt(v.ty.as_ref(), |t| self.vararg_type(t))));
        let ret = opt(f.ret.as_ref(), |r| self.ret(r));
        let body = self.block(&f.body);
        self.t.funcs.push(FuncNode {
            type_params,
            params,
            vararg,
            ret,
            body,
            params_span: sp(f.params_span),
            end_span: sp(f.end_span),
            span: sp(f.span),
        });
        self.t.funcs.len() as u32 - 1
    }

    fn decorators(&mut self, ds: &[ast::Decorator]) -> Range {
        if ds.is_empty() {
            return Range::default();
        }
        let items: Vec<DecoratorNode> = ds
            .iter()
            .map(|d| DecoratorNode {
                path: self.names(&d.path),
                args: opt(d.args.as_ref(), |a| self.args(a)),
                span: sp(d.span),
                call: self.expr(&d.call),
            })
            .collect();
        push_all!(self, decorators, items)
    }

    fn exprs(&mut self, es: &[ast::Expr]) -> Range {
        let items: Vec<ExprNode> = es.iter().map(|e| self.expr_node(e)).collect();
        push_all!(self, exprs, items)
    }

    fn expr(&mut self, e: &ast::Expr) -> u32 {
        let node = self.expr_node(e);
        self.t.exprs.push(node);
        self.t.exprs.len() as u32 - 1
    }

    fn expr_node(&mut self, e: &ast::Expr) -> ExprNode {
        use ast::ExprKind as K;
        let data = match &e.kind {
            K::Nil => ExprData::Nil,
            K::True => ExprData::True,
            K::False => ExprData::False,
            K::Number(n) => ExprData::Number(self.numeral(n, e.span)),
            K::String(s) => ExprData::String(self.string(s, e.span)),
            K::Vararg => ExprData::Vararg,
            K::Function(f) => ExprData::Function(self.func(f)),
            K::Table(fields) => {
                let items: Vec<FieldData> = fields
                    .iter()
                    .map(|f| match f {
                        ast::TableField::Keyed { key, value, span } => {
                            let key = self.expr(key);
                            let value = self.expr(value);
                            self.t.spans.push(sp(*span));
                            FieldData::Keyed {
                                key,
                                value,
                                span: self.t.spans.len() as u32 - 1,
                            }
                        }
                        ast::TableField::Named { name, value } => FieldData::Named {
                            name: self.name(name),
                            value: self.expr(value),
                        },
                        ast::TableField::Positional(v) => FieldData::Positional(self.expr(v)),
                    })
                    .collect();
                ExprData::Table(push_all!(self, fields, items))
            }
            K::Interp(parts) => {
                let items: Vec<InterpData> = parts
                    .iter()
                    .map(|p| match p {
                        ast::InterpPart::Text { raw, value, span } => {
                            let (value, in_buf) = self.value_in(sp(*span), raw, value);
                            InterpData::Text {
                                raw: self.raw_in(sp(*span), raw),
                                value,
                                in_buf,
                                span: sp(*span),
                            }
                        }
                        ast::InterpPart::Expr(e) => InterpData::Expr(self.expr(e)),
                    })
                    .collect();
                ExprData::Interp(push_all!(self, interps, items))
            }
            K::Name(n) => ExprData::Name(self.name(n)),
            K::Index { obj, key } => ExprData::Index {
                obj: self.expr(obj),
                key: self.expr(key),
            },
            K::Field { obj, name } => ExprData::Field {
                obj: self.expr(obj),
                name: self.name(name),
            },
            K::Call { callee, args } => ExprData::Call {
                callee: self.expr(callee),
                args: self.args(args),
            },
            K::Method { obj, name, args } => ExprData::Method {
                obj: self.expr(obj),
                method: self.method(name, args),
            },
            K::OptField { obj, name } => ExprData::OptField {
                obj: self.expr(obj),
                name: self.name(name),
            },
            K::OptIndex { obj, key } => ExprData::OptIndex {
                obj: self.expr(obj),
                key: self.expr(key),
            },
            K::OptMethod { obj, name, args } => ExprData::OptMethod {
                obj: self.expr(obj),
                method: self.method(name, args),
            },
            K::OptCall { callee, args } => ExprData::OptCall {
                callee: self.expr(callee),
                args: self.args(args),
            },
            K::Paren(e) => ExprData::Paren(self.expr(e)),
            K::Binary { op, lhs, rhs } => ExprData::Binary {
                op: *op,
                lhs: self.expr(lhs),
                rhs: self.expr(rhs),
            },
            K::Unary { op, operand } => ExprData::Unary {
                op: *op,
                operand: self.expr(operand),
            },
            K::Cast { expr, ty } => ExprData::Cast {
                expr: self.expr(expr),
                ty: self.ty(ty),
            },
            K::Error => ExprData::Error,
        };
        ExprNode {
            span: sp(e.span),
            data,
        }
    }

    fn method(&mut self, name: &ast::Name, args: &ast::Args) -> u32 {
        let node = MethodNode {
            name: self.name(name),
            args: self.args(args),
        };
        self.t.methods.push(node);
        self.t.methods.len() as u32 - 1
    }

    fn args(&mut self, a: &ast::Args) -> u32 {
        let data = match a {
            ast::Args::Parens { args, span } => ArgsData::Parens {
                args: self.exprs(args),
                span: sp(*span),
            },
            ast::Args::Table(e) => ArgsData::Table(self.expr(e)),
            ast::Args::String(e) => ArgsData::String(self.expr(e)),
        };
        self.t.args.push(data);
        self.t.args.len() as u32 - 1
    }

    /// A numeral: its token's text, which is the node's span.
    fn numeral(&self, n: &ast::NumberLit, span: wowlua_span::Span) -> Sp {
        let s = sp(span);
        assert_eq!(
            self.t.src.bytes().get(s.start as usize..s.end as usize),
            Some(n.raw.as_bytes()),
            "a numeral is its source text"
        );
        s
    }

    /// Where `raw` is in the source, at or after `near.start`.
    fn raw_in(&self, near: Sp, raw: &[u8]) -> Sp {
        let src = self.t.src.bytes();
        let start = near.start as usize;
        let at = (start..=src.len().saturating_sub(raw.len()))
            .find(|&i| &src[i..i + raw.len()] == raw)
            .expect("literal text is in the source");
        Sp {
            start: at as u32,
            end: (at + raw.len()) as u32,
        }
    }

    /// `value` as source bytes inside `near`, or else copied to `strbuf`.
    fn value_in(&mut self, near: Sp, raw: &[u8], value: &[u8]) -> (Sp, bool) {
        let raw_at = self.raw_in(near, raw);
        let src = self.t.src.bytes();
        let (lo, hi) = (raw_at.start as usize, raw_at.end as usize);
        if let Some(i) =
            (lo..=hi.saturating_sub(value.len())).find(|&i| &src[i..i + value.len()] == value)
        {
            return (
                Sp {
                    start: i as u32,
                    end: (i + value.len()) as u32,
                },
                false,
            );
        }
        let start = self.t.strbuf.len() as u32;
        self.t.strbuf.extend_from_slice(value);
        (
            Sp {
                start,
                end: self.t.strbuf.len() as u32,
            },
            true,
        )
    }

    fn string(&mut self, s: &ast::StringLit, near: wowlua_span::Span) -> u32 {
        let near = sp(near);
        let raw = self.raw_in(near, &s.raw);
        let (value, in_buf) = self.value_in(near, &s.raw, &s.value);
        self.t.strs.push(StrNode {
            raw,
            value,
            in_buf,
            long: s.long,
        });
        self.t.strs.len() as u32 - 1
    }

    fn types(&mut self, ts: &[ast::Type]) -> Range {
        let items: Vec<TypeNode> = ts.iter().map(|t| self.type_node(t)).collect();
        push_all!(self, types, items)
    }

    fn ty(&mut self, t: &ast::Type) -> u32 {
        let node = self.type_node(t);
        self.t.types.push(node);
        self.t.types.len() as u32 - 1
    }

    fn type_node(&mut self, t: &ast::Type) -> TypeNode {
        use ast::TypeKind as K;
        let data = match &t.kind {
            K::Nil => TypeData::Nil,
            K::True => TypeData::True,
            K::False => TypeData::False,
            K::AnyFunction => TypeData::AnyFunction,
            K::String(s) => TypeData::String(self.string(s, t.span)),
            K::Number(n) => TypeData::Number(self.numeral(n, t.span)),
            K::Ref { path, args } => {
                let path = self.names(path);
                let items: Vec<TypeArgData> = args
                    .iter()
                    .map(|a| match a {
                        ast::TypeArg::Type(t) => TypeArgData::Type(self.ty(t)),
                        ast::TypeArg::Pack(p) => {
                            let p = self.type_pack(p);
                            self.t.type_packs.push(p);
                            TypeArgData::Pack(self.t.type_packs.len() as u32 - 1)
                        }
                    })
                    .collect();
                TypeData::Ref {
                    path,
                    args: push_all!(self, type_args, items),
                }
            }
            K::Typeof(e) => TypeData::Typeof(self.expr(e)),
            K::Table(fields) => {
                let items: Vec<TableTypeFieldData> = fields
                    .iter()
                    .map(|f| match f {
                        ast::TableTypeField::Named {
                            name,
                            optional,
                            readonly,
                            ty,
                        } => TableTypeFieldData::Named {
                            name: self.name(name),
                            optional: *optional,
                            readonly: *readonly,
                            ty: self.ty(ty),
                        },
                        ast::TableTypeField::NamedString { key, ty, span } => {
                            TableTypeFieldData::NamedString {
                                key: self.string(key, *span),
                                ty: self.ty(ty),
                                span: sp(*span),
                            }
                        }
                        ast::TableTypeField::Indexer { key, value } => {
                            TableTypeFieldData::Indexer {
                                key: self.ty(key),
                                value: self.ty(value),
                            }
                        }
                        ast::TableTypeField::Array(t) => TableTypeFieldData::Array(self.ty(t)),
                    })
                    .collect();
                TypeData::Table(push_all!(self, table_type_fields, items))
            }
            K::Function(f) => {
                let type_params = self.type_params(&f.type_params);
                let items: Vec<ParamTypeNode> = f
                    .params
                    .iter()
                    .map(|p| ParamTypeNode {
                        name: p.name.as_ref().map(|n| self.name(n)),
                        ty: self.ty(&p.ty),
                    })
                    .collect();
                let params = push_all!(self, param_types, items);
                let vararg = opt(f.vararg.as_ref(), |v| self.vararg_type(v));
                let ret = self.ret(&f.ret);
                self.t.fn_types.push(FnTypeNode {
                    type_params,
                    params,
                    vararg,
                    ret,
                });
                TypeData::Function(self.t.fn_types.len() as u32 - 1)
            }
            K::Paren(t) => TypeData::Paren(self.ty(t)),
            K::Optional(t) => TypeData::Optional(self.ty(t)),
            K::Array(t) => TypeData::Array(self.ty(t)),
            K::Union(ts) => TypeData::Union(self.types(ts)),
            K::Intersection(ts) => TypeData::Intersection(self.types(ts)),
            K::Error => TypeData::Error,
        };
        TypeNode {
            span: sp(t.span),
            data,
        }
    }

    fn type_pack(&mut self, p: &ast::TypePack) -> TypePackData {
        match p {
            ast::TypePack::Named(n) => TypePackData::Named(self.name(n)),
            ast::TypePack::List {
                types,
                vararg,
                span,
            } => TypePackData::List {
                types: self.types(types),
                vararg: opt(vararg.as_deref(), |v| self.vararg_type(v)),
                span: sp(*span),
            },
        }
    }

    fn vararg_type(&mut self, v: &ast::VarargType) -> u32 {
        let data = match v {
            ast::VarargType::Type(t) => VarargTypeData::Type(self.ty(t)),
            ast::VarargType::Pack(n) => VarargTypeData::Pack(self.name(n)),
            ast::VarargType::Expand(t) => VarargTypeData::Expand(self.ty(t)),
        };
        self.t.vararg_types.push(data);
        self.t.vararg_types.len() as u32 - 1
    }

    fn ret(&mut self, r: &ast::RetType) -> u32 {
        let data = match r {
            ast::RetType::Single(t) => RetData::Single(self.ty(t)),
            ast::RetType::List {
                types,
                vararg,
                span,
            } => RetData::List {
                types: self.types(types),
                vararg: opt(vararg.as_deref(), |v| self.vararg_type(v)),
                span: sp(*span),
            },
            ast::RetType::Pack(n) => RetData::Pack(self.name(n)),
            ast::RetType::Predicate { name, ty } => RetData::Predicate {
                name: self.name(name),
                ty: self.ty(ty),
            },
        };
        self.t.rets.push(data);
        self.t.rets.len() as u32 - 1
    }

    fn type_params(&mut self, ps: &[ast::TypeParam]) -> Range {
        let items: Vec<TypeParamNode> = ps
            .iter()
            .map(|p| TypeParamNode {
                name: self.name(&p.name),
                constraint: opt(p.constraint.as_ref(), |t| self.ty(t)),
                pack: p.pack,
            })
            .collect();
        push_all!(self, type_params, items)
    }

    fn class(&mut self, c: &ast::ClassDecl) -> u32 {
        let type_params = self.type_params(&c.type_params);
        let extends = opt(c.extends.as_ref(), |t| self.ty(t));
        let implements = self.types(&c.implements);
        let items: Vec<ClassMemberNode> = c
            .members
            .iter()
            .map(|m| {
                use ast::ClassMemberKind as K;
                let decorators = self.decorators(&m.decorators);
                let modifiers = push_all!(self, modifiers, m.modifiers.clone());
                let data = match &m.kind {
                    K::Field {
                        name,
                        optional,
                        ty,
                        value,
                    } => ClassMemberData::Field {
                        name: self.name(name),
                        optional: *optional,
                        ty: self.ty(ty),
                        value: opt(value.as_ref(), |e| self.expr(e)),
                    },
                    K::Method {
                        name,
                        func,
                        has_body,
                    } => ClassMemberData::Method {
                        name: self.name(name),
                        func: self.func(func),
                        has_body: *has_body,
                    },
                    K::Getter { name, ty, body } => ClassMemberData::Getter {
                        name: self.name(name),
                        ty: opt(ty.as_ref(), |t| self.ty(t)),
                        body: self.block(body),
                    },
                    K::Setter { name, param, body } => {
                        let name = self.name(name);
                        let p = self.typed_name(param);
                        self.t.typed_names.push(p);
                        let param = self.t.typed_names.len() as u32 - 1;
                        ClassMemberData::Setter {
                            name,
                            param,
                            body: self.block(body),
                        }
                    }
                    K::Constructor { func, has_body } => ClassMemberData::Constructor {
                        func: self.func(func),
                        has_body: *has_body,
                    },
                };
                ClassMemberNode {
                    decorators,
                    modifiers,
                    span: sp(m.span),
                    data,
                }
            })
            .collect();
        let members = push_all!(self, class_members, items);
        self.t.classes.push(ClassNode {
            is_abstract: c.is_abstract,
            name: self.name(&c.name),
            type_params,
            extends,
            implements,
            members,
            end_span: sp(c.end_span),
        });
        self.t.classes.len() as u32 - 1
    }

    fn interface(&mut self, i: &ast::InterfaceDecl) -> u32 {
        let type_params = self.type_params(&i.type_params);
        let extends = self.types(&i.extends);
        let items: Vec<InterfaceMemberData> = i
            .members
            .iter()
            .map(|m| match m {
                ast::InterfaceMember::Field {
                    readonly,
                    name,
                    optional,
                    ty,
                } => InterfaceMemberData::Field {
                    readonly: *readonly,
                    name: self.name(name),
                    optional: *optional,
                    ty: self.ty(ty),
                },
                ast::InterfaceMember::Method { name, sig } => InterfaceMemberData::Method {
                    name: self.name(name),
                    sig: self.func(sig),
                },
            })
            .collect();
        let members = push_all!(self, interface_members, items);
        self.t.interfaces.push(InterfaceNode {
            name: self.name(&i.name),
            type_params,
            extends,
            members,
        });
        self.t.interfaces.len() as u32 - 1
    }

    fn enum_decl(&mut self, e: &ast::EnumDecl) -> u32 {
        let items: Vec<EnumItemNode> = e
            .items
            .iter()
            .map(|i| EnumItemNode {
                name: self.name(&i.name),
                value: opt(i.value.as_ref(), |e| self.expr(e)),
            })
            .collect();
        let items = push_all!(self, enum_items, items);
        self.t.enums.push(EnumNode {
            name: self.name(&e.name),
            items,
        });
        self.t.enums.len() as u32 - 1
    }

    fn alias(&mut self, a: &ast::TypeAlias) -> u32 {
        let type_params = self.type_params(&a.type_params);
        let ty = self.ty(&a.ty);
        self.t.aliases.push(AliasNode {
            name: self.name(&a.name),
            type_params,
            ty,
        });
        self.t.aliases.len() as u32 - 1
    }

    fn declare(&mut self, d: &ast::Declare) -> u32 {
        let data = match d {
            ast::Declare::Function { name, sig } => {
                let name = self.func_name(name);
                DeclareData::Function {
                    name,
                    sig: self.func(sig),
                }
            }
            ast::Declare::Global { name, ty } => DeclareData::Global {
                name: self.name(name),
                ty: self.ty(ty),
            },
            ast::Declare::Namespace { name, members } => DeclareData::Namespace {
                name: self.name(name),
                members: self.ns_members(members),
            },
            ast::Declare::Class {
                name,
                type_params,
                extends,
                implements,
                members,
            } => DeclareData::Class {
                name: self.name(name),
                type_params: self.type_params(type_params),
                extends: opt(extends.as_ref(), |t| self.ty(t)),
                implements: self.types(implements),
                members: self.decl_members(members),
            },
            ast::Declare::Interface(i) => DeclareData::Interface(self.interface(i)),
            ast::Declare::Event { name, params } => DeclareData::Event {
                name: self.name(name),
                params: self.func(params),
            },
            ast::Declare::Message { name, params } => DeclareData::Message {
                name: self.name(name),
                params: self.func(params),
            },
            ast::Declare::Module { path, members } => {
                // The path is the declaration's only string: find it from
                // the first member's start backwards, or from the start of
                // the file, by its raw text.
                let path = self.string(
                    path,
                    wowlua_span::Span {
                        file: self.t.file,
                        start: 0,
                        end: 0,
                    },
                );
                DeclareData::Module {
                    path,
                    members: self.ns_members(members),
                }
            }
        };
        self.t.declares.push(data);
        self.t.declares.len() as u32 - 1
    }

    fn ns_members(&mut self, ms: &[ast::NsMember]) -> Range {
        let items: Vec<NsMemberNode> = ms
            .iter()
            .map(|m| {
                use ast::NsMemberKind as K;
                let decorators = self.decorators(&m.decorators);
                let data = match &m.kind {
                    K::Function { name, sig } => NsMemberData::Function {
                        name: self.name(name),
                        sig: self.func(sig),
                    },
                    K::Value { is_const, name, ty } => NsMemberData::Value {
                        is_const: *is_const,
                        name: self.name(name),
                        ty: self.ty(ty),
                    },
                    K::Namespace { name, members } => NsMemberData::Namespace {
                        name: self.name(name),
                        members: self.ns_members(members),
                    },
                    K::Class {
                        name,
                        type_params,
                        extends,
                        members,
                    } => NsMemberData::Class {
                        name: self.name(name),
                        type_params: self.type_params(type_params),
                        extends: opt(extends.as_ref(), |t| self.ty(t)),
                        members: self.decl_members(members),
                    },
                    K::Interface(i) => NsMemberData::Interface(self.interface(i)),
                    K::TypeAlias(a) => NsMemberData::TypeAlias(self.alias(a)),
                    K::Enum(e) => NsMemberData::Enum(self.enum_decl(e)),
                };
                NsMemberNode {
                    decorators,
                    span: sp(m.span),
                    data,
                }
            })
            .collect();
        push_all!(self, ns_members, items)
    }

    fn decl_members(&mut self, ms: &[ast::DeclMember]) -> Range {
        let items: Vec<DeclMemberNode> = ms
            .iter()
            .map(|m| {
                let decorators = self.decorators(&m.decorators);
                let data = match &m.kind {
                    ast::DeclMemberKind::Function { name, sig } => DeclMemberData::Function {
                        name: self.name(name),
                        sig: self.func(sig),
                    },
                    ast::DeclMemberKind::Field {
                        readonly,
                        name,
                        optional,
                        ty,
                    } => DeclMemberData::Field {
                        readonly: *readonly,
                        name: self.name(name),
                        optional: *optional,
                        ty: self.ty(ty),
                    },
                };
                DeclMemberNode {
                    decorators,
                    is_static: m.is_static,
                    span: sp(m.span),
                    data,
                }
            })
            .collect();
        push_all!(self, decl_members, items)
    }

    fn import(&mut self, i: &ast::Import) -> u32 {
        let clause = match &i.clause {
            ast::ImportClause::Namespace(n) => ImportClauseData::Namespace(self.name(n)),
            ast::ImportClause::SideEffect => ImportClauseData::SideEffect,
            ast::ImportClause::Names(specs) => {
                let items: Vec<ImportSpecNode> = specs
                    .iter()
                    .map(|s| ImportSpecNode {
                        name: self.name(&s.name),
                        alias: s.alias.as_ref().map(|a| self.name(a)),
                    })
                    .collect();
                ImportClauseData::Names(push_all!(self, import_specs, items))
            }
        };
        // The specifier is the statement's last token; it is found by its
        // text after the clause.
        let near = match clause {
            ImportClauseData::Namespace(n) => n,
            ImportClauseData::Names(r) if r.len > 0 => {
                let last = self.t.import_specs[(r.start + r.len - 1) as usize];
                last.alias.unwrap_or(last.name)
            }
            _ => Sp { start: 0, end: 0 },
        };
        let from = self.string(
            &i.from,
            wowlua_span::Span {
                file: self.t.file,
                start: near.end,
                end: near.end,
            },
        );
        self.t.imports.push(ImportNode {
            clause,
            type_only: i.type_only,
            from,
        });
        self.t.imports.len() as u32 - 1
    }
}
