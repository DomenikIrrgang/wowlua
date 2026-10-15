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
    Func(Box<Func>),
    /// An instance of a class or interface, an enum, or a generic alias
    /// application that is expanded on demand.
    Named(DefId, Vec<TypeId>),
    /// The class value of a class (`ClassOf<T>`), or a namespace table.
    ClassOf(DefId),
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
        self.data.push(ty);
        let Types { data, index, .. } = self;
        index.insert_unique(hash, id, |&id| FxBuildHasher.hash_one(&data[id.0 as usize]));
        id
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
        self.intern(Ty::Func(Box::new(f)))
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
        self.data.push(Ty::Record(r));
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
            self.maps
                .insert_unique(hash, base.clone(), |m| ordered_hash(m));
        }
    }

    /// The union of `members`: flattened, deduplicated; `never` dropped,
    /// `any` absorbs everything, literals absorbed by their primitive.
    pub fn union(&mut self, members: &[TypeId]) -> TypeId {
        let mut flat: Vec<TypeId> = Vec::new();
        for &m in members {
            match self.get(m) {
                Ty::Union(inner) => {
                    for &i in inner.clone().iter() {
                        if !flat.contains(&i) {
                            flat.push(i);
                        }
                    }
                }
                Ty::Never => {}
                _ => {
                    if !flat.contains(&m) {
                        flat.push(m);
                    }
                }
            }
        }
        if flat.contains(&Types::ANY) {
            return Types::ANY;
        }
        if flat.contains(&Types::ERROR) {
            return Types::ERROR;
        }
        // Literals absorbed by their primitive; true | false is boolean.
        let has = |flat: &[TypeId], t: TypeId| flat.contains(&t);
        let prims: Vec<TypeId> = flat.clone();
        flat.retain(|&m| match self.get(m) {
            Ty::Bool(_) => !has(&prims, Types::BOOLEAN),
            Ty::Num(_) => !has(&prims, Types::NUMBER),
            Ty::Str(_) => !has(&prims, Types::STRING),
            _ => true,
        });
        if flat.contains(&Types::TRUE) && flat.contains(&Types::FALSE) {
            flat.retain(|&m| m != Types::TRUE && m != Types::FALSE);
            flat.push(Types::BOOLEAN);
        }
        match flat.len() {
            0 => Types::NEVER,
            1 => flat[0],
            _ => {
                flat.sort();
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

    // The arena holds one `Ty` per type, and a type-checked data addon has
    // over a million of them, mostly records.
    #[test]
    fn types_stay_small() {
        assert!(std::mem::size_of::<Ty>() <= 48);
        assert!(std::mem::size_of::<Record>() <= 40);
    }
}
