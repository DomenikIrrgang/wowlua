//! The type representation (spec chapter 2): hash-consed types (`TypeId`),
//! records, functions with type packs, named types (classes, interfaces,
//! enums, generic aliases) and their display in messages.

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;
use std::hash::BuildHasher;

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

/// A record's fields, in the order they were added. The hasher takes no
/// room in the record (a type-checked data addon has a million records);
/// nothing depends on the hash values.
pub type Fields = IndexMap<Box<str>, Field, FxBuildHasher>;

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
        self.next_record += 1;
        r.id = self.next_record;
        let id = TypeId(self.data.len() as u32);
        self.data.push(Ty::Record(r));
        id
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

/// Formats numbers like Lua (`%.14g`) for literal types.
pub fn number_text(bits: u64) -> String {
    crate::constant::format_g14(f64::from_bits(bits))
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;

    // The arena holds one `Ty` per type, and a type-checked data addon has
    // over a million of them, mostly records.
    #[test]
    fn types_stay_small() {
        assert!(std::mem::size_of::<Ty>() <= 80);
        assert!(std::mem::size_of::<Record>() <= 80);
    }
}
