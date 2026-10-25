//! The declarations the generator collects, and their `.d.wowlua` text.
//! Types are kept as wowlua type syntax.

use indexmap::IndexMap;
use std::collections::BTreeSet;
use std::fmt::Write;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sig {
    pub type_params: Vec<String>,
    pub params: Vec<ParamDecl>,
    /// `...: T`
    pub vararg: Option<String>,
    pub returns: Vec<String>,
    /// `...T` after the fixed results.
    pub ret_vararg: Option<String>,
    pub deprecated: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub ty: String,
    pub optional: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Namespace {
    pub functions: IndexMap<String, Vec<Sig>>,
    pub values: IndexMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Class {
    pub bases: Vec<String>,
    /// Further parents (LuaLS `@class A : B, C, D`: C and D), whose
    /// members the declarations copy in; the engine inherits from them.
    pub also: Vec<String>,
    pub methods: IndexMap<String, Vec<Sig>>,
    pub statics: IndexMap<String, Vec<Sig>>,
    /// name → (type, optional)
    pub fields: IndexMap<String, (String, bool)>,
    /// `@widget("…")`: the `CreateFrame` type.
    pub widget: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Decls {
    pub functions: IndexMap<String, Vec<Sig>>,
    pub namespaces: IndexMap<String, Namespace>,
    pub classes: IndexMap<String, Class>,
    /// `type Name = T`
    pub aliases: IndexMap<String, String>,
    /// Structures: name → fields (name, type, optional).
    pub interfaces: IndexMap<String, Vec<(String, String, bool)>>,
    /// `Enum.X` constants.
    pub enums: IndexMap<String, Vec<(String, f64)>>,
    /// `Constants.X` values.
    pub constants: IndexMap<String, Vec<(String, String)>>,
    pub events: IndexMap<String, Vec<ParamDecl>>,
    pub globals: IndexMap<String, String>,
    /// Globals only a load-on-demand Blizzard addon defines: name → addon.
    pub global_addons: IndexMap<String, String>,
    /// What the functions do, written as their doc comments
    /// (`crate::docs`).
    pub docs: crate::docs::Docs,
}

/// The doc comment of the function `key` names, if documented.
fn doc_comment(out: &mut String, docs: &crate::docs::Docs, key: &str, indent: &str) {
    let Some(doc) = docs.get(key) else {
        return;
    };
    for line in crate::docs::lines(doc) {
        if line.is_empty() {
            let _ = writeln!(out, "{indent}---");
        } else {
            let _ = writeln!(out, "{indent}--- {line}");
        }
    }
}

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// A name usable as an identifier.
pub fn ident(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if LUA_KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

pub fn is_ident(name: &str) -> bool {
    ident(name) == name
}

impl Decls {
    /// The declarations as a `.d.wowlua` file.
    pub fn emit(&self, header: &str) -> String {
        let mut out = String::new();
        for line in header.lines() {
            let _ = writeln!(out, "-- {line}");
        }
        out.push('\n');
        for (name, ty) in &self.aliases {
            let _ = writeln!(out, "type {name} = {ty}");
        }
        if !self.aliases.is_empty() {
            out.push('\n');
        }
        for (name, fields) in &self.interfaces {
            let _ = writeln!(out, "declare interface {name}");
            for (f, ty, optional) in fields {
                let _ = writeln!(
                    out,
                    "    {}{}: {ty}",
                    field_name(f),
                    if *optional { "?" } else { "" }
                );
            }
            out.push_str("end\n\n");
        }
        for (name, class) in &self.classes {
            if let Some(w) = &class.widget {
                let _ = writeln!(out, "@widget(\"{w}\")");
            }
            let _ = write!(out, "declare class {name}");
            if let Some(b) = class.bases.first() {
                let _ = write!(out, " extends {b}");
            }
            out.push('\n');
            for (f, (ty, optional)) in &class.fields {
                let _ = writeln!(
                    out,
                    "    {}{}: {ty}",
                    field_name(f),
                    if *optional { "?" } else { "" }
                );
            }
            for (m, sigs) in &class.methods {
                doc_comment(&mut out, &self.docs, &self.method_key(name, m), "    ");
                for sig in sigs {
                    let _ = writeln!(
                        out,
                        "    {}function {}{}",
                        deprecated(sig),
                        ident(m),
                        sig_text(sig)
                    );
                }
            }
            for (m, sigs) in &class.statics {
                doc_comment(&mut out, &self.docs, &self.method_key(name, m), "    ");
                for sig in sigs {
                    let _ = writeln!(
                        out,
                        "    {}static function {}{}",
                        deprecated(sig),
                        ident(m),
                        sig_text(sig)
                    );
                }
            }
            out.push_str("end\n\n");
        }
        for (name, sigs) in &self.functions {
            doc_comment(&mut out, &self.docs, name, "");
            for sig in sigs {
                let _ = writeln!(
                    out,
                    "{}declare function {}{}",
                    deprecated(sig),
                    ident(name),
                    sig_text(sig)
                );
            }
        }
        if !self.functions.is_empty() {
            out.push('\n');
        }
        for (name, ty) in &self.globals {
            if let Some(addon) = self.global_addons.get(name) {
                let _ = writeln!(out, "@onDemand(\"{addon}\")");
            }
            let _ = writeln!(out, "declare global {name}: {ty}");
        }
        if !self.globals.is_empty() {
            out.push('\n');
        }
        for (name, ns) in &self.namespaces {
            let _ = writeln!(out, "declare namespace {name}");
            for (f, sigs) in &ns.functions {
                doc_comment(&mut out, &self.docs, &format!("{name}.{f}"), "    ");
                for sig in sigs {
                    let _ = writeln!(
                        out,
                        "    {}function {}{}",
                        deprecated(sig),
                        ident(f),
                        sig_text(sig)
                    );
                }
            }
            for (v, ty) in &ns.values {
                let _ = writeln!(out, "    {}: {ty}", ident(v));
            }
            out.push_str("end\n\n");
        }
        if !self.enums.is_empty() {
            out.push_str("declare namespace Enum\n");
            for (name, members) in &self.enums {
                let _ = writeln!(out, "    namespace {}", ident(name));
                for (m, v) in members {
                    // The type grammar has no negative literals.
                    let ty = if *v < 0.0 {
                        "number".to_string()
                    } else {
                        number(*v)
                    };
                    let _ = writeln!(out, "        const {}: {ty}", ident(m));
                }
                out.push_str("    end\n");
            }
            out.push_str("end\n\n");
        }
        if !self.constants.is_empty() {
            out.push_str("declare namespace Constants\n");
            for (name, members) in &self.constants {
                let _ = writeln!(out, "    namespace {}", ident(name));
                for (m, ty) in members {
                    let _ = writeln!(out, "        const {}: {ty}", ident(m));
                }
                out.push_str("    end\n");
            }
            out.push_str("end\n\n");
        }
        for (name, params) in &self.events {
            let list: Vec<String> = params
                .iter()
                .map(|p| {
                    format!(
                        "{}: {}{}",
                        ident(&p.name),
                        p.ty,
                        if p.optional && !p.ty.ends_with('?') {
                            "?"
                        } else {
                            ""
                        }
                    )
                })
                .collect();
            let _ = writeln!(out, "declare event {name}({})", list.join(", "));
        }
        out
    }

    /// The documentation key of method `m` of `class`: its own, else the
    /// first parent's (bases, then the classes it copies members from)
    /// that documents it (`Region:SetPoint` is `ScriptRegionResizing`'s).
    fn method_key(&self, class: &str, m: &str) -> String {
        let own = format!("{class}:{m}");
        if self.docs.contains_key(&own) {
            return own;
        }
        let mut seen = BTreeSet::new();
        let mut todo = vec![class.to_string()];
        while let Some(c) = todo.pop() {
            if !seen.insert(c.clone()) {
                continue;
            }
            let key = format!("{c}:{m}");
            if self.docs.contains_key(&key) {
                return key;
            }
            if let Some(def) = self.classes.get(&c) {
                // (nearest first: the base, then the others in order)
                for p in def.also.iter().rev().chain(def.bases.iter().rev()) {
                    let p = p.split('<').next().unwrap_or(p).trim().to_string();
                    todo.push(p);
                }
            }
        }
        own
    }

    /// Every type name the declarations refer to (first path segment).
    pub fn referenced_types(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut add = |ty: &str| collect_type_names(ty, &mut out);
        let sigs = |sigs: &Vec<Sig>, add: &mut dyn FnMut(&str)| {
            for s in sigs {
                // Type parameters are not references.
                let mut local = BTreeSet::new();
                for p in &s.params {
                    collect_type_names(&p.ty, &mut local);
                }
                for r in s
                    .returns
                    .iter()
                    .chain(s.vararg.iter())
                    .chain(s.ret_vararg.iter())
                {
                    collect_type_names(r, &mut local);
                }
                for name in local {
                    if !s.type_params.contains(&name) {
                        add(&name);
                    }
                }
            }
        };
        for s in self.functions.values() {
            sigs(s, &mut add);
        }
        for ns in self.namespaces.values() {
            for s in ns.functions.values() {
                sigs(s, &mut add);
            }
            for t in ns.values.values() {
                add(t);
            }
        }
        for c in self.classes.values() {
            for b in &c.bases {
                add(b);
            }
            for s in c.methods.values().chain(c.statics.values()) {
                sigs(s, &mut add);
            }
            for (t, _) in c.fields.values() {
                add(t);
            }
        }
        for t in self.aliases.values() {
            add(t);
        }
        for fields in self.interfaces.values() {
            for (_, t, _) in fields {
                add(t);
            }
        }
        for params in self.events.values() {
            for p in params {
                add(&p.ty);
            }
        }
        for t in self.globals.values() {
            add(t);
        }
        out
    }

    /// Every type name the declarations define.
    pub fn defined_types(&self) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        out.extend(self.aliases.keys().cloned());
        out.extend(self.interfaces.keys().cloned());
        out.extend(self.classes.keys().cloned());
        out
    }
}

fn deprecated(sig: &Sig) -> &'static str {
    if sig.deprecated { "@deprecated " } else { "" }
}

fn field_name(name: &str) -> String {
    if is_ident(name) {
        name.to_string()
    } else {
        format!("[\"{}\"]", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// `<T>(a: number, b: string?): (boolean, ...string)`
pub fn sig_text(sig: &Sig) -> String {
    let mut out = String::new();
    if !sig.type_params.is_empty() {
        let _ = write!(out, "<{}>", sig.type_params.join(", "));
    }
    let mut params: Vec<String> = sig
        .params
        .iter()
        .map(|p| {
            let ty = if p.optional && !p.ty.ends_with('?') {
                optional(&p.ty)
            } else {
                p.ty.clone()
            };
            format!("{}: {ty}", ident(&p.name))
        })
        .collect();
    if let Some(v) = &sig.vararg {
        params.push(format!("...: {v}"));
    }
    let _ = write!(out, "({})", params.join(", "));
    match (sig.returns.len(), &sig.ret_vararg) {
        (0, None) => {}
        (1, None) => {
            let _ = write!(out, ": {}", sig.returns[0]);
        }
        (_, v) => {
            let mut rs = sig.returns.clone();
            if let Some(v) = v {
                rs.push(format!("...{v}"));
            }
            let _ = write!(out, ": ({})", rs.join(", "));
        }
    }
    out
}

/// `any` in a written type as `unknown` (quoted literals kept).
pub fn unknown_for_any(ty: &str) -> String {
    let mut out = String::with_capacity(ty.len());
    let mut word = String::new();
    let mut quoted = false;
    let flush = |word: &mut String, out: &mut String| {
        out.push_str(if word == "any" { "unknown" } else { word });
        word.clear();
    };
    for c in ty.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if !quoted && (c.is_ascii_alphanumeric() || c == '_') {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

impl Sig {
    fn without_any(&mut self) {
        for p in &mut self.params {
            p.ty = unknown_for_any(&p.ty);
        }
        self.vararg = self.vararg.as_deref().map(unknown_for_any);
        self.returns = self.returns.iter().map(|r| unknown_for_any(r)).collect();
        self.ret_vararg = self.ret_vararg.as_deref().map(unknown_for_any);
    }
}

impl Decls {
    /// Every `any` the declarations write as `unknown` (decision T288):
    /// values nothing types say so, and a caller checks them before use.
    pub fn without_any(&mut self) {
        let sigs = |m: &mut IndexMap<String, Vec<Sig>>| {
            m.values_mut().flatten().for_each(Sig::without_any);
        };
        sigs(&mut self.functions);
        for ns in self.namespaces.values_mut() {
            sigs(&mut ns.functions);
            for v in ns.values.values_mut() {
                *v = unknown_for_any(v);
            }
        }
        for c in self.classes.values_mut() {
            sigs(&mut c.methods);
            sigs(&mut c.statics);
            for (t, _) in c.fields.values_mut() {
                *t = unknown_for_any(t);
            }
        }
        for t in self.aliases.values_mut() {
            // names or any other string (`FrameType = "Frame" | … | any`)
            *t = if t.contains('"') && t.split('|').any(|m| m.trim() == "any") {
                t.split('|')
                    .map(|m| if m.trim() == "any" { " string" } else { m })
                    .collect::<Vec<_>>()
                    .join("|")
            } else {
                unknown_for_any(t)
            };
        }
        for fields in self.interfaces.values_mut() {
            for (_, t, _) in fields.iter_mut() {
                *t = unknown_for_any(t);
            }
        }
        for params in self.events.values_mut() {
            for p in params.iter_mut() {
                p.ty = unknown_for_any(&p.ty);
            }
        }
        for t in self.globals.values_mut() {
            *t = unknown_for_any(t);
        }
    }
}

/// A signature as a function type: `(a: T, b: U?) -> R`.
///
/// ```
/// use wowlua_types_gen::model::{fn_type, ParamDecl, Sig};
/// let sig = Sig {
///     params: vec![ParamDecl { name: "f".into(), ty: "(t: T) -> ()".into(), optional: true }],
///     ..Sig::default()
/// };
/// assert_eq!(fn_type(&sig), "(f: ((t: T) -> ())?) -> ()");
/// ```
pub fn fn_type(sig: &Sig) -> String {
    let text = sig_text(sig);
    // `(params): results` → `(params) -> results`
    let close = {
        let mut depth = 0i32;
        let mut at = text.len();
        let mut prev = ' ';
        for (i, c) in text.char_indices() {
            match c {
                '(' | '<' | '{' => depth += 1,
                // (the `>` of `->` closes nothing)
                '>' if prev == '-' => {}
                ')' | '>' | '}' => {
                    depth -= 1;
                    if depth == 0 && c == ')' {
                        at = i;
                        break;
                    }
                }
                _ => {}
            }
            prev = c;
        }
        at
    };
    let (params, rest) = text.split_at((close + 1).min(text.len()));
    match rest.strip_prefix(": ") {
        Some(results) => format!("{params} -> {results}"),
        None => format!("{params} -> ()"),
    }
}

/// `T?`, parenthesized where needed.
pub fn optional(ty: &str) -> String {
    if ty.contains('|') || ty.contains("->") || ty.contains('&') {
        format!("({ty})?")
    } else {
        format!("{ty}?")
    }
}

/// Type names (first path segment) in wowlua type text.
pub fn collect_type_names(ty: &str, out: &mut BTreeSet<String>) {
    let bytes = ty.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' {
            // Skip string literals.
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            let word = &ty[start..i];
            // A parameter name before `:` is not a type.
            let rest = ty[i..].trim_start();
            let is_param_name = rest.starts_with(':') && !rest.starts_with("::");
            let first = word.split('.').next().unwrap_or(word);
            if !is_param_name && !BUILTIN.contains(&first) {
                out.insert(word.to_string());
            }
            continue;
        }
        if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
}

const BUILTIN: &[&str] = &[
    "any",
    "unknown",
    "never",
    "nil",
    "boolean",
    "number",
    "string",
    "table",
    "function",
    "thread",
    "userdata",
    "true",
    "false",
    "Self",
    "Secret",
    "ClassOf",
    "EventName",
    "MessageName",
    "EventArgs",
    "Thread",
    "AddonNamespace",
    "is",
];
