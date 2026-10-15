//! Reusing a plain Lua file's type check in a later flavor (#6). A file is
//! checked once with its reads recorded; a later flavor reuses the results
//! when everything the file read of the declarations is the same there, as
//! a canonical fingerprint shows: a walk from the names it looked up that
//! writes every type and declaration it reaches, with labels instead of the
//! flavor's ids.

use super::FileSettings;
use super::declared::{Peeked, Root};
use super::env::{Def, Env, Member, ParamInfo};
use super::types::{DefId, Field, Func, Pack, ParamId, Record, Rest, Ty, TypeId, Unborn};
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use std::collections::{BTreeSet, VecDeque};
use wowlua_diagnostics::Diagnostic;
use wowlua_span::{FileId, Span};

/// What later flavors of one build can reuse.
#[derive(Default)]
pub struct CheckCache {
    /// By the file's path and `FileId` (within one build, the same bytes;
    /// the input index can differ between flavors); one entry per distinct
    /// fingerprint (at most `MAX_ENTRIES`).
    entries: FxHashMap<(Box<str>, FileId), Vec<CheckEntry>>,
    /// The package's extra and assigned globals of the flavor that stored
    /// entries; a flavor with other ones reuses nothing.
    package: Option<(FxHashSet<String>, FxHashSet<String>)>,
    /// Files reused and checked, for tests.
    pub hits: usize,
    pub misses: usize,
}

const MAX_ENTRIES: usize = 3;

struct CheckEntry {
    settings: FileSettings,
    roots: Vec<Root>,
    /// The labels of the declarations whose contents the file read.
    read: BTreeSet<u32>,
    fingerprint: Vec<u8>,
    /// What the order of its unions depends on.
    unborn: Unborn,
    diagnostics: Vec<Diagnostic>,
    type_names: Vec<(Span, String)>,
}

/// The cache of a check, and whether to store what it checks.
pub struct Reuse<'a> {
    pub cache: &'a mut CheckCache,
    pub store: bool,
}

/// What a reused file gives.
pub(crate) struct Reused {
    pub diagnostics: Vec<Diagnostic>,
    pub type_names: Vec<(Span, String)>,
}

impl CheckCache {
    /// Whether this flavor's package globals allow reuse.
    pub(crate) fn package_matches(
        &self,
        extra: &FxHashSet<String>,
        assigned: &FxHashSet<String>,
    ) -> bool {
        self.package
            .as_ref()
            .is_some_and(|(e, a)| e == extra && a == assigned)
    }

    /// A stored check of `file` whose reads give the same answers in `env`.
    pub(crate) fn lookup(
        &self,
        env: &Env,
        file: (&str, FileId),
        settings: FileSettings,
    ) -> Option<Reused> {
        let entry = self
            .entries
            .get(&(file.0.into(), file.1))?
            .iter()
            .find(|e| {
                e.settings == settings
                    && env.types.unborn_holds(&e.unborn)
                    && fingerprint(env, &e.roots, Expand::Labels(&e.read))
                        .is_some_and(|(bytes, _)| bytes == e.fingerprint)
            })?;
        Some(Reused {
            diagnostics: entry.diagnostics.clone(),
            type_names: entry.type_names.clone(),
        })
    }

    /// Stores the check of `file`: `reads` is what it read of `env`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn store(
        &mut self,
        env: &Env,
        file: (&str, FileId),
        settings: FileSettings,
        reads: &super::declared::Reads,
        unborn: Unborn,
        diagnostics: &[Diagnostic],
        type_names: &[(Span, String)],
        package: (&FxHashSet<String>, &FxHashSet<String>),
    ) {
        if self.package.is_none() {
            self.package = Some((package.0.clone(), package.1.clone()));
        } else if !self.package_matches(package.0, package.1) {
            return;
        }
        if reads.opaque {
            return;
        }
        let entries = self.entries.entry((file.0.into(), file.1)).or_default();
        if entries.len() >= MAX_ENTRIES {
            return;
        }
        let roots: Vec<Root> = reads.roots.keys().cloned().collect();
        // Every declaration it read must be reached from what it looked up.
        let Some((fingerprint, read)) = fingerprint(env, &roots, Expand::Defs(&reads.defs)) else {
            return;
        };
        entries.push(CheckEntry {
            settings,
            roots,
            read,
            fingerprint,
            unborn,
            diagnostics: diagnostics.to_vec(),
            type_names: type_names.to_vec(),
        });
    }
}

/// Which declarations the walk writes out in full: those the file read, by
/// their ids in the flavor that checked it, or by their labels elsewhere.
enum Expand<'a> {
    Defs(&'a BTreeSet<DefId>),
    Labels(&'a BTreeSet<u32>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Node {
    Type(TypeId),
    Def(DefId),
    Param(ParamId),
}

struct Walk<'e> {
    env: &'e Env,
    out: Vec<u8>,
    labels: FxHashMap<Node, u32>,
    queue: VecDeque<Node>,
}

impl Walk<'_> {
    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        self.out.extend_from_slice(b);
    }

    fn tag(&mut self, t: u8) {
        self.out.push(t);
    }

    fn num(&mut self, n: u64) {
        self.out.extend_from_slice(&n.to_le_bytes());
    }

    /// The node's label; new nodes are queued.
    fn label(&mut self, node: Node) {
        let next = self.labels.len() as u32;
        let label = *self.labels.entry(node).or_insert_with(|| {
            self.queue.push_back(node);
            next
        });
        self.out.extend_from_slice(&label.to_le_bytes());
    }

    fn ty(&mut self, t: TypeId) {
        self.label(Node::Type(t));
    }

    fn opt_ty(&mut self, t: Option<TypeId>) {
        match t {
            Some(t) => {
                self.tag(1);
                self.ty(t);
            }
            None => self.tag(0),
        }
    }

    fn rest(&mut self, r: &Option<Rest>) {
        match r {
            None => self.tag(0),
            Some(Rest::Variadic(t)) => {
                self.tag(1);
                self.ty(*t);
            }
            Some(Rest::Pack(p)) => {
                self.tag(2);
                self.label(Node::Param(*p));
            }
        }
    }

    fn pack(&mut self, p: &Pack) {
        let Pack { types, rest } = p;
        self.num(types.len() as u64);
        for &t in types {
            self.ty(t);
        }
        self.rest(rest);
    }

    fn func(&mut self, f: &Func) {
        let Func {
            type_params,
            params,
            rest,
            ret,
            predicate,
            unannotated,
        } = f;
        self.num(type_params.len() as u64);
        for &p in type_params {
            self.label(Node::Param(p));
        }
        self.num(params.len() as u64);
        for super::types::Param { name, ty, optional } in params {
            self.bytes(name.as_deref().unwrap_or("\0").as_bytes());
            self.ty(*ty);
            self.tag(u8::from(*optional));
        }
        self.rest(rest);
        self.pack(ret);
        match predicate {
            Some((i, t)) => {
                self.tag(1);
                self.num(*i as u64);
                self.ty(*t);
            }
            None => self.tag(0),
        }
        self.tag(u8::from(*unannotated));
    }

    fn type_node(&mut self, t: TypeId) {
        let ty = self.env.types.get(t);
        self.tag(b'T');
        match ty {
            Ty::Any => self.tag(0),
            Ty::Unknown => self.tag(1),
            Ty::Never => self.tag(2),
            Ty::Error => self.tag(3),
            Ty::Nil => self.tag(4),
            Ty::Boolean => self.tag(5),
            Ty::Number => self.tag(6),
            Ty::String => self.tag(7),
            Ty::Table => self.tag(8),
            Ty::Function => self.tag(9),
            Ty::Thread => self.tag(10),
            Ty::Userdata => self.tag(11),
            Ty::Bool(b) => {
                self.tag(12);
                self.tag(u8::from(*b));
            }
            // The bits, so `0` and `-0` differ.
            Ty::Num(bits) => {
                self.tag(13);
                self.num(*bits);
            }
            Ty::Str(s) => {
                self.tag(14);
                self.bytes(s.as_bytes());
            }
            Ty::Union(m) | Ty::Intersection(m) => {
                self.tag(if matches!(ty, Ty::Union(_)) { 15 } else { 16 });
                self.num(m.len() as u64);
                for &x in m {
                    self.ty(x);
                }
            }
            Ty::Array(e) => {
                self.tag(17);
                self.ty(*e);
            }
            Ty::Map(k, v) => {
                self.tag(18);
                self.ty(*k);
                self.ty(*v);
            }
            // Its identity is its label; `id` differs between flavors.
            Ty::Record(Record {
                fields,
                indexer,
                open,
                id: _,
            }) => {
                self.tag(19);
                self.num(fields.len() as u64);
                for (
                    name,
                    Field {
                        ty,
                        optional,
                        readonly,
                    },
                ) in fields.iter()
                {
                    self.bytes(name.as_bytes());
                    self.ty(*ty);
                    self.tag(u8::from(*optional));
                    self.tag(u8::from(*readonly));
                }
                self.opt_ty(indexer.map(|(k, _)| k));
                self.opt_ty(indexer.map(|(_, v)| v));
                self.tag(u8::from(*open));
            }
            Ty::Func(f) => {
                self.tag(20);
                self.func(f);
            }
            Ty::Named(d, args) => {
                self.tag(21);
                self.label(Node::Def(*d));
                self.num(args.len() as u64);
                for &a in args {
                    self.ty(a);
                }
            }
            Ty::ClassOf(d) => {
                self.tag(22);
                self.label(Node::Def(*d));
            }
            Ty::EnumMember(d, i) => {
                self.tag(23);
                self.label(Node::Def(*d));
                self.num(u64::from(*i));
            }
            Ty::Param(p) => {
                self.tag(24);
                self.label(Node::Param(*p));
            }
            Ty::Secret(t) => {
                self.tag(25);
                self.ty(*t);
            }
            Ty::Module(m) => {
                self.tag(26);
                self.num(u64::from(*m));
            }
        }
    }

    fn member(&mut self, name: &str, m: &Member) {
        let Member {
            ty,
            kind,
            optional,
            readonly,
            visibility,
            renamable,
        } = m;
        self.bytes(name.as_bytes());
        self.ty(*ty);
        self.bytes(format!("{kind:?}{visibility:?}").as_bytes());
        self.tag(u8::from(*optional));
        self.tag(u8::from(*readonly));
        self.tag(u8::from(*renamable));
    }

    fn def_node(&mut self, d: DefId, full: bool) {
        let Def {
            name,
            kind,
            params,
            members,
            statics,
            bases,
            alias,
            enum_members,
            is_abstract,
            widget,
            native,
            indexer,
            span,
        } = self.env.def_unrecorded(d);
        self.tag(b'D');
        self.bytes(name.as_bytes());
        self.bytes(format!("{kind:?}").as_bytes());
        if !full {
            self.tag(0);
            return;
        }
        self.tag(1);
        self.num(params.len() as u64);
        for &p in params {
            self.label(Node::Param(p));
        }
        for list in [members, statics] {
            self.num(list.len() as u64);
            for (k, m) in list.iter() {
                self.member(k, m);
            }
        }
        self.num(bases.len() as u64);
        for &b in bases {
            self.ty(b);
        }
        self.opt_ty(*alias);
        self.num(enum_members.len() as u64);
        for (k, t) in enum_members {
            self.bytes(k.as_bytes());
            self.ty(*t);
        }
        self.tag(u8::from(*is_abstract));
        self.bytes(widget.as_deref().unwrap_or("\0").as_bytes());
        self.tag(u8::from(*native));
        self.opt_ty(indexer.map(|(k, _)| k));
        self.opt_ty(indexer.map(|(_, v)| v));
        // Set only for package classes and enums, whose files are the same
        // in every flavor.
        match span {
            Some(s) => self.bytes(format!("{:?}", (s.file, s.start, s.end)).as_bytes()),
            None => self.tag(0),
        }
    }

    fn param_node(&mut self, p: ParamId) {
        let ParamInfo {
            name,
            constraint,
            pack,
        } = &self.env.params[p.0 as usize];
        self.tag(b'P');
        self.bytes(name.as_bytes());
        self.opt_ty(*constraint);
        self.tag(u8::from(*pack));
    }
}

/// The canonical fingerprint of what `roots` give in `env`, with the
/// declarations `expand` names written out in full; with the labels of
/// those declarations. `None` when a declaration the file read is not
/// reached from its roots (it cannot be compared).
fn fingerprint(env: &Env, roots: &[Root], expand: Expand<'_>) -> Option<(Vec<u8>, BTreeSet<u32>)> {
    let mut w = Walk {
        env,
        out: Vec::new(),
        labels: FxHashMap::default(),
        queue: VecDeque::new(),
    };
    for root in roots {
        w.bytes(format!("{root:?}").as_bytes());
        match env.declared.peek(root) {
            Peeked::Type(Some(t)) => {
                w.tag(1);
                w.ty(t);
            }
            Peeked::Def(Some(d)) => {
                w.tag(1);
                w.label(Node::Def(d));
            }
            Peeked::Func(Some(f)) => {
                w.tag(1);
                w.func(f);
            }
            Peeked::Text(Some(s)) => {
                w.tag(1);
                w.bytes(s.as_bytes());
            }
            Peeked::Type(None) | Peeked::Def(None) | Peeked::Func(None) | Peeked::Text(None) => {
                w.tag(0)
            }
        }
    }
    let mut read = BTreeSet::new();
    while let Some(node) = w.queue.pop_front() {
        match node {
            Node::Type(t) => w.type_node(t),
            Node::Def(d) => {
                let label = w.labels[&node];
                let full = match &expand {
                    Expand::Defs(ids) => ids.contains(&d),
                    Expand::Labels(labels) => labels.contains(&label),
                };
                if full {
                    read.insert(label);
                }
                w.def_node(d, full);
            }
            Node::Param(p) => w.param_node(p),
        }
    }
    if let Expand::Defs(ids) = expand
        && read.len() != ids.len()
    {
        return None;
    }
    Some((w.out, read))
}
