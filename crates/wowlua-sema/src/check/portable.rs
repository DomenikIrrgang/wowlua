//! Types kept from one check to the next (the language server's first pass,
//! decision T375): written without the check's ids (named types by their
//! name, globally declared or a module's by its path; type parameters by
//! their place among the written ones) and read back into another check's
//! arena. Anything that cannot be written so (a function with enclosing
//! type parameters, a parameter no written function declares, a named type
//! with no name) gives `None`: the module is checked instead.

use super::env::Env;
use super::types::{DefId, Field, Fields, Func, Pack, Param, ParamId, Record, Rest, Ty, TypeId};
use rustc_hash::FxHashMap;

/// A named type, as another check finds it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DefKey {
    /// Declared globally (by declarations, or a namespace's `N.T`).
    Global(Box<str>),
    /// A module's own: its path (`src_rel`) and name.
    Module(Box<str>, Box<str>),
}

/// A written type; children are earlier nodes.
#[derive(Clone, Debug, PartialEq, Hash)]
enum Node {
    /// A type without ids (builtins and literals).
    Leaf(Ty),
    Union(Vec<u32>),
    Intersection(Vec<u32>),
    Array(u32),
    Map(u32, u32),
    Record {
        fields: Vec<(Box<str>, u32, bool, bool)>,
        indexer: Option<(u32, u32)>,
        open: bool,
    },
    Func(Box<FuncNode>),
    Named(DefKey, Vec<u32>),
    ClassOf(DefKey),
    ClassOfParam(u32),
    EventArgs(u32),
    KeyOf(u32),
    IndexOf(u32, u32),
    EnumMember(DefKey, u32),
    Param(u32),
    Secret(u32),
    Module(Box<str>),
}

#[derive(Clone, Debug, PartialEq, Hash)]
struct FuncNode {
    type_params: Vec<u32>,
    params: Vec<(Option<Box<str>>, u32, bool)>,
    rest: Option<RestNode>,
    ret: Vec<u32>,
    ret_rest: Option<RestNode>,
    predicate: Option<(usize, u32)>,
    unannotated: bool,
}

#[derive(Clone, Debug, PartialEq, Hash)]
enum RestNode {
    Variadic(u32),
    Pack(u32),
}

/// A written type parameter.
#[derive(Clone, Debug, PartialEq, Hash)]
struct ParamNode {
    name: Box<str>,
    pack: bool,
    constraint: Option<u32>,
}

/// A module's exports read back: by name, the types, and the counts.
pub(crate) type Exports = (FxHashMap<Box<str>, TypeId>, FxHashMap<Box<str>, usize>);

/// A module's exports, written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Portable {
    nodes: Vec<Node>,
    params: Vec<ParamNode>,
    exports: Vec<(Box<str>, u32)>,
    caps: Vec<(Box<str>, usize)>,
    /// The module's methods whose results its check inferred (L56): the
    /// class, the method, its type.
    results: Vec<(DefKey, Box<str>, u32)>,
}

/// How many nodes a module's exports may take before they are not kept (a
/// data module's tables are checked again rather than stored).
const BUDGET: usize = 200_000;

/// Writes types of `env`.
pub(crate) struct Writer<'a> {
    env: &'a Env<'a>,
    keys: &'a FxHashMap<DefId, DefKey>,
    /// Input index → its path, for module tables.
    paths: &'a [&'a str],
    out: Portable,
    memo: FxHashMap<TypeId, u32>,
    params: FxHashMap<ParamId, u32>,
    /// Parameters a written function declares.
    declared: Vec<bool>,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(
        env: &'a Env<'a>,
        keys: &'a FxHashMap<DefId, DefKey>,
        paths: &'a [&'a str],
    ) -> Self {
        Writer {
            env,
            keys,
            paths,
            out: Portable::default(),
            memo: FxHashMap::default(),
            params: FxHashMap::default(),
            declared: Vec::new(),
        }
    }

    /// A module's exports and their counts, written; `None` when a type
    /// cannot be.
    pub(crate) fn exports(
        mut self,
        exports: &FxHashMap<Box<str>, TypeId>,
        caps: &FxHashMap<Box<str>, usize>,
        results: &[(DefId, Box<str>, TypeId)],
    ) -> Option<Portable> {
        let mut names: Vec<(&Box<str>, &TypeId)> = exports.iter().collect();
        names.sort();
        for (name, &t) in names {
            let node = self.ty(t)?;
            self.out.exports.push((name.clone(), node));
        }
        for (d, name, t) in results {
            let key = self.keys.get(d)?.clone();
            let node = self.ty(*t)?;
            self.out.results.push((key, name.clone(), node));
        }
        // every parameter is one a written function declares
        if self.declared.iter().any(|d| !d) {
            return None;
        }
        let mut caps: Vec<(Box<str>, usize)> = caps.iter().map(|(n, &c)| (n.clone(), c)).collect();
        caps.sort();
        self.out.caps = caps;
        Some(self.out)
    }

    fn push(&mut self, node: Node) -> Option<u32> {
        if self.out.nodes.len() >= BUDGET {
            return None;
        }
        self.out.nodes.push(node);
        Some(self.out.nodes.len() as u32 - 1)
    }

    fn key(&self, d: DefId) -> Option<DefKey> {
        self.keys.get(&d).cloned()
    }

    fn param(&mut self, p: ParamId) -> Option<u32> {
        if let Some(&i) = self.params.get(&p) {
            return Some(i);
        }
        let info = self.env.params.get(p.0 as usize)?;
        let i = self.out.params.len() as u32;
        self.params.insert(p, i);
        self.out.params.push(ParamNode {
            name: info.name.clone(),
            pack: info.pack,
            constraint: None,
        });
        self.declared.push(false);
        if let Some(c) = info.constraint {
            let c = self.ty(c)?;
            self.out.params[i as usize].constraint = Some(c);
        }
        Some(i)
    }

    fn rest(&mut self, r: &Option<Rest>) -> Option<Option<RestNode>> {
        Some(match r {
            None => None,
            Some(Rest::Variadic(t)) => Some(RestNode::Variadic(self.ty(*t)?)),
            Some(Rest::Pack(p)) => Some(RestNode::Pack(self.param(*p)?)),
        })
    }

    fn ty(&mut self, t: TypeId) -> Option<u32> {
        if let Some(&n) = self.memo.get(&t) {
            return Some(n);
        }
        let node = match self.env.types.get(t).clone() {
            ty @ (Ty::Any
            | Ty::Unknown
            | Ty::Never
            | Ty::Error
            | Ty::Nil
            | Ty::Boolean
            | Ty::Number
            | Ty::String
            | Ty::Table
            | Ty::Function
            | Ty::Thread
            | Ty::Userdata
            | Ty::Bool(_)
            | Ty::Num(_)
            | Ty::Str(_)) => Node::Leaf(ty),
            Ty::Union(m) => Node::Union(m.iter().map(|&m| self.ty(m)).collect::<Option<_>>()?),
            Ty::Intersection(m) => {
                Node::Intersection(m.iter().map(|&m| self.ty(m)).collect::<Option<_>>()?)
            }
            Ty::Array(e) => Node::Array(self.ty(e)?),
            Ty::Map(k, v) => Node::Map(self.ty(k)?, self.ty(v)?),
            Ty::Record(r) => {
                let mut fields = Vec::with_capacity(r.fields.len());
                for (name, f) in r.fields.iter() {
                    fields.push((name.into(), self.ty(f.ty)?, f.optional, f.readonly));
                }
                let indexer = match r.indexer {
                    Some((k, v)) => Some((self.ty(k)?, self.ty(v)?)),
                    None => None,
                };
                Node::Record {
                    fields,
                    indexer,
                    open: r.open,
                }
            }
            Ty::Func(f) => {
                if !f.outer.is_empty() {
                    return None;
                }
                let mut type_params = Vec::with_capacity(f.type_params.len());
                for &p in &f.type_params {
                    let i = self.param(p)?;
                    self.declared[i as usize] = true;
                    type_params.push(i);
                }
                let mut params = Vec::with_capacity(f.params.len());
                for p in &f.params {
                    params.push((p.name.clone(), self.ty(p.ty)?, p.optional));
                }
                let rest = self.rest(&f.rest)?;
                let ret = f
                    .ret
                    .types
                    .iter()
                    .map(|&t| self.ty(t))
                    .collect::<Option<_>>()?;
                let ret_rest = self.rest(&f.ret.rest)?;
                let predicate = match f.predicate {
                    Some((i, t)) => Some((i, self.ty(t)?)),
                    None => None,
                };
                Node::Func(Box::new(FuncNode {
                    type_params,
                    params,
                    rest,
                    ret,
                    ret_rest,
                    predicate,
                    unannotated: f.unannotated,
                }))
            }
            Ty::Named(d, args) => Node::Named(
                self.key(d)?,
                args.iter().map(|&a| self.ty(a)).collect::<Option<_>>()?,
            ),
            Ty::ClassOf(d) => Node::ClassOf(self.key(d)?),
            Ty::ClassOfParam(p) => Node::ClassOfParam(self.param(p)?),
            Ty::EventArgs(e) => Node::EventArgs(self.ty(e)?),
            Ty::KeyOf(e) => Node::KeyOf(self.ty(e)?),
            Ty::IndexOf(o, k) => Node::IndexOf(self.ty(o)?, self.ty(k)?),
            Ty::EnumMember(d, i) => Node::EnumMember(self.key(d)?, i),
            Ty::Param(p) => Node::Param(self.param(p)?),
            Ty::Secret(s) => Node::Secret(self.ty(s)?),
            Ty::Module(m) => Node::Module((*self.paths.get(m as usize)?).into()),
        };
        let n = self.push(node)?;
        self.memo.insert(t, n);
        Some(n)
    }
}

impl Portable {
    /// The exports read into `env`: by name, the types, and the counts.
    /// `def` finds a named type, `module` an input by its path; `None` when
    /// one is not there (the module is checked instead).
    pub(crate) fn read(
        &self,
        env: &mut Env,
        def: &dyn Fn(&Env, &DefKey) -> Option<DefId>,
        module: &dyn Fn(&str) -> Option<u32>,
    ) -> Option<Exports> {
        // (looked up before anything is made: a missing one makes nothing)
        let mut defs: FxHashMap<&DefKey, DefId> = FxHashMap::default();
        for (k, _, _) in &self.results {
            if !defs.contains_key(k) {
                defs.insert(k, def(env, k)?);
            }
        }
        for node in &self.nodes {
            if let Node::Named(k, _) | Node::ClassOf(k) | Node::EnumMember(k, _) = node
                && !defs.contains_key(k)
            {
                defs.insert(k, def(env, k)?);
            }
            if let Node::Module(path) = node {
                module(path)?;
            }
        }
        let params: Vec<ParamId> = self
            .params
            .iter()
            .map(|p| env.new_param(&p.name, p.pack))
            .collect();
        let mut ids: Vec<TypeId> = Vec::with_capacity(self.nodes.len());
        let id = |ids: &Vec<TypeId>, n: u32| ids[n as usize];
        let rest = |ids: &Vec<TypeId>, r: &Option<RestNode>| match r {
            None => None,
            Some(RestNode::Variadic(t)) => Some(Rest::Variadic(ids[*t as usize])),
            Some(RestNode::Pack(p)) => Some(Rest::Pack(params[*p as usize])),
        };
        for node in &self.nodes {
            let t = match node {
                Node::Leaf(ty) => env.types.intern(ty.clone()),
                Node::Union(m) => {
                    let m: Vec<TypeId> = m.iter().map(|&n| id(&ids, n)).collect();
                    env.types.union(&m)
                }
                Node::Intersection(m) => env
                    .types
                    .intern(Ty::Intersection(m.iter().map(|&n| id(&ids, n)).collect())),
                Node::Array(e) => env.types.array(id(&ids, *e)),
                Node::Map(k, v) => env.types.intern(Ty::Map(id(&ids, *k), id(&ids, *v))),
                Node::Record {
                    fields,
                    indexer,
                    open,
                } => {
                    let mut fs = Fields::default();
                    for (name, t, optional, readonly) in fields {
                        fs.insert(
                            name.clone(),
                            Field {
                                ty: id(&ids, *t),
                                optional: *optional,
                                readonly: *readonly,
                            },
                        );
                    }
                    env.types.record(Record {
                        fields: fs,
                        indexer: indexer.map(|(k, v)| (id(&ids, k), id(&ids, v))),
                        open: *open,
                        id: 0,
                    })
                }
                Node::Func(f) => env.types.func(Func {
                    type_params: f.type_params.iter().map(|&p| params[p as usize]).collect(),
                    params: f
                        .params
                        .iter()
                        .map(|(name, t, optional)| Param {
                            name: name.clone(),
                            ty: id(&ids, *t),
                            optional: *optional,
                        })
                        .collect(),
                    rest: rest(&ids, &f.rest),
                    ret: Pack {
                        types: f.ret.iter().map(|&n| id(&ids, n)).collect(),
                        rest: rest(&ids, &f.ret_rest),
                    },
                    predicate: f.predicate.map(|(i, t)| (i, id(&ids, t))),
                    unannotated: f.unannotated,
                    outer: Vec::new(),
                }),
                Node::Named(k, args) => env.types.intern(Ty::Named(
                    defs[k],
                    args.iter().map(|&n| id(&ids, n)).collect(),
                )),
                Node::ClassOf(k) => env.types.intern(Ty::ClassOf(defs[k])),
                Node::ClassOfParam(p) => env.types.intern(Ty::ClassOfParam(params[*p as usize])),
                Node::EventArgs(e) => env.types.intern(Ty::EventArgs(id(&ids, *e))),
                Node::KeyOf(e) => env.types.intern(Ty::KeyOf(id(&ids, *e))),
                Node::IndexOf(o, k) => env.types.intern(Ty::IndexOf(id(&ids, *o), id(&ids, *k))),
                Node::EnumMember(k, i) => env.types.intern(Ty::EnumMember(defs[k], *i)),
                Node::Param(p) => env.types.intern(Ty::Param(params[*p as usize])),
                Node::Secret(s) => env.types.intern(Ty::Secret(id(&ids, *s))),
                Node::Module(path) => env.types.intern(Ty::Module(module(path)?)),
            };
            ids.push(t);
        }
        for (p, node) in params.iter().zip(&self.params) {
            if let Some(c) = node.constraint {
                env.params[p.0 as usize].constraint = Some(ids[c as usize]);
            }
        }
        let exports = self
            .exports
            .iter()
            .map(|(name, n)| (name.clone(), ids[*n as usize]))
            .collect();
        let caps = self.caps.iter().cloned().collect();
        // the methods' inferred results, as the module's check left them
        for (k, name, n) in &self.results {
            let d = defs[k];
            let t = ids[*n as usize];
            let def = env.def_mut(d);
            if let Some(m) = def
                .members
                .get_mut(name)
                .or_else(|| def.statics.get_mut(name))
            {
                m.ty = t;
            }
            if let Some(done) = env.results.get_mut(&d).and_then(|m| m.get_mut(name))
                && !*done
            {
                *done = true;
                env.inferred += 1;
            }
        }
        Some((exports, caps))
    }
}

/// Every named type of `env` with a name another check finds it by:
/// globally declared ones, and modules' own by their path.
pub(crate) fn def_keys(env: &Env, paths: &[&str]) -> FxHashMap<DefId, DefKey> {
    let mut keys: FxHashMap<DefId, DefKey> = FxHashMap::default();
    for (name, d) in env.declared.global_type_entries() {
        keys.entry(d).or_insert_with(|| DefKey::Global(name.into()));
    }
    for (&m, types) in &env.module_types {
        let Some(path) = paths.get(m) else { continue };
        for (name, &d) in types {
            keys.entry(d)
                .or_insert_with(|| DefKey::Module((*path).into(), name.clone()));
        }
    }
    keys
}

/// The named type `key` names in `env`.
pub(crate) fn find_def(env: &Env, key: &DefKey, index: &FxHashMap<&str, usize>) -> Option<DefId> {
    match key {
        DefKey::Global(name) => env.declared.global_type(name),
        DefKey::Module(path, name) => {
            let m = index.get(&**path)?;
            env.module_types.get(m)?.get(name).copied()
        }
    }
}

impl Writer<'_> {
    /// A fingerprint of module `m`'s interface: what an importer reads of
    /// it, its exports and the types it declares (their members, bases,
    /// bodies), without positions. `None` when a part cannot be written.
    pub(crate) fn interface(
        mut self,
        m: usize,
        exports: &FxHashMap<Box<str>, TypeId>,
        caps: &FxHashMap<Box<str>, usize>,
    ) -> Option<u64> {
        use std::hash::{BuildHasher, Hash, Hasher};
        let mut h = rustc_hash::FxBuildHasher.build_hasher();
        let mut names: Vec<(&Box<str>, &TypeId)> = exports.iter().collect();
        names.sort();
        for (name, &t) in names {
            (name, self.ty(t)?).hash(&mut h);
        }
        let mut counts: Vec<(&Box<str>, &usize)> = caps.iter().collect();
        counts.sort();
        counts.hash(&mut h);
        let env = self.env;
        let mut types: Vec<(&Box<str>, &DefId)> = env
            .module_types
            .get(&m)
            .map(|t| t.iter().collect())
            .unwrap_or_default();
        types.sort();
        for (name, &d) in types {
            let def = env.def_unrecorded(d);
            name.hash(&mut h);
            def.kind.hash(&mut h);
            for &p in &def.params {
                self.param(p)?.hash(&mut h);
            }
            for members in [&def.members, &def.statics] {
                members.len().hash(&mut h);
                for (n, mem) in members {
                    (n, self.ty(mem.ty)?, mem.kind, mem.optional, mem.readonly).hash(&mut h);
                    mem.visibility.hash(&mut h);
                }
            }
            for &b in &def.bases {
                self.ty(b)?.hash(&mut h);
            }
            def.alias.map(|a| self.ty(a)).hash(&mut h);
            for (n, t) in &def.enum_members {
                (n, self.ty(*t)?).hash(&mut h);
            }
            if let Some((k, v)) = def.indexer {
                (self.ty(k)?, self.ty(v)?).hash(&mut h);
            }
            (
                def.is_abstract,
                &def.widget,
                def.native,
                def.recursive_alias,
            )
                .hash(&mut h);
            def.self_param.map(|p| self.param(p)).hash(&mut h);
        }
        self.out.nodes.hash(&mut h);
        self.out.params.hash(&mut h);
        Some(h.finish())
    }
}

/// What a long-lived checker keeps of each module's check, by its path:
/// the exports, written, and what they were checked from: the module's
/// bytes, the package's context (declarations, globals), and the modules
/// it imports (directly or not), each by its interface's fingerprint (or,
/// where that is not known, its bytes). Typing in a function's body leaves
/// a module's interface as it was: what imports it is taken, not checked.
#[derive(Default)]
pub struct ModuleCache {
    entries: FxHashMap<Box<str>, ModuleEntry>,
    /// Each file's globals (what `@global` declares, what it assigns), by
    /// its path, with its bytes' hash.
    pub(crate) globals: FxHashMap<Box<str>, (u64, super::FileGlobals)>,
    /// Modules taken from the cache and checked, for tests.
    pub hits: usize,
    pub misses: usize,
}

struct ModuleEntry {
    hash: u64,
    deps: Vec<Dep>,
    context: u64,
    /// `None`: not kept (it cannot be written, or makes `typeof` aliases).
    exports: Option<Portable>,
    interface: Option<u64>,
}

/// A module an entry's check read, as it was then: its bytes, its
/// interface when it was checked before the entry's module, and the types
/// it declares (all an earlier module reads of a later one).
#[derive(Clone, Debug, PartialEq)]
struct Dep {
    path: Box<str>,
    bytes: u64,
    interface: Option<u64>,
    types: Option<u64>,
}

/// A check's use of the module cache.
pub struct ModuleReuse<'a> {
    pub cache: &'a mut ModuleCache,
    /// The inputs checked whatever is kept (the edited files); `None`:
    /// nothing is taken from the cache, only kept for later (a full check
    /// stays exact).
    pub take_except: Option<std::collections::HashSet<usize>>,
}

/// What the cache compares within one check.
pub(crate) struct Facts<'a> {
    pub paths: Vec<&'a str>,
    pub index: FxHashMap<&'a str, usize>,
    /// Each input's bytes, hashed (0 without a parse).
    pub hashes: Vec<u64>,
    pub context: u64,
    /// The inputs each input imports, directly or not.
    pub deps: Vec<Vec<usize>>,
    /// The interfaces of the modules checked or taken so far.
    pub interfaces: FxHashMap<usize, Option<u64>>,
    /// The fingerprints of the types modules declare, as asked for.
    types: FxHashMap<usize, Option<u64>>,
    /// Modules that make `typeof` aliases (before any check: resolving one
    /// takes it off the environment's list): always checked, as their
    /// check resolves them for their importers.
    typeof_modules: rustc_hash::FxHashSet<usize>,
}

impl<'a> Facts<'a> {
    pub(crate) fn new(
        env: &Env,
        files: &'a [crate::FileInput],
        analysis: &crate::Analysis,
        hashes: Vec<u64>,
        context: u64,
    ) -> Self {
        let paths: Vec<&str> = files.iter().map(|f| f.src_rel.as_str()).collect();
        let index = paths.iter().enumerate().map(|(i, &p)| (p, i)).collect();
        let direct: Vec<Vec<usize>> = analysis
            .modules
            .iter()
            .map(|m| {
                m.iter()
                    .flat_map(|m| &m.imports)
                    .filter_map(|i| match i.target {
                        crate::Target::Module(t) => Some(t),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        let deps = (0..files.len())
            .map(|i| {
                let mut seen = rustc_hash::FxHashSet::default();
                let mut todo: Vec<usize> = direct.get(i).cloned().unwrap_or_default();
                while let Some(d) = todo.pop() {
                    if d != i && seen.insert(d) {
                        todo.extend(direct.get(d).into_iter().flatten().copied());
                    }
                }
                let mut deps: Vec<usize> = seen.into_iter().collect();
                deps.sort_unstable();
                deps
            })
            .collect();
        Facts {
            paths,
            index,
            hashes,
            context,
            deps,
            interfaces: FxHashMap::default(),
            types: FxHashMap::default(),
            typeof_modules: env.typeof_aliases.values().copied().collect(),
        }
    }

    /// The fingerprint of the types module `d` declares.
    fn types_of(&mut self, env: &Env, keys: &FxHashMap<DefId, DefKey>, d: usize) -> Option<u64> {
        if let Some(&t) = self.types.get(&d) {
            return t;
        }
        let none = FxHashMap::default();
        let t = Writer::new(env, keys, &self.paths).interface(d, &none, &FxHashMap::default());
        self.types.insert(d, t);
        t
    }

    /// Input `i`'s imports as they are now.
    fn deps_of(&mut self, env: &Env, keys: &FxHashMap<DefId, DefKey>, i: usize) -> Vec<Dep> {
        let deps = self.deps[i].clone();
        deps.into_iter()
            .map(|d| Dep {
                path: self.paths[d].into(),
                bytes: self.hashes[d],
                interface: self.interfaces.get(&d).copied().flatten(),
                types: self.types_of(env, keys, d),
            })
            .collect()
    }

    /// Whether an entry was checked from what input `i` reads now: its
    /// bytes, the context, and each import as far as it read it: the
    /// interface of one checked before it, the declared types of one
    /// checked after it (its bytes where a fingerprint is not known).
    fn holds(
        &mut self,
        env: &Env,
        keys: &FxHashMap<DefId, DefKey>,
        entry: &ModuleEntry,
        i: usize,
    ) -> bool {
        if entry.hash != self.hashes[i]
            || entry.context != self.context
            || entry.deps.len() != self.deps[i].len()
        {
            return false;
        }
        let deps = self.deps[i].clone();
        entry.deps.iter().zip(deps).all(|(dep, d)| {
            *dep.path == *self.paths[d]
                && match self.interfaces.get(&d).copied() {
                    // checked (or taken) before: its interface
                    Some(now) => match (dep.interface, now) {
                        (Some(then), Some(now)) => then == now,
                        _ => dep.bytes == self.hashes[d],
                    },
                    // not yet: its declared types
                    None => match (dep.types, self.types_of(env, keys, d)) {
                        (Some(then), Some(now)) => dep.interface.is_none() && then == now,
                        _ => dep.bytes == self.hashes[d],
                    },
                }
        })
    }
}

/// Each input's bytes, hashed (0 without a parse).
pub(crate) fn hashes(files: &[crate::FileInput]) -> Vec<u64> {
    use std::hash::BuildHasher;
    files
        .iter()
        .map(|f| {
            f.parse
                .map_or(0, |p| rustc_hash::FxBuildHasher.hash_one(p.source()))
        })
        .collect()
}

impl ModuleCache {
    /// Input `i`'s exports and counts, read into `env`, when what they
    /// were checked from is as it is now.
    pub(crate) fn take(
        &mut self,
        env: &mut Env,
        facts: &mut Facts,
        keys: &FxHashMap<DefId, DefKey>,
        i: usize,
    ) -> Option<Exports> {
        if facts.typeof_modules.contains(&i) {
            return None;
        }
        let entry = self.entries.get(facts.paths[i])?;
        if !facts.holds(env, keys, entry, i) {
            return None;
        }
        let found = entry.exports.as_ref()?.read(
            env,
            &|env, k| find_def(env, k, &facts.index),
            &|path| facts.index.get(path).map(|&m| m as u32),
        )?;
        facts.interfaces.insert(i, entry.interface);
        self.hits += 1;
        Some(found)
    }

    /// Keeps input `i`'s exports, just checked, with its interface.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn keep(
        &mut self,
        env: &Env,
        facts: &mut Facts,
        keys: &FxHashMap<DefId, DefKey>,
        i: usize,
        exports: &FxHashMap<Box<str>, TypeId>,
        caps: &FxHashMap<Box<str>, usize>,
    ) {
        self.misses += 1;
        let interface = Writer::new(env, keys, &facts.paths).interface(i, exports, caps);
        // (the same as kept: written again only when it changed)
        let same = match self.entries.get(facts.paths[i]) {
            Some(e) => e.interface == interface && facts.holds(env, keys, e, i),
            None => false,
        };
        facts.interfaces.insert(i, interface);
        if same {
            return;
        }
        let written = (!facts.typeof_modules.contains(&i))
            .then(|| {
                // (the module's methods whose results its check inferred)
                let mut results: Vec<(DefId, Box<str>, TypeId)> = Vec::new();
                for &d in env
                    .module_types
                    .get(&i)
                    .into_iter()
                    .flat_map(|t| t.values())
                {
                    let def = env.def_unrecorded(d);
                    for (name, &done) in env.results.get(&d).into_iter().flatten() {
                        let member = def.members.get(name).or_else(|| def.statics.get(name));
                        if let (true, Some(m)) = (done, member) {
                            results.push((d, name.clone(), m.ty));
                        }
                    }
                }
                results.sort_by(|a, b| (a.0.0, &a.1).cmp(&(b.0.0, &b.1)));
                Writer::new(env, keys, &facts.paths).exports(exports, caps, &results)
            })
            .flatten();
        self.entries.insert(
            facts.paths[i].into(),
            ModuleEntry {
                hash: facts.hashes[i],
                deps: facts.deps_of(env, keys, i),
                context: facts.context,
                exports: written,
                interface,
            },
        );
    }
}

// ------------------------------------------------------------- on disk --

/// The format of [`ModuleCache::to_bytes`]; another number is not read.
const FORMAT: u32 = 1;

/// Writes the cache's parts as bytes: numbers as LEB128, strings with
/// their length.
#[derive(Default)]
struct Enc(Vec<u8>);

impl Enc {
    fn u(&mut self, mut n: u64) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                self.0.push(b);
                return;
            }
            self.0.push(b | 0x80);
        }
    }
    fn b(&mut self, b: bool) {
        self.0.push(u8::from(b));
    }
    fn s(&mut self, s: &str) {
        self.u(s.len() as u64);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn opt_u(&mut self, n: Option<u64>) {
        match n {
            Some(n) => {
                self.0.push(1);
                self.u(n);
            }
            None => self.0.push(0),
        }
    }
    fn ids(&mut self, ids: &[u32]) {
        self.u(ids.len() as u64);
        for &i in ids {
            self.u(u64::from(i));
        }
    }
    fn key(&mut self, k: &DefKey) {
        match k {
            DefKey::Global(n) => {
                self.0.push(0);
                self.s(n);
            }
            DefKey::Module(p, n) => {
                self.0.push(1);
                self.s(p);
                self.s(n);
            }
        }
    }
    fn rest(&mut self, r: &Option<RestNode>) {
        match r {
            None => self.0.push(0),
            Some(RestNode::Variadic(t)) => {
                self.0.push(1);
                self.u(u64::from(*t));
            }
            Some(RestNode::Pack(p)) => {
                self.0.push(2);
                self.u(u64::from(*p));
            }
        }
    }
    fn leaf(&mut self, t: &Ty) {
        let tag = match t {
            Ty::Any => 0,
            Ty::Unknown => 1,
            Ty::Never => 2,
            Ty::Error => 3,
            Ty::Nil => 4,
            Ty::Boolean => 5,
            Ty::Number => 6,
            Ty::String => 7,
            Ty::Table => 8,
            Ty::Function => 9,
            Ty::Thread => 10,
            Ty::Userdata => 11,
            Ty::Bool(b) => {
                self.0.push(12);
                self.b(*b);
                return;
            }
            Ty::Num(n) => {
                self.0.push(13);
                self.0.extend_from_slice(&n.to_le_bytes());
                return;
            }
            Ty::Str(s) => {
                self.0.push(14);
                self.s(s);
                return;
            }
            // (no other type is a leaf)
            _ => 0,
        };
        self.0.push(tag);
    }
    fn node(&mut self, n: &Node) {
        match n {
            Node::Leaf(t) => {
                self.0.push(0);
                self.leaf(t);
            }
            Node::Union(m) => {
                self.0.push(1);
                self.ids(m);
            }
            Node::Intersection(m) => {
                self.0.push(2);
                self.ids(m);
            }
            Node::Array(e) => {
                self.0.push(3);
                self.u(u64::from(*e));
            }
            Node::Map(k, v) => {
                self.0.push(4);
                self.u(u64::from(*k));
                self.u(u64::from(*v));
            }
            Node::Record {
                fields,
                indexer,
                open,
            } => {
                self.0.push(5);
                self.u(fields.len() as u64);
                for (name, t, optional, readonly) in fields {
                    self.s(name);
                    self.u(u64::from(*t));
                    self.b(*optional);
                    self.b(*readonly);
                }
                match indexer {
                    Some((k, v)) => {
                        self.0.push(1);
                        self.u(u64::from(*k));
                        self.u(u64::from(*v));
                    }
                    None => self.0.push(0),
                }
                self.b(*open);
            }
            Node::Func(f) => {
                self.0.push(6);
                self.ids(&f.type_params);
                self.u(f.params.len() as u64);
                for (name, t, optional) in &f.params {
                    match name {
                        Some(n) => {
                            self.0.push(1);
                            self.s(n);
                        }
                        None => self.0.push(0),
                    }
                    self.u(u64::from(*t));
                    self.b(*optional);
                }
                self.rest(&f.rest);
                self.ids(&f.ret);
                self.rest(&f.ret_rest);
                match f.predicate {
                    Some((i, t)) => {
                        self.0.push(1);
                        self.u(i as u64);
                        self.u(u64::from(t));
                    }
                    None => self.0.push(0),
                }
                self.b(f.unannotated);
            }
            Node::Named(k, args) => {
                self.0.push(7);
                self.key(k);
                self.ids(args);
            }
            Node::ClassOf(k) => {
                self.0.push(8);
                self.key(k);
            }
            Node::ClassOfParam(p) => {
                self.0.push(9);
                self.u(u64::from(*p));
            }
            Node::EventArgs(e) => {
                self.0.push(10);
                self.u(u64::from(*e));
            }
            Node::KeyOf(e) => {
                self.0.push(11);
                self.u(u64::from(*e));
            }
            Node::IndexOf(o, k) => {
                self.0.push(12);
                self.u(u64::from(*o));
                self.u(u64::from(*k));
            }
            Node::EnumMember(k, i) => {
                self.0.push(13);
                self.key(k);
                self.u(u64::from(*i));
            }
            Node::Param(p) => {
                self.0.push(14);
                self.u(u64::from(*p));
            }
            Node::Secret(s) => {
                self.0.push(15);
                self.u(u64::from(*s));
            }
            Node::Module(p) => {
                self.0.push(16);
                self.s(p);
            }
        }
    }
    fn portable(&mut self, p: &Portable) {
        self.u(p.nodes.len() as u64);
        for n in &p.nodes {
            self.node(n);
        }
        self.u(p.params.len() as u64);
        for q in &p.params {
            self.s(&q.name);
            self.b(q.pack);
            self.opt_u(q.constraint.map(u64::from));
        }
        self.u(p.exports.len() as u64);
        for (name, n) in &p.exports {
            self.s(name);
            self.u(u64::from(*n));
        }
        self.u(p.caps.len() as u64);
        for (name, c) in &p.caps {
            self.s(name);
            self.u(*c as u64);
        }
        self.u(p.results.len() as u64);
        for (k, name, n) in &p.results {
            self.key(k);
            self.s(name);
            self.u(u64::from(*n));
        }
    }
    fn names(&mut self, names: &Option<Vec<String>>) {
        match names {
            Some(v) => {
                self.0.push(1);
                self.u(v.len() as u64);
                for n in v {
                    self.s(n);
                }
            }
            None => self.0.push(0),
        }
    }
}

/// Reads what [`Enc`] wrote; `None` at anything else.
struct Dec<'a>(&'a [u8]);

impl Dec<'_> {
    fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }
    fn u(&mut self) -> Option<u64> {
        let mut n = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            n |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(n);
            }
        }
        None
    }
    fn u32(&mut self) -> Option<u32> {
        u32::try_from(self.u()?).ok()
    }
    /// A count, no more than the bytes left could hold (a damaged one
    /// allocates nothing).
    fn len(&mut self) -> Option<usize> {
        let n = usize::try_from(self.u()?).ok()?;
        (n <= self.0.len()).then_some(n)
    }
    fn b(&mut self) -> Option<bool> {
        match self.byte()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
    fn s(&mut self) -> Option<Box<str>> {
        let n = self.len()?;
        let (s, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(std::str::from_utf8(s).ok()?.into())
    }
    fn opt_u(&mut self) -> Option<Option<u64>> {
        match self.byte()? {
            0 => Some(None),
            1 => Some(Some(self.u()?)),
            _ => None,
        }
    }
    fn ids(&mut self) -> Option<Vec<u32>> {
        let n = self.len()?;
        (0..n).map(|_| self.u32()).collect()
    }
    fn key(&mut self) -> Option<DefKey> {
        match self.byte()? {
            0 => Some(DefKey::Global(self.s()?)),
            1 => Some(DefKey::Module(self.s()?, self.s()?)),
            _ => None,
        }
    }
    fn rest(&mut self) -> Option<Option<RestNode>> {
        match self.byte()? {
            0 => Some(None),
            1 => Some(Some(RestNode::Variadic(self.u32()?))),
            2 => Some(Some(RestNode::Pack(self.u32()?))),
            _ => None,
        }
    }
    fn leaf(&mut self) -> Option<Ty> {
        Some(match self.byte()? {
            0 => Ty::Any,
            1 => Ty::Unknown,
            2 => Ty::Never,
            3 => Ty::Error,
            4 => Ty::Nil,
            5 => Ty::Boolean,
            6 => Ty::Number,
            7 => Ty::String,
            8 => Ty::Table,
            9 => Ty::Function,
            10 => Ty::Thread,
            11 => Ty::Userdata,
            12 => Ty::Bool(self.b()?),
            13 => {
                if self.0.len() < 8 {
                    return None;
                }
                let (n, rest) = self.0.split_at(8);
                self.0 = rest;
                Ty::Num(u64::from_le_bytes(n.try_into().ok()?))
            }
            14 => Ty::Str(self.s()?),
            _ => return None,
        })
    }
    fn node(&mut self) -> Option<Node> {
        Some(match self.byte()? {
            0 => Node::Leaf(self.leaf()?),
            1 => Node::Union(self.ids()?),
            2 => Node::Intersection(self.ids()?),
            3 => Node::Array(self.u32()?),
            4 => Node::Map(self.u32()?, self.u32()?),
            5 => {
                let n = self.len()?;
                let mut fields = Vec::with_capacity(n);
                for _ in 0..n {
                    fields.push((self.s()?, self.u32()?, self.b()?, self.b()?));
                }
                let indexer = match self.byte()? {
                    0 => None,
                    1 => Some((self.u32()?, self.u32()?)),
                    _ => return None,
                };
                Node::Record {
                    fields,
                    indexer,
                    open: self.b()?,
                }
            }
            6 => {
                let type_params = self.ids()?;
                let n = self.len()?;
                let mut params = Vec::with_capacity(n);
                for _ in 0..n {
                    let name = match self.byte()? {
                        0 => None,
                        1 => Some(self.s()?),
                        _ => return None,
                    };
                    params.push((name, self.u32()?, self.b()?));
                }
                let rest = self.rest()?;
                let ret = self.ids()?;
                let ret_rest = self.rest()?;
                let predicate = match self.byte()? {
                    0 => None,
                    1 => Some((usize::try_from(self.u()?).ok()?, self.u32()?)),
                    _ => return None,
                };
                Node::Func(Box::new(FuncNode {
                    type_params,
                    params,
                    rest,
                    ret,
                    ret_rest,
                    predicate,
                    unannotated: self.b()?,
                }))
            }
            7 => Node::Named(self.key()?, self.ids()?),
            8 => Node::ClassOf(self.key()?),
            9 => Node::ClassOfParam(self.u32()?),
            10 => Node::EventArgs(self.u32()?),
            11 => Node::KeyOf(self.u32()?),
            12 => Node::IndexOf(self.u32()?, self.u32()?),
            13 => Node::EnumMember(self.key()?, self.u32()?),
            14 => Node::Param(self.u32()?),
            15 => Node::Secret(self.u32()?),
            16 => Node::Module(self.s()?),
            _ => return None,
        })
    }
    fn portable(&mut self) -> Option<Portable> {
        let n = self.len()?;
        let mut nodes = Vec::with_capacity(n);
        for _ in 0..n {
            nodes.push(self.node()?);
        }
        // (every child an earlier node, as the reader takes them)
        let fine = |i: u32, at: usize| (i as usize) < at;
        for (at, node) in nodes.iter().enumerate() {
            let ok = match node {
                Node::Union(m) | Node::Intersection(m) | Node::Named(_, m) => {
                    m.iter().all(|&i| fine(i, at))
                }
                Node::Array(e) | Node::EventArgs(e) | Node::KeyOf(e) | Node::Secret(e) => {
                    fine(*e, at)
                }
                Node::Map(k, v) | Node::IndexOf(k, v) => fine(*k, at) && fine(*v, at),
                Node::Record {
                    fields, indexer, ..
                } => {
                    fields.iter().all(|f| fine(f.1, at))
                        && indexer.is_none_or(|(k, v)| fine(k, at) && fine(v, at))
                }
                Node::Func(f) => {
                    let rest = |r: &Option<RestNode>| match r {
                        Some(RestNode::Variadic(t)) => fine(*t, at),
                        _ => true,
                    };
                    f.params.iter().all(|p| fine(p.1, at))
                        && f.ret.iter().all(|&t| fine(t, at))
                        && rest(&f.rest)
                        && rest(&f.ret_rest)
                        && f.predicate.is_none_or(|(_, t)| fine(t, at))
                }
                _ => true,
            };
            if !ok {
                return None;
            }
        }
        let n = self.len()?;
        let mut params = Vec::with_capacity(n);
        for _ in 0..n {
            params.push(ParamNode {
                name: self.s()?,
                pack: self.b()?,
                constraint: self.opt_u()?.map(u32::try_from).transpose().ok()?,
            });
        }
        let count = nodes.len();
        let p = params.len();
        let in_nodes = |i: u32| (i as usize) < count;
        let in_params = |i: u32| (i as usize) < p;
        // (every parameter a node names is one written)
        for node in &nodes {
            let ok = match node {
                Node::ClassOfParam(i) | Node::Param(i) => in_params(*i),
                Node::Func(f) => {
                    let pack = |r: &Option<RestNode>| match r {
                        Some(RestNode::Pack(q)) => in_params(*q),
                        _ => true,
                    };
                    f.type_params.iter().all(|&q| in_params(q))
                        && pack(&f.rest)
                        && pack(&f.ret_rest)
                }
                _ => true,
            };
            if !ok {
                return None;
            }
        }
        if params
            .iter()
            .any(|q| q.constraint.is_some_and(|c| !in_nodes(c)))
        {
            return None;
        }
        let n = self.len()?;
        let mut exports = Vec::with_capacity(n);
        for _ in 0..n {
            let name = self.s()?;
            let i = self.u32()?;
            in_nodes(i).then_some(())?;
            exports.push((name, i));
        }
        let n = self.len()?;
        let mut caps = Vec::with_capacity(n);
        for _ in 0..n {
            caps.push((self.s()?, usize::try_from(self.u()?).ok()?));
        }
        let n = self.len()?;
        let mut results = Vec::with_capacity(n);
        for _ in 0..n {
            let (k, name, i) = (self.key()?, self.s()?, self.u32()?);
            in_nodes(i).then_some(())?;
            results.push((k, name, i));
        }
        Some(Portable {
            nodes,
            params,
            exports,
            caps,
            results,
        })
    }
    fn names(&mut self) -> Option<Option<Vec<String>>> {
        match self.byte()? {
            0 => Some(None),
            1 => {
                let n = self.len()?;
                (0..n)
                    .map(|_| self.s().map(String::from))
                    .collect::<Option<Vec<_>>>()
                    .map(Some)
            }
            _ => None,
        }
    }
}

impl ModuleCache {
    /// The cache as bytes, for a later process (spec compiler §13.9 Phase
    /// 2): each module's entry and each file's globals. In path order, so
    /// the same cache gives the same bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut e = Enc::default();
        e.u(u64::from(FORMAT));
        let mut paths: Vec<&Box<str>> = self.entries.keys().collect();
        paths.sort();
        e.u(paths.len() as u64);
        for path in paths {
            let m = &self.entries[path];
            e.s(path);
            e.u(m.hash);
            e.u(m.context);
            e.opt_u(m.interface);
            e.u(m.deps.len() as u64);
            for d in &m.deps {
                e.s(&d.path);
                e.u(d.bytes);
                e.opt_u(d.interface);
                e.opt_u(d.types);
            }
            match &m.exports {
                Some(p) => {
                    e.0.push(1);
                    e.portable(p);
                }
                None => e.0.push(0),
            }
        }
        let mut files: Vec<&Box<str>> = self.globals.keys().collect();
        files.sort();
        e.u(files.len() as u64);
        for file in files {
            let (hash, g) = &self.globals[file];
            e.s(file);
            e.u(*hash);
            e.names(&g.decorated);
            e.names(&g.assigned);
        }
        e.0
    }

    /// A cache [`ModuleCache::to_bytes`] wrote; `None` for bytes of another
    /// format, or cut short or damaged.
    pub fn from_bytes(bytes: &[u8]) -> Option<ModuleCache> {
        let mut d = Dec(bytes);
        if d.u()? != u64::from(FORMAT) {
            return None;
        }
        let mut cache = ModuleCache::default();
        let n = d.len()?;
        for _ in 0..n {
            let path = d.s()?;
            let hash = d.u()?;
            let context = d.u()?;
            let interface = d.opt_u()?;
            let count = d.len()?;
            let mut deps = Vec::with_capacity(count);
            for _ in 0..count {
                deps.push(Dep {
                    path: d.s()?,
                    bytes: d.u()?,
                    interface: d.opt_u()?,
                    types: d.opt_u()?,
                });
            }
            let exports = match d.byte()? {
                0 => None,
                1 => Some(d.portable()?),
                _ => return None,
            };
            cache.entries.insert(
                path,
                ModuleEntry {
                    hash,
                    deps,
                    context,
                    exports,
                    interface,
                },
            );
        }
        let n = d.len()?;
        for _ in 0..n {
            let file = d.s()?;
            let hash = d.u()?;
            let globals = super::FileGlobals {
                decorated: d.names()?,
                assigned: d.names()?,
            };
            cache.globals.insert(file, (hash, globals));
        }
        d.0.is_empty().then_some(cache)
    }

    /// The modules kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
