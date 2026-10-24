//! A file's outline (`textDocument/documentSymbol`): its top-level
//! functions, locals, constants, classes (with their methods), interfaces,
//! enums and type aliases.

use crate::text;
use serde_json::{Value as Json, json};
use wowlua_span::{SourceDb, Span};
use wowlua_syntax::view::{Block, ClassMemberKind, StmtKind};

// The protocol's symbol kinds.
const CLASS: u32 = 5;
const METHOD: u32 = 6;
const FIELD: u32 = 8;
const ENUM: u32 = 10;
const INTERFACE: u32 = 11;
const FUNCTION: u32 = 12;
const VARIABLE: u32 = 13;
const CONSTANT: u32 = 14;
const TYPE_PARAMETER: u32 = 26;

fn symbol(src: &str, name: &str, kind: u32, whole: Span, at: Span, children: Vec<Json>) -> Json {
    let w = whole.range();
    let a = at.range();
    json!({
        "name": name,
        "kind": kind,
        "range": text::range(src, w.start, w.end),
        "selectionRange": text::range(src, a.start, a.end),
        "children": children,
    })
}

/// The outline of `src`.
pub fn symbols(src: &str) -> Vec<Json> {
    let mut db = SourceDb::new();
    let file = db.add("outline", String::new());
    let parse = wowlua_syntax::parse(src.as_bytes(), file);
    block(src, parse.view().block())
}

fn block(src: &str, b: Block<'_>) -> Vec<Json> {
    let mut out = Vec::new();
    for s in b.stmts() {
        stmt(src, s.kind(), s.span(), &mut out);
    }
    out
}

fn stmt(src: &str, kind: StmtKind<'_>, span: Span, out: &mut Vec<Json>) {
    match kind {
        StmtKind::Function { name, .. } => {
            let path: Vec<&str> = name.path().iter().map(|n| n.text()).collect();
            let joined = match name.method() {
                Some(m) => format!("{}:{}", path.join("."), m.text()),
                None => path.join("."),
            };
            let kind = if name.method().is_some() {
                METHOD
            } else {
                FUNCTION
            };
            out.push(symbol(src, &joined, kind, span, name.span(), Vec::new()));
        }
        StmtKind::LocalFunction { name, .. } => {
            out.push(symbol(
                src,
                name.text(),
                FUNCTION,
                span,
                name.span(),
                Vec::new(),
            ));
        }
        StmtKind::Local { names, .. } => {
            for n in names.iter() {
                out.push(symbol(
                    src,
                    n.name.text(),
                    VARIABLE,
                    span,
                    n.name.span(),
                    Vec::new(),
                ));
            }
        }
        StmtKind::Const { name, .. } => {
            out.push(symbol(
                src,
                name.text(),
                CONSTANT,
                span,
                name.span(),
                Vec::new(),
            ));
        }
        StmtKind::Class(c) => {
            let mut children = Vec::new();
            for m in c.members().iter() {
                match m.kind() {
                    ClassMemberKind::Method { name, .. } => {
                        children.push(symbol(
                            src,
                            name.text(),
                            METHOD,
                            m.span(),
                            name.span(),
                            Vec::new(),
                        ));
                    }
                    ClassMemberKind::Field { name, .. } => {
                        children.push(symbol(
                            src,
                            name.text(),
                            FIELD,
                            m.span(),
                            name.span(),
                            Vec::new(),
                        ));
                    }
                    _ => {}
                }
            }
            let n = c.name();
            out.push(symbol(src, n.text(), CLASS, span, n.span(), children));
        }
        StmtKind::Interface(i) => {
            let n = i.name();
            out.push(symbol(src, n.text(), INTERFACE, span, n.span(), Vec::new()));
        }
        StmtKind::Enum(e) => {
            let n = e.name();
            out.push(symbol(src, n.text(), ENUM, span, n.span(), Vec::new()));
        }
        StmtKind::TypeAlias(t) => {
            let n = t.name();
            out.push(symbol(
                src,
                n.text(),
                TYPE_PARAMETER,
                span,
                n.span(),
                Vec::new(),
            ));
        }
        StmtKind::Export(inner) => stmt(src, inner.kind(), span, out),
        _ => {}
    }
}
