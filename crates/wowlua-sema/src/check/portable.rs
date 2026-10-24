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
    ) -> Option<Portable> {
        let mut names: Vec<(&Box<str>, &TypeId)> = exports.iter().collect();
        names.sort();
        for (name, &t) in names {
            let node = self.ty(t)?;
            self.out.exports.push((name.clone(), node));
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
            .then(|| Writer::new(env, keys, &facts.paths).exports(exports, caps))
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
