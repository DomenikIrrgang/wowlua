//! Classes and enums across the package (spec chapter 3): what each class
//! extends (another wowlua class, a native widget type, or an unknown
//! value), widget classes, and the structural checks that need no types
//! (decision T75): reserved names, `override`, abstract methods, `super`,
//! widget restrictions, field/method clashes, initializers and enum values.

use crate::constant::{self, Const};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::Span;
use wowlua_syntax::Parse;
use wowlua_syntax::view::*;

#[derive(Clone, Debug, PartialEq)]
pub struct ClassInfo {
    pub name: String,
    pub is_abstract: bool,
    pub base: Base,
    /// The native widget type (`CreateFrame`'s first argument) of a widget
    /// class (spec §3.2).
    pub widget: Option<String>,
    pub members: Vec<Member>,
}

impl ClassInfo {
    /// Instance fields with an initializer.
    pub fn has_field_inits(&self) -> bool {
        self.members.iter().any(|m| {
            !m.is_static
                && matches!(
                    m.kind,
                    MemberKind::Field {
                        has_value: true,
                        ..
                    }
                )
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    None,
    /// A wowlua class: (input file, class name).
    Class {
        file: usize,
        name: String,
    },
    /// A declared native widget type.
    Widget {
        name: String,
        widget: String,
    },
    /// Any other value (checked at run time).
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    pub kind: MemberKind,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_override: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Field { has_value: bool },
    Method,
    Getter,
    Setter,
    Constructor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumInfo {
    pub name: String,
    /// Members in declaration order.
    pub members: Vec<(String, Const)>,
}

impl EnumInfo {
    pub fn value(&self, member: &str) -> Option<&Const> {
        self.members
            .iter()
            .find(|(n, _)| n == member)
            .map(|(_, v)| v)
    }
}

/// Reserved member names (spec §3.1.7); every other `__` name that is not
/// an allowed metamethod is reserved for the runtime too.
const RESERVED: &[&str] = &[
    "new",
    "__index",
    "__newindex",
    "__name",
    "__super",
    "__init",
    "__ctor",
    "__initFields",
    "__class",
    "__meta",
    "__getters",
    "__setters",
];

pub const METAMETHODS: &[&str] = &[
    "__tostring",
    "__eq",
    "__lt",
    "__le",
    "__add",
    "__sub",
    "__mul",
    "__div",
    "__mod",
    "__pow",
    "__unm",
    "__concat",
    "__call",
];

/// A native widget type from a declaration: `@widget("Button") declare
/// class Button extends Frame`.
#[derive(Clone, Debug)]
pub struct WidgetDecl {
    pub widget: Option<String>,
    pub base: Option<String>,
}

/// Native widget types declared by declaration files.
pub fn widget_decls(parses: &[&Parse]) -> HashMap<String, WidgetDecl> {
    let mut out = HashMap::new();
    for parse in parses {
        for s in parse.view().block().stmts() {
            let StmtKind::Declare(d) = s.kind() else {
                continue;
            };
            let DeclareKind::Class { name, extends, .. } = d.kind() else {
                continue;
            };
            let widget = s.decorators().iter().find_map(|d| {
                if d.path().len() != 1 || d.path().at(0).text() != "widget" {
                    return None;
                }
                match d.args().as_ref()?.exprs().first().map(|e| e.kind()) {
                    Some(ExprKind::String(s)) => {
                        Some(String::from_utf8_lossy(&s.value()).into_owned())
                    }
                    _ => None,
                }
            });
            let base = extends
                .and_then(type_path)
                .and_then(|p| match p.as_slice() {
                    [one] => Some(one.clone()),
                    _ => None,
                });
            out.insert(name.text().to_string(), WidgetDecl { widget, base });
        }
    }
    out
}

impl WidgetDecl {
    /// The widget type, following declared bases.
    fn resolve(name: &str, decls: &HashMap<String, WidgetDecl>) -> Option<String> {
        let mut current = name.to_string();
        for _ in 0..32 {
            let d = decls.get(&current)?;
            if let Some(w) = &d.widget {
                return Some(w.clone());
            }
            current = d.base.clone()?;
        }
        None
    }
}

/// The path of a type reference (`a.b.C<T>` → `[a, b, C]`).
pub fn type_path(t: Type<'_>) -> Option<Vec<String>> {
    match t.kind() {
        TypeKind::Ref { path, .. } => Some(path.iter().map(|n| n.text().to_string()).collect()),
        TypeKind::Paren(inner) => type_path(inner),
        _ => None,
    }
}

/// The top-level class and enum declarations of a file.
pub fn declarations(parse: &Parse) -> (Vec<ClassDecl<'_>>, Vec<(EnumDecl<'_>, Span)>) {
    let mut classes = Vec::new();
    let mut enums = Vec::new();
    for s in parse.view().block().stmts() {
        let kind = match s.kind() {
            StmtKind::Export(inner) => inner.kind(),
            k => k,
        };
        match kind {
            StmtKind::Class(c) => classes.push(c),
            StmtKind::Enum(e) => enums.push((e, s.span())),
            _ => {}
        }
    }
    (classes, enums)
}

fn has(member: ClassMember<'_>, m: Modifier) -> bool {
    member.modifiers().contains(&m)
}

/// The member summary of a class declaration (overload signatures are
/// compile-time only and left out).
pub fn members(c: ClassDecl<'_>) -> Vec<Member> {
    let mut out = Vec::new();
    for m in c.members() {
        if has(m, Modifier::Overload) {
            continue;
        }
        let (name, kind) = match m.kind() {
            ClassMemberKind::Field { name, value, .. } => (
                name,
                MemberKind::Field {
                    has_value: value.is_some(),
                },
            ),
            ClassMemberKind::Method { name, .. } => (name, MemberKind::Method),
            ClassMemberKind::Getter { name, .. } => (name, MemberKind::Getter),
            ClassMemberKind::Setter { name, .. } => (name, MemberKind::Setter),
            ClassMemberKind::Constructor { func, .. } => {
                out.push(Member {
                    name: "constructor".into(),
                    kind: MemberKind::Constructor,
                    is_static: false,
                    is_abstract: false,
                    is_override: false,
                    span: m.span(),
                });
                // its parameter properties are fields (decision L35)
                for p in func.params().iter().filter(|p| p.is_property()) {
                    out.push(Member {
                        name: p.name().text().to_string(),
                        kind: MemberKind::Field { has_value: true },
                        is_static: false,
                        is_abstract: false,
                        is_override: false,
                        span: p.name().span(),
                    });
                }
                continue;
            }
        };
        out.push(Member {
            name: name.text().to_string(),
            kind,
            is_static: has(m, Modifier::Static),
            is_abstract: has(m, Modifier::Abstract),
            is_override: has(m, Modifier::Override),
            span: name.span(),
        });
    }
    out
}

/// Folds the values of an enum (spec §3.4.1); E3010 and E3011.
pub fn enum_info(
    e: EnumDecl<'_>,
    env: &dyn Fn(&str) -> Option<Const>,
    out: &mut Vec<Diagnostic>,
) -> EnumInfo {
    let mut members: Vec<(String, Const)> = Vec::new();
    let mut next = 1.0;
    let mut numeric: Option<bool> = None;
    for item in e.items() {
        let value = match &item.value {
            Some(expr) => match constant::eval(*expr, env) {
                Some(c @ (Const::Number(_) | Const::String(_))) => c,
                _ => {
                    out.push(
                        Diagnostic::error(
                            "E3010",
                            "enum member values must be number or string constants",
                        )
                        .with_label(expr.span(), "not a constant"),
                    );
                    continue;
                }
            },
            None => match numeric {
                Some(false) => {
                    out.push(
                        Diagnostic::error(
                            "E3010",
                            format!(
                                "enum `{}` has string values, so `{}` needs a value",
                                e.name().text(),
                                item.name.text()
                            ),
                        )
                        .with_label(item.name.span(), "no value"),
                    );
                    continue;
                }
                _ => Const::Number(next),
            },
        };
        let is_number = matches!(value, Const::Number(_));
        match numeric {
            Some(n) if n != is_number => out.push(
                Diagnostic::error(
                    "E3010",
                    format!("enum `{}` mixes numeric and string values", e.name().text()),
                )
                .with_label(item.name.span(), "here"),
            ),
            _ => numeric = Some(is_number),
        }
        if let Const::Number(n) = value {
            next = n + 1.0;
        }
        if members.iter().any(|(_, v)| *v == value) {
            out.push(
                Diagnostic::error(
                    "E3011",
                    format!("duplicate value in enum `{}`", e.name().text()),
                )
                .with_label(item.name.span(), "this value is used by an earlier member"),
            );
        }
        if members.iter().any(|(n, _)| n == item.name.text()) {
            out.push(
                Diagnostic::error(
                    "E3011",
                    format!(
                        "duplicate member `{}` in enum `{}`",
                        item.name.text(),
                        e.name().text()
                    ),
                )
                .with_label(item.name.span(), "declared before"),
            );
        }
        members.push((item.name.text().to_string(), value));
    }
    EnumInfo {
        name: e.name().text().to_string(),
        members,
    }
}

/// The package's classes: `(file, name)` → info.
pub type ClassTable = HashMap<(usize, String), Arc<ClassInfo>>;

/// The superclass chain of a class (nearest first), as far as it is known;
/// `complete` is false when it ends in an unknown value or a cycle.
pub fn ancestors(table: &ClassTable, info: &ClassInfo) -> (Vec<Arc<ClassInfo>>, bool) {
    let mut out: Vec<Arc<ClassInfo>> = Vec::new();
    let mut base = info.base.clone();
    for _ in 0..64 {
        match base {
            Base::None => return (out, true),
            Base::Widget { .. } => return (out, true),
            Base::Unknown => return (out, false),
            Base::Class { file, name } => match table.get(&(file, name)) {
                Some(c) => {
                    if out.iter().any(|a| Arc::ptr_eq(a, c)) {
                        return (out, false);
                    }
                    out.push(c.clone());
                    base = c.base.clone();
                }
                None => return (out, false),
            },
        }
    }
    (out, false)
}

/// The structural checks of one class declaration.
pub fn check_class(
    c: ClassDecl<'_>,
    info: &ClassInfo,
    table: &ClassTable,
    out: &mut Vec<Diagnostic>,
) {
    let (chain, complete) = ancestors(table, info);
    let ends_in_widget = info.widget.is_some();
    let mut seen: HashMap<&str, MemberKind> = HashMap::new();
    for m in &info.members {
        let name = m.name.as_str();
        if m.kind == MemberKind::Constructor {
            continue;
        }
        if RESERVED.contains(&name) || (name.starts_with("__") && !METAMETHODS.contains(&name)) {
            out.push(
                Diagnostic::error("E3004", format!("`{name}` is a reserved member name"))
                    .with_label(m.span, "reserved"),
            );
        }
        if info.widget.is_some() {
            if METAMETHODS.contains(&name) {
                out.push(
                    Diagnostic::error(
                        "E3007",
                        format!(
                            "widget class `{}` cannot define the metamethod `{name}`",
                            info.name
                        ),
                    )
                    .with_label(m.span, "the frame's metatable belongs to WoW"),
                );
            }
            if matches!(m.kind, MemberKind::Getter | MemberKind::Setter) {
                out.push(
                    Diagnostic::error(
                        "E3043",
                        format!("widget class `{}` cannot declare accessors", info.name),
                    )
                    .with_label(m.span, "the frame's metatable belongs to WoW"),
                );
            }
        }
        if m.is_abstract && !info.is_abstract {
            out.push(
                Diagnostic::error(
                    "E3003",
                    format!(
                        "abstract method `{name}` in class `{}`, which is not abstract",
                        info.name
                    ),
                )
                .with_label(m.span, "declare the class `abstract class`"),
            );
        }
        // A field and a method or accessor with the same name (E3042).
        let is_field = matches!(m.kind, MemberKind::Field { .. });
        if let Some(prev) = seen.get(name)
            && matches!(prev, MemberKind::Field { .. }) != is_field
        {
            out.push(
                Diagnostic::error(
                    "E3042",
                    format!("`{name}` is declared as a field and as a method or accessor"),
                )
                .with_label(m.span, "here"),
            );
        }
        // A field declared twice (a parameter property among them; E3046).
        if is_field && matches!(seen.get(name), Some(MemberKind::Field { .. })) {
            out.push(
                Diagnostic::error("E3046", format!("the field `{name}` is declared twice"))
                    .with_label(m.span, "here"),
            );
        }
        seen.entry(name).or_insert(m.kind);
        let inherited = chain.iter().find_map(|a| {
            a.members
                .iter()
                .find(|x| x.name == m.name && x.kind != MemberKind::Constructor)
        });
        if let Some(x) = inherited
            && matches!(x.kind, MemberKind::Field { .. }) != is_field
        {
            out.push(
                Diagnostic::error(
                    "E3042",
                    format!(
                        "`{name}` is a field in one class and a method or accessor in the other"
                    ),
                )
                .with_label(m.span, "here")
                .with_label(x.span, "inherited member"),
            );
        }
        // `override` (E3005): methods and accessors.
        if is_field {
            continue;
        }
        match (inherited, m.is_override) {
            (Some(x), false) if !matches!(x.kind, MemberKind::Field { .. }) => out.push(
                Diagnostic::error(
                    "E3005",
                    format!("`{name}` overrides a member of a superclass; mark it `override`"),
                )
                .with_label(m.span, "missing `override`")
                .with_label(x.span, "overridden member"),
            ),
            (None, true) if complete && !ends_in_widget => out.push(
                Diagnostic::error(
                    "E3005",
                    format!("`{name}` is marked `override`, but no superclass declares it"),
                )
                .with_label(m.span, "nothing to override"),
            ),
            _ => {}
        }
    }
    // Abstract methods a concrete class leaves unimplemented (E3003).
    if !info.is_abstract {
        let mut state: HashMap<String, bool> = HashMap::new();
        for a in chain.iter().rev() {
            for m in a.members.iter().filter(|m| m.kind == MemberKind::Method) {
                state.insert(m.name.clone(), m.is_abstract);
            }
        }
        for m in info.members.iter().filter(|m| m.kind == MemberKind::Method) {
            state.insert(m.name.clone(), m.is_abstract);
        }
        let mut missing: Vec<String> = state
            .into_iter()
            .filter(|(_, a)| *a)
            .map(|(n, _)| n)
            .collect();
        missing.sort();
        if !missing.is_empty() {
            out.push(
                Diagnostic::error(
                    "E3003",
                    format!(
                        "class `{}` does not implement the abstract method{} {}",
                        info.name,
                        if missing.len() > 1 { "s" } else { "" },
                        missing
                            .iter()
                            .map(|n| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
                .with_label(c.name().span(), "not abstract"),
            );
        }
    }
    check_bodies(c, info, out);
}

/// `super` rules (E3001, E3040) and `...` in initializers (E3041).
fn check_bodies(c: ClassDecl<'_>, info: &ClassInfo, out: &mut Vec<Diagnostic>) {
    let derived = info.base != Base::None;
    for m in c.members() {
        match m.kind() {
            ClassMemberKind::Field { value: Some(v), .. } => {
                if let Some(span) = vararg_in(v) {
                    out.push(
                        Diagnostic::error("E3041", "`...` cannot be used in a field initializer")
                            .with_label(span, "here"),
                    );
                }
            }
            ClassMemberKind::Constructor {
                func,
                has_body: true,
            } => {
                check_super_names(func.body(), func, out);
                let mut supers = Vec::new();
                let mut self_before: Option<Span> = None;
                for (i, s) in func.body().stmts().iter().enumerate() {
                    if is_super_call(s) {
                        supers.push((i, s.span()));
                    } else if supers.is_empty() && self_before.is_none() {
                        self_before = self_use(s);
                    }
                }
                let nested = nested_super_calls(func.body());
                for span in nested {
                    out.push(
                        Diagnostic::error(
                            "E3001",
                            "`super(...)` must be a statement at the top of the constructor body",
                        )
                        .with_label(span, "here"),
                    );
                }
                if !derived {
                    for (_, span) in &supers {
                        out.push(
                            Diagnostic::error(
                                "E3001",
                                format!("class `{}` has no superclass to call", info.name),
                            )
                            .with_label(*span, "here"),
                        );
                    }
                    continue;
                }
                match supers.as_slice() {
                    [] => out.push(
                        Diagnostic::error(
                            "E3001",
                            "the constructor of a derived class must call `super(...)`",
                        )
                        .with_label(func.params_span(), "this constructor"),
                    ),
                    [(_, _), rest @ ..] => {
                        for (_, span) in rest {
                            out.push(
                                Diagnostic::error("E3001", "`super(...)` is called more than once")
                                    .with_label(*span, "again here"),
                            );
                        }
                    }
                }
                if let (Some(span), false) = (self_before, supers.is_empty()) {
                    out.push(
                        Diagnostic::error("E3001", "`self` is used before `super(...)`")
                            .with_label(span, "used here"),
                    );
                }
            }
            ClassMemberKind::Method { func, .. } => check_super_names(func.body(), func, out),
            _ => {}
        }
    }
}

fn is_super_call(s: Stmt<'_>) -> bool {
    matches!(s.kind(), StmtKind::Call(e) if matches!(e.kind(), ExprKind::Call { callee, .. } if matches!(callee.kind(), ExprKind::Name(n) if n.text() == "super")))
}

/// `super(...)` calls below the top level of a constructor body.
fn nested_super_calls(body: Block<'_>) -> Vec<Span> {
    let mut out = Vec::new();
    for s in body.stmts() {
        let top = is_super_call(s);
        wowlua_syntax::visit::walk_stmt(s, &mut |e| {
            if let ExprKind::Call { callee, .. } = e.kind()
                && matches!(callee.kind(), ExprKind::Name(n) if n.text() == "super")
                && !(top && matches!(s.kind(), StmtKind::Call(x) if x.span() == e.span()))
            {
                out.push(e.span());
            }
        });
    }
    out
}

/// The first use of `self` in a statement.
fn self_use(s: Stmt<'_>) -> Option<Span> {
    let mut found = None;
    wowlua_syntax::visit::walk_stmt(s, &mut |e| {
        if found.is_none()
            && let ExprKind::Name(n) = e.kind()
            && n.text() == "self"
        {
            found = Some(n.span());
        }
    });
    found
}

/// Locals and parameters named `super` (E3040).
fn check_super_names(body: Block<'_>, func: Func<'_>, out: &mut Vec<Diagnostic>) {
    let mut names: Vec<Span> = func
        .params()
        .iter()
        .filter(|p| p.name().text() == "super")
        .map(|p| p.name().span())
        .collect();
    wowlua_syntax::visit::walk_stmts(body, &mut |s| match s.kind() {
        StmtKind::Local { names: n, .. } => names.extend(
            n.iter()
                .filter(|x| x.name.text() == "super")
                .map(|x| x.name.span()),
        ),
        StmtKind::LocalFunction { name, .. } if name.text() == "super" => names.push(name.span()),
        _ => {}
    });
    for span in names {
        out.push(
            Diagnostic::error(
                "E3040",
                "`super` cannot be a local name inside a class body",
            )
            .with_label(span, "here"),
        );
    }
}

/// `...` in an initializer, outside nested functions.
fn vararg_in(e: Expr<'_>) -> Option<Span> {
    let mut found = None;
    wowlua_syntax::visit::walk_expr_shallow(e, &mut |x| {
        if found.is_none() && matches!(x.kind(), ExprKind::Vararg) {
            found = Some(x.span());
        }
    });
    found
}

/// Resolves what each class extends and which classes are widget classes.
#[allow(clippy::too_many_arguments)]
pub fn resolve_base(
    path: &[String],
    file: usize,
    local_classes: &HashSet<String>,
    locals: &HashSet<String>,
    named_imports: &HashMap<String, (usize, String)>,
    namespace_imports: &HashMap<String, usize>,
    class_files: &dyn Fn(usize, &str) -> bool,
    widgets: &HashMap<String, WidgetDecl>,
) -> Base {
    match path {
        [name] => {
            if local_classes.contains(name) {
                return Base::Class {
                    file,
                    name: name.clone(),
                };
            }
            if let Some((m, export)) = named_imports.get(name) {
                return if class_files(*m, export) {
                    Base::Class {
                        file: *m,
                        name: export.clone(),
                    }
                } else {
                    Base::Unknown
                };
            }
            if locals.contains(name) || namespace_imports.contains_key(name) {
                return Base::Unknown;
            }
            match WidgetDecl::resolve(name, widgets) {
                Some(widget) => Base::Widget {
                    name: name.clone(),
                    widget,
                },
                None => Base::Unknown,
            }
        }
        [ns, name] => match namespace_imports.get(ns) {
            Some(&m) if class_files(m, name) => Base::Class {
                file: m,
                name: name.clone(),
            },
            _ => Base::Unknown,
        },
        _ => Base::Unknown,
    }
}
