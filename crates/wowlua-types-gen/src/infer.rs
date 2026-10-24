//! Signatures of Blizzard's UI functions from their Lua (decision T288):
//! the global functions and the mixins' functions its code defines, with
//! the parameters the definition names and the types its code shows.
//!
//! A parameter's type comes from what the function does with it: passed to
//! a typed API function (that parameter's type), arithmetic or a numeric
//! `for` (`number`), compared with a literal (its type), indexed or called
//! with `:` (`table`), called (`function`); and from the literals Blizzard's
//! own calls pass. A parameter the function tests for `nil`, or that a call
//! leaves out, is optional. Results come from the function's `return`s:
//! literals, typed API calls, its parameters, operators. Anything else, or
//! evidence that disagrees, is `unknown`: never `any`.

use crate::model::{ParamDecl, Sig};
use std::collections::{BTreeSet, HashMap};
use wowlua_syntax::view::*;

/// The typed API the code calls: a function's (or `ns.member`'s) parameter
/// and result types.
pub trait Api {
    fn signature(&self, name: &str) -> Option<&Sig>;
}

impl Api for HashMap<String, Sig> {
    fn signature(&self, name: &str) -> Option<&Sig> {
        self.get(name)
    }
}

/// Where a function lives: a global, or a table's (`Mixin.F`, `Mixin:M`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    Global(String),
    Table {
        table: String,
        member: String,
        method: bool,
    },
}

/// What one function's code shows.
#[derive(Default)]
struct Evidence {
    params: Vec<String>,
    vararg: bool,
    /// Per parameter: the types it was used as.
    types: Vec<BTreeSet<String>>,
    /// Per parameter: tested for `nil`.
    guarded: Vec<bool>,
    /// The result lists of its `return`s (`None`: unknown).
    returns: Vec<Vec<Option<String>>>,
    /// A `return` ends in a call of unknown results.
    open_results: bool,
    /// `self.x = p`: fields given a parameter, typed once the function is
    /// read (field, parameter).
    field_params: Vec<(String, usize)>,
}

/// Functions, by owner, with the fewest arguments a call passes and the
/// literal types it passes.
#[derive(Default)]
struct Calls {
    min_args: HashMap<String, usize>,
    /// The most arguments a call passes (`None`: one ends in `...` or a call).
    max_args: HashMap<String, Option<usize>>,
    arg_types: HashMap<(String, usize), BTreeSet<String>>,
    /// Arguments whose type a call's code doesn't show.
    untyped_args: std::collections::HashSet<(String, usize)>,
    /// Arguments a call passes as a literal `nil`: optional parameters.
    nil_args: std::collections::HashSet<(String, usize)>,
    /// What the code does with a function's results: their types by
    /// position, and those it tests for `nil`.
    ret_types: HashMap<(String, usize), BTreeSet<String>>,
    ret_nil: std::collections::HashSet<(String, usize)>,
    /// What a mixin's methods put in their own fields (`self.x = …`), by
    /// (table, field); `None`: values of different or unknown types.
    fields: HashMap<(String, String), Option<String>>,
}

fn literal_type(e: Expr<'_>) -> Option<&'static str> {
    Some(match e.kind() {
        ExprKind::String(_) | ExprKind::Interp(_) => "string",
        ExprKind::Number(_) => "number",
        ExprKind::True | ExprKind::False => "boolean",
        ExprKind::Table(_) => "table",
        ExprKind::Function(_) => "function",
        _ => return None,
    })
}

/// A callee's name as the API declares it: `F`, `C_X.F`.
fn callee_name(e: Expr<'_>) -> Option<String> {
    match e.kind() {
        ExprKind::Name(n) => Some(n.text().to_string()),
        ExprKind::Field { obj, name } => match obj.kind() {
            ExprKind::Name(o) => Some(format!("{}.{}", o.text(), name.text())),
            _ => None,
        },
        ExprKind::Paren(inner) => callee_name(inner),
        _ => None,
    }
}

fn args_of(a: Args<'_>) -> Vec<Expr<'_>> {
    match a.kind() {
        ArgsKind::Parens { args, .. } => args.iter().collect(),
        ArgsKind::Table(e) | ArgsKind::String(e) => vec![e],
    }
}

/// Ends in a call or `...` (more values than one).
fn is_multi(e: Expr<'_>) -> bool {
    matches!(
        e.kind(),
        ExprKind::Call { .. } | ExprKind::Method { .. } | ExprKind::Vararg
    )
}

/// The base of a declared type, without its `?`.
fn base(ty: &str) -> String {
    ty.trim().trim_end_matches('?').trim().to_string()
}

/// A block's statements, those of its `do … end` blocks in their place.
fn flatten<'a>(b: Block<'a>, out: &mut Vec<Stmt<'a>>) {
    for s in b.stmts() {
        match s.kind() {
            StmtKind::Do(inner) => flatten(inner, out),
            _ => out.push(s),
        }
    }
}

/// The one named type a value's type allows, seen through `?`,
/// parentheses, `nil` and `Secret<…>` (`(AuraData | Secret<AuraData>)?`
/// is an `AuraData`).
fn record_name(ty: &str) -> Option<String> {
    fn strip(t: &str) -> &str {
        let mut t = t.trim();
        loop {
            let before = t;
            t = t.trim_end_matches('?').trim();
            if t.starts_with('(') && t.ends_with(')') {
                t = t[1..t.len() - 1].trim();
            }
            if let Some(inner) = t.strip_prefix("Secret<").and_then(|r| r.strip_suffix('>')) {
                t = inner.trim();
            }
            if t == before {
                return t;
            }
        }
    }
    let whole = strip(ty);
    let mut name: Option<&str> = None;
    for part in whole.split('|') {
        let p = strip(part);
        if p == "nil" {
            continue;
        }
        if !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        match name {
            Some(n) if n != p => return None,
            _ => name = Some(p),
        }
    }
    name.map(str::to_string)
}

/// Types that say nothing.
fn says_nothing(ty: &str) -> bool {
    matches!(base(ty).as_str(), "any" | "unknown" | "" | "nil")
}

struct Walker<'a, A: Api> {
    api: &'a A,
    ev: Evidence,
    calls: &'a mut Calls,
    /// The function's locals: the type every value given them shows
    /// (`None`: values of different or unknown types).
    locals: HashMap<String, Option<String>>,
    /// Locals holding a call's result: (callee, position).
    results: HashMap<String, (String, usize)>,
    /// The table whose method this is (`self` is one of it).
    owner: Option<String>,
}

/// `a.b.c` as text.
fn dotted_path(e: Expr<'_>) -> Option<String> {
    match e.kind() {
        ExprKind::Name(n) => Some(n.text().to_string()),
        ExprKind::Field { obj, name } => Some(format!("{}.{}", dotted_path(obj)?, name.text())),
        _ => None,
    }
}

/// What Lua's library and WoW's string functions give.
fn builtin_result(name: &str) -> Option<&'static str> {
    Some(match name {
        "format" | "string.format" | "tostring" | "strlower" | "strupper" | "string.lower"
        | "string.upper" | "strsub" | "string.sub" | "string.rep" | "strtrim" | "strjoin"
        | "string.join" | "table.concat" | "strconcat" | "date" => "string",
        "tonumber" => "number?",
        "math.floor" | "math.ceil" | "math.abs" | "math.max" | "math.min" | "math.sqrt"
        | "math.random" | "math.fmod" | "floor" | "ceil" | "abs" | "max" | "min" | "mod"
        | "random" | "strlen" | "string.len" | "GetTime" | "time" | "debugprofilestop" => "number",
        "type" => "string",
        _ => return None,
    })
}

/// A string's methods that give a string or a number.
fn string_method(name: &str) -> Option<&'static str> {
    Some(match name {
        "format" | "lower" | "upper" | "sub" | "rep" | "reverse" | "trim" => "string",
        "len" | "byte" => "number",
        _ => return None,
    })
}

/// The mixin a value is made from: `CreateFromMixins(M, …)`,
/// `CreateAndInitFromMixin(M, …)`, `Mixin({}, M, …)`.
fn mixin_made(callee: &str, args: &[Expr<'_>]) -> Option<String> {
    let at = match callee {
        "CreateFromMixins" | "CreateAndInitFromMixin" | "CreateFromMixinsAndInit" => 0,
        "Mixin" => 1,
        _ => return None,
    };
    match args.get(at)?.kind() {
        ExprKind::Name(n) => Some(n.text().to_string()),
        _ => None,
    }
}

impl<'a, A: Api> Walker<'a, A> {
    fn param(&self, e: Expr<'_>) -> Option<usize> {
        match e.kind() {
            ExprKind::Name(n) => self.ev.params.iter().position(|p| p == n.text()),
            ExprKind::Paren(inner) => self.param(inner),
            _ => None,
        }
    }

    /// The call result a value is: a local holding one, or a call itself
    /// (its first result).
    fn result_of(&self, e: Expr<'_>) -> Option<(String, usize)> {
        match e.kind() {
            ExprKind::Name(n) if self.param(e).is_none() => self.results.get(n.text()).cloned(),
            ExprKind::Paren(inner) => self.result_of(inner),
            ExprKind::Call { callee, .. } => callee_name(callee).map(|n| (n, 0)),
            _ => None,
        }
    }

    fn note(&mut self, e: Expr<'_>, ty: &str) {
        if let Some(i) = self.param(e) {
            self.ev.types[i].insert(base(ty));
        } else if let Some(key) = self.result_of(e) {
            self.calls
                .ret_types
                .entry(key)
                .or_default()
                .insert(base(ty));
        }
    }

    fn guard(&mut self, e: Expr<'_>) {
        if let Some(i) = self.param(e) {
            self.ev.guarded[i] = true;
        } else if let Some(key) = self.result_of(e) {
            self.calls.ret_nil.insert(key);
        }
    }

    /// A condition: `if p`, `not p`, `p == nil` test `p` for `nil`.
    fn condition(&mut self, e: Expr<'_>) {
        match e.kind() {
            ExprKind::Name(_) | ExprKind::Paren(_) => self.guard(e),
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => self.condition(operand),
            ExprKind::Binary {
                op: BinOp::And | BinOp::Or,
                lhs,
                rhs,
            } => {
                self.condition(lhs);
                self.condition(rhs);
            }
            _ => {}
        }
        self.expr(e);
    }

    /// The type of a value, where it is plain.
    fn type_of(&self, e: Expr<'_>) -> Option<String> {
        if let Some(t) = literal_type(e) {
            return Some(t.into());
        }
        match e.kind() {
            ExprKind::Nil => Some("nil".into()),
            ExprKind::Paren(inner) => self.type_of(inner),
            ExprKind::Name(n) => match self.param(e) {
                Some(i) => {
                    let types = &self.ev.types[i];
                    (types.len() == 1)
                        .then(|| types.iter().next().cloned())
                        .flatten()
                }
                None => self.locals.get(n.text()).cloned().flatten(),
            },
            // `self.x` in a mixin's method: what its methods put there (the
            // first pass's findings, read as `T#x`)
            ExprKind::Field { obj, name } => {
                // a constant or an enum value (`Constants.X.Y`, `Enum.X.Y`),
                // read as `=` + its path
                if let Some(path) = dotted_path(e)
                    && let Some(sig) = self.api.signature(&format!("={path}"))
                {
                    return sig.returns.first().cloned();
                }
                // a field of a value of a declared type (`auraData.name`,
                // `auraData` an `AuraData`): the type's field
                let owner = match obj.kind() {
                    ExprKind::Name(o) if o.text() == "self" => self.owner.clone()?,
                    _ => record_name(&self.type_of(obj)?)?,
                };
                let sig = self.api.signature(&format!("{owner}#{}", name.text()))?;
                sig.returns.first().cloned()
            }
            ExprKind::Method { obj, name, .. } => {
                let o = self.type_of(obj)?;
                (o == "string")
                    .then(|| string_method(name.text()))
                    .flatten()
                    .map(str::to_string)
            }
            ExprKind::Binary { op, .. } => match op {
                BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::IDiv
                | BinOp::Mod
                | BinOp::Pow
                | BinOp::BAnd
                | BinOp::BOr
                | BinOp::BXor
                | BinOp::Shl
                | BinOp::Shr => Some("number".into()),
                BinOp::Concat => Some("string".into()),
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    Some("boolean".into())
                }
                // `a or b`: their type, when both have the same
                BinOp::Or | BinOp::Coalesce => {
                    let ExprKind::Binary { lhs, rhs, .. } = e.kind() else {
                        return None;
                    };
                    let (l, r) = (self.type_of(lhs)?, self.type_of(rhs)?);
                    (base(&l) == base(&r)).then_some(r)
                }
                _ => None,
            },
            ExprKind::Unary { op, .. } => match op {
                UnOp::Not => Some("boolean".into()),
                UnOp::Neg | UnOp::Len | UnOp::BNot => Some("number".into()),
            },
            ExprKind::Call { callee, args } => {
                let name = callee_name(callee)?;
                if let Some(m) = mixin_made(&name, &args_of(args)) {
                    return Some(m);
                }
                if let Some(t) = builtin_result(&name) {
                    return Some(t.into());
                }
                let sig = self.api.signature(&name)?;
                let r = sig.returns.first()?;
                (!says_nothing(r)).then(|| r.clone())
            }
            _ => None,
        }
    }

    fn expr(&mut self, e: Expr<'_>) {
        match e.kind() {
            ExprKind::Binary { op, lhs, rhs } => {
                match op {
                    BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::IDiv
                    | BinOp::Mod
                    | BinOp::Pow
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge => {
                        // (a comparison with a string compares strings)
                        let other_string = |x: Expr<'_>| matches!(literal_type(x), Some("string"));
                        if !(other_string(lhs) || other_string(rhs)) {
                            self.note(lhs, "number");
                            self.note(rhs, "number");
                        }
                    }
                    BinOp::Eq | BinOp::Ne => {
                        for (a, b) in [(lhs, rhs), (rhs, lhs)] {
                            if matches!(b.kind(), ExprKind::Nil) {
                                self.guard(a);
                            } else if let Some(t) = literal_type(b)
                                && t != "table"
                                && t != "function"
                            {
                                self.note(a, t);
                            }
                        }
                    }
                    BinOp::And | BinOp::Or | BinOp::Coalesce => {
                        // `p or default`, `p and p.x`: `p` may be nil
                        self.guard(lhs);
                    }
                    _ => {}
                }
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Unary { op, operand } => {
                match op {
                    UnOp::Not => self.guard(operand),
                    UnOp::Neg => self.note(operand, "number"),
                    _ => {}
                }
                self.expr(operand);
            }
            // (indexing says only that it is some object: no evidence)
            ExprKind::Field { obj, .. } | ExprKind::OptField { obj, .. } => {
                self.expr(obj);
            }
            ExprKind::Index { obj, key } | ExprKind::OptIndex { obj, key } => {
                self.expr(obj);
                self.expr(key);
            }
            ExprKind::Method { obj, args, .. } | ExprKind::OptMethod { obj, args, .. } => {
                self.expr(obj);
                for a in args_of(args) {
                    self.expr(a);
                }
            }
            ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
                if self.param(callee).is_some() {
                    self.note(callee, "function");
                }
                let args = args_of(args);
                if let Some(name) = callee_name(callee) {
                    // what the callee's parameters take
                    if let Some(sig) = self.api.signature(&name) {
                        for (i, a) in args.iter().enumerate() {
                            if let Some(p) = sig.params.get(i) {
                                if !says_nothing(&p.ty) {
                                    self.note(*a, &p.ty);
                                }
                                // passed on to an optional parameter: optional
                                if p.optional || p.ty.ends_with('?') {
                                    self.guard(*a);
                                }
                            }
                        }
                    }
                    // what Blizzard's calls pass
                    let count = if args.last().is_some_and(|a| is_multi(*a)) {
                        usize::MAX
                    } else {
                        args.len()
                    };
                    let min = self.calls.min_args.entry(name.clone()).or_insert(count);
                    *min = (*min).min(count);
                    let max = self.calls.max_args.entry(name.clone()).or_insert(Some(0));
                    *max = match (*max, count) {
                        (Some(m), c) if c != usize::MAX => Some(m.max(c)),
                        _ => None,
                    };
                    for (i, a) in args.iter().enumerate() {
                        if args.len() == i + 1 && is_multi(*a) {
                            break;
                        }
                        if matches!(a.kind(), ExprKind::Nil) {
                            self.calls.nil_args.insert((name.clone(), i));
                            continue;
                        }
                        match self.type_of(*a) {
                            Some(t) if !says_nothing(&t) => {
                                self.calls
                                    .arg_types
                                    .entry((name.clone(), i))
                                    .or_default()
                                    .insert(base(&t));
                            }
                            _ => {
                                self.calls.untyped_args.insert((name.clone(), i));
                            }
                        }
                    }
                }
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Paren(inner) | ExprKind::Cast { expr: inner, .. } => self.expr(inner),
            ExprKind::Table(fields) => {
                for f in fields.iter() {
                    match f {
                        TableField::Keyed { key, value, .. } => {
                            self.expr(key);
                            self.expr(value);
                        }
                        TableField::Named { value, .. } | TableField::Positional(value) => {
                            self.expr(value)
                        }
                    }
                }
            }
            // a closure: its uses of the parameters count, its returns not
            ExprKind::Function(f) => {
                let saved = std::mem::take(&mut self.ev.returns);
                let open = self.ev.open_results;
                self.block(f.body());
                self.ev.returns = saved;
                self.ev.open_results = open;
            }
            _ => {}
        }
    }

    fn block(&mut self, b: Block<'_>) {
        for s in b.stmts() {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: Stmt<'_>) {
        match s.kind() {
            StmtKind::Local { names, values } => {
                values.iter().for_each(|v| self.expr(v));
                let vals: Vec<Expr<'_>> = values.iter().collect();
                for (i, n) in names.iter().enumerate() {
                    let ty = vals.get(i).and_then(|v| self.type_of(*v));
                    let name = n.name.text().to_string();
                    self.results.remove(&name);
                    // `local a, b = F(…)`: a holds F's first result, b its second
                    let last = vals.len().saturating_sub(1);
                    let from = match vals.get(i.min(last)).map(|v| v.kind()) {
                        Some(ExprKind::Call { callee, .. }) if i >= last => {
                            callee_name(callee).map(|c| (c, i - last))
                        }
                        Some(ExprKind::Call { callee, .. }) => callee_name(callee).map(|c| (c, 0)),
                        _ => None,
                    };
                    if let Some(key) = from {
                        self.results.insert(name.clone(), key);
                    }
                    self.locals.insert(name, ty);
                }
            }
            StmtKind::Assign { targets, values } => {
                // `p = p or default`
                for (t, v) in targets.iter().zip(values.iter()) {
                    if let ExprKind::Binary {
                        op: BinOp::Or, lhs, ..
                    } = v.kind()
                        && self.param(t).is_some()
                        && self.param(t) == self.param(lhs)
                    {
                        self.guard(t);
                    }
                }
                targets
                    .iter()
                    .chain(values.iter())
                    .for_each(|e| self.expr(e));
                // `self.x = v` in a mixin's method: the field's evidence
                if let Some(owner) = self.owner.clone() {
                    for (t, v) in targets.iter().zip(values.iter()) {
                        if let ExprKind::Field { obj, name } = t.kind()
                            && matches!(obj.kind(), ExprKind::Name(o) if o.text() == "self")
                        {
                            if let Some(i) = self.param(v) {
                                self.ev.field_params.push((name.text().to_string(), i));
                                continue;
                            }
                            let ty = self.type_of(v).filter(|t| t != "nil");
                            let key = (owner.clone(), name.text().to_string());
                            match (self.calls.fields.get(&key), ty) {
                                (None, ty) => {
                                    self.calls.fields.insert(key, ty);
                                }
                                (Some(Some(old)), Some(new)) if *old == new => {}
                                _ => {
                                    self.calls.fields.insert(key, None);
                                }
                            }
                        }
                    }
                }
                // `a, b = F(…)`: the locals hold F's results from here on
                let vals: Vec<Expr<'_>> = values.iter().collect();
                let last = vals.len().saturating_sub(1);
                for (i, t) in targets.iter().enumerate() {
                    let ExprKind::Name(n) = t.kind() else {
                        continue;
                    };
                    if self.param(t).is_some() {
                        continue;
                    }
                    self.results.remove(n.text());
                    let from = match vals.get(i.min(last)).map(|v| v.kind()) {
                        Some(ExprKind::Call { callee, .. }) if i >= last => {
                            callee_name(callee).map(|c| (c, i - last))
                        }
                        Some(ExprKind::Call { callee, .. }) => callee_name(callee).map(|c| (c, 0)),
                        _ => None,
                    };
                    if let Some(key) = from {
                        self.results.insert(n.text().to_string(), key);
                    }
                }
                // a local given another value: its type, if the same
                for (t, v) in targets.iter().zip(values.iter()) {
                    if let ExprKind::Name(n) = t.kind()
                        && self.locals.contains_key(n.text())
                    {
                        let ty = self.type_of(v);
                        let slot = self.locals.get_mut(n.text()).expect("tracked");
                        if *slot != ty {
                            *slot = None;
                        }
                    }
                }
            }
            StmtKind::Compound { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            StmtKind::Call(e) => self.expr(e),
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => {
                self.condition(cond);
                self.block(body);
            }
            StmtKind::Repeat { body, cond } => {
                self.block(body);
                self.condition(cond);
            }
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                self.condition(cond);
                self.block(then_block);
                for (c, b) in elseifs {
                    self.condition(c);
                    self.block(b);
                }
                if let Some(b) = else_block {
                    self.block(b);
                }
            }
            StmtKind::NumericFor {
                start,
                limit,
                step,
                body,
                ..
            } => {
                for e in [Some(start), Some(limit), step].into_iter().flatten() {
                    self.note(e, "number");
                    self.expr(e);
                }
                self.block(body);
            }
            StmtKind::GenericFor { exprs, body, .. } => {
                exprs.iter().for_each(|e| self.expr(e));
                self.block(body);
            }
            StmtKind::Function { func, .. } | StmtKind::LocalFunction { func, .. } => {
                let saved = std::mem::take(&mut self.ev.returns);
                let open = self.ev.open_results;
                self.block(func.body());
                self.ev.returns = saved;
                self.ev.open_results = open;
            }
            StmtKind::Return(values) => {
                let values: Vec<Expr<'_>> = values.iter().collect();
                values.iter().for_each(|v| self.expr(*v));
                let mut list: Vec<Option<String>> = Vec::new();
                for (i, v) in values.iter().enumerate() {
                    let last = i + 1 == values.len();
                    if last && is_multi(*v) {
                        // a call's results: a known API call's, else open
                        // (its typed values, then open where it ends in `...`)
                        let known = match v.kind() {
                            ExprKind::Call { callee, .. } => callee_name(callee)
                                .and_then(|n| self.api.signature(&n))
                                .filter(|s| !s.returns.is_empty() || s.ret_vararg.is_none())
                                .map(|s| (s.returns.clone(), s.ret_vararg.is_some())),
                            _ => None,
                        };
                        // a mixin made, a builtin's value: over a declared result
                        let special = match v.kind() {
                            ExprKind::Call { callee, args } => callee_name(callee).and_then(|n| {
                                mixin_made(&n, &args_of(args))
                                    .or_else(|| builtin_result(&n).map(str::to_string))
                            }),
                            _ => None,
                        };
                        let known = if special.is_some() { None } else { known };
                        match (known, special.or_else(|| self.type_of(*v))) {
                            (Some((rs, open)), _) => {
                                list.extend(rs.into_iter().map(Some));
                                if open {
                                    self.ev.open_results = true;
                                }
                            }
                            // a builtin's or a string method's one value
                            (None, Some(t)) => list.push(Some(t)),
                            (None, None) => self.ev.open_results = true,
                        }
                    } else {
                        list.push(self.type_of(*v));
                    }
                }
                self.ev.returns.push(list);
            }
            _ => {}
        }
    }
}

/// One function's signature from its evidence and Blizzard's calls of it.
fn signature(ev: &Evidence, call_name: &str, calls: &Calls, method: bool) -> Sig {
    // a method's call passes `self` first in the call's own count
    let min = calls.min_args.get(call_name).copied().unwrap_or(usize::MAX);
    let params = ev
        .params
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut types = ev.types[i].clone();
            if !method && let Some(t) = calls.arg_types.get(&(call_name.to_string(), i)) {
                types.extend(t.iter().cloned());
            }
            let ty = match types.len() {
                1 => types.into_iter().next().expect("one"),
                _ => "unknown".into(),
            };
            ParamDecl {
                name: crate::model::ident(name),
                ty,
                optional: ev.guarded[i]
                    || (!method && i >= min)
                    || (!method && calls.nil_args.contains(&(call_name.to_string(), i))),
            }
        })
        .collect();
    // results: each position's types, `nil` where a `return` gives fewer
    let width = ev.returns.iter().map(Vec::len).max().unwrap_or(0);
    let mut returns = Vec::new();
    for pos in 0..width {
        let mut types: BTreeSet<String> = BTreeSet::new();
        let mut unknown = false;
        for r in &ev.returns {
            match r.get(pos) {
                Some(Some(t)) => {
                    types.insert(t.clone());
                }
                Some(None) => unknown = true,
                None => {
                    types.insert("nil".into());
                }
            }
        }
        let nil = types.remove("nil");
        let ty = if unknown || types.len() > 1 {
            "unknown".to_string()
        } else {
            match types.into_iter().next() {
                Some(t) if nil => format!("{}?", base(&t)),
                Some(t) => t,
                None => "nil".into(),
            }
        };
        returns.push(ty);
    }
    Sig {
        params,
        vararg: ev.vararg.then(|| "unknown".into()),
        returns,
        ret_vararg: ev.open_results.then(|| "unknown".into()),
        ..Sig::default()
    }
}

/// What Blizzard's UI code shows of the functions it defines and calls.
pub struct Inferred {
    /// The functions it defines at its top level, by owner.
    pub defined: HashMap<Owner, Sig>,
    calls: Calls,
}

impl Inferred {
    /// The mixins' fields whose values agree on a type: ((table, field), type).
    pub fn fields(&self) -> impl Iterator<Item = (&(String, String), &String)> {
        self.calls
            .fields
            .iter()
            .filter_map(|(k, t)| t.as_ref().map(|t| (k, t)))
    }

    /// A function it calls but does not define (a C function), from its
    /// calls: as many parameters as a call passes (those a call leaves
    /// out optional), typed where every call shows the same type; its
    /// results `...unknown`. `None` when no call passes a known count.
    pub fn called(&self, name: &str) -> Option<Sig> {
        let max = (*self.calls.max_args.get(name)?)?;
        let min = self.calls.min_args.get(name).copied().unwrap_or(0);
        let params = (0..max)
            .map(|i| {
                let key = (name.to_string(), i);
                let types = self.calls.arg_types.get(&key);
                let ty = match types {
                    Some(t) if t.len() == 1 && !self.calls.untyped_args.contains(&key) => {
                        t.iter().next().cloned().expect("one")
                    }
                    _ => "unknown".into(),
                };
                ParamDecl {
                    name: format!("arg{}", i + 1),
                    ty,
                    optional: i >= min || self.calls.nil_args.contains(&key),
                }
            })
            .collect();
        // its results as the code uses them, up to the last one it shows
        let width = (0..16)
            .rev()
            .find(|&i| self.calls.ret_types.contains_key(&(name.to_string(), i)))
            .map_or(0, |i| i + 1);
        let returns = (0..width)
            .map(|i| {
                let key = (name.to_string(), i);
                let ty = match self.calls.ret_types.get(&key) {
                    Some(t) if t.len() == 1 => t.iter().next().cloned().expect("one"),
                    _ => "unknown".into(),
                };
                if ty != "unknown" && self.calls.ret_nil.contains(&key) {
                    crate::model::optional(&ty)
                } else {
                    ty
                }
            })
            .collect();
        Some(Sig {
            params,
            returns,
            ret_vararg: Some("unknown".into()),
            ..Sig::default()
        })
    }
}

/// The signatures of the functions Blizzard's UI code (`files`: path and
/// text) defines at its top level, by owner.
pub fn functions(files: &[(String, String)], api: &impl Api) -> HashMap<Owner, Sig> {
    analyze(files, api).defined
}

/// What Blizzard's UI code (`files`: path and text) shows.
pub fn analyze(files: &[(String, String)], api: &impl Api) -> Inferred {
    let mut calls = Calls::default();
    let mut found: Vec<(Owner, Evidence)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (path, text) in files {
        if !path.to_ascii_lowercase().ends_with(".lua") {
            continue;
        }
        let mut db = wowlua_span::SourceDb::new();
        let file = db.add("ui.lua", text.clone());
        let parse = wowlua_syntax::parse(text.as_bytes(), file);
        if parse.has_errors() {
            continue;
        }
        let mut locals: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut top_locals: HashMap<String, Option<String>> = HashMap::new();
        let mut top_results: HashMap<String, (String, usize)> = HashMap::new();
        // the top level's statements, a `do … end` block's as its own
        // (Blizzard's deprecation files define their functions in one)
        let mut stmts = Vec::new();
        flatten(parse.view().block(), &mut stmts);
        for s in stmts {
            match s.kind() {
                StmtKind::Local { names, .. } => {
                    for n in names {
                        locals.insert(n.name.text().to_string());
                    }
                }
                StmtKind::LocalFunction { name, .. } => {
                    locals.insert(name.text().to_string());
                }
                _ => {}
            }
            // `function X(…)`, `function T:M(…)`, `function T.F(…)`, and
            // `X = function(…)`, `T.F = function(…)`
            let defined = match s.kind() {
                StmtKind::Function { name, func } => {
                    let root = name.path().at(0).text().to_string();
                    match (name.path().len(), name.method()) {
                        (1, None) => Some((Owner::Global(root), func)),
                        (1, Some(m)) => Some((
                            Owner::Table {
                                table: root,
                                member: m.text().to_string(),
                                method: true,
                            },
                            func,
                        )),
                        (2, None) => Some((
                            Owner::Table {
                                table: root,
                                member: name.path().at(1).text().to_string(),
                                method: false,
                            },
                            func,
                        )),
                        _ => None,
                    }
                }
                StmtKind::Assign { targets, values } if targets.len() == 1 && values.len() == 1 => {
                    match (targets.at(0).kind(), values.at(0).kind()) {
                        (ExprKind::Name(n), ExprKind::Function(func)) => {
                            Some((Owner::Global(n.text().to_string()), func))
                        }
                        (ExprKind::Field { obj, name }, ExprKind::Function(func)) => {
                            match obj.kind() {
                                ExprKind::Name(o) => Some((
                                    Owner::Table {
                                        table: o.text().to_string(),
                                        member: name.text().to_string(),
                                        method: false,
                                    },
                                    func,
                                )),
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let Some((owner, func)) = defined else {
                // calls at the top level count too (its locals kept from
                // one statement to the next)
                let mut w = Walker {
                    api,
                    ev: Evidence::default(),
                    calls: &mut calls,
                    locals: std::mem::take(&mut top_locals),
                    results: std::mem::take(&mut top_results),
                    owner: None,
                };
                w.stmt(s);
                top_locals = w.locals;
                top_results = w.results;
                continue;
            };
            let root = match &owner {
                Owner::Global(n) => n,
                Owner::Table { table, .. } => table,
            };
            if locals.contains(root) {
                continue;
            }
            let params: Vec<String> = func
                .params()
                .iter()
                .map(|p| p.name().text().to_string())
                .collect();
            let n = params.len();
            // what a declaration already says of its parameters (`T:M`,
            // `T.F`, `F`): the evidence starts there
            let declared_key = match &owner {
                Owner::Global(g) => g.clone(),
                Owner::Table {
                    table,
                    member,
                    method,
                } => format!("{table}{}{member}", if *method { ":" } else { "." }),
            };
            let mut types = vec![BTreeSet::new(); n];
            if let Some(sig) = api.signature(&declared_key) {
                for (i, p) in sig.params.iter().enumerate().take(n) {
                    if !says_nothing(&p.ty) {
                        types[i].insert(base(&p.ty));
                    }
                }
            }
            let mut w = Walker {
                api,
                ev: Evidence {
                    params,
                    vararg: func.vararg().is_some(),
                    types,
                    guarded: vec![false; n],
                    ..Evidence::default()
                },
                calls: &mut calls,
                locals: HashMap::new(),
                results: HashMap::new(),
                owner: match &owner {
                    Owner::Table {
                        table,
                        method: true,
                        ..
                    } => Some(table.clone()),
                    _ => None,
                },
            };
            w.block(func.body());
            // fields given a parameter: its type, now that all is read
            if let Some(table) = w.owner.clone() {
                for (field, i) in std::mem::take(&mut w.ev.field_params) {
                    let types = &w.ev.types[i];
                    let ty = (types.len() == 1)
                        .then(|| types.iter().next().cloned())
                        .flatten();
                    let key = (table.clone(), field);
                    match (w.calls.fields.get(&key), ty) {
                        (None, ty) => {
                            w.calls.fields.insert(key, ty);
                        }
                        (Some(Some(old)), Some(new)) if *old == new => {}
                        _ => {
                            w.calls.fields.insert(key, None);
                        }
                    }
                }
            }
            // the first definition counts (as the client loads them)
            if seen.insert(owner.clone()) {
                found.push((owner, w.ev));
            }
        }
    }
    let defined = found
        .into_iter()
        .map(|(owner, ev)| {
            let (call_name, method) = match &owner {
                Owner::Global(n) => (n.clone(), false),
                Owner::Table {
                    table,
                    member,
                    method,
                } => (format!("{table}.{member}"), *method),
            };
            let sig = signature(&ev, &call_name, &calls, method);
            (owner, sig)
        })
        .collect();
    Inferred { defined, calls }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> HashMap<String, Sig> {
        let mut m = HashMap::new();
        m.insert(
            "UnitName".to_string(),
            Sig {
                params: vec![ParamDecl {
                    name: "unit".into(),
                    ty: "UnitToken".into(),
                    optional: false,
                }],
                returns: vec!["string".into()],
                ..Sig::default()
            },
        );
        m.insert(
            "C_Rec.Get".to_string(),
            Sig {
                returns: vec!["Rec?".into()],
                ..Sig::default()
            },
        );
        m.insert(
            "Rec#name".to_string(),
            Sig {
                returns: vec!["string".into()],
                ..Sig::default()
            },
        );
        m
    }

    #[test]
    fn a_field_of_a_typed_value() {
        assert_eq!(
            one(
                "function F()\n  local r = C_Rec.Get()\n  return r.name\nend\n",
                "F"
            ),
            "(): string"
        );
        assert_eq!(
            record_name("((AuraData | Secret<AuraData>))?").as_deref(),
            Some("AuraData")
        );
        assert_eq!(record_name("A | B"), None);
    }

    fn one(src: &str, name: &str) -> String {
        let files = vec![("a.lua".to_string(), src.to_string())];
        let f = functions(&files, &api());
        let sig = &f[&Owner::Global(name.into())];
        crate::model::sig_text(sig)
    }

    #[test]
    fn parameters_from_their_uses() {
        assert_eq!(
            one(
                "function Show(unit, count, label, frame)\n  local n = UnitName(unit)\n  for i = 1, count do end\n  if label == \"x\" then end\n  frame:Show()\nend\n",
                "Show"
            ),
            "(unit: UnitToken, count: number, label: string, frame: unknown)"
        );
        // used two ways, or not at all: unknown; tested for nil: optional
        assert_eq!(
            one(
                "function F(a, b)\n  if not b then return end\n  a = a + 1\n  print(a .. \"x\")\nend\n",
                "F"
            ),
            "(a: number, b: unknown?)"
        );
    }

    #[test]
    fn results_from_returns() {
        assert_eq!(one("function F() return 1 end", "F"), "(): number");
        assert_eq!(
            one(
                "function F(x)\n  if x then return nil end\n  return \"a\", true\nend\n",
                "F"
            ),
            "(x: unknown?): (string?, boolean?)"
        );
        assert_eq!(
            one("function F() return UnitName(\"player\") end", "F"),
            "(): string"
        );
        assert_eq!(
            one("function F() return Other() end", "F"),
            "(): (...unknown)"
        );
        assert_eq!(
            one("function F() local t = {} return t end", "F"),
            "(): table"
        );
        assert_eq!(
            one("function F(c) return (\"x%s\"):format(c) end", "F"),
            "(c: unknown): string"
        );
        assert_eq!(
            one("function F() return CreateFromMixins(FooMixin) end", "F"),
            "(): FooMixin"
        );
        assert_eq!(
            one(
                "function F(a) local v = 1 if a then v = \"x\" end return v end",
                "F"
            ),
            "(a: unknown?): unknown"
        );
        assert_eq!(one("function F() end", "F"), "()");
    }

    #[test]
    fn c_functions_from_their_calls() {
        let files = vec![(
            "a.lua".to_string(),
            "local n = C_X.Count(1, \"a\")\nC_X.Count(2)\nC_X.Count(3, UnitName(\"player\"), t)\n"
                .to_string(),
        )];
        let f = analyze(&files, &api());
        assert_eq!(
            crate::model::sig_text(&f.called("C_X.Count").unwrap()),
            "(arg1: number, arg2: string?, arg3: unknown?): (...unknown)"
        );
        assert!(f.called("C_X.Never").is_none());
        // its results as the code uses them
        let files = vec![(
            "a.lua".to_string(),
            "local title, done = GetTitle(1)\nif done then end\nprint(UnitName(title))\nlocal n = GetCount() + 1\n".to_string(),
        )];
        let f = analyze(&files, &api());
        assert_eq!(
            crate::model::sig_text(&f.called("GetTitle").unwrap()),
            "(arg1: number): (UnitToken, ...unknown)"
        );
        assert_eq!(
            crate::model::sig_text(&f.called("GetCount").unwrap()),
            "(): (number, ...unknown)"
        );
    }

    #[test]
    fn mixin_fields_across_two_passes() {
        let files = vec![(
            "a.lua".to_string(),
            "function MapMixin:SetMapID(mapID)\n  self.mapID = mapID\n  self.n = mapID + 1\nend\nfunction MapMixin:GetCount()\n  return self.n\nend\nfunction MapMixin:GetMapID()\n  return self.mapID or 0\nend\n".to_string(),
        )];
        let mut api = api();
        let first = analyze(&files, &api);
        for ((t, f), ty) in first.fields() {
            api.insert(
                format!("{t}#{f}"),
                Sig {
                    returns: vec![ty.clone()],
                    ..Sig::default()
                },
            );
        }
        let second = analyze(&files, &api);
        let get = &second.defined[&Owner::Table {
            table: "MapMixin".into(),
            member: "GetCount".into(),
            method: true,
        }];
        assert_eq!(crate::model::sig_text(get), "(): number");
        let id = &second.defined[&Owner::Table {
            table: "MapMixin".into(),
            member: "GetMapID".into(),
            method: true,
        }];
        assert_eq!(crate::model::sig_text(id), "(): number");
    }

    #[test]
    fn blizzard_calls_count() {
        let files = vec![(
            "a.lua".to_string(),
            "function F(a, b) print(a, b) end\nF(1)\nF(2, \"x\")\n".to_string(),
        )];
        let f = functions(&files, &api());
        assert_eq!(
            crate::model::sig_text(&f[&Owner::Global("F".into())]),
            "(a: number, b: string?)"
        );
    }
}
