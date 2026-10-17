//! The flat syntax tree (#19): a file's nodes in a few arrays, children
//! referred to by `u32` index, lists as contiguous ranges of an array.
//! Names and numerals are ranges of the source, which the tree keeps;
//! strings keep a decoded value only where it differs from their source
//! bytes. Read it through the [views](crate::view).

use crate::ops::{BinOp, CompoundOp, Directive, Modifier, UnOp};
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
    // The views show `Error` nodes; the parser builds none today.
    #[allow(dead_code)]
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
    Ref {
        path: Range,
        args: Range,
    },
    Typeof(u32),
    Table(Range),
    Function(u32),
    Paren(u32),
    Optional(u32),
    Array(u32),
    Union(Range),
    Intersection(Range),
    // The views show `Error` nodes; the parser builds none today.
    #[allow(dead_code)]
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
    pub(crate) fn empty(src: &[u8], file: FileId) -> Tree {
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
    pub(crate) fn shrink(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    // Every node is a few u32: the tree's memory is these sizes times the
    // node counts (`cargo xtask parse-stats`).
    #[test]
    fn nodes_stay_small() {
        assert!(size_of::<Sp>() == 8);
        assert!(size_of::<ExprNode>() <= 24);
        assert!(size_of::<FieldData>() <= 16);
        assert!(size_of::<StrNode>() <= 20);
        assert!(size_of::<TypeNode>() <= 28);
        assert!(size_of::<StmtNode>() <= 40);
        assert!(size_of::<ParamNode>() <= 24);
        assert!(size_of::<FuncNode>() <= 64);
    }
}
