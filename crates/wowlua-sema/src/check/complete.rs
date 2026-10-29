//! Completion (the language server's, spec §16.2). The editor puts
//! [`PLACEHOLDER`] where the cursor is, and the check of that text records
//! what could stand there when it meets it: the names in scope where a
//! name is read, the members of the value's type after `.` and `:`, the
//! type names where a type is written.

use super::body::Checker;
use super::env::{DefKind, DefTarget, Env, MemberKind, Scope, Visibility};
use super::types::{DefId, Ty, TypeId, Types};
use rustc_hash::FxHashSet;
use wowlua_span::Span;
use wowlua_syntax::view::Names;

/// The name the editor writes at the cursor.
pub const PLACEHOLDER: &str = "__wowlua_complete__";

/// The frame methods whose first argument is an event's name (typed
/// `string` by the game's API): completed with the game's events.
pub const EVENT_METHODS: &[&str] = &[
    "RegisterEvent",
    "RegisterUnitEvent",
    "UnregisterEvent",
    "IsEventRegistered",
];

/// What a completion is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Variable,
    Constant,
    Function,
    Method,
    Field,
    Property,
    Class,
    Interface,
    Alias,
    Enum,
    EnumMember,
    Module,
    TypeParameter,
    /// A built-in type (`number`).
    Builtin,
    /// A string the expected type names (`"UNIT_HEALTH"`).
    Literal,
    /// A core decorator (`@script`).
    Decorator,
}

/// Where the cursor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// Where a value's name is read.
    Names,
    /// After `.` (`method`: `:`).
    Members { method: bool },
    /// Where a type is written.
    Types,
    /// In a string, where the expected type names strings (an event name,
    /// an anchor point).
    Strings,
    /// After `@`: the decorators that apply to what follows.
    Decorators,
}

/// What a decorator is put on (as the lowerer tells them apart).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum On {
    Function,
    Method,
    Constructor,
    Accessor,
    Field,
    Class,
    Enum,
    Local,
    Import,
    Declare,
    Type,
    Other,
}

/// What a statement's decorators are put on.
pub(crate) fn on_stmt(s: wowlua_syntax::view::Stmt<'_>) -> On {
    use wowlua_syntax::view::StmtKind as K;
    match s.kind() {
        K::Export(inner) => on_stmt(inner),
        K::Function { .. } | K::LocalFunction { .. } => On::Function,
        K::Class(_) => On::Class,
        K::Enum(_) => On::Enum,
        K::Local { .. } | K::Const { .. } | K::LocalDestructure { .. } => On::Local,
        K::Import(_) => On::Import,
        K::Declare(_) => On::Declare,
        K::Interface(_) | K::TypeAlias(_) => On::Type,
        _ => On::Other,
    }
}

/// The core decorators a user writes (spec §4.3): name, how it is written,
/// what it does. (`@intrinsic` is the compiler's own; `@deferred` is put on
/// parameters.)
const CORE_DECORATORS: &[(&str, &str, &str)] = &[
    (
        "deprecated",
        "@deprecated(msg?)",
        "Warning W3030 at every use; in debug builds, a warning at run time on first use.",
    ),
    (
        "global",
        "@global(name?)",
        "Also assigns the final value to the global `name` (the declaration's name by default), for XML scripts, key bindings and other addons.",
    ),
    (
        "if",
        "@if(condition)",
        "Conditional compilation: the declaration is left out when the build-time condition is false (`@if(build.debug)`).",
    ),
    (
        "script",
        "@script(name, opts?)",
        "The method handles the frame script `name` (`SetScript`; `{ hook = true }`: `HookScript`) on every frame of the class.",
    ),
    (
        "inline",
        "@inline",
        "Inlines the function at its calls in optimized builds (a body that is a single `return`).",
    ),
    (
        "reflect",
        "@reflect(opts?)",
        "Emits full reflection metadata.",
    ),
    (
        "profile",
        "@profile(opts?)",
        "Measures calls and time when the profile instruments the build.",
    ),
    (
        "noprofile",
        "@noprofile",
        "Left out of `instrument = \"all\"`.",
    ),
    (
        "trace",
        "@trace(opts?)",
        "Logs entry, arguments, results and errors at `trace` level.",
    ),
    (
        "debugOnly",
        "@debugOnly",
        "Outside debug builds the declaration and every call statement to it are removed.",
    ),
    (
        "checked",
        "@checked",
        "Checks the parameters' types at run time, whatever the profile.",
    ),
    (
        "unchecked",
        "@unchecked",
        "Never checks the parameters' types at run time.",
    ),
    ("nocheck", "@nocheck", "Skips type checking of the body."),
    (
        "widget",
        "@widget(type)",
        "Marks a native widget type for widget classes (`declare class`).",
    ),
    (
        "onDemand",
        "@onDemand(addon)",
        "The global exists only once Blizzard's load-on-demand addon `addon` has loaded.",
    ),
];

/// Whether core decorator `name` applies to `on` (the lowerer's E3021).
fn applies(name: &str, on: On) -> bool {
    use On::*;
    match name {
        "if" | "deprecated" => true,
        "global" => matches!(on, Function | Class | Local | Enum),
        "inline" | "debugOnly" => matches!(on, Function | Method),
        "script" => on == Method,
        "nocheck" => matches!(on, Function | Method | Constructor | Accessor | Class),
        "widget" | "onDemand" => on == Declare,
        "profile" | "noprofile" => matches!(on, Function | Method | Constructor | Class),
        "trace" | "checked" | "unchecked" => matches!(on, Function | Method | Constructor),
        "reflect" => matches!(on, Function | Class | Enum),
        _ => false,
    }
}

/// One thing that could stand at the cursor.
#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub kind: Kind,
    /// Its type as messages show it.
    pub detail: String,
    /// Lower first: the nearest (locals, a class's own members), then the
    /// project's, then the game's.
    pub rank: u8,
    /// Where it is declared (for its doc comment).
    pub target: Option<DefTarget>,
    /// `target` resolved, once the package is checked.
    pub at: Option<Span>,
    /// What it does, where no doc comment says (a core decorator).
    pub doc: Option<String>,
    /// An export of another module the file does not import yet: the
    /// specifier its import names (auto-import, T506).
    pub import: Option<Box<str>>,
}

/// An export of another module a name completion can offer with its
/// import (T506).
#[derive(Clone, Debug)]
pub struct Importable {
    pub name: Box<str>,
    /// The module's specifier, as the file would import it.
    pub from: Box<str>,
    pub kind: Kind,
}

/// What a completion check found.
#[derive(Clone, Debug)]
pub struct Completions {
    pub context: Context,
    pub items: Vec<Item>,
    /// Names were left out for the prefix (the game's globals): a longer
    /// prefix gives others (`matches`).
    pub incomplete: bool,
    /// The file the cursor is in.
    pub file: wowlua_span::FileId,
}

/// A completion check under way: the word typed so far, and what was found
/// at the placeholder (the last time the check met it).
#[derive(Debug, Default)]
pub struct Completing {
    pub prefix: Box<str>,
    pub found: Option<Completions>,
    /// The signatures of the innermost call whose arguments hold the
    /// placeholder (signature help): one per overload.
    pub signature: Option<Vec<Signature>>,
}

/// One form of a function, for signature help: its parameters as written
/// (`name: T`), its rest (`...: T`), its results.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub params: Vec<String>,
    pub rest: Option<String>,
    pub results: String,
}

/// Whether `name` matches what was typed: its letters in order, the first
/// at the start of the name or of a word in it (`spell` matches
/// `GetSpellInfo`), case ignored; a single letter only at the start (the
/// game has thousands of names with a word that starts with it). The
/// editor ranks and filters further.
pub fn matches(prefix: &str, name: &str) -> bool {
    let mut typed = prefix.chars().map(|c| c.to_ascii_lowercase());
    let Some(first) = typed.next() else {
        return true;
    };
    let rest: Vec<char> = typed.collect();
    let chars: Vec<char> = name.chars().collect();
    (0..chars.len())
        .filter(|&i| {
            i == 0
                || (!rest.is_empty()
                    && (chars[i - 1] == '_'
                        || (chars[i].is_ascii_uppercase() && !chars[i - 1].is_ascii_uppercase())))
        })
        .filter(|&i| chars[i].to_ascii_lowercase() == first)
        .any(|i| {
            let mut left = rest.iter().peekable();
            for c in &chars[i + 1..] {
                if left.peek().is_some_and(|&&p| p == c.to_ascii_lowercase()) {
                    left.next();
                }
            }
            left.peek().is_none()
        })
}

fn is_name(s: &str) -> bool {
    s != PLACEHOLDER
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The built-in type names (spec §2.2.1).
const BUILTIN_TYPES: &[&str] = &[
    "any", "unknown", "never", "boolean", "number", "string", "table", "thread", "userdata", "nil",
    "true", "false", "function",
];

impl Env<'_> {
    /// Records what was found at the placeholder.
    fn found(
        &mut self,
        context: Context,
        items: Vec<Item>,
        incomplete: bool,
        file: wowlua_span::FileId,
    ) {
        if let Some(c) = self.completing.as_mut() {
            c.found = Some(Completions {
                context,
                items,
                incomplete,
                file,
            });
        }
    }

    /// The completions found are module `m`'s members: each is declared as
    /// its export.
    pub(crate) fn export_targets(&mut self, m: usize) {
        if let Some(found) = self.completing.as_mut().and_then(|c| c.found.as_mut()) {
            for item in &mut found.items {
                if item.target.is_none() {
                    item.target = Some(DefTarget::Export(m, item.label.as_str().into()));
                }
            }
        }
    }

    fn prefix(&self) -> Box<str> {
        self.completing
            .as_ref()
            .map(|c| c.prefix.clone())
            .unwrap_or_default()
    }

    /// A completion's kind for a value of type `t`.
    fn value_kind(&self, t: TypeId, constant: bool) -> Kind {
        match self.types.get(t) {
            Ty::Func(_) => Kind::Function,
            Ty::Intersection(parts)
                if parts
                    .iter()
                    .any(|&p| matches!(self.types.get(p), Ty::Func(_))) =>
            {
                Kind::Function
            }
            Ty::ClassOf(d) => self.def_kind(*d),
            Ty::EnumMember(..) => Kind::EnumMember,
            _ if constant => Kind::Constant,
            _ => Kind::Variable,
        }
    }

    fn def_kind(&self, d: DefId) -> Kind {
        match self.view(d).kind() {
            DefKind::Class => Kind::Class,
            DefKind::Interface => Kind::Interface,
            DefKind::Alias => Kind::Alias,
            DefKind::Enum => Kind::Enum,
            DefKind::Namespace => Kind::Module,
        }
    }

    fn type_item(&self, label: &str, d: DefId, rank: u8) -> Item {
        Item {
            import: None,
            label: label.to_string(),
            kind: self.def_kind(d),
            detail: String::new(),
            rank,
            target: self.def(d).span.map(DefTarget::At),
            at: None,
            doc: None,
        }
    }

    /// The type names that can stand where `path` ends in the placeholder.
    pub(crate) fn complete_types(
        &mut self,
        path: Names<'_>,
        scope: &Scope,
        file: wowlua_span::FileId,
    ) {
        let prefix = self.prefix();
        let mut items = Vec::new();
        let mut incomplete = false;
        let module = scope.module;
        if path.len() == 1 {
            for (name, _) in &scope.params {
                items.push(Item {
                    import: None,
                    label: name.to_string(),
                    kind: Kind::TypeParameter,
                    detail: String::new(),
                    rank: 0,
                    target: None,
                    at: None,
                    doc: None,
                });
            }
            if scope.self_ty.is_some() {
                items.push(Item {
                    import: None,
                    label: "Self".into(),
                    kind: Kind::TypeParameter,
                    detail: String::new(),
                    rank: 0,
                    target: None,
                    at: None,
                    doc: None,
                });
            }
            for b in BUILTIN_TYPES {
                items.push(Item {
                    import: None,
                    label: b.to_string(),
                    kind: Kind::Builtin,
                    detail: String::new(),
                    rank: 1,
                    target: None,
                    at: None,
                    doc: None,
                });
            }
            if let Some(m) = module {
                let own: Vec<(Box<str>, DefId)> = self
                    .module_types
                    .get(&m)
                    .into_iter()
                    .chain(self.module_imports.get(&m))
                    .flatten()
                    .map(|(n, &d)| (n.clone(), d))
                    .collect();
                for (n, d) in own {
                    items.push(self.type_item(&n, d, 0));
                }
                let namespaces: Vec<Box<str>> = self
                    .module_namespaces
                    .get(&m)
                    .into_iter()
                    .flatten()
                    .map(|(n, _)| n.clone())
                    .collect();
                for n in namespaces {
                    items.push(Item {
                        import: None,
                        label: n.to_string(),
                        kind: Kind::Module,
                        detail: String::new(),
                        rank: 0,
                        target: None,
                        at: None,
                        doc: None,
                    });
                }
            }
            // the game's types: those the prefix matches (a longer one
            // matches others)
            incomplete = prefix.chars().count() < 2;
            if !prefix.is_empty() {
                let global: Vec<(Box<str>, DefId)> = self
                    .declared
                    .global_type_entries()
                    .filter(|(n, _)| !n.contains('.') && matches(&prefix, n))
                    .map(|(n, d)| (n.into(), d))
                    .collect();
                for (n, d) in global {
                    items.push(self.type_item(&n, d, 2));
                }
            }
        } else {
            let head: Vec<&str> = path.iter().take(path.len() - 1).map(|n| n.text()).collect();
            // `M.T` through `import * as M`
            let namespace = (head.len() == 1)
                .then(|| {
                    module
                        .and_then(|m| self.module_namespaces.get(&m))
                        .and_then(|ns| ns.get(head[0]))
                        .copied()
                })
                .flatten();
            match namespace {
                Some(m) => {
                    let types: Vec<(Box<str>, DefId)> = self
                        .module_types
                        .get(&m)
                        .into_iter()
                        .flatten()
                        .map(|(n, &d)| (n.clone(), d))
                        .collect();
                    for (n, d) in types {
                        items.push(self.type_item(&n, d, 0));
                    }
                }
                // a namespace of the game's types (`Enum.X`)
                None => {
                    let lead = format!("{}.", head.join("."));
                    let inner: Vec<(Box<str>, DefId)> = self
                        .declared
                        .global_type_entries()
                        .filter_map(|(n, d)| {
                            let rest = n.strip_prefix(lead.as_str())?;
                            (!rest.contains('.')).then(|| (rest.into(), d))
                        })
                        .collect();
                    for (n, d) in inner {
                        items.push(self.type_item(&n, d, 0));
                    }
                }
            }
        }
        self.found(Context::Types, items, incomplete, file);
    }
}

/// The strings a type names: its string literals, through unions and a
/// type parameter's constraint.
fn string_literals(env: &Env<'_>, t: TypeId, out: &mut Vec<String>, depth: u32) {
    if depth > 8 {
        return;
    }
    for m in env.types.members(t) {
        match env.types.get(m) {
            Ty::Str(s) => out.push(s.to_string()),
            Ty::Param(p) => {
                if let Some(c) = env.params[p.0 as usize].constraint {
                    string_literals(env, c, out, depth + 1);
                }
            }
            _ => {}
        }
    }
}

impl Checker<'_, '_> {
    /// The placeholder is met: the innermost call whose arguments are being
    /// checked gives the signature help.
    pub(crate) fn note_signature(&mut self) {
        let Some(&callee) = self.call_stack.last() else {
            return;
        };
        let mut funcs: Vec<super::types::Func> = Vec::new();
        for m in self.env.types.members(callee) {
            match self.env.types.get(m).clone() {
                Ty::Func(f) => funcs.push((*f).clone()),
                Ty::Intersection(parts) => {
                    for p in parts {
                        if let Ty::Func(f) = self.env.types.get(p).clone() {
                            funcs.push((*f).clone());
                        }
                    }
                }
                _ => {}
            }
        }
        let shown: Vec<Signature> = funcs
            .iter()
            .map(|f| Signature {
                params: f
                    .params
                    .iter()
                    .map(|p| {
                        let ty = self.env.display(p.ty);
                        let mark = if p.optional && !ty.ends_with('?') {
                            "?"
                        } else {
                            ""
                        };
                        match &p.name {
                            Some(n) => format!("{n}: {ty}{mark}"),
                            None => format!("{ty}{mark}"),
                        }
                    })
                    .collect(),
                rest: f.rest.as_ref().map(|r| match r {
                    super::types::Rest::Variadic(t) => format!("...: {}", self.env.display(*t)),
                    super::types::Rest::Pack(_) => "...".to_string(),
                }),
                results: self.env.display_pack(&f.ret, 0),
            })
            .collect();
        if let Some(c) = self.env.completing.as_mut() {
            c.signature = (!shown.is_empty()).then_some(shown);
        }
    }

    /// Whether `e` is the placeholder string of a completion check.
    pub(crate) fn completes_string(&self, e: wowlua_syntax::view::Expr<'_>) -> bool {
        self.env.completing.is_some()
            && matches!(e.kind(), wowlua_syntax::view::ExprKind::String(s) if &*s.value() == PLACEHOLDER.as_bytes())
    }

    /// The strings the expected type names, where the placeholder string
    /// `at` is checked against it. A call checked against several overloads
    /// meets it once for each: what each names is kept.
    pub(crate) fn complete_strings(&mut self, expected: Option<TypeId>, at: Span) {
        self.note_signature();
        let mut names = Vec::new();
        if let Some(t) = expected {
            string_literals(self.env, t, &mut names, 0);
        }
        let mut items: Vec<Item> = match self.env.completing.as_mut().and_then(|c| c.found.take()) {
            Some(found) if found.context == Context::Strings => found.items,
            _ => Vec::new(),
        };
        for n in names {
            if items.iter().any(|i| i.label == n) {
                continue;
            }
            items.push(Item {
                import: None,
                label: n,
                kind: Kind::Literal,
                detail: String::new(),
                rank: 0,
                target: None,
                at: None,
                doc: None,
            });
        }
        self.env.found(Context::Strings, items, false, at.file);
    }

    /// The game's events, where the placeholder string `at` is an event
    /// method's argument; each with its payload.
    pub(crate) fn complete_events(&mut self, at: Span) {
        self.note_signature();
        let events: Vec<(Box<str>, super::types::Pack)> = self
            .env
            .events
            .iter()
            .map(|(n, p)| (n.clone(), p.clone()))
            .collect();
        let items = events
            .into_iter()
            .map(|(n, pack)| Item {
                import: None,
                label: n.to_string(),
                kind: Kind::Literal,
                detail: self.env.display_pack(&pack, 0),
                rank: 0,
                target: None,
                at: None,
                doc: None,
            })
            .collect();
        self.env.found(Context::Strings, items, false, at.file);
    }

    /// After `@` (decorators put on `on`): the core decorators that apply,
    /// and where user decorators may stand the functions in scope and the
    /// modules a dotted decorator reads (`@events.`); a dotted one's
    /// members; in a decorator's string argument, the game's events for the
    /// event decorators.
    pub(crate) fn complete_decorators(
        &mut self,
        decorators: wowlua_syntax::view::Decorators<'_>,
        on: On,
    ) {
        if self.env.completing.is_none() {
            return;
        }
        for d in decorators {
            let path = d.path();
            if let Some(last) = path.last()
                && last.text() == PLACEHOLDER
            {
                if path.len() == 1 {
                    self.complete_decorator_names(last.span(), on);
                } else {
                    let head = path.at(0).text();
                    let mut t = match self.lookup_var(head) {
                        Some(v) => self.var_type(v),
                        None => self.env.declared.global(head).unwrap_or(Types::ANY),
                    };
                    for i in 1..path.len() - 1 {
                        t = self
                            .field_type_quiet(t, path.at(i).text())
                            .unwrap_or(Types::ANY);
                    }
                    self.complete_members(t, false, last.span());
                }
            }
            let Some(args) = d.args() else { continue };
            let id = match path.len() {
                1 => self.intrinsic_path(path.at(0).text(), None),
                2 => self.intrinsic_path(path.at(0).text(), Some(path.at(1).text())),
                _ => None,
            };
            for e in args.exprs() {
                if self.completes_string(e)
                    && id.as_deref().is_some_and(|i| i.starts_with("events."))
                {
                    self.complete_events(e.span());
                }
            }
        }
    }

    fn complete_decorator_names(&mut self, at: Span, on: On) {
        let mut items: Vec<Item> = CORE_DECORATORS
            .iter()
            .filter(|(n, _, _)| applies(n, on))
            .map(|(n, written, doc)| Item {
                import: None,
                label: n.to_string(),
                kind: Kind::Decorator,
                detail: written.to_string(),
                rank: 0,
                target: None,
                at: None,
                doc: Some(doc.to_string()),
            })
            .collect();
        // user decorators: functions, and modules for `@events.on`
        let user = matches!(
            on,
            On::Function | On::Method | On::Constructor | On::Accessor | On::Field | On::Class
        );
        if user {
            for (name, v) in self.visible_locals() {
                if !is_name(&name) || items.iter().any(|i| *i.label == *name) {
                    continue;
                }
                let t = self.var_type(v);
                let kind = match self.env.value_kind(t, false) {
                    Kind::Function => Kind::Function,
                    _ if matches!(self.env.types.get(t), Ty::Record(_) | Ty::ClassOf(_)) => {
                        Kind::Module
                    }
                    _ => continue,
                };
                let target = match self.import_targets.get(&name) {
                    Some(t) if self.import_vars.get(&name) == Some(&v) => Some(t.clone()),
                    _ => None,
                };
                items.push(Item {
                    import: None,
                    label: name.to_string(),
                    kind,
                    detail: self.env.display(t),
                    rank: 1,
                    target,
                    at: None,
                    doc: None,
                });
            }
        }
        self.env.found(Context::Decorators, items, false, at.file);
    }

    /// Whether `name` is the placeholder of a completion check.
    pub(crate) fn completes(&self, name: &str) -> bool {
        name == PLACEHOLDER && self.env.completing.is_some()
    }

    /// The names that can be read where the placeholder `at` is: the
    /// locals in scope, the project's globals, and the game's globals the
    /// prefix matches.
    pub(crate) fn complete_names(&mut self, at: Span) {
        self.note_signature();
        let prefix = self.env.prefix();
        let mut items = Vec::new();
        let mut seen: FxHashSet<Box<str>> = FxHashSet::default();
        for (name, v) in self.visible_locals() {
            if !is_name(&name) || !seen.insert(name.clone()) {
                continue;
            }
            let t = self.var_type(v);
            let var = &self.vars[v.0 as usize];
            let kind = if self.import_vars.get(&name) == Some(&v)
                && self
                    .scope
                    .module
                    .and_then(|m| self.env.module_namespaces.get(&m))
                    .is_some_and(|ns| ns.contains_key(&name))
            {
                Kind::Module
            } else {
                self.env.value_kind(t, var.is_const)
            };
            let target = match self.import_targets.get(&name) {
                Some(t) if self.import_vars.get(&name) == Some(&v) => Some(t.clone()),
                _ => var.decl.map(|start| {
                    DefTarget::At(Span {
                        file: at.file,
                        start,
                        end: start + name.len() as u32,
                    })
                }),
            };
            items.push(Item {
                import: None,
                label: name.to_string(),
                kind,
                detail: self.env.display(t),
                rank: 0,
                target,
                at: None,
                doc: None,
            });
        }
        // the project's globals, then the game's the prefix matches
        let mut own: Vec<&str> = self
            .extra_globals
            .iter()
            .chain(self.assigned_globals.iter())
            .map(String::as_str)
            .filter(|n| is_name(n) && !seen.contains(*n))
            .collect();
        own.sort_unstable();
        own.dedup();
        let own: Vec<String> = own.into_iter().map(str::to_string).collect();
        for name in own {
            let t = self
                .env
                .declared
                .global(&name)
                .or_else(|| self.global_types.get(name.as_str()).copied())
                .unwrap_or(Types::ANY);
            seen.insert(name.as_str().into());
            items.push(Item {
                import: None,
                kind: self.env.value_kind(t, false),
                detail: self.env.display(t),
                rank: 1,
                target: Some(DefTarget::Global(name.as_str().into())),
                at: None,
                doc: None,
                label: name,
            });
        }
        let incomplete = prefix.chars().count() < 2;
        if !prefix.is_empty() {
            let game: Vec<(Box<str>, TypeId)> = self
                .env
                .declared
                .global_entries()
                .filter(|(n, _)| is_name(n) && !seen.contains(*n) && matches(&prefix, n))
                .map(|(n, t)| (n.into(), t))
                .collect();
            for (name, t) in game {
                items.push(Item {
                    import: None,
                    label: name.to_string(),
                    kind: self.env.value_kind(t, false),
                    detail: self.env.display(t),
                    rank: 2,
                    target: Some(DefTarget::Global(name)),
                    at: None,
                    doc: None,
                });
            }
        }
        // the exports of the package's other modules, each with its
        // import (auto-import, T506)
        if !prefix.is_empty() {
            for imp in &self.importable {
                if matches(&prefix, &imp.name) && !seen.contains(&*imp.name) {
                    items.push(Item {
                        label: imp.name.to_string(),
                        kind: imp.kind,
                        detail: format!("import from \"{}\"", imp.from),
                        rank: 1,
                        target: None,
                        at: None,
                        doc: None,
                        import: Some(imp.from.clone()),
                    });
                }
            }
        }
        self.env.found(Context::Names, items, incomplete, at.file);
    }

    /// The members of a value of type `o` (after `:`, the methods),
    /// where the placeholder `at` is. Of a union, those every part has.
    pub(crate) fn complete_members(&mut self, o: TypeId, method: bool, at: Span) {
        self.note_signature();
        let mut parts: Vec<Vec<Item>> = Vec::new();
        for m in self.env.types.members(o) {
            if m == Types::NIL {
                continue;
            }
            let mut items = Vec::new();
            self.members_of(m, method, &mut items, 0);
            parts.push(items);
        }
        let mut items = parts.pop().unwrap_or_default();
        for p in &parts {
            items.retain(|i| p.iter().any(|j| j.label == i.label));
        }
        let mut seen: FxHashSet<String> = FxHashSet::default();
        items.retain(|i| seen.insert(i.label.clone()));
        self.env
            .found(Context::Members { method }, items, false, at.file);
    }

    fn member_item(
        &mut self,
        label: &str,
        t: TypeId,
        kind: Kind,
        method: bool,
        rank: u8,
        at: Option<Span>,
    ) -> Option<Item> {
        let callable = matches!(kind, Kind::Method | Kind::Function);
        if method && !callable {
            return None;
        }
        let shown = if method { self.drop_first(t) } else { t };
        Some(Item {
            import: None,
            label: label.to_string(),
            kind,
            detail: self.env.display(shown),
            rank,
            target: at.map(DefTarget::At),
            at: None,
            doc: None,
        })
    }

    fn members_of(&mut self, t: TypeId, method: bool, out: &mut Vec<Item>, depth: u32) {
        if depth > 8 {
            return;
        }
        match self.env.types.get(t).clone() {
            // `s:upper()`
            Ty::String | Ty::Str(_) => {
                if !method {
                    return;
                }
                let Some(lib) = self.env.declared.global("string") else {
                    return;
                };
                let fields: Vec<(Box<str>, TypeId, Option<Span>)> =
                    match self.env.types.get(lib).clone() {
                        Ty::ClassOf(d) => self
                            .env
                            .def(d)
                            .members
                            .iter()
                            .map(|(n, m)| (n.clone(), m.ty, m.span))
                            .collect(),
                        Ty::Record(r) => r
                            .fields
                            .iter()
                            .map(|(n, f)| (n.into(), f.ty, None))
                            .collect(),
                        _ => Vec::new(),
                    };
                for (n, ty, span) in fields {
                    out.extend(self.member_item(&n, ty, Kind::Method, true, 1, span));
                }
            }
            Ty::Record(r) => {
                let fields: Vec<(Box<str>, TypeId)> =
                    r.fields.iter().map(|(n, f)| (n.into(), f.ty)).collect();
                for (n, ty) in fields {
                    let kind = match self.env.value_kind(ty, false) {
                        Kind::Function => Kind::Function,
                        Kind::Variable | Kind::Constant => Kind::Field,
                        k => k,
                    };
                    out.extend(self.member_item(&n, ty, kind, method, 0, None));
                }
            }
            Ty::Named(d, args) => {
                if self.env.view(d).kind() != DefKind::Enum {
                    self.instance_members(d, &args, t, method, out);
                }
            }
            Ty::ClassOf(d) => self.static_members(d, method, out),
            Ty::Param(p) => {
                if let Some(c) = self.env.params[p.0 as usize].constraint {
                    self.members_of(c, method, out, depth + 1);
                }
            }
            Ty::Intersection(parts) => {
                for p in parts {
                    if !matches!(self.env.types.get(p), Ty::Func(_)) {
                        self.members_of(p, method, out, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    /// Whether a member of `owner` with visibility `v` can be read here.
    fn visible(&self, v: Visibility, owner: DefId) -> bool {
        match v {
            Visibility::Public => true,
            Visibility::Private => self.classes.iter().any(|(cd, _)| *cd == owner),
            Visibility::Protected => self
                .classes
                .iter()
                .any(|(cd, _)| *cd == owner || self.env.derives(*cd, owner)),
        }
    }

    /// The instance members of class or interface `d` (its own, then its
    /// bases').
    fn instance_members(
        &mut self,
        d: DefId,
        args: &[TypeId],
        receiver: TypeId,
        method: bool,
        out: &mut Vec<Item>,
    ) {
        let mut names: Vec<Box<str>> = Vec::new();
        let mut seen_names: FxHashSet<Box<str>> = FxHashSet::default();
        let mut seen: FxHashSet<DefId> = FxHashSet::default();
        let mut todo: Vec<(DefId, Vec<TypeId>)> = vec![(d, args.to_vec())];
        while let Some((def, args)) = todo.pop() {
            if !seen.insert(def) || seen.len() > 64 {
                continue;
            }
            for n in self.env.def(def).members.keys() {
                if seen_names.insert(n.clone()) {
                    names.push(n.clone());
                }
            }
            for b in self.env.bases_of(def, &args).into_iter().rev() {
                if let Ty::Named(bd, bargs) = self.env.types.get(b).clone() {
                    todo.push((bd, bargs));
                }
            }
        }
        for n in names {
            let Some((m, owner)) = self.env.instance_member_as(d, args, &n, Some(receiver)) else {
                continue;
            };
            if !self.visible(m.visibility, owner) {
                continue;
            }
            let rank = if owner == d {
                0
            } else if self.env.view(owner).native() {
                2
            } else {
                1
            };
            let (kind, ty) = match m.kind {
                MemberKind::Method => (Kind::Method, self.env.with_self(m.ty, receiver)),
                MemberKind::Getter | MemberKind::Setter => (Kind::Property, m.ty),
                MemberKind::Field => match self.env.value_kind(m.ty, false) {
                    Kind::Function => (Kind::Function, m.ty),
                    _ => (Kind::Field, m.ty),
                },
            };
            out.extend(self.member_item(&n, ty, kind, method, rank, m.span));
        }
    }

    /// `C.` of a class (its statics, inherited ones too, and its methods),
    /// an enum (its members) or a namespace (its members).
    fn static_members(&mut self, d: DefId, method: bool, out: &mut Vec<Item>) {
        match self.env.view(d).kind() {
            DefKind::Enum => {
                if method {
                    return;
                }
                let members = self.env.def(d).enum_members.clone();
                for (i, (n, _)) in members.iter().enumerate() {
                    let t = self.env.types.intern(Ty::EnumMember(d, i as u32));
                    out.push(Item {
                        import: None,
                        label: n.to_string(),
                        kind: Kind::EnumMember,
                        detail: self.env.display(t),
                        rank: 0,
                        target: None,
                        at: None,
                        doc: None,
                    });
                }
            }
            DefKind::Namespace => {
                let members: Vec<(Box<str>, TypeId, Option<Span>)> = self
                    .env
                    .def(d)
                    .members
                    .iter()
                    .map(|(n, m)| (n.clone(), m.ty, m.span))
                    .collect();
                for (n, ty, span) in members {
                    let kind = match self.env.value_kind(ty, false) {
                        Kind::Variable | Kind::Constant => Kind::Field,
                        k => k,
                    };
                    out.extend(self.member_item(&n, ty, kind, method, 0, span));
                }
            }
            _ => {
                let mut current = Some(d);
                let mut rank = 0;
                let mut seen: FxHashSet<DefId> = FxHashSet::default();
                while let Some(c) = current {
                    if !seen.insert(c) || seen.len() > 64 {
                        break;
                    }
                    let statics: Vec<(Box<str>, super::env::Member)> = self
                        .env
                        .def(c)
                        .statics
                        .iter()
                        .map(|(n, m)| (n.clone(), m.clone()))
                        .collect();
                    for (n, m) in statics {
                        if !self.visible(m.visibility, c) {
                            continue;
                        }
                        let kind = match m.kind {
                            MemberKind::Method => Kind::Method,
                            MemberKind::Getter | MemberKind::Setter => Kind::Property,
                            MemberKind::Field => match self.env.value_kind(m.ty, false) {
                                Kind::Function => Kind::Function,
                                _ => Kind::Field,
                            },
                        };
                        out.extend(self.member_item(&n, m.ty, kind, method, rank, m.span));
                    }
                    rank = 1;
                    current = self.env.view(c).bases().first().and_then(|&b| {
                        match self.env.types.get(b) {
                            Ty::Named(bd, _) => Some(*bd),
                            _ => None,
                        }
                    });
                }
                // instance methods through the class (`C.method(self, …)`)
                if !method {
                    let inst = self.env.instance(d);
                    let mut methods = Vec::new();
                    let args: Vec<TypeId> = match self.env.types.get(inst).clone() {
                        Ty::Named(_, a) => a,
                        _ => Vec::new(),
                    };
                    self.instance_members(d, &args, inst, false, &mut methods);
                    out.extend(
                        methods
                            .into_iter()
                            .filter(|i| i.kind == Kind::Method)
                            .map(|i| Item {
                                rank: i.rank.max(1),
                                ..i
                            }),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn names_match_the_letters_typed_from_a_word_start() {
        assert!(matches("", "anything"));
        assert!(matches("gsi", "GetSpellInfo"));
        assert!(matches("spell", "GetSpellInfo"));
        assert!(matches("info", "C_Spell_info"));
        assert!(!matches("pell", "GetSpellInfo"));
        // one letter: the start only
        assert!(matches("g", "GetTime"));
        assert!(!matches("s", "GetSpellInfo"));
    }
}
