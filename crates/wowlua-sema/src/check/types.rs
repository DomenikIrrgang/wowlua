//! The type representation (spec chapter 2): hash-consed types (`TypeId`),
//! records, functions with type packs, named types (classes, interfaces,
//! enums, generic aliases) and their display in messages.

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

/// A declared named type (class, interface, enum, alias, namespace).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId(pub u32);

/// A type parameter (or pack parameter) of a generic declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Any,
    Unknown,
    Never,
    /// An erroneous type: assignable both ways, so errors do not cascade.
    Error,
    Nil,
    Boolean,
    Number,
    String,
    /// Any table.
    Table,
    /// Any function.
    Function,
    Thread,
    Userdata,
    Bool(bool),
    /// A number literal (bits of the `f64`).
    Num(u64),
    Str(Box<str>),
    /// Flattened, deduplicated, at least two members.
    Union(Vec<TypeId>),
    /// Function overload sets and combined table shapes.
    Intersection(Vec<TypeId>),
    Array(TypeId),
    Map(TypeId, TypeId),
    Record(Record),
    // Boxed: the arena holds a slot of the largest variant for every type,
    // and functions are few next to the records and literals of table data.
    Func(Arc<Func>),
    /// An instance of a class or interface, an enum, or a generic alias
    /// application that is expanded on demand.
    Named(DefId, Vec<TypeId>),
    /// The class value of a class (`ClassOf<T>`), or a namespace table.
    ClassOf(DefId),
    /// `ClassOf<T>` for a type parameter `T`: binds `T` to the class (or
    /// enum) of the table passed, and becomes that `ClassOf` once bound.
    ClassOfParam(ParamId),
    /// `EventArgs<E>` as a rest (`...EventArgs<E>`): the payload of event or
    /// message `E`, any values until a call knows `E` (decision L44).
    EventArgs(TypeId),
    EnumMember(DefId, u32),
    Param(ParamId),
    Secret(TypeId),
    /// A module's export table (`import * as M`).
    Module(u32),
}

/// A field map, in the order the fields were added. The hasher takes no
/// room in the map; nothing depends on the hash values.
pub type FieldMap = IndexMap<Box<str>, Field, FxBuildHasher>;

/// A record's fields, in the order they were added. Cloning costs a
/// reference count, so reading a record copies no fields:
/// - records with the same fields in the same order share one map
///   ([`Types::record`]): a type-checked data addon has a million records
///   but few distinct field maps;
/// - a table built field by field (`M.x = …`) makes a new record per field,
///   and each one shares its predecessor's fields (`with_appended`), so
///   that costs about N²/64 stored fields instead of N².
#[derive(Clone, Default)]
pub struct Fields {
    /// The fields up to the last flattening.
    base: Option<Arc<FieldMap>>,
    /// Fields appended since, newest first, at most `TAIL_MAX`.
    tail: Option<Arc<Tail>>,
}

struct Tail {
    name: Box<str>,
    field: Field,
    prev: Option<Arc<Tail>>,
    /// The nodes in this list, this one included.
    len: usize,
}

impl Fields {
    /// Appended fields kept apart before they are flattened into the base.
    const TAIL_MAX: usize = 32;

    pub fn len(&self) -> usize {
        self.base.as_ref().map_or(0, |b| b.len()) + self.tail.as_ref().map_or(0, |t| t.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, name: &str) -> Option<&Field> {
        if let Some(f) = self.base.as_ref().and_then(|b| b.get(name)) {
            return Some(f);
        }
        let mut node = self.tail.as_deref();
        while let Some(n) = node {
            if &*n.name == name {
                return Some(&n.field);
            }
            node = n.prev.as_deref();
        }
        None
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The fields in the order they were added.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Field)> {
        let mut tail = Vec::new();
        let mut node = self.tail.as_deref();
        while let Some(n) = node {
            tail.push(n);
            node = n.prev.as_deref();
        }
        let base = self.base.as_deref().into_iter().flatten();
        base.map(|(name, field)| (&**name, field))
            .chain(tail.into_iter().rev().map(|n| (&*n.name, &n.field)))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|(name, _)| name)
    }

    pub fn values(&self) -> impl Iterator<Item = &Field> {
        self.iter().map(|(_, field)| field)
    }

    /// A version with `name` added last, sharing these fields. `name` must
    /// not be a field yet.
    pub fn with_appended(&self, name: Box<str>, field: Field) -> Fields {
        debug_assert!(!self.contains_key(&name));
        let len = self.tail.as_ref().map_or(0, |t| t.len) + 1;
        if len > Self::TAIL_MAX {
            let mut flat = self.clone();
            flat.insert(name, field);
            return flat;
        }
        Fields {
            base: self.base.clone(),
            tail: Some(Arc::new(Tail {
                name,
                field,
                prev: self.tail.clone(),
                len,
            })),
        }
    }

    /// Sets `name` as `IndexMap::insert` does: a new name goes last, an
    /// existing one keeps its place. Copies the map if it is shared.
    pub fn insert(&mut self, name: Box<str>, field: Field) {
        let map = self.flatten();
        map.insert(name, field);
    }

    /// Moves the tail into a base of its own and returns it.
    fn flatten(&mut self) -> &mut FieldMap {
        let base = self.base.get_or_insert_with(Default::default);
        if let Some(tail) = self.tail.take() {
            let map = Arc::make_mut(base);
            let mut nodes = Vec::with_capacity(tail.len);
            let mut node = Some(&*tail);
            while let Some(n) = node {
                nodes.push(n);
                node = n.prev.as_deref();
            }
            map.reserve(nodes.len());
            for n in nodes.into_iter().rev() {
                map.insert(n.name.clone(), n.field);
            }
        }
        Arc::make_mut(base)
    }
}

impl From<FieldMap> for Fields {
    fn from(map: FieldMap) -> Self {
        Fields {
            base: (!map.is_empty()).then(|| Arc::new(map)),
            tail: None,
        }
    }
}

/// Equal fields, in any order (as `IndexMap`'s equality).
impl PartialEq for Fields {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(name, f)| other.get(name) == Some(f))
    }
}

impl Eq for Fields {}

impl std::fmt::Debug for Fields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Record {
    pub fields: Fields,
    pub indexer: Option<(TypeId, TypeId)>,
    /// Still being built in the function that created it (spec §2.4.2).
    pub open: bool,
    /// A literal identity, so that open tables are not merged.
    pub id: u32,
}

impl std::hash::Hash for Record {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Every record literal has its own identity.
        self.id.hash(state);
        self.fields.len().hash(state);
    }
}

impl std::hash::Hash for Field {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ty.hash(state);
        self.optional.hash(state);
        self.readonly.hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub ty: TypeId,
    pub optional: bool,
    pub readonly: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Func {
    pub type_params: Vec<ParamId>,
    pub params: Vec<Param>,
    pub rest: Option<Rest>,
    pub ret: Pack,
    /// `param is T`: the parameter index and the type.
    pub predicate: Option<(usize, TypeId)>,
    /// Declared without annotations (W2055 instead of E2006).
    pub unannotated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Param {
    pub name: Option<Box<str>>,
    pub ty: TypeId,
    pub optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Rest {
    Variadic(TypeId),
    Pack(ParamId),
}

/// A sequence of types: results, or arguments.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Pack {
    pub types: Vec<TypeId>,
    pub rest: Option<Rest>,
}

impl Pack {
    pub fn single(t: TypeId) -> Pack {
        Pack {
            types: vec![t],
            rest: None,
        }
    }

    pub fn empty() -> Pack {
        Pack::default()
    }

    /// Any number of values of any type.
    pub fn any() -> Pack {
        Pack {
            types: Vec::new(),
            rest: Some(Rest::Variadic(Types::ANY)),
        }
    }
}

impl Ty {
    /// The types this one is made of.
    pub fn children(&self) -> Vec<TypeId> {
        match self {
            Ty::Any
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
            | Ty::Str(_)
            | Ty::ClassOf(_)
            | Ty::EnumMember(..)
            | Ty::Param(_)
            | Ty::ClassOfParam(_)
            | Ty::Module(_) => Vec::new(),
            Ty::Union(m) | Ty::Intersection(m) => m.clone(),
            Ty::Array(t) | Ty::Secret(t) | Ty::EventArgs(t) => vec![*t],
            Ty::Map(k, v) => vec![*k, *v],
            Ty::Named(_, args) => args.clone(),
            Ty::Record(Record {
                fields,
                indexer,
                open: _,
                id: _,
            }) => fields
                .values()
                .map(|f| f.ty)
                .chain(indexer.iter().flat_map(|&(k, v)| [k, v]))
                .collect(),
            Ty::Func(f) => {
                let Func {
                    type_params: _,
                    params,
                    rest,
                    ret,
                    predicate,
                    unannotated: _,
                } = f.as_ref();
                let rest_ty = |r: &Option<Rest>| match r {
                    Some(Rest::Variadic(t)) => Some(*t),
                    _ => None,
                };
                params
                    .iter()
                    .map(|p| p.ty)
                    .chain(rest_ty(rest))
                    .chain(ret.types.iter().copied())
                    .chain(rest_ty(&ret.rest))
                    .chain(predicate.map(|(_, t)| t))
                    .collect()
            }
        }
    }
}

/// The type arena.
#[derive(Clone, Debug)]
pub struct Types {
    data: Vec<Ty>,
    /// The ids of the hash-consed types, hashed and compared through `data`,
    /// so each type is stored once. Records are not in it: each one has its
    /// own identity, so a lookup could never find one. Hashed with
    /// `FxBuildHasher`.
    index: hashbrown::HashTable<TypeId>,
    next_record: u32,
    /// The distinct field maps of records, compared in order (see
    /// `share_fields`).
    maps: hashbrown::HashTable<Arc<FieldMap>>,
    /// The maps in `maps`, in the order they were added: their hashes and
    /// addresses (for `rollback`).
    maps_added: Vec<(u64, usize)>,
    /// While a check is watched (`watch_births`): which of the types made
    /// since are born of it, and what a union's order depends on.
    births: Option<Births>,
}

/// Types made since `from`: born of the check that made them, when they are
/// one of its records or contain one. Their order among each other is the
/// order the check made them in, wherever it runs; any other type's id
/// depends on what was made before.
#[derive(Clone, Debug)]
struct Births {
    from: usize,
    born: Vec<bool>,
    /// The members not born of the check of the unions it made (besides
    /// the builtins), when they have a [`Shape`].
    unborn: Vec<TypeId>,
    /// A union has a member not born of the check without a shape: its
    /// order can differ where other types were made before.
    order_sensitive: bool,
}

/// A type told by its structure, to find it in another arena: a builtin, a
/// literal, or an array, map, secret or union of those.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Builtin(TypeId),
    Num(u64),
    Str(Box<str>),
    Array(Box<Shape>),
    Map(Box<Shape>, Box<Shape>),
    Secret(Box<Shape>),
    Union(Vec<Shape>),
    /// A function without type parameters over shapes.
    Func(Box<FuncShape>),
}

/// The shape of a function type (see [`Func`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuncShape {
    params: Vec<(Option<Box<str>>, Shape, bool)>,
    rest: Option<Shape>,
    ret: Vec<Shape>,
    ret_rest: Option<Shape>,
    predicate: Option<(usize, Shape)>,
    unannotated: bool,
}

/// The members not born of a watched check of the unions it made, in the
/// order of their ids: their shapes, and whether they were made before it.
/// Where the same members (by shape) were made before the check in the
/// same order, and the others were not, its unions have the same order.
pub type Unborn = Vec<(Shape, bool)>;

/// A point in the arena's history to go back to (`Types::rollback`).
#[derive(Clone, Copy, Debug)]
pub struct Checkpoint {
    len: usize,
    next_record: u32,
    maps: usize,
}

impl Default for Types {
    fn default() -> Self {
        Self::new()
    }
}

impl Types {
    pub const ANY: TypeId = TypeId(0);
    pub const UNKNOWN: TypeId = TypeId(1);
    pub const NEVER: TypeId = TypeId(2);
    pub const ERROR: TypeId = TypeId(3);
    pub const NIL: TypeId = TypeId(4);
    pub const BOOLEAN: TypeId = TypeId(5);
    pub const NUMBER: TypeId = TypeId(6);
    pub const STRING: TypeId = TypeId(7);
    pub const TABLE: TypeId = TypeId(8);
    pub const FUNCTION: TypeId = TypeId(9);
    pub const THREAD: TypeId = TypeId(10);
    pub const USERDATA: TypeId = TypeId(11);
    pub const TRUE: TypeId = TypeId(12);
    pub const FALSE: TypeId = TypeId(13);

    pub fn new() -> Self {
        let mut t = Types {
            data: Vec::new(),
            index: hashbrown::HashTable::new(),
            next_record: 0,
            maps: hashbrown::HashTable::new(),
            maps_added: Vec::new(),
            births: None,
        };
        for ty in [
            Ty::Any,
            Ty::Unknown,
            Ty::Never,
            Ty::Error,
            Ty::Nil,
            Ty::Boolean,
            Ty::Number,
            Ty::String,
            Ty::Table,
            Ty::Function,
            Ty::Thread,
            Ty::Userdata,
            Ty::Bool(true),
            Ty::Bool(false),
        ] {
            t.intern(ty);
        }
        t
    }

    pub fn intern(&mut self, ty: Ty) -> TypeId {
        debug_assert!(
            !matches!(ty, Ty::Record(_)),
            "records go through Types::record"
        );
        let hash = FxBuildHasher.hash_one(&ty);
        if let Some(&id) = self.index.find(hash, |&id| self.data[id.0 as usize] == ty) {
            return id;
        }
        let id = TypeId(self.data.len() as u32);
        self.note_birth(&ty);
        self.data.push(ty);
        let Types { data, index, .. } = self;
        index.insert_unique(hash, id, |&id| FxBuildHasher.hash_one(&data[id.0 as usize]));
        id
    }

    /// Starts watching which types are born of the check that follows.
    pub fn watch_births(&mut self) {
        self.births = Some(Births {
            from: self.data.len(),
            born: Vec::new(),
            unborn: Vec::new(),
            order_sensitive: false,
        });
    }

    /// Stops watching: what the order of the unions made since depends on,
    /// or `None` when it can differ where the types made before differ in
    /// ways shapes do not tell.
    pub fn births(&mut self) -> Option<Unborn> {
        let mut b = self.births.take()?;
        if b.order_sensitive {
            return None;
        }
        b.unborn.sort();
        b.unborn.dedup();
        b.unborn
            .iter()
            .map(|&m| Some((self.shape(m)?, (m.0 as usize) < b.from)))
            .collect()
    }

    /// Whether `unborn` holds in this arena: the members made before are
    /// here, in the same order, and the others are not.
    pub fn unborn_holds(&self, unborn: &Unborn) -> bool {
        let mut last = None;
        unborn
            .iter()
            .all(|(shape, before)| match self.find_shape(shape) {
                Some(id) if *before => {
                    last < Some(id) && {
                        last = Some(id);
                        true
                    }
                }
                found => found.is_none() && !*before,
            })
    }

    /// Whether `t` has a shape (as `shape(t).is_some()`, without making it).
    fn has_shape(&self, t: TypeId) -> bool {
        if t <= Types::FALSE {
            return true;
        }
        let variadic = |r: &Option<Rest>| match r {
            None => true,
            Some(Rest::Variadic(t)) => self.has_shape(*t),
            Some(Rest::Pack(_)) => false,
        };
        match self.get(t) {
            Ty::Num(_) | Ty::Str(_) => true,
            Ty::Array(e) | Ty::Secret(e) => self.has_shape(*e),
            Ty::Map(k, v) => self.has_shape(*k) && self.has_shape(*v),
            Ty::Union(m) => m.iter().all(|&m| self.has_shape(m)),
            Ty::Func(f) => {
                f.type_params.is_empty()
                    && f.params.iter().all(|p| self.has_shape(p.ty))
                    && variadic(&f.rest)
                    && f.ret.types.iter().all(|&t| self.has_shape(t))
                    && variadic(&f.ret.rest)
                    && f.predicate.is_none_or(|(_, t)| self.has_shape(t))
            }
            _ => false,
        }
    }

    /// `t`'s shape, if it has one.
    fn shape(&self, t: TypeId) -> Option<Shape> {
        if t <= Types::FALSE {
            return Some(Shape::Builtin(t));
        }
        Some(match self.get(t) {
            Ty::Num(bits) => Shape::Num(*bits),
            Ty::Str(s) => Shape::Str(s.clone()),
            Ty::Array(e) => Shape::Array(Box::new(self.shape(*e)?)),
            Ty::Map(k, v) => Shape::Map(Box::new(self.shape(*k)?), Box::new(self.shape(*v)?)),
            Ty::Secret(t) => Shape::Secret(Box::new(self.shape(*t)?)),
            Ty::Union(m) => Shape::Union(m.iter().map(|&m| self.shape(m)).collect::<Option<_>>()?),
            Ty::Func(f) => {
                let Func {
                    type_params,
                    params,
                    rest,
                    ret,
                    predicate,
                    unannotated,
                } = f.as_ref();
                if !type_params.is_empty() {
                    return None;
                }
                let variadic = |r: &Option<Rest>| -> Option<Option<Shape>> {
                    match r {
                        None => Some(None),
                        Some(Rest::Variadic(t)) => Some(Some(self.shape(*t)?)),
                        Some(Rest::Pack(_)) => None,
                    }
                };
                Shape::Func(Box::new(FuncShape {
                    params: params
                        .iter()
                        .map(|p| Some((p.name.clone(), self.shape(p.ty)?, p.optional)))
                        .collect::<Option<_>>()?,
                    rest: variadic(rest)?,
                    ret: ret
                        .types
                        .iter()
                        .map(|&t| self.shape(t))
                        .collect::<Option<_>>()?,
                    ret_rest: variadic(&ret.rest)?,
                    predicate: match predicate {
                        Some((i, t)) => Some((*i, self.shape(*t)?)),
                        None => None,
                    },
                    unannotated: *unannotated,
                }))
            }
            _ => return None,
        })
    }

    /// The type of shape `s` in this arena, if it is here.
    fn find_shape(&self, s: &Shape) -> Option<TypeId> {
        let ty = match s {
            Shape::Builtin(t) => return Some(*t),
            Shape::Num(bits) => Ty::Num(*bits),
            Shape::Str(s) => Ty::Str(s.clone()),
            Shape::Array(e) => Ty::Array(self.find_shape(e)?),
            Shape::Map(k, v) => Ty::Map(self.find_shape(k)?, self.find_shape(v)?),
            Shape::Secret(t) => Ty::Secret(self.find_shape(t)?),
            Shape::Func(f) => {
                let FuncShape {
                    params,
                    rest,
                    ret,
                    ret_rest,
                    predicate,
                    unannotated,
                } = f.as_ref();
                let variadic = |r: &Option<Shape>| -> Option<Option<Rest>> {
                    match r {
                        None => Some(None),
                        Some(s) => Some(Some(Rest::Variadic(self.find_shape(s)?))),
                    }
                };
                Ty::Func(Arc::new(Func {
                    type_params: Vec::new(),
                    params: params
                        .iter()
                        .map(|(name, s, optional)| {
                            Some(Param {
                                name: name.clone(),
                                ty: self.find_shape(s)?,
                                optional: *optional,
                            })
                        })
                        .collect::<Option<_>>()?,
                    rest: variadic(rest)?,
                    ret: Pack {
                        types: ret
                            .iter()
                            .map(|s| self.find_shape(s))
                            .collect::<Option<_>>()?,
                        rest: variadic(ret_rest)?,
                    },
                    predicate: match predicate {
                        Some((i, s)) => Some((*i, self.find_shape(s)?)),
                        None => None,
                    },
                    unannotated: *unannotated,
                }))
            }
            // Members are in the order of their ids.
            Shape::Union(m) => {
                let mut ids = m
                    .iter()
                    .map(|m| self.find_shape(m))
                    .collect::<Option<Vec<_>>>()?;
                ids.sort();
                Ty::Union(ids)
            }
        };
        let hash = FxBuildHasher.hash_one(&ty);
        self.index
            .find(hash, |&id| self.data[id.0 as usize] == ty)
            .copied()
    }

    fn is_born(&self, id: TypeId) -> bool {
        self.births.as_ref().is_some_and(|b| {
            (id.0 as usize)
                .checked_sub(b.from)
                .is_some_and(|k| b.born[k])
        })
    }

    /// Notes whether `ty`, about to be added, is born of the watched check.
    fn note_birth(&mut self, ty: &Ty) {
        if self.births.is_none() {
            return;
        }
        let born = matches!(ty, Ty::Record(_)) || ty.children().iter().any(|&c| self.is_born(c));
        if let Some(b) = &mut self.births {
            b.born.push(born);
        }
    }

    pub fn get(&self, id: TypeId) -> &Ty {
        &self.data[id.0 as usize]
    }

    pub fn num(&mut self, n: f64) -> TypeId {
        self.intern(Ty::Num(n.to_bits()))
    }

    pub fn str(&mut self, s: &str) -> TypeId {
        self.intern(Ty::Str(s.into()))
    }

    pub fn array(&mut self, t: TypeId) -> TypeId {
        self.intern(Ty::Array(t))
    }

    pub fn func(&mut self, f: Func) -> TypeId {
        self.intern(Ty::Func(Arc::new(f)))
    }

    pub fn optional(&mut self, t: TypeId) -> TypeId {
        self.union(&[t, Types::NIL])
    }

    /// A new record type (each literal gets its own identity).
    pub fn record(&mut self, mut r: Record) -> TypeId {
        self.share_fields(&mut r.fields);
        self.next_record += 1;
        r.id = self.next_record;
        let id = TypeId(self.data.len() as u32);
        let ty = Ty::Record(r);
        self.note_birth(&ty);
        self.data.push(ty);
        id
    }

    /// Points `fields` at a stored map with the same fields in the same
    /// order, if there is one, so equal maps are stored once. Only the
    /// storage is shared: each record keeps its own identity, and a map is
    /// copied before it is changed.
    fn share_fields(&mut self, fields: &mut Fields) {
        if fields.tail.is_some() {
            return;
        }
        let Some(base) = &mut fields.base else {
            return;
        };
        let hash = ordered_hash(base);
        if let Some(found) = self.maps.find(hash, |m| same_order(m, base)) {
            *base = found.clone();
        } else {
            // Kept for the rest of the check: without the room it grew.
            if let Some(map) = Arc::get_mut(base) {
                map.shrink_to_fit();
            }
            self.maps_added.push((hash, Arc::as_ptr(base) as usize));
            self.maps
                .insert_unique(hash, base.clone(), |m| ordered_hash(m));
        }
    }

    /// Where the arena is now.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            len: self.data.len(),
            next_record: self.next_record,
            maps: self.maps_added.len(),
        }
    }

    /// Forgets every type made since `cp`, as if they were never made:
    /// later types get the ids they would have had then. Nothing may hold
    /// one of those ids any more.
    pub fn rollback(&mut self, cp: Checkpoint) {
        for k in (cp.len..self.data.len()).rev() {
            if matches!(self.data[k], Ty::Record(_)) {
                continue;
            }
            let hash = FxBuildHasher.hash_one(&self.data[k]);
            if let Ok(entry) = self.index.find_entry(hash, |&id| id.0 as usize == k) {
                entry.remove();
            } else {
                unreachable!("every type but a record is in the index");
            }
        }
        self.data.truncate(cp.len);
        self.next_record = cp.next_record;
        if let Some(b) = &mut self.births {
            b.born.truncate(cp.len.saturating_sub(b.from));
            b.unborn.retain(|m| (m.0 as usize) < cp.len);
        }
        for (hash, map) in self.maps_added.drain(cp.maps..).rev() {
            if let Ok(entry) = self
                .maps
                .find_entry(hash, |m| Arc::as_ptr(m) as usize == map)
            {
                entry.remove();
            }
        }
    }

    /// The union of `members`: flattened, deduplicated; `never` dropped,
    /// `any` absorbs everything, literals absorbed by their primitive.
    pub fn union(&mut self, members: &[TypeId]) -> TypeId {
        // `a | a` is `a`: only this function makes unions, so a union's
        // members are normalized already. A watched check notes the members
        // of every union it makes, so there a union takes the long way.
        if let [a, rest @ ..] = members
            && rest.iter().all(|r| r == a)
            && (self.births.is_none() || !matches!(self.get(*a), Ty::Union(_)))
        {
            return *a;
        }
        let mut flat: Vec<TypeId> = Vec::with_capacity(members.len());
        for &m in members {
            match self.get(m) {
                Ty::Union(inner) => flat.extend_from_slice(inner),
                Ty::Never => {}
                _ => flat.push(m),
            }
        }
        // The members end up sorted by id, so sorting first changes nothing.
        flat.sort_unstable();
        flat.dedup();
        if flat.binary_search(&Types::ANY).is_ok() {
            return Types::ANY;
        }
        if flat.binary_search(&Types::ERROR).is_ok() {
            return Types::ERROR;
        }
        // Literals absorbed by their primitive; true | false is boolean.
        let has = |t: TypeId| flat.binary_search(&t).is_ok();
        let (boolean, number, string) =
            (has(Types::BOOLEAN), has(Types::NUMBER), has(Types::STRING));
        if boolean || number || string {
            flat.retain(|&m| match self.get(m) {
                Ty::Bool(_) => !boolean,
                Ty::Num(_) => !number,
                Ty::Str(_) => !string,
                _ => true,
            });
        }
        if flat.binary_search(&Types::TRUE).is_ok() && flat.binary_search(&Types::FALSE).is_ok() {
            flat.retain(|&m| m != Types::TRUE && m != Types::FALSE);
            flat.push(Types::BOOLEAN);
            flat.sort_unstable();
        }
        match flat.len() {
            0 => Types::NEVER,
            1 => flat[0],
            _ => {
                // The builtins have the same ids everywhere, and members
                // born of a watched check keep their order wherever it runs.
                // Other members keep it where the types made before give
                // the same order (`Unborn`).
                if self.births.is_some() {
                    let ordered: Vec<TypeId> =
                        flat.iter().copied().filter(|&m| m > Types::FALSE).collect();
                    if ordered.len() >= 2 {
                        let unborn: Vec<TypeId> =
                            ordered.into_iter().filter(|&m| !self.is_born(m)).collect();
                        let shaped = unborn.iter().all(|&m| self.has_shape(m));
                        if let Some(b) = &mut self.births {
                            b.order_sensitive |= !shaped;
                            b.unborn.extend(unborn);
                        }
                    }
                }
                self.intern(Ty::Union(flat))
            }
        }
    }

    /// The members of a union (or the type itself).
    pub fn members(&self, t: TypeId) -> Vec<TypeId> {
        match self.get(t) {
            Ty::Union(m) => m.clone(),
            _ => vec![t],
        }
    }

    /// `t` without `nil`.
    pub fn non_nil(&mut self, t: TypeId) -> TypeId {
        let members: Vec<TypeId> = self
            .members(t)
            .into_iter()
            .filter(|&m| m != Types::NIL)
            .collect();
        if members.len() == self.members(t).len() {
            return t;
        }
        self.union(&members)
    }

    /// `t` may be `nil`.
    pub fn is_optional(&self, t: TypeId) -> bool {
        matches!(self.get(t), Ty::Any | Ty::Unknown | Ty::Error | Ty::Nil)
            || self.members(t).contains(&Types::NIL)
    }

    /// The literal's primitive (`"x"` → `string`), for inferred locals.
    pub fn widen(&mut self, t: TypeId) -> TypeId {
        match self.get(t).clone() {
            Ty::Bool(_) => Types::BOOLEAN,
            Ty::Num(_) => Types::NUMBER,
            Ty::Str(_) => Types::STRING,
            Ty::Union(m) => {
                let widened: Vec<TypeId> = m.iter().map(|&x| self.widen(x)).collect();
                self.union(&widened)
            }
            _ => t,
        }
    }

    /// `widen` for a local's value or an inferred result: a `false` or
    /// `true` beside a type that is neither `nil` nor a boolean stays a
    /// literal, so a truthiness test removes it (`ok and t` is `false | T`,
    /// and where it is truthy it is `T`).
    pub fn widen_value(&mut self, t: TypeId) -> TypeId {
        let Ty::Union(m) = self.get(t).clone() else {
            return self.widen(t);
        };
        let other = m
            .iter()
            .any(|&x| !matches!(self.get(x), Ty::Nil | Ty::Bool(_) | Ty::Boolean));
        if !other {
            return self.widen(t);
        }
        let widened: Vec<TypeId> = m
            .iter()
            .map(|&x| match self.get(x) {
                Ty::Bool(_) => x,
                _ => self.widen(x),
            })
            .collect();
        self.union(&widened)
    }
}

fn ordered_hash(map: &FieldMap) -> u64 {
    let mut h = FxBuildHasher.build_hasher();
    map.len().hash(&mut h);
    for (name, field) in map {
        name.hash(&mut h);
        field.hash(&mut h);
    }
    h.finish()
}

/// The same fields in the same order: display, runtime checks and
/// diagnostics follow a record's field order.
fn same_order(a: &FieldMap, b: &FieldMap) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

/// Formats numbers like Lua (`%.14g`) for literal types.
pub fn number_text(bits: u64) -> String {
    crate::constant::format_g14(f64::from_bits(bits))
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;

    fn field(ty: u32) -> Field {
        Field {
            ty: TypeId(ty),
            optional: ty.is_multiple_of(3),
            readonly: false,
        }
    }

    #[track_caller]
    fn same(fields: &Fields, model: &FieldMap) {
        let got: Vec<(&str, &Field)> = fields.iter().collect();
        let want: Vec<(&str, &Field)> = model.iter().map(|(k, v)| (&**k, v)).collect();
        assert_eq!(got, want);
        assert_eq!(fields.len(), model.len());
        assert_eq!(fields.is_empty(), model.is_empty());
        for (k, v) in model {
            assert_eq!(fields.get(k), Some(v));
        }
        assert_eq!(fields.get("missing"), None);
        assert!(fields.keys().eq(model.keys().map(|k| &**k)));
        assert!(fields.values().eq(model.values()));
    }

    /// Every version of a table built field by field reads as a plain map
    /// with the same insertions, across flattenings, and older versions
    /// are unchanged by later ones.
    #[test]
    fn appended_versions_read_as_maps() {
        let mut versions = vec![(Fields::default(), FieldMap::default())];
        for i in 0..200u32 {
            let (fields, model) = versions.last().unwrap();
            let mut model = model.clone();
            model.insert(format!("f{i}").into(), field(i));
            let fields = fields.with_appended(format!("f{i}").into(), field(i));
            versions.push((fields, model));
        }
        for (fields, model) in &versions {
            same(fields, model);
            assert_eq!(fields, &Fields::from(model.clone()));
        }
        // Inserting into a version copies what it shares.
        let (mut fields, mut model) = versions[100].clone();
        fields.insert("f3".into(), field(7));
        model.insert("f3".into(), field(7));
        fields.insert("new".into(), field(8));
        model.insert("new".into(), field(8));
        same(&fields, &model);
        same(&versions[100].0, &versions[100].1);
        same(&versions[101].0, &versions[101].1);
    }

    /// Records with equal fields in the same order share one map; a
    /// different order is kept; each record keeps its own identity, and
    /// changing one copy leaves the other alone.
    #[test]
    fn equal_field_maps_are_stored_once() {
        let mut types = Types::new();
        let map = |names: &[&str]| -> FieldMap {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| ((*n).into(), field(i as u32)))
                .collect()
        };
        let record = |types: &mut Types, names: &[&str]| {
            types.record(Record {
                fields: map(names).into(),
                ..Record::default()
            })
        };
        let base = |types: &Types, t: TypeId| match types.get(t) {
            Ty::Record(r) => r.fields.base.clone().expect("fields"),
            _ => unreachable!(),
        };
        let a = record(&mut types, &["x", "y"]);
        let b = record(&mut types, &["x", "y"]);
        let c = record(&mut types, &["y", "x"]);
        assert_ne!(a, b);
        assert!(Arc::ptr_eq(&base(&types, a), &base(&types, b)));
        assert!(!Arc::ptr_eq(&base(&types, a), &base(&types, c)));
        let Ty::Record(mut r) = types.get(b).clone() else {
            unreachable!()
        };
        r.fields.insert("z".into(), field(2));
        same(&r.fields, &map(&["x", "y", "z"]));
        let Ty::Record(ra) = types.get(a) else {
            unreachable!()
        };
        same(&ra.fields, &map(&["x", "y"]));
    }

    /// The field entries the arena holds: each shared map and tail node
    /// counted once.
    fn stored_fields(types: &Types) -> usize {
        let mut bases = std::collections::HashSet::new();
        let mut tails = std::collections::HashSet::new();
        let mut stored = 0;
        for ty in &types.data {
            let Ty::Record(r) = ty else { continue };
            if let Some(b) = &r.fields.base
                && bases.insert(Arc::as_ptr(b))
            {
                stored += b.len();
            }
            let mut node = r.fields.tail.as_ref();
            while let Some(n) = node {
                if !tails.insert(Arc::as_ptr(n)) {
                    break;
                }
                stored += 1;
                node = n.prev.as_ref();
            }
        }
        stored
    }

    /// A table built field by field, as `extend_open` builds it: one
    /// record per added field, each kept. A copied field map per version
    /// would hold 2000 · 2001 / 2 entries.
    #[test]
    fn tables_built_field_by_field_share_their_fields() {
        let mut types = Types::new();
        let mut owner = types.record(Record {
            open: true,
            ..Record::default()
        });
        for i in 0..2000u32 {
            let Ty::Record(mut r) = types.get(owner).clone() else {
                unreachable!()
            };
            r.fields = r.fields.with_appended(format!("f{i}").into(), field(i));
            owner = types.record(r);
        }
        let Ty::Record(r) = types.get(owner) else {
            unreachable!()
        };
        assert_eq!(r.fields.len(), 2000);
        assert_eq!(r.fields.keys().nth(1234), Some("f1234"));
        let stored = stored_fields(&types);
        assert!(stored < 2000 * 2000 / 32, "{stored} field entries");
    }

    /// A watched check tells what its unions' order depends on: members
    /// born of it keep their order anywhere; others need the same shapes
    /// made before it in the same order; a member without a shape makes
    /// the order unknowable.
    #[test]
    fn unions_tell_what_their_order_depends_on() {
        let rec = |types: &mut Types, name: &str| {
            types.record(Record {
                fields: [(name.into(), field(1))]
                    .into_iter()
                    .collect::<FieldMap>()
                    .into(),
                ..Record::default()
            })
        };
        let str_shape = |s: &str| Shape::Str(s.into());
        let mut types = Types::new();
        let old = types.str("old");
        let older = types.str("older");
        types.watch_births();
        let a = rec(&mut types, "a");
        let b = rec(&mut types, "b");
        let arr = types.array(a);
        types.union(&[a, b, arr, Types::NIL, Types::FALSE]);
        types.union(&[a, Types::NUMBER]);
        assert_eq!(types.births(), Some(vec![]), "records, builtins");
        types.watch_births();
        let c = rec(&mut types, "c");
        let new = types.str("new");
        let strings = types.array(Types::STRING);
        types.union(&[c, older, new, strings, old]);
        let unborn = types.births().expect("every member has a shape");
        assert_eq!(
            unborn,
            vec![
                (str_shape("old"), true),
                (str_shape("older"), true),
                (str_shape("new"), false),
                (Shape::Array(Box::new(Shape::Builtin(Types::STRING))), false),
            ]
        );
        assert!(types.unborn_holds(&vec![(str_shape("old"), true)]));
        // Elsewhere: in another order, missing, or made before.
        let mut other = Types::new();
        other.str("older");
        other.str("old");
        assert!(!other.unborn_holds(&unborn), "another order");
        let mut other = Types::new();
        other.str("old");
        assert!(!other.unborn_holds(&unborn), "`older` is not there");
        let mut other = Types::new();
        other.str("old");
        other.str("older");
        assert!(other.unborn_holds(&unborn));
        other.array(Types::STRING);
        assert!(!other.unborn_holds(&unborn), "`string[]` made before");
        // A union of shapes has one, whatever the order of its members.
        let ns = types.union(&[Types::NUMBER, Types::STRING]);
        let ns_arr = types.array(ns);
        let mut other = Types::new();
        let found = other.union(&[Types::STRING, Types::NUMBER]);
        let found = other.array(found);
        assert_eq!(
            types
                .shape(ns_arr)
                .and_then(|shape| other.find_shape(&shape)),
            Some(found)
        );
        // A function over shapes has one, found again elsewhere.
        types.watch_births();
        let r = rec(&mut types, "r");
        let f = types.func(Func {
            type_params: Vec::new(),
            params: vec![Param {
                name: Some("x".into()),
                ty: Types::NUMBER,
                optional: false,
            }],
            rest: None,
            ret: Pack {
                types: vec![Types::STRING],
                rest: Some(Rest::Variadic(Types::BOOLEAN)),
            },
            predicate: None,
            unannotated: false,
        });
        types.union(&[r, f]);
        let unborn = types.births().expect("the function has a shape");
        assert_eq!(unborn.len(), 1);
        let mut other = Types::new();
        let g = other.func(match types.get(f) {
            Ty::Func(func) => (**func).clone(),
            _ => unreachable!(),
        });
        assert_eq!(other.find_shape(&unborn[0].0), Some(g));
        types.watch_births();
        let d = rec(&mut types, "d");
        let named = types.intern(Ty::Named(DefId(0), Vec::new()));
        types.union(&[d, named]);
        assert_eq!(types.births(), None, "a member without a shape");
        types.watch_births();
        let cp = types.checkpoint();
        let f = rec(&mut types, "f");
        rec(&mut types, "f2");
        types.rollback(cp);
        let g = types.str("g");
        assert_eq!(f, g, "the record's id is the string's now");
        let h = rec(&mut types, "h");
        types.union(&[g, h]);
        assert_eq!(
            types.births(),
            Some(vec![(str_shape("g"), false)]),
            "rolled back births are forgotten"
        );
    }

    /// `has_shape` is `shape(..).is_some()`.
    #[test]
    fn has_shape_agrees_with_shape() {
        let mut types = Types::new();
        let s = types.str("s");
        let arr = types.array(s);
        let named = types.intern(Ty::Named(DefId(0), Vec::new()));
        let bad = types.array(named);
        let f = types.func(Func {
            type_params: Vec::new(),
            params: vec![Param {
                name: None,
                ty: arr,
                optional: true,
            }],
            rest: Some(Rest::Variadic(Types::NUMBER)),
            ret: Pack::single(s),
            predicate: None,
            unannotated: false,
        });
        let g = types.func(Func {
            type_params: Vec::new(),
            params: Vec::new(),
            rest: None,
            ret: Pack::single(bad),
            predicate: None,
            unannotated: false,
        });
        let u = types.union(&[s, arr, Types::NIL]);
        for t in [Types::ANY, s, arr, named, bad, f, g, u] {
            assert_eq!(
                types.has_shape(t),
                types.shape(t).is_some(),
                "{:?}",
                types.get(t)
            );
        }
    }

    /// After a rollback the arena is as it was: the same types get the
    /// same ids again, and nothing made since is found any more.
    #[test]
    fn rollback_forgets_what_came_after() {
        let mut types = Types::new();
        let n = types.num(1.0);
        let r0 = types.record(Record {
            fields: [("a".into(), field(1))]
                .into_iter()
                .collect::<FieldMap>()
                .into(),
            ..Record::default()
        });
        let cp = types.checkpoint();
        let before = types.data.len();
        let s = types.str("x");
        let u = types.union(&[n, s]);
        let arr = types.array(u);
        let r1 = types.record(Record {
            fields: [("b".into(), field(2))]
                .into_iter()
                .collect::<FieldMap>()
                .into(),
            ..Record::default()
        });
        let Ty::Record(rec1) = types.get(r1).clone() else {
            unreachable!()
        };
        types.rollback(cp);
        assert_eq!(types.data.len(), before);
        assert_eq!(
            types.index.len(),
            before - 1,
            "every type but the one record"
        );
        assert_eq!(types.maps.len(), 1);
        // Made again in the same order: the same ids.
        assert_eq!(types.str("x"), s);
        assert_eq!(types.union(&[n, s]), u);
        assert_eq!(types.array(u), arr);
        let again = types.record(Record {
            fields: [("b".into(), field(2))]
                .into_iter()
                .collect::<FieldMap>()
                .into(),
            ..Record::default()
        });
        assert_eq!(again, r1);
        let Ty::Record(rec2) = types.get(again) else {
            unreachable!()
        };
        assert_eq!(rec2.id, rec1.id);
        // What existed before the checkpoint is still there.
        assert_eq!(types.num(1.0), n);
        assert!(matches!(types.get(r0), Ty::Record(_)));
    }

    // The arena holds one `Ty` per type, and a type-checked data addon has
    // over a million of them, mostly records.
    #[test]
    fn types_stay_small() {
        assert!(std::mem::size_of::<Ty>() <= 48);
        assert!(std::mem::size_of::<Record>() <= 40);
    }
}
