//! Declarations (spec §2.9) and type resolution (spec §2.2): the global
//! type namespace and declared globals, the types of each module, classes,
//! interfaces, enums, aliases and namespaces, and the conversion of type
//! syntax into types.

use super::types::*;
use super::types::{Field, Param};
use crate::constant::Const;
use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;
use rustc_hash::FxHashMap;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::Span;
use wowlua_syntax::ast::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefKind {
    Class,
    Interface,
    Alias,
    Enum,
    Namespace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Field,
    Method,
    Getter,
    Setter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub ty: TypeId,
    pub kind: MemberKind,
    pub optional: bool,
    pub readonly: bool,
    pub visibility: Visibility,
    /// A private or protected method the `rename-private` minification
    /// pass may rename (spec optimization §15.4.1): not abstract, not
    /// decorated, not a metamethod, of a class that is no widget class.
    pub renamable: bool,
}

impl Member {
    fn field(ty: TypeId) -> Member {
        Member {
            ty,
            kind: MemberKind::Field,
            optional: false,
            readonly: false,
            visibility: Visibility::Public,
            renamable: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Def {
    pub name: String,
    pub kind: DefKind,
    pub params: Vec<ParamId>,
    /// Instance members (classes, interfaces) or namespace members.
    pub members: IndexMap<Box<str>, Member, FxBuildHasher>,
    /// Static members of a class (`new` included).
    pub statics: IndexMap<Box<str>, Member, FxBuildHasher>,
    /// The superclass (classes) or extended interfaces.
    pub bases: Vec<TypeId>,
    pub alias: Option<TypeId>,
    /// Enum members and their literal types.
    pub enum_members: Vec<(Box<str>, TypeId)>,
    pub is_abstract: bool,
    pub widget: Option<String>,
    /// Declared by a declaration (no wowlua class table).
    pub native: bool,
    pub indexer: Option<(TypeId, TypeId)>,
    pub span: Option<Span>,
}

impl Def {
    fn new(name: &str, kind: DefKind) -> Def {
        Def {
            name: name.to_string(),
            kind,
            params: Vec::new(),
            members: IndexMap::default(),
            statics: IndexMap::default(),
            bases: Vec::new(),
            alias: None,
            enum_members: Vec::new(),
            is_abstract: false,
            widget: None,
            native: false,
            indexer: None,
            span: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParamInfo {
    pub name: Box<str>,
    pub constraint: Option<TypeId>,
    pub pack: bool,
}

/// Where a type name is resolved.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    /// The module whose types are in scope (input index).
    pub module: Option<usize>,
    /// Type parameters, innermost last.
    pub params: Vec<(Box<str>, ParamId)>,
    /// `Self` in class and interface bodies.
    pub self_ty: Option<TypeId>,
}

/// One declaration source: a file and whether it is a module (its own
/// types) or global declarations.
pub struct Source<'a> {
    pub index: usize,
    pub parse: &'a wowlua_syntax::Parse,
    /// `.d.wowlua`: every top-level type is global.
    pub declaration: bool,
    /// Imported type names: local name → (module index, export name).
    pub imports: Vec<(Box<str>, usize, Box<str>)>,
    /// `import * as M`: local name → module index.
    pub namespaces: Vec<(Box<str>, usize)>,
    /// Folded enum values of the module.
    pub enums: FxHashMap<String, Vec<(String, Const)>>,
    /// A standard-library module's declaration file: the module it
    /// declares. Its functions and globals type that module's exports
    /// (as callers see them) instead of becoming globals.
    pub declares: Option<usize>,
    /// Globals of a primitive type that the parsed text leaves out (the
    /// WoW API): declared at the start of the file's turn.
    pub globals: wowlua_api::Globals,
}

#[derive(Default)]
pub struct Env {
    pub types: Types,
    /// Read through `def`, which records the read (`Declared::note_def`).
    defs: Vec<Def>,
    pub params: Vec<ParamInfo>,
    /// Types declared at the top level of each module.
    pub module_types: FxHashMap<usize, FxHashMap<Box<str>, DefId>>,
    /// Imported type names per module.
    pub module_imports: FxHashMap<usize, FxHashMap<Box<str>, DefId>>,
    pub module_namespaces: FxHashMap<usize, FxHashMap<Box<str>, usize>>,
    /// The global names the declarations give (values, types, handlers,
    /// on-demand globals, the callbacks registry), read through recording
    /// accessors.
    pub declared: super::declared::Declared,
    /// Export types from standard-library declaration files, by module.
    pub declared_exports: FxHashMap<usize, FxHashMap<Box<str>, TypeId>>,
    pub events: IndexMap<Box<str>, Pack, FxBuildHasher>,
    pub messages: IndexMap<Box<str>, Pack, FxBuildHasher>,
    pub diagnostics: Vec<Diagnostic>,
    /// Aliases whose bodies are not resolved yet (while building), with the
    /// module of their scope.
    lazy_aliases: FxHashMap<DefId, (TypeAlias, Option<usize>)>,
    /// Types of `typeof(expr)` expressions, by span (set by the checker).
    pub typeof_types: FxHashMap<(u32, u32), TypeId>,
    /// Name spans of the definitions of renamable methods, with their class.
    pub renamable_defs: Vec<(wowlua_span::Span, DefId)>,
}

pub const ADDON_NAMESPACE: &str = "AddonNamespace";

impl Env {
    pub fn def(&self, id: DefId) -> &Def {
        self.declared.note_def(id);
        &self.defs[id.0 as usize]
    }

    /// The number of declarations.
    pub fn def_count(&self) -> usize {
        self.defs.len()
    }

    /// A declaration, without recording the read (fingerprints, and the
    /// rename-private reachability after every file is checked).
    pub(crate) fn def_unrecorded(&self, id: DefId) -> &Def {
        &self.defs[id.0 as usize]
    }

    pub fn def_mut(&mut self, id: DefId) -> &mut Def {
        &mut self.defs[id.0 as usize]
    }

    fn add_def(&mut self, def: Def) -> DefId {
        let id = DefId(self.defs.len() as u32);
        self.defs.push(def);
        id
    }

    pub fn new_param(&mut self, name: &str, pack: bool) -> ParamId {
        let id = ParamId(self.params.len() as u32);
        self.params.push(ParamInfo {
            name: name.into(),
            constraint: None,
            pack,
        });
        id
    }

    /// The instance type of a class or interface without type arguments.
    pub fn instance(&mut self, def: DefId) -> TypeId {
        let params: Vec<TypeId> = self
            .def(def)
            .params
            .clone()
            .into_iter()
            .map(|p| self.types.intern(Ty::Param(p)))
            .collect();
        self.types.intern(Ty::Named(def, params))
    }

    /// Builds the environment from the declaration sources (declaration
    /// files first, then modules).
    pub fn build(sources: &[Source]) -> Env {
        let mut env = Env::default();
        // The ambient addon namespace: undeclared fields stay `any`.
        let mut ns = Def::new(ADDON_NAMESPACE, DefKind::Interface);
        ns.native = true;
        ns.indexer = Some((Types::STRING, Types::ANY));
        let ns_id = env.add_def(ns);
        env.declared.set_global_type(ADDON_NAMESPACE.into(), ns_id);

        // Pass 1: names, so declarations may refer to each other in any order.
        let mut pending: Vec<(DefId, Pending)> = Vec::new();
        for src in sources {
            for s in &src.parse.chunk.block.stmts {
                env.register_stmt(src, s, &mut pending);
            }
        }
        // `ns.__wowlua`: the runtime's table (its interface is declared in
        // the compiler's `runtime.d.wowlua`).
        if let Some(rt) = env.declared.global_type("WowluaRuntime") {
            let ty = env.types.intern(Ty::Named(rt, Vec::new()));
            let mut m = Member::field(ty);
            m.optional = true;
            env.def_mut(ns_id).members.insert("__wowlua".into(), m);
        }
        for src in sources {
            let mut imports = FxHashMap::default();
            for (local, module, export) in &src.imports {
                if let Some(&d) = env.module_types.get(module).and_then(|t| t.get(export)) {
                    imports.insert(local.clone(), d);
                }
            }
            env.module_imports.insert(src.index, imports);
            env.module_namespaces
                .insert(src.index, src.namespaces.iter().cloned().collect());
        }
        // Type parameters exist before any body refers to a definition
        // (a generic class of another module may be resolved later).
        for (def, p) in &pending {
            let tps: &[TypeParam] = match p {
                Pending::Alias(a, _) => &a.type_params,
                Pending::Interface(i, _) => &i.type_params,
                Pending::Class(c, _) => &c.type_params,
                Pending::DeclClass(tps, ..) => tps,
                _ => continue,
            };
            if env.def(*def).params.is_empty() && !tps.is_empty() {
                let ids = tps
                    .iter()
                    .map(|t| env.new_param(&t.name.text, t.pack))
                    .collect();
                env.def_mut(*def).params = ids;
            }
        }
        // Pass 2: bodies. An alias is resolved where it is first used if its
        // own turn has not come yet (aliases of other modules, too, refer to
        // each other in any order).
        for (def, p) in &pending {
            if let Pending::Alias(a, m) = p {
                let module = env.scope_of(*m, sources).module;
                env.lazy_aliases.insert(*def, ((*a).clone(), module));
            }
        }
        for (def, p) in &pending {
            if matches!(p, Pending::Alias(..)) {
                env.force_alias(*def);
            } else {
                env.resolve_pending(*def, p, sources);
            }
        }
        // Pass 3: global values, events and messages.
        for src in sources {
            env.collect_table_globals(src);
            for s in &src.parse.chunk.block.stmts {
                env.collect_values(src, s);
            }
        }
        env
    }

    fn register(&mut self, src: &Source, name: &str, def: Def, global: bool) -> DefId {
        let map_global = global || src.declaration;
        // Ambient interfaces merge (spec §2.4.7).
        if def.kind == DefKind::Interface
            && map_global
            && let Some(existing) = self.declared.global_type(name)
            && self.def(existing).kind == DefKind::Interface
        {
            return existing;
        }
        let id = self.add_def(def);
        if map_global {
            self.declared.set_global_type(name.into(), id);
        } else {
            self.module_types
                .entry(src.index)
                .or_default()
                .insert(name.into(), id);
        }
        id
    }

    fn register_stmt<'a>(
        &mut self,
        src: &Source<'a>,
        s: &'a Stmt,
        pending: &mut Vec<(DefId, Pending<'a>)>,
    ) {
        let kind = match &s.kind {
            StmtKind::Export(inner) => &inner.kind,
            k => k,
        };
        let widget = s.decorators.iter().find_map(widget_arg);
        match kind {
            StmtKind::TypeAlias(a) => {
                let id = self.register(
                    src,
                    &a.name.text,
                    Def::new(&a.name.text, DefKind::Alias),
                    false,
                );
                pending.push((id, Pending::Alias(a, src.index)));
            }
            StmtKind::Interface(i) => {
                let id = self.register(
                    src,
                    &i.name.text,
                    Def::new(&i.name.text, DefKind::Interface),
                    false,
                );
                pending.push((id, Pending::Interface(i, src.index)));
            }
            StmtKind::Class(c) => {
                let mut def = Def::new(&c.name.text, DefKind::Class);
                def.is_abstract = c.is_abstract;
                def.span = Some(c.name.span);
                let id = self.register(src, &c.name.text, def, false);
                pending.push((id, Pending::Class(c, src.index)));
            }
            StmtKind::Enum(e) => {
                let mut def = Def::new(&e.name.text, DefKind::Enum);
                def.span = Some(e.name.span);
                let id = self.register(src, &e.name.text, def, false);
                pending.push((id, Pending::Enum(e, src.index)));
            }
            StmtKind::Declare(d) => match d.as_ref() {
                Declare::Class {
                    name,
                    type_params,
                    extends,
                    members,
                    ..
                } => {
                    let mut def = Def::new(&name.text, DefKind::Class);
                    def.native = true;
                    def.widget = widget;
                    let id = self.register(src, &name.text, def, true);
                    pending.push((
                        id,
                        Pending::DeclClass(type_params, extends.as_ref(), members, src.index),
                    ));
                }
                Declare::Interface(i) => {
                    let mut def = Def::new(&i.name.text, DefKind::Interface);
                    def.native = true;
                    let id = self.register(src, &i.name.text, def, true);
                    pending.push((id, Pending::Interface(i, src.index)));
                }
                Declare::Namespace { name, members } => {
                    self.register_namespace(src, &name.text, members, pending);
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// A namespace and the types declared in it (`N.T`).
    fn register_namespace<'a>(
        &mut self,
        src: &Source<'a>,
        path: &str,
        members: &'a [NsMember],
        pending: &mut Vec<(DefId, Pending<'a>)>,
    ) -> DefId {
        let id = match self.declared.global_type(path) {
            Some(id) if self.def(id).kind == DefKind::Namespace => id,
            _ => {
                let mut def = Def::new(path, DefKind::Namespace);
                def.native = true;
                self.register(src, path, def, true)
            }
        };
        for m in members {
            let widget = m.decorators.iter().find_map(widget_arg);
            match &m.kind {
                NsMemberKind::Namespace { name, members } => {
                    let inner = format!("{path}.{}", name.text);
                    self.register_namespace(src, &inner, members, pending);
                }
                NsMemberKind::Class {
                    name,
                    type_params,
                    extends,
                    members,
                } => {
                    let full = format!("{path}.{}", name.text);
                    let mut def = Def::new(&full, DefKind::Class);
                    def.native = true;
                    def.widget = widget;
                    let cid = self.register(src, &full, def, true);
                    pending.push((
                        cid,
                        Pending::DeclClass(type_params, extends.as_ref(), members, src.index),
                    ));
                }
                NsMemberKind::Interface(i) => {
                    let full = format!("{path}.{}", i.name.text);
                    let iid = self.register(src, &full, Def::new(&full, DefKind::Interface), true);
                    pending.push((iid, Pending::Interface(i, src.index)));
                }
                NsMemberKind::TypeAlias(a) => {
                    let full = format!("{path}.{}", a.name.text);
                    let aid = self.register(src, &full, Def::new(&full, DefKind::Alias), true);
                    pending.push((aid, Pending::Alias(a, src.index)));
                }
                NsMemberKind::Enum(e) => {
                    let full = format!("{path}.{}", e.name.text);
                    let eid = self.register(src, &full, Def::new(&full, DefKind::Enum), true);
                    pending.push((eid, Pending::Enum(e, src.index)));
                }
                _ => {}
            }
        }
        pending.push((id, Pending::Namespace(members, src.index, path.to_string())));
        id
    }

    /// Resolves an alias's body now, if it is still pending (a cycle reads
    /// the alias as `any`).
    fn force_alias(&mut self, def: DefId) {
        let Some((a, module)) = self.lazy_aliases.remove(&def) else {
            return;
        };
        let mut scope = Scope {
            module,
            ..Scope::default()
        };
        self.def_type_params(def, &a.type_params, &mut scope);
        let body = self.resolve(&a.ty, &scope);
        self.def_mut(def).alias = Some(body);
    }

    fn scope_of(&self, module: usize, sources: &[Source]) -> Scope {
        let declaration = sources.iter().any(|s| s.index == module && s.declaration);
        Scope {
            module: (!declaration).then_some(module),
            ..Scope::default()
        }
    }

    fn resolve_pending(&mut self, def: DefId, p: &Pending, sources: &[Source]) {
        match p {
            Pending::Alias(a, m) => {
                let mut scope = self.scope_of(*m, sources);
                self.def_type_params(def, &a.type_params, &mut scope);
                let body = self.resolve(&a.ty, &scope);
                self.def_mut(def).alias = Some(body);
            }
            Pending::Interface(i, m) => {
                let mut scope = self.scope_of(*m, sources);
                self.def_type_params(def, &i.type_params, &mut scope);
                scope.self_ty = Some(self.instance(def));
                for e in &i.extends {
                    let t = self.resolve(e, &scope);
                    self.def_mut(def).bases.push(t);
                }
                for member in &i.members {
                    let (name, m) = match member {
                        InterfaceMember::Field {
                            readonly,
                            name,
                            optional,
                            ty,
                        } => {
                            let mut t = self.resolve(ty, &scope);
                            if *optional {
                                t = self.types.optional(t);
                            }
                            (
                                name,
                                Member {
                                    ty: t,
                                    kind: MemberKind::Field,
                                    optional: *optional,
                                    readonly: *readonly,
                                    visibility: Visibility::Public,
                                    renamable: false,
                                },
                            )
                        }
                        InterfaceMember::Method { name, sig } => {
                            let f = self.func_sig(sig, &scope, true);
                            let t = self.types.func(f);
                            (
                                name,
                                Member {
                                    ty: t,
                                    kind: MemberKind::Method,
                                    optional: false,
                                    readonly: true,
                                    visibility: Visibility::Public,
                                    renamable: false,
                                },
                            )
                        }
                    };
                    self.add_member(def, &name.text, m, false);
                }
            }
            Pending::Class(c, m) => self.resolve_class(def, c, *m, sources),
            Pending::DeclClass(type_params, extends, members, m) => {
                let mut scope = self.scope_of(*m, sources);
                self.def_type_params(def, type_params, &mut scope);
                scope.self_ty = Some(self.instance(def));
                if let Some(e) = extends {
                    let t = self.resolve(e, &scope);
                    self.def_mut(def).bases.push(t);
                }
                for member in members.iter() {
                    match &member.kind {
                        DeclMemberKind::Function { name, sig } => {
                            let f = self.func_sig(sig, &scope, true);
                            let t = self.types.func(f);
                            let kind = if member.is_static {
                                MemberKind::Field
                            } else {
                                MemberKind::Method
                            };
                            let m = Member {
                                ty: t,
                                kind,
                                optional: false,
                                readonly: true,
                                visibility: Visibility::Public,
                                renamable: false,
                            };
                            self.add_member(def, &name.text, m, member.is_static);
                        }
                        DeclMemberKind::Field {
                            readonly,
                            name,
                            optional,
                            ty,
                        } => {
                            let mut t = self.resolve(ty, &scope);
                            if *optional {
                                t = self.types.optional(t);
                            }
                            let m = Member {
                                ty: t,
                                kind: MemberKind::Field,
                                optional: *optional,
                                readonly: *readonly,
                                visibility: Visibility::Public,
                                renamable: false,
                            };
                            self.add_member(def, &name.text, m, member.is_static);
                        }
                    }
                }
            }
            Pending::Enum(e, m) => {
                let values: Vec<(String, Const)> = sources
                    .iter()
                    .find(|s| s.index == *m)
                    .and_then(|s| s.enums.get(e.name.text.as_ref()).cloned())
                    .unwrap_or_else(|| {
                        let mut d = Vec::new();
                        crate::classes::enum_info(e, &|_| None, &mut d).members
                    });
                let members: Vec<(Box<str>, TypeId)> = values
                    .into_iter()
                    .map(|(n, v)| {
                        let t = match v {
                            Const::Number(x) => self.types.num(x),
                            Const::String(s) => self.types.str(&String::from_utf8_lossy(&s)),
                            _ => Types::ERROR,
                        };
                        (n.into(), t)
                    })
                    .collect();
                self.def_mut(def).enum_members = members;
            }
            Pending::Namespace(members, m, _path) => {
                let scope = self.scope_of(*m, sources);
                for member in members.iter() {
                    match &member.kind {
                        NsMemberKind::Function { name, sig } => {
                            let f = self.func_sig(sig, &scope, true);
                            let t = self.types.func(f);
                            self.add_overload(def, &name.text, t);
                        }
                        NsMemberKind::Value { name, ty, is_const } => {
                            let t = self.resolve(ty, &scope);
                            let mut mem = Member::field(t);
                            mem.readonly = *is_const;
                            self.def_mut(def).members.insert(name.text.clone(), mem);
                        }
                        NsMemberKind::Namespace { name, .. } => {
                            let full = format!("{}.{}", self.def(def).name, name.text);
                            if let Some(inner) = self.declared.global_type(full.as_str()) {
                                let t = self.types.intern(Ty::ClassOf(inner));
                                let mut mem = Member::field(t);
                                mem.readonly = true;
                                self.def_mut(def).members.insert(name.text.clone(), mem);
                            }
                        }
                        NsMemberKind::Enum(e) => {
                            let full = format!("{}.{}", self.def(def).name, e.name.text);
                            if let Some(inner) = self.declared.global_type(full.as_str()) {
                                let t = self.types.intern(Ty::ClassOf(inner));
                                let mut mem = Member::field(t);
                                mem.readonly = true;
                                self.def_mut(def).members.insert(e.name.text.clone(), mem);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn add_member(&mut self, def: DefId, name: &str, m: Member, is_static: bool) {
        let target = if is_static {
            &mut self.def_mut(def).statics
        } else {
            &mut self.def_mut(def).members
        };
        match target.get_mut(name) {
            // Repeated function declarations form an overload set.
            Some(existing) if matches!(m.kind, MemberKind::Method) || existing.kind == m.kind => {
                let (a, b) = (existing.ty, m.ty);
                if matches!(m.kind, MemberKind::Method) {
                    let set = overload_set(&mut self.types, a, b);
                    let target = if is_static {
                        &mut self.defs[def.0 as usize].statics
                    } else {
                        &mut self.defs[def.0 as usize].members
                    };
                    target.get_mut(name).expect("present").ty = set;
                } else {
                    *existing = m;
                }
            }
            _ => {
                target.insert(name.into(), m);
            }
        }
    }

    fn add_overload(&mut self, def: DefId, name: &str, t: TypeId) {
        let existing = self.def(def).members.get(name).map(|m| m.ty);
        let ty = match existing {
            Some(e) => overload_set(&mut self.types, e, t),
            None => t,
        };
        let mut m = Member::field(ty);
        m.kind = MemberKind::Field;
        m.readonly = true;
        self.def_mut(def).members.insert(name.into(), m);
    }

    fn resolve_class(&mut self, def: DefId, c: &ClassDecl, module: usize, sources: &[Source]) {
        let declaration = sources
            .iter()
            .find(|s| s.index == module)
            .is_none_or(|s| s.declaration);
        let mut scope = self.scope_of(module, sources);
        self.def_type_params(def, &c.type_params, &mut scope);
        let instance = self.instance(def);
        scope.self_ty = Some(instance);
        if let Some(e) = &c.extends {
            let t = self.resolve(e, &scope);
            self.def_mut(def).bases.push(t);
            if let Ty::Named(b, _) = self.types.get(t).clone() {
                let w = self.def(b).widget.clone().or_else(|| {
                    // A widget class's superclass chain.
                    self.def(b)
                        .bases
                        .first()
                        .and_then(|&bt| match self.types.get(bt) {
                            Ty::Named(bb, _) => self.def(*bb).widget.clone(),
                            _ => None,
                        })
                });
                self.def_mut(def).widget = w;
            }
        }
        let mut ctor: Option<TypeId> = None;
        for m in &c.members {
            if m.modifiers.contains(&Modifier::Overload) {
                continue;
            }
            let is_static = m.modifiers.contains(&Modifier::Static);
            let visibility = if m.modifiers.contains(&Modifier::Private) {
                Visibility::Private
            } else if m.modifiers.contains(&Modifier::Protected) {
                Visibility::Protected
            } else {
                Visibility::Public
            };
            let readonly = m.modifiers.contains(&Modifier::Readonly);
            match &m.kind {
                ClassMemberKind::Field {
                    name, optional, ty, ..
                } => {
                    let mut t = self.resolve(ty, &scope);
                    if *optional {
                        t = self.types.optional(t);
                    }
                    let member = Member {
                        ty: t,
                        kind: MemberKind::Field,
                        optional: *optional,
                        readonly,
                        visibility,
                        renamable: false,
                    };
                    self.add_member(def, &name.text, member, is_static);
                }
                ClassMemberKind::Method { name, func, .. } => {
                    let f = self.func_sig(func, &scope, false);
                    let t = self.types.func(f);
                    let renamable = visibility != Visibility::Public
                        && !declaration
                        && self.def(def).widget.is_none()
                        && m.decorators.is_empty()
                        && !m.modifiers.contains(&Modifier::Abstract)
                        && !name.text.starts_with("__");
                    if renamable {
                        self.renamable_defs.push((name.span, def));
                    }
                    let member = Member {
                        ty: t,
                        kind: if is_static {
                            MemberKind::Field
                        } else {
                            MemberKind::Method
                        },
                        optional: false,
                        readonly: true,
                        visibility,
                        renamable,
                    };
                    self.add_member(def, &name.text, member, is_static);
                }
                ClassMemberKind::Getter { name, ty, .. } => {
                    let t = ty
                        .as_ref()
                        .map(|t| self.resolve(t, &scope))
                        .unwrap_or(Types::ANY);
                    let member = Member {
                        ty: t,
                        kind: MemberKind::Getter,
                        optional: false,
                        readonly: true,
                        visibility,
                        renamable: false,
                    };
                    let target = if is_static {
                        &mut self.def_mut(def).statics
                    } else {
                        &mut self.def_mut(def).members
                    };
                    match target.get_mut(name.text.as_ref()) {
                        // A setter declared first: the property is writable.
                        Some(existing) if existing.kind == MemberKind::Setter => {
                            existing.kind = MemberKind::Getter;
                            existing.readonly = false;
                            existing.ty = t;
                        }
                        _ => {
                            target.insert(name.text.clone(), member);
                        }
                    }
                }
                ClassMemberKind::Setter { name, param, .. } => {
                    let t = param
                        .ty
                        .as_ref()
                        .map(|t| self.resolve(t, &scope))
                        .unwrap_or(Types::ANY);
                    let target = if is_static {
                        &mut self.def_mut(def).statics
                    } else {
                        &mut self.def_mut(def).members
                    };
                    match target.get_mut(name.text.as_ref()) {
                        Some(existing) if existing.kind == MemberKind::Getter => {
                            existing.readonly = false
                        }
                        _ => {
                            target.insert(
                                name.text.clone(),
                                Member {
                                    ty: t,
                                    kind: MemberKind::Setter,
                                    optional: false,
                                    readonly: false,
                                    visibility,
                                    renamable: false,
                                },
                            );
                        }
                    }
                }
                ClassMemberKind::Constructor { func, .. } => {
                    let mut f = self.func_sig(func, &scope, false);
                    f.type_params = self.def(def).params.clone();
                    f.ret = Pack::single(instance);
                    ctor = Some(self.types.func(f));
                }
            }
        }
        // `new`: the constructor, or the superclass's with this class's
        // instance type (spec §3.1.2).
        let new = match ctor {
            Some(t) => t,
            None => {
                let base_new =
                    self.def(def)
                        .bases
                        .first()
                        .and_then(|&b| match self.types.get(b).clone() {
                            Ty::Named(bd, _) => self.def(bd).statics.get("new").map(|m| m.ty),
                            _ => None,
                        });
                let mut f = match base_new.map(|t| self.types.get(t).clone()) {
                    Some(Ty::Func(f)) => *f,
                    _ => Func {
                        // Root widget classes: (name?, parent?, template?, id?).
                        params: if self.def(def).widget.is_some() && self.def(def).bases.first().is_some_and(|&b| matches!(self.types.get(b), Ty::Named(bd, _) if self.def(*bd).native)) {
                            ["name", "parent", "template", "id"]
                                .iter()
                                .map(|n| Param {
                                    name: Some((*n).into()),
                                    ty: Types::ANY,
                                    optional: true,
                                })
                                .collect()
                        } else {
                            Vec::new()
                        },
                        ..Func::default()
                    },
                };
                f.type_params = self.def(def).params.clone();
                f.ret = Pack::single(instance);
                self.types.func(f)
            }
        };
        let mut m = Member::field(new);
        m.readonly = true;
        self.def_mut(def).statics.insert("new".into(), m);
    }

    /// The globals of `src`'s table, as `collect_values` declares the
    /// statements they stand for. No other declaration of the file writes
    /// the same names, so declaring them first gives the same result.
    fn collect_table_globals(&mut self, src: &Source) {
        debug_assert!(src.globals.is_empty() || src.declares.is_none());
        for g in src.globals.iter() {
            if let Some(addon) = g.on_demand {
                self.declared
                    .set_on_demand(g.name.into(), addon.to_string());
            }
            let t = match g.ty {
                wowlua_api::Prim::Any => Types::ANY,
                wowlua_api::Prim::Boolean => Types::BOOLEAN,
                wowlua_api::Prim::Number => Types::NUMBER,
                wowlua_api::Prim::String => Types::STRING,
                wowlua_api::Prim::Table => Types::TABLE,
            };
            self.declared.set_global(g.name.into(), t);
        }
    }

    fn collect_values(&mut self, src: &Source, s: &Stmt) {
        let StmtKind::Declare(d) = &s.kind else {
            return;
        };
        if let Some(addon) = s
            .decorators
            .iter()
            .find(|d| d.is_named("onDemand"))
            .and_then(
                |d| match d.args.as_ref()?.exprs().first().map(|e| &e.kind) {
                    Some(ExprKind::String(lit)) => {
                        Some(String::from_utf8_lossy(&lit.value).into_owned())
                    }
                    _ => None,
                },
            )
        {
            let name = match d.as_ref() {
                Declare::Global { name, .. } => Some(name.text.clone()),
                Declare::Function { name, .. } if name.path.len() == 1 => {
                    Some(name.path[0].text.clone())
                }
                _ => None,
            };
            if let Some(name) = name {
                self.declared.set_on_demand(name, addon);
            }
        }
        let scope = self.scope_of(src.index, &[]);
        let scope = Scope {
            module: (!src.declaration).then_some(src.index).or(scope.module),
            ..Scope::default()
        };
        if let Some(module) = src.declares {
            let (name, t) = match d.as_ref() {
                Declare::Function { name, sig }
                    if name.path.len() == 1 && name.method.is_none() =>
                {
                    let f = self.func_sig(sig, &scope, true);
                    (name.path[0].text.clone(), self.types.func(f))
                }
                Declare::Global { name, ty } => (name.text.clone(), self.resolve(ty, &scope)),
                _ => return,
            };
            self.declared_exports
                .entry(module)
                .or_default()
                .insert(name, t);
            return;
        }
        match d.as_ref() {
            Declare::Function { name, sig } => {
                let f = self.func_sig(sig, &scope, true);
                let t = self.types.func(f);
                if name.path.len() == 1 && name.method.is_none() {
                    let key: Box<str> = name.path[0].text.clone();
                    let ty = match self.declared.global(&key) {
                        Some(e) => overload_set(&mut self.types, e, t),
                        None => t,
                    };
                    self.declared.set_global(key, ty);
                }
            }
            Declare::Global { name, ty } => {
                let t = self.resolve(ty, &scope);
                self.declared.set_global(name.text.clone(), t);
            }
            Declare::Namespace { name, .. } => {
                if let Some(def) = self.declared.global_type(name.text.as_ref()) {
                    let t = self.types.intern(Ty::ClassOf(def));
                    self.declared.set_global(name.text.clone(), t);
                }
            }
            Declare::Event { name, params } | Declare::Message { name, params } => {
                let f = self.func_sig(params, &scope, true);
                let pack = Pack {
                    types: f.params.iter().map(|p| p.ty).collect(),
                    rest: f.rest.clone(),
                };
                self.declared.set_handler(
                    name.text.clone(),
                    Func {
                        ret: Pack::any(),
                        ..f
                    },
                );
                if matches!(d.as_ref(), Declare::Event { .. }) {
                    self.events.insert(name.text.clone(), pack);
                } else {
                    self.messages.insert(name.text.clone(), pack);
                }
            }
            _ => {}
        }
    }

    /// The type parameters of a definition (created before pass 2) in
    /// scope, with their constraints resolved.
    fn def_type_params(&mut self, def: DefId, params: &[TypeParam], scope: &mut Scope) {
        let ids = self.def(def).params.clone();
        if ids.len() != params.len() {
            let ids = self.type_params(params, scope);
            if self.def(def).params.is_empty() {
                self.def_mut(def).params = ids;
            }
            return;
        }
        for (p, &id) in params.iter().zip(&ids) {
            scope.params.push((p.name.text.clone(), id));
        }
        for (p, &id) in params.iter().zip(&ids) {
            if let Some(c) = &p.constraint {
                let t = self.resolve(c, scope);
                self.params[id.0 as usize].constraint = Some(t);
            }
        }
    }

    fn type_params(&mut self, params: &[TypeParam], scope: &mut Scope) -> Vec<ParamId> {
        let mut out = Vec::new();
        for p in params {
            let id = self.new_param(&p.name.text, p.pack);
            scope.params.push((p.name.text.clone(), id));
            out.push(id);
        }
        for (p, &id) in params.iter().zip(&out) {
            if let Some(c) = &p.constraint {
                let t = self.resolve(c, scope);
                self.params[id.0 as usize].constraint = Some(t);
            }
        }
        out
    }

    /// The signature of a function body. `declared`: a declaration (no
    /// body); otherwise unannotated parameters are `any` and an omitted
    /// return type is inferred by the checker (`...any` here).
    pub fn func_sig(&mut self, f: &FuncBody, scope: &Scope, declared: bool) -> Func {
        let mut scope = scope.clone();
        let type_params = self.type_params(&f.type_params, &mut scope);
        let mut unannotated = false;
        let params: Vec<Param> = f
            .params
            .iter()
            .map(|p| {
                let ty = match &p.ty {
                    Some(t) => self.resolve(t, &scope),
                    None => {
                        unannotated = true;
                        Types::ANY
                    }
                };
                Param {
                    name: Some(p.name.text.clone()),
                    ty,
                    optional: p.default.is_some() || self.types.is_optional(ty),
                }
            })
            .collect();
        let rest = f.vararg.as_ref().map(|v| match &v.ty {
            None => Rest::Variadic(Types::ANY),
            Some(VarargType::Type(t)) => Rest::Variadic(self.resolve(t, &scope)),
            Some(VarargType::Pack(n)) => {
                match scope.params.iter().rev().find(|(name, _)| name == &n.text) {
                    Some((_, id)) => Rest::Pack(*id),
                    None => Rest::Variadic(Types::ANY),
                }
            }
            Some(VarargType::Expand(_)) => Rest::Variadic(Types::ANY),
        });
        let (ret, predicate) = match &f.ret {
            None if declared => (Pack::empty(), None),
            None => (Pack::any(), None),
            Some(r) => self.ret_type(r, &scope, &params),
        };
        // A function without any annotation is unannotated too.
        let bare = f.params.iter().all(|p| p.ty.is_none())
            && f.ret.is_none()
            && f.vararg.as_ref().is_none_or(|v| v.ty.is_none());
        Func {
            type_params,
            params,
            rest,
            ret,
            predicate,
            unannotated: (unannotated || bare) && !declared,
        }
    }

    fn ret_type(
        &mut self,
        r: &RetType,
        scope: &Scope,
        params: &[Param],
    ) -> (Pack, Option<(usize, TypeId)>) {
        match r {
            RetType::Single(t) => (Pack::single(self.resolve(t, scope)), None),
            RetType::List { types, vararg, .. } => {
                let types = types.iter().map(|t| self.resolve(t, scope)).collect();
                let rest = vararg.as_deref().map(|v| self.vararg(v, scope));
                (Pack { types, rest }, None)
            }
            RetType::Pack(n) => match scope.params.iter().rev().find(|(name, _)| name == &n.text) {
                Some((_, id)) => (
                    Pack {
                        types: Vec::new(),
                        rest: Some(Rest::Pack(*id)),
                    },
                    None,
                ),
                None => (Pack::any(), None),
            },
            RetType::Predicate { name, ty } => {
                let t = self.resolve(ty, scope);
                let index = params
                    .iter()
                    .position(|p| p.name.as_deref() == Some(name.text.as_ref()));
                (Pack::single(Types::BOOLEAN), index.map(|i| (i, t)))
            }
        }
    }

    fn vararg(&mut self, v: &VarargType, scope: &Scope) -> Rest {
        match v {
            VarargType::Type(t) => Rest::Variadic(self.resolve(t, scope)),
            VarargType::Pack(n) => {
                match scope.params.iter().rev().find(|(name, _)| name == &n.text) {
                    Some((_, id)) => Rest::Pack(*id),
                    None => Rest::Variadic(Types::ANY),
                }
            }
            VarargType::Expand(_) => Rest::Variadic(Types::ANY),
        }
    }

    /// Looks up a type name in a scope: type parameters, the module's own
    /// and imported types, then declared types.
    pub fn lookup_type(&self, name: &str, scope: &Scope) -> Option<DefId> {
        if let Some(m) = scope.module {
            if let Some(&d) = self.module_types.get(&m).and_then(|t| t.get(name)) {
                return Some(d);
            }
            if let Some(&d) = self.module_imports.get(&m).and_then(|t| t.get(name)) {
                return Some(d);
            }
        }
        self.declared.global_type(name)
    }

    /// Converts type syntax into a type; unknown names are E2002.
    pub fn resolve(&mut self, t: &Type, scope: &Scope) -> TypeId {
        match &t.kind {
            TypeKind::Nil => Types::NIL,
            TypeKind::True => Types::TRUE,
            TypeKind::False => Types::FALSE,
            TypeKind::AnyFunction => Types::FUNCTION,
            TypeKind::String(s) => self.types.str(&String::from_utf8_lossy(&s.value)),
            TypeKind::Number(n) => match n.value {
                Some(v) => self.types.num(v),
                None => Types::NUMBER,
            },
            TypeKind::Paren(inner) => self.resolve(inner, scope),
            TypeKind::Optional(inner) => {
                let i = self.resolve(inner, scope);
                self.types.optional(i)
            }
            TypeKind::Array(inner) => {
                let i = self.resolve(inner, scope);
                self.types.array(i)
            }
            TypeKind::Union(ms) => {
                let ms: Vec<TypeId> = ms.iter().map(|m| self.resolve(m, scope)).collect();
                self.types.union(&ms)
            }
            TypeKind::Intersection(ms) => {
                let ms: Vec<TypeId> = ms.iter().map(|m| self.resolve(m, scope)).collect();
                self.types.intern(Ty::Intersection(ms))
            }
            TypeKind::Table(fields) => self.table_type(fields, scope),
            TypeKind::Function(f) => {
                let mut scope = scope.clone();
                let type_params = self.type_params(&f.type_params, &mut scope);
                let params: Vec<Param> = f
                    .params
                    .iter()
                    .map(|p| {
                        let ty = self.resolve(&p.ty, &scope);
                        Param {
                            name: p.name.as_ref().map(|n| n.text.clone()),
                            ty,
                            optional: self.types.is_optional(ty),
                        }
                    })
                    .collect();
                let rest = f.vararg.as_ref().map(|v| self.vararg(v, &scope));
                let (ret, predicate) = self.ret_type(&f.ret, &scope, &params);
                self.types.func(Func {
                    type_params,
                    params,
                    rest,
                    ret,
                    predicate,
                    unannotated: false,
                })
            }
            // Resolved by the checker (needs expression types).
            TypeKind::Typeof(e) => self
                .typeof_types
                .get(&(e.span.start, e.span.end))
                .copied()
                .unwrap_or(Types::ANY),
            TypeKind::Error => Types::ERROR,
            TypeKind::Ref { path, args } => self.resolve_ref(path, args, t.span, scope),
        }
    }

    fn table_type(&mut self, fields: &[TableTypeField], scope: &Scope) -> TypeId {
        let mut record = Record::default();
        let mut array = None;
        for f in fields {
            match f {
                TableTypeField::Named {
                    name,
                    optional,
                    readonly,
                    ty,
                } => {
                    let mut t = self.resolve(ty, scope);
                    if *optional {
                        t = self.types.optional(t);
                    }
                    record.fields.insert(
                        name.text.clone(),
                        Field {
                            ty: t,
                            optional: *optional,
                            readonly: *readonly,
                        },
                    );
                }
                TableTypeField::NamedString { key, ty, .. } => {
                    let t = self.resolve(ty, scope);
                    record.fields.insert(
                        String::from_utf8_lossy(&key.value).into(),
                        Field {
                            ty: t,
                            optional: false,
                            readonly: false,
                        },
                    );
                }
                TableTypeField::Indexer { key, value } => {
                    let k = self.resolve(key, scope);
                    let v = self.resolve(value, scope);
                    record.indexer = Some((k, v));
                }
                TableTypeField::Array(t) => array = Some(self.resolve(t, scope)),
            }
        }
        match (array, record.fields.is_empty(), record.indexer) {
            (Some(a), true, None) => self.types.array(a),
            (None, true, Some((k, v))) => self.types.intern(Ty::Map(k, v)),
            (Some(a), _, None) => {
                record.indexer = Some((Types::NUMBER, a));
                self.types.record(record)
            }
            _ => self.types.record(record),
        }
    }

    fn resolve_ref(
        &mut self,
        path: &[Name],
        args: &[TypeArg],
        span: Span,
        scope: &Scope,
    ) -> TypeId {
        let arg_types: Vec<TypeId> = args
            .iter()
            .map(|a| match a {
                TypeArg::Type(t) => self.resolve(t, scope),
                TypeArg::Pack(_) => Types::ANY,
            })
            .collect();
        if path.len() == 1 {
            let name = path[0].text.as_ref();
            if let Some((_, id)) = scope.params.iter().rev().find(|(n, _)| n.as_ref() == name) {
                return self.types.intern(Ty::Param(*id));
            }
            let builtin = match name {
                "any" => Some(Types::ANY),
                "unknown" => Some(Types::UNKNOWN),
                "never" => Some(Types::NEVER),
                "boolean" => Some(Types::BOOLEAN),
                "number" => Some(Types::NUMBER),
                "string" => Some(Types::STRING),
                "table" => Some(Types::TABLE),
                "thread" => Some(Types::THREAD),
                "userdata" => Some(Types::USERDATA),
                _ => None,
            };
            if let Some(b) = builtin {
                return b;
            }
            match name {
                "Self" => return scope.self_ty.unwrap_or(Types::ANY),
                "Secret" => {
                    let inner = arg_types.first().copied().unwrap_or(Types::UNKNOWN);
                    return self.types.intern(Ty::Secret(inner));
                }
                "ClassOf" => {
                    return match arg_types.first().map(|&a| self.types.get(a).clone()) {
                        Some(Ty::Named(d, _)) => self.types.intern(Ty::ClassOf(d)),
                        _ => Types::TABLE,
                    };
                }
                "EventName" => {
                    self.declared.note_opaque();
                    let names: Vec<TypeId> = self
                        .events
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .iter()
                        .map(|n| self.types.str(n))
                        .collect();
                    return if names.is_empty() {
                        Types::STRING
                    } else {
                        self.types.union(&names)
                    };
                }
                "MessageName" => {
                    self.declared.note_opaque();
                    let names: Vec<TypeId> = self
                        .messages
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .iter()
                        .map(|n| self.types.str(n))
                        .collect();
                    return if names.is_empty() {
                        Types::STRING
                    } else {
                        self.types.union(&names)
                    };
                }
                "Thread" => return Types::THREAD,
                "EventArgs" => return Types::ANY,
                _ => {}
            }
        }
        // `M.Type` through `import * as M`, or a namespace type `N.T`.
        let def = if path.len() == 2
            && let Some(m) = scope
                .module
                .and_then(|m| self.module_namespaces.get(&m))
                .and_then(|ns| ns.get(path[0].text.as_ref()))
                .copied()
        {
            self.module_types
                .get(&m)
                .and_then(|t| t.get(path[1].text.as_ref()))
                .copied()
        } else {
            let full: Vec<&str> = path.iter().map(|n| n.text.as_ref()).collect();
            let joined = full.join(".");
            if path.len() == 1 {
                self.lookup_type(&joined, scope)
            } else {
                self.declared.global_type(joined.as_str())
            }
        };
        let Some(def) = def else {
            let joined: Vec<&str> = path.iter().map(|n| n.text.as_ref()).collect();
            self.diagnostics.push(
                Diagnostic::error("E2002", format!("unknown type `{}`", joined.join(".")))
                    .with_label(span, "not declared"),
            );
            return Types::ERROR;
        };
        let expected = self.def(def).params.len();
        if !args.is_empty() && args.len() != expected {
            self.diagnostics.push(
                Diagnostic::error(
                    "E2013",
                    format!(
                        "`{}` takes {expected} type argument{}, but {} were given",
                        self.def(def).name,
                        if expected == 1 { "" } else { "s" },
                        args.len()
                    ),
                )
                .with_label(span, "here"),
            );
        }
        // Missing type arguments are `any`.
        let mut full_args = arg_types;
        full_args.resize(expected, Types::ANY);
        match self.def(def).kind {
            DefKind::Alias => {
                if self.def(def).alias.is_none() {
                    self.force_alias(def);
                }
                let body = self.def(def).alias.unwrap_or(Types::ANY);
                if expected == 0 {
                    body
                } else {
                    let params = self.def(def).params.clone();
                    super::subst::substitute(&mut self.types, body, &params, &full_args)
                }
            }
            DefKind::Namespace => self.types.intern(Ty::ClassOf(def)),
            _ => self.types.intern(Ty::Named(def, full_args)),
        }
    }
}

enum Pending<'a> {
    Alias(&'a TypeAlias, usize),
    Interface(&'a InterfaceDecl, usize),
    Class(&'a ClassDecl, usize),
    DeclClass(&'a [TypeParam], Option<&'a Type>, &'a [DeclMember], usize),
    Enum(&'a EnumDecl, usize),
    Namespace(&'a [NsMember], usize, String),
}

fn widget_arg(d: &Decorator) -> Option<String> {
    if !d.is_named("widget") {
        return None;
    }
    match d.args.as_ref()?.exprs().first().map(|e| &e.kind) {
        Some(ExprKind::String(s)) => Some(String::from_utf8_lossy(&s.value).into_owned()),
        _ => None,
    }
}

/// `a` and `b` as one overload set (in declaration order).
pub fn overload_set(types: &mut Types, a: TypeId, b: TypeId) -> TypeId {
    let mut members = match types.get(a) {
        Ty::Intersection(m) => m.clone(),
        _ => vec![a],
    };
    members.push(b);
    types.intern(Ty::Intersection(members))
}
