//! LuaLS annotations (`---@class`, `---@param`, …) → declarations (spec
//! package manager §10.9, `wowlua types convert`). Supported tags: `@meta`,
//! `@class` (several parents: the first is the superclass, the others'
//! members are copied in), `@field`, `@param`, `@return`, `@overload`,
//! `@alias` (with `---|` literal unions), `@enum`, `@generic`, `@type`,
//! `@deprecated`. Other tags are ignored.

use crate::model::{Class, Decls, ParamDecl, Sig, optional};
use std::collections::HashMap;

/// Converts the annotated files (path, source) into declarations.
pub fn convert(files: &[(String, String)]) -> Decls {
    let mut c = Converter::default();
    for (_, source) in files {
        c.file(source);
    }
    c.finish()
}

#[derive(Default)]
struct Converter {
    out: Decls,
    /// Secondary parents of classes, applied at the end.
    mixins: Vec<(String, Vec<String>)>,
}

#[derive(Default, Debug)]
struct Block {
    class: Option<(String, Vec<String>)>,
    fields: Vec<(String, String, bool)>,
    params: Vec<ParamDecl>,
    vararg: Option<String>,
    returns: Vec<String>,
    overloads: Vec<String>,
    generics: Vec<String>,
    alias: Option<(String, Vec<String>)>,
    ty: Option<String>,
    deprecated: bool,
    enum_name: Option<String>,
    /// `---#nopage`: Ketho's mark for a function the wiki has no page for.
    nopage: bool,
}

impl Converter {
    fn file(&mut self, source: &str) {
        let mut block = Block::default();
        // Local names of class tables: `local Frame = {}` after `@class Frame`.
        let mut locals: HashMap<String, String> = HashMap::new();
        for raw in source.lines() {
            let line = raw.trim();
            if let Some(tag) = line.strip_prefix("---") {
                let tag = tag.trim_start();
                if let Some(rest) = tag.strip_prefix('|') {
                    // An alias member: `---| "value" # comment`.
                    if let Some((_, members)) = block.alias.as_mut() {
                        let value = rest.trim().split(" #").next().unwrap_or("").trim();
                        let value = value.strip_prefix('>').unwrap_or(value).trim();
                        if !value.is_empty() {
                            members.push(convert_type(value));
                        }
                    }
                    continue;
                }
                if let Some(t) = tag.strip_prefix('@') {
                    self.tag(t, &mut block);
                } else if tag.trim_end() == "#nopage" {
                    block.nopage = true;
                }
                continue;
            }
            if line.starts_with("--") || line.is_empty() {
                // A blank or comment line ends a pending alias.
                if line.is_empty() {
                    self.flush_alias(&mut block);
                }
                continue;
            }
            // The annotated code: aliases end here, a class applies to it.
            self.flush_only_alias(&mut block);
            self.code(line, &mut block, &mut locals);
            block = Block::default();
        }
        self.flush_alias(&mut block);
        if let Some((name, parents)) = block.class.take() {
            self.declare_class(&name, parents, std::mem::take(&mut block.fields));
        }
    }

    fn flush_alias(&mut self, block: &mut Block) {
        self.flush_only_alias(block);
        if let Some((name, parents)) = block.class.take() {
            self.declare_class(&name, parents, std::mem::take(&mut block.fields));
        }
    }

    fn flush_only_alias(&mut self, block: &mut Block) {
        if let Some((name, members)) = block.alias.take() {
            if members.is_empty() {
                return;
            }
            let ty = collapse(&members);
            if crate::model::is_ident(&name) {
                self.out.aliases.entry(name).or_insert(ty);
            }
        }
    }

    fn tag(&mut self, t: &str, block: &mut Block) {
        let (name, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
        let rest = rest.trim();
        match name {
            "class" => {
                let rest = rest
                    .strip_prefix("(exact)")
                    .or_else(|| rest.strip_prefix("(partial)"))
                    .unwrap_or(rest)
                    .trim();
                let (cname, parents) = match rest.split_once(':') {
                    Some((n, p)) => (
                        n.trim().to_string(),
                        p.split(',')
                            .map(|x| x.trim().to_string())
                            .filter(|x| !x.is_empty())
                            .collect(),
                    ),
                    None => (
                        rest.split_whitespace().next().unwrap_or("").to_string(),
                        Vec::new(),
                    ),
                };
                // A new class in the same block: declare the previous one.
                if let Some((n, p)) = block.class.take() {
                    self.declare_class(&n, p, std::mem::take(&mut block.fields));
                }
                block.class = Some((cname, parents));
            }
            "field" => {
                let rest = strip_visibility(rest);
                let (fname, tail) = rest
                    .split_once(char::is_whitespace)
                    .unwrap_or((rest, "any"));
                if fname.starts_with('[') {
                    return;
                }
                let optional = fname.ends_with('?');
                let fname = fname.trim_end_matches('?');
                let ty = convert_type(type_part(tail));
                block.fields.push((fname.to_string(), ty, optional));
            }
            "param" => {
                let (pname, tail) = rest
                    .split_once(char::is_whitespace)
                    .unwrap_or((rest, "any"));
                let ty = convert_type(type_part(tail));
                if pname == "..." {
                    block.vararg = Some(ty);
                    return;
                }
                let optional = pname.ends_with('?');
                block.params.push(ParamDecl {
                    name: pname.trim_end_matches('?').to_string(),
                    ty,
                    optional,
                });
            }
            "return" => {
                let ty = type_part(rest);
                if !ty.is_empty() {
                    block.returns.push(convert_type(ty));
                }
            }
            "overload" => block.overloads.push(rest.to_string()),
            "generic" => {
                for g in rest.split(',') {
                    let g = g.split(':').next().unwrap_or("").trim();
                    if !g.is_empty() {
                        block.generics.push(g.to_string());
                    }
                }
            }
            "alias" => {
                // Consecutive one-line aliases: declare the previous one.
                self.flush_only_alias(block);
                let (aname, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let tail = type_part(tail.trim());
                let members = if tail.is_empty() {
                    Vec::new()
                } else {
                    vec![convert_type(tail)]
                };
                block.alias = Some((aname.to_string(), members));
            }
            "type" => block.ty = Some(convert_type(type_part(rest))),
            "deprecated" => block.deprecated = true,
            "enum" => {
                block.enum_name = Some(rest.split_whitespace().next().unwrap_or("").to_string())
            }
            _ => {}
        }
    }

    fn declare_class(
        &mut self,
        name: &str,
        mut parents: Vec<String>,
        fields: Vec<(String, String, bool)>,
    ) {
        // Dotted names (`TextureInfoTable.margin`) are referred to as `any`.
        if name.is_empty() || !crate::model::is_ident(name) {
            return;
        }
        // `ItemLocation : ItemLocationData, ItemLocationMixin`: the mixin is
        // the base (what the API's functions take), the data's fields are
        // copied in (decision T284).
        if parents.len() > 1
            && parents[0].ends_with("Data")
            && let Some(i) = parents.iter().position(|p| p.ends_with("Mixin"))
        {
            let mixin = parents.remove(i);
            parents.insert(0, mixin);
        }
        let entry = self.out.classes.entry(name.to_string()).or_default();
        if entry.bases.is_empty() {
            entry.bases = parents.iter().take(1).cloned().collect();
        }
        for (f, ty, optional) in fields {
            entry.fields.entry(f).or_insert((ty, optional));
        }
        if parents.len() > 1 {
            if entry.also.is_empty() {
                entry.also = parents[1..].to_vec();
            }
            self.mixins.push((name.to_string(), parents[1..].to_vec()));
        }
    }

    fn sig(&self, block: &Block, params: &[String], method: bool) -> Sig {
        // Parameters in the order of the function definition.
        let mut out = Vec::new();
        let mut vararg = block.vararg.clone();
        for p in params {
            if p == "..." {
                vararg.get_or_insert_with(|| "any".into());
                continue;
            }
            if method && p == "self" {
                continue;
            }
            let found = block.params.iter().find(|x| &x.name == p).cloned();
            out.push(found.unwrap_or(ParamDecl {
                name: p.clone(),
                ty: "any".into(),
                optional: true,
            }));
        }
        let (returns, ret_vararg) = split_returns(&block.returns);
        // A function without a page and without annotations has an unknown
        // signature, not an empty one: Blizzard's code calls
        // `SelectActiveQuest(self:GetID())`, and
        // `GetQuestLogCompletionText(2)` answers a string (the full probe).
        if block.nopage && block.params.is_empty() && block.returns.is_empty() {
            return Sig {
                params: out,
                vararg: Some(vararg.unwrap_or_else(|| "any".into())),
                ret_vararg: Some("any".into()),
                deprecated: block.deprecated,
                ..Sig::default()
            };
        }
        Sig {
            type_params: block.generics.clone(),
            params: out,
            vararg,
            returns,
            ret_vararg,
            deprecated: block.deprecated,
        }
    }

    fn code(&mut self, line: &str, block: &mut Block, locals: &mut HashMap<String, String>) {
        let class = block.class.take();
        if let Some(rest) = line.strip_prefix("function ") {
            let Some((name, params)) = rest.split_once('(') else {
                return;
            };
            let params: Vec<String> = params
                .split(')')
                .next()
                .unwrap_or("")
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            let name = name.trim();
            let method = name.contains(':');
            let mut sigs = vec![self.sig(block, &params, method)];
            for o in &block.overloads {
                if let Some(mut sig) = overload_sig(o, block.deprecated) {
                    // Method overloads name `self` first.
                    if method && sig.params.first().is_some_and(|p| p.name == "self") {
                        sig.params.remove(0);
                    }
                    sigs.push(sig);
                }
            }
            if let Some((table, member)) = name.split_once(':') {
                let class = locals
                    .get(table)
                    .cloned()
                    .unwrap_or_else(|| table.to_string());
                let c = self.out.classes.entry(class).or_default();
                c.methods
                    .entry(member.to_string())
                    .or_default()
                    .extend(sigs);
            } else if let Some((table, member)) = name.rsplit_once('.') {
                if let Some(class) = locals.get(table).cloned() {
                    let c = self.out.classes.entry(class).or_default();
                    c.statics
                        .entry(member.to_string())
                        .or_default()
                        .extend(sigs);
                } else if self.out.classes.contains_key(table) {
                    let c = self.out.classes.entry(table.to_string()).or_default();
                    c.statics
                        .entry(member.to_string())
                        .or_default()
                        .extend(sigs);
                } else {
                    let ns = self.out.namespaces.entry(table.to_string()).or_default();
                    ns.functions
                        .entry(member.to_string())
                        .or_default()
                        .extend(sigs);
                }
            } else {
                self.out
                    .functions
                    .entry(name.to_string())
                    .or_default()
                    .extend(sigs);
            }
            if let Some((n, p)) = class {
                self.declare_class(&n, p, std::mem::take(&mut block.fields));
            }
            return;
        }
        // `local X = {}` / `X = {}` / `X = Y`
        let assignment = line.strip_prefix("local ").unwrap_or(line);
        if let Some((lhs, rhs)) = assignment.split_once('=') {
            let lhs = lhs.trim();
            let rhs = rhs.trim();
            if let Some((n, p)) = class {
                locals.insert(lhs.to_string(), n.clone());
                self.declare_class(&n, p, std::mem::take(&mut block.fields));
                // A global class table (`UIParent = {}`) is a global of
                // that class.
                if !line.starts_with("local ") && is_global_name(lhs) && !lhs.contains('.') {
                    self.out.globals.entry(lhs.to_string()).or_insert(n);
                }
                return;
            }
            if let Some(ty) = &block.ty {
                if !line.starts_with("local ") && is_global_name(lhs) {
                    self.out
                        .globals
                        .entry(lhs.to_string())
                        .or_insert(ty.clone());
                }
                return;
            }
            if !line.starts_with("local ")
                && rhs.starts_with('{')
                && is_global_name(lhs)
                && !lhs.contains('.')
            {
                self.out.namespaces.entry(lhs.to_string()).or_default();
            } else if !line.starts_with("local ") && is_global_name(lhs) && !lhs.contains('.') {
                // Unannotated globals (`GlobalColors`): colors, else `any`.
                let ty = if rhs.starts_with("CreateColor(") {
                    "ColorMixin"
                } else {
                    "any"
                };
                self.out
                    .globals
                    .entry(lhs.to_string())
                    .or_insert_with(|| ty.into());
            }
        } else if let Some((n, p)) = class {
            self.declare_class(&n, p, std::mem::take(&mut block.fields));
        }
    }

    fn finish(mut self) -> Decls {
        // Members of secondary parents are copied into the class.
        for (class, parents) in std::mem::take(&mut self.mixins) {
            for p in parents {
                let Some(parent) = self.out.classes.get(&p).cloned() else {
                    continue;
                };
                let target: &mut Class = self.out.classes.entry(class.clone()).or_default();
                for (m, sigs) in parent.methods {
                    target.methods.entry(m).or_insert(sigs);
                }
                for (f, t) in parent.fields {
                    target.fields.entry(f).or_insert(t);
                }
            }
        }
        // Namespaces that are also classes stay classes.
        let classes: Vec<String> = self.out.classes.keys().cloned().collect();
        for c in classes {
            if let Some(ns) = self.out.namespaces.shift_remove(&c) {
                let class = self.out.classes.get_mut(&c).expect("present");
                for (f, sigs) in ns.functions {
                    class.statics.entry(f).or_insert(sigs);
                }
            }
        }
        self.out
    }
}

/// A union's text; literals are dropped when their primitive is a member
/// (`string | "A" | "B"` is `string`).
fn collapse(parts: &[String]) -> String {
    let has_string = parts.iter().any(|p| p == "string");
    let has_number = parts.iter().any(|p| p == "number");
    let kept: Vec<&String> = parts
        .iter()
        .filter(|p| {
            let string_lit = p.starts_with('"');
            let number_lit = p.starts_with(|c: char| c.is_ascii_digit() || c == '-');
            !(string_lit && has_string || number_lit && has_number)
        })
        .collect();
    kept.iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn is_global_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !n.starts_with(|c: char| c.is_ascii_digit())
}

fn strip_visibility(s: &str) -> &str {
    for v in ["public ", "protected ", "private ", "package "] {
        if let Some(rest) = s.strip_prefix(v) {
            return rest.trim_start();
        }
    }
    s
}

/// The type at the start of a tag's text (up to the description).
fn type_part(s: &str) -> &str {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    let mut in_string: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_string {
            if c == q {
                in_string = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' | b'\'' | b'`' => in_string = Some(c),
            b'(' | b'<' | b'{' | b'[' => depth += 1,
            b')' | b'>' | b'}' | b']' => depth -= 1,
            b' ' | b'\t' if depth <= 0 => {
                // `fun(): number` and `A | B` continue after spaces.
                let rest = s[i..].trim_start();
                let prev = s[..i].trim_end();
                if rest.starts_with('|')
                    || prev.ends_with('|')
                    || prev.ends_with(':')
                    || prev.ends_with(',')
                    || rest.starts_with(':') && prev.ends_with(')')
                {
                    i += 1;
                    continue;
                }
                return &s[..i];
            }
            b'#' if depth <= 0 => return s[..i].trim_end(),
            _ => {}
        }
        i += 1;
    }
    s
}

fn split_returns(returns: &[String]) -> (Vec<String>, Option<String>) {
    let mut fixed = Vec::new();
    let mut rest = None;
    for r in returns {
        if let Some(v) = r.strip_prefix("...") {
            rest = Some(if v.is_empty() {
                "any".into()
            } else {
                v.to_string()
            });
        } else {
            fixed.push(r.clone());
        }
    }
    (fixed, rest)
}

/// `fun(a: number, b?: string): boolean` as an overload signature.
fn overload_sig(text: &str, deprecated: bool) -> Option<Sig> {
    let t = parse_type(text.trim())?;
    match t {
        LType::Fun(params, vararg, returns) => {
            let (returns, ret_vararg) = split_returns(&returns);
            Some(Sig {
                type_params: Vec::new(),
                params,
                vararg,
                returns,
                ret_vararg,
                deprecated,
            })
        }
        _ => None,
    }
}

/// A LuaLS type as wowlua type text (`any` when it cannot be read).
pub fn convert_type(text: &str) -> String {
    match parse_type(text.trim()) {
        Some(t) => render(&t),
        None => "any".into(),
    }
}

#[derive(Debug, Clone)]
enum LType {
    Name(String),
    Literal(String),
    Union(Vec<LType>),
    Optional(Box<LType>),
    Array(Box<LType>),
    Map(Box<LType>, Box<LType>),
    Fun(Vec<ParamDecl>, Option<String>, Vec<String>),
    Record(Vec<(String, String, bool)>),
}

fn render(t: &LType) -> String {
    match t {
        LType::Name(n) => n.clone(),
        LType::Literal(l) => l.clone(),
        LType::Union(ms) => {
            let mut parts: Vec<String> = Vec::new();
            for m in ms {
                let r = match m {
                    LType::Fun(..) => format!("({})", render(m)),
                    _ => render(m),
                };
                if !parts.contains(&r) {
                    parts.push(r);
                }
            }
            collapse(&parts)
        }
        LType::Optional(inner) => optional(&render(inner)),
        LType::Array(inner) => match **inner {
            LType::Union(_) | LType::Fun(..) | LType::Optional(_) => {
                format!("({})[]", render(inner))
            }
            _ => format!("{}[]", render(inner)),
        },
        LType::Map(k, v) => format!("{{ [{}]: {} }}", render(k), render(v)),
        LType::Fun(params, vararg, returns) => {
            let mut ps: Vec<String> = params
                .iter()
                .map(|p| {
                    let ty = if p.optional {
                        optional(&p.ty)
                    } else {
                        p.ty.clone()
                    };
                    format!("{}: {ty}", crate::model::ident(&p.name))
                })
                .collect();
            if let Some(v) = vararg {
                ps.push(format!("...{v}"));
            }
            let (fixed, rest) = split_returns(returns);
            let ret = match (fixed.len(), &rest) {
                (0, None) => "()".to_string(),
                (1, None) => {
                    let r = &fixed[0];
                    if r.contains("->") || r.contains('|') {
                        format!("({r})")
                    } else {
                        r.clone()
                    }
                }
                _ => {
                    let mut rs = fixed.clone();
                    if let Some(v) = rest {
                        rs.push(format!("...{v}"));
                    }
                    format!("({})", rs.join(", "))
                }
            };
            format!("({}) -> {ret}", ps.join(", "))
        }
        LType::Record(fields) => {
            let fs: Vec<String> = fields
                .iter()
                .map(|(n, t, o)| {
                    let key = if crate::model::is_ident(n) {
                        n.clone()
                    } else {
                        format!("[{n:?}]")
                    };
                    format!("{key}{}: {t}", if *o { "?" } else { "" })
                })
                .collect();
            format!("{{ {} }}", fs.join(", "))
        }
    }
}

fn parse_type(text: &str) -> Option<LType> {
    let tokens = tokenize(text)?;
    let mut p = Parser { tokens, pos: 0 };
    let t = p.union()?;
    (p.pos == p.tokens.len()).then_some(t)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Name(String),
    Str(String),
    Num(String),
    Sym(char),
    Ellipsis,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'"' || c == b'\'' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != c {
                j += 1;
            }
            if j >= b.len() {
                return None;
            }
            out.push(Tok::Str(s[start..j].to_string()));
            i = j + 1;
        } else if c == b'`' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != b'`' {
                j += 1;
            }
            out.push(Tok::Name(s[start..j.min(b.len())].to_string()));
            i = j + 1;
        } else if c == b'.' && s[i..].starts_with("...") {
            out.push(Tok::Ellipsis);
            i += 3;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.' || b[i] == b'-')
            {
                // `-` only inside names like `Lua-API`? stop at `->`.
                if b[i] == b'-' {
                    break;
                }
                i += 1;
            }
            out.push(Tok::Name(s[start..i].trim_end_matches('.').to_string()));
        } else if c.is_ascii_digit()
            || (c == b'-' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            i += 1;
            while i < b.len()
                && (b[i].is_ascii_digit()
                    || b[i] == b'.'
                    || b[i] == b'x'
                    || b[i].is_ascii_hexdigit())
            {
                i += 1;
            }
            out.push(Tok::Num(s[start..i].to_string()));
        } else {
            out.push(Tok::Sym(c as char));
            i += 1;
        }
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Sym(c)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn union(&mut self) -> Option<LType> {
        let mut members = vec![self.postfix()?];
        while self.eat('|') {
            members.push(self.postfix()?);
        }
        Some(if members.len() == 1 {
            members.pop().expect("one")
        } else {
            LType::Union(members)
        })
    }

    fn postfix(&mut self) -> Option<LType> {
        let mut t = self.primary()?;
        loop {
            if self.eat('?') {
                t = LType::Optional(Box::new(t));
            } else if self.peek() == Some(&Tok::Sym('['))
                && self.tokens.get(self.pos + 1) == Some(&Tok::Sym(']'))
            {
                self.pos += 2;
                t = LType::Array(Box::new(t));
            } else {
                return Some(t);
            }
        }
    }

    fn primary(&mut self) -> Option<LType> {
        match self.peek()?.clone() {
            Tok::Str(s) => {
                self.pos += 1;
                Some(LType::Literal(format!("{s:?}")))
            }
            Tok::Num(n) => {
                self.pos += 1;
                Some(LType::Literal(n))
            }
            Tok::Sym('(') => {
                self.pos += 1;
                let t = self.union()?;
                self.eat(')').then_some(t)
            }
            Tok::Sym('{') => {
                self.pos += 1;
                let mut fields = Vec::new();
                let mut indexer = None;
                while !self.eat('}') {
                    if self.eat('[') {
                        let k = self.union()?;
                        if !self.eat(']') || !self.eat(':') {
                            return None;
                        }
                        let v = self.union()?;
                        indexer = Some((k, v));
                    } else {
                        let Tok::Name(n) = self.peek()?.clone() else {
                            return None;
                        };
                        self.pos += 1;
                        let optional = self.eat('?');
                        if !self.eat(':') {
                            return None;
                        }
                        let v = self.union()?;
                        fields.push((n, render(&v), optional));
                    }
                    self.eat(',');
                    self.eat(';');
                }
                match (fields.is_empty(), indexer) {
                    (true, Some((k, v))) => Some(LType::Map(Box::new(k), Box::new(v))),
                    (true, None) => Some(LType::Name("table".into())),
                    (false, _) => Some(LType::Record(fields)),
                }
            }
            Tok::Sym('[') => {
                // A tuple `[A, B]`: an array of the union.
                self.pos += 1;
                let mut members = Vec::new();
                while !self.eat(']') {
                    members.push(self.union()?);
                    self.eat(',');
                }
                Some(LType::Array(Box::new(LType::Union(members))))
            }
            Tok::Name(n) => {
                self.pos += 1;
                match n.as_str() {
                    "fun" => self.fun(),
                    "table" if self.eat('<') => {
                        let k = self.union()?;
                        if !self.eat(',') {
                            return None;
                        }
                        let v = self.union()?;
                        self.eat('>').then(|| LType::Map(Box::new(k), Box::new(v)))
                    }
                    _ => {
                        // Generic arguments `Name<T>` are dropped.
                        if self.eat('<') {
                            let mut depth = 1;
                            while depth > 0 {
                                match self.peek()? {
                                    Tok::Sym('<') => depth += 1,
                                    Tok::Sym('>') => depth -= 1,
                                    _ => {}
                                }
                                self.pos += 1;
                            }
                        }
                        Some(LType::Name(map_name(&n)))
                    }
                }
            }
            Tok::Ellipsis => {
                self.pos += 1;
                Some(LType::Name("any".into()))
            }
            _ => None,
        }
    }

    fn fun(&mut self) -> Option<LType> {
        if !self.eat('(') {
            return Some(LType::Name("function".into()));
        }
        let mut params = Vec::new();
        let mut vararg = None;
        let mut index = 0;
        while !self.eat(')') {
            if self.peek() == Some(&Tok::Ellipsis) {
                self.pos += 1;
                let ty = if self.eat(':') {
                    render(&self.union()?)
                } else {
                    "any".into()
                };
                vararg = Some(ty);
            } else {
                let name = match self.peek()?.clone() {
                    // A bare `self` (method overloads).
                    Tok::Name(n)
                        if n == "self"
                            && self
                                .tokens
                                .get(self.pos + 1)
                                .is_some_and(|t| *t == Tok::Sym(',') || *t == Tok::Sym(')')) =>
                    {
                        self.pos += 1;
                        self.eat(',');
                        params.push(ParamDecl {
                            name: "self".into(),
                            ty: "Self".into(),
                            optional: false,
                        });
                        continue;
                    }
                    Tok::Name(n)
                        if self
                            .tokens
                            .get(self.pos + 1)
                            .is_some_and(|t| *t == Tok::Sym(':') || *t == Tok::Sym('?')) =>
                    {
                        self.pos += 1;
                        n
                    }
                    _ => {
                        index += 1;
                        format!("arg{index}")
                    }
                };
                let optional = self.eat('?');
                // Unnamed parameters are just a type.
                let ty = if self.eat(':') || name.starts_with("arg") {
                    render(&self.union()?)
                } else {
                    "any".into()
                };
                params.push(ParamDecl { name, ty, optional });
            }
            self.eat(',');
        }
        let mut returns = Vec::new();
        if self.eat(':') {
            loop {
                if self.peek() == Some(&Tok::Ellipsis) {
                    self.pos += 1;
                    returns.push("...any".into());
                } else {
                    let t = self.postfix_or_union_in_return()?;
                    returns.push(render(&t));
                }
                if !self.eat(',') {
                    break;
                }
            }
        }
        Some(LType::Fun(params, vararg, returns))
    }

    /// A result type: `A|B` binds tighter than the result list's `,`.
    fn postfix_or_union_in_return(&mut self) -> Option<LType> {
        let mut members = vec![self.postfix()?];
        while self.eat('|') {
            members.push(self.postfix()?);
        }
        Some(if members.len() == 1 {
            members.pop().expect("one")
        } else {
            LType::Union(members)
        })
    }
}

fn map_name(n: &str) -> String {
    match n {
        "integer" | "float" | "double" => "number".into(),
        "bool" => "boolean".into(),
        "lightuserdata" => "userdata".into(),
        "void" => "nil".into(),
        "self" => "Self".into(),
        "fun" | "function" => "function".into(),
        // Blizzard enum values are numbers (`Enum.ItemQuality`).
        other if other.starts_with("Enum.") => "number".into(),
        other if other.contains('.') => "any".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types() {
        assert_eq!(convert_type("string|nil"), "string | nil");
        assert_eq!(convert_type("number?"), "number?");
        assert_eq!(
            convert_type("table<string, number>"),
            "{ [string]: number }"
        );
        assert_eq!(
            convert_type("fun(a: integer, b?: string): boolean"),
            "(a: number, b: string?) -> boolean"
        );
        assert_eq!(convert_type("Frame[]"), "Frame[]");
        assert_eq!(convert_type("\"LEFT\"|\"RIGHT\""), "\"LEFT\" | \"RIGHT\"");
        assert_eq!(
            convert_type("fun(...: any): number, string"),
            "(...any) -> (number, string)"
        );
        assert_eq!(type_part("number|nil  the value"), "number|nil");
        assert_eq!(
            type_part("fun(a: number): boolean description"),
            "fun(a: number): boolean"
        );
    }

    #[test]
    fn classes_and_functions() {
        let src = r#"---@meta _
---@class Frame : Region, ScriptObject
---@field label? string
local Frame = {}

---@param scriptType ScriptFrame
---@param bindingType? LE_SCRIPT_BINDING_TYPE
---@return function handler
function Frame:GetScript(scriptType, bindingType) end

---@alias Anchor
---| "TOP" # top
---| "LEFT"

---@param name string
---@return number?
---@overload fun(id: number): number?
function GetThing(name) end

C_Thing = {}
---@param ... any
function C_Thing.Do(...) end
"#;
        let d = convert(&[("x.lua".into(), src.into())]);
        let frame = &d.classes["Frame"];
        assert_eq!(frame.bases, ["Region"]);
        assert_eq!(frame.fields["label"], ("string".into(), true));
        let sig = &frame.methods["GetScript"][0];
        assert_eq!(sig.params.len(), 2);
        assert!(sig.params[1].optional);
        assert_eq!(d.aliases["Anchor"], "\"TOP\" | \"LEFT\"");
        assert_eq!(d.functions["GetThing"].len(), 2);
        assert_eq!(
            d.namespaces["C_Thing"].functions["Do"][0].vararg.as_deref(),
            Some("any")
        );
    }

    /// A class of a data class and a mixin extends the mixin and gets the
    /// data's fields (decision T284).
    #[test]
    fn data_and_mixin_parents() {
        let src = "---@class ItemLocationData\n---@field bagID? number\nlocal D = {}\n\n---@class ItemLocationMixin\nlocal M = {}\n\n---@return boolean\nfunction M:IsValid() end\n\n---@class ItemLocation : ItemLocationData, ItemLocationMixin\nItemLocation = {}\n";
        let d = convert(&[("x.lua".into(), src.into())]);
        let loc = &d.classes["ItemLocation"];
        assert_eq!(loc.bases, ["ItemLocationMixin"]);
        assert_eq!(loc.fields["bagID"], ("number".into(), true));
    }

    /// A function the wiki has no page for (`---#nopage`) and nothing
    /// annotates takes and answers anything; its parameter names stay.
    #[test]
    fn functions_without_a_page_are_untyped() {
        let src = "---#nopage  \n---[Documentation](https://warcraft.wiki.gg/wiki/API_SelectActiveQuest)\nfunction SelectActiveQuest() end\n\n---#nopage  \nfunction AutoLootMailItem(index) end\n\n---@param x number\nfunction Typed(x) end\n";
        let d = convert(&[("x.lua".into(), src.into())]);
        let select = &d.functions["SelectActiveQuest"][0];
        assert!(select.params.is_empty());
        assert_eq!(select.vararg.as_deref(), Some("any"));
        assert_eq!(select.ret_vararg.as_deref(), Some("any"));
        let auto = &d.functions["AutoLootMailItem"][0];
        assert_eq!(auto.params[0].name, "index");
        assert_eq!(auto.vararg.as_deref(), Some("any"));
        let typed = &d.functions["Typed"][0];
        assert_eq!(typed.vararg, None);
        assert_eq!(typed.ret_vararg, None);
    }
}
