//! Substitution of type and pack parameters (generic instantiation).

use super::types::*;
use rustc_hash::FxHashMap;

/// What each type and pack parameter stands for.
#[derive(Clone, Debug, Default)]
pub struct Subst {
    pub types: FxHashMap<ParamId, TypeId>,
    pub packs: FxHashMap<ParamId, Pack>,
}

impl Subst {
    pub fn is_empty(&self) -> bool {
        self.types.is_empty() && self.packs.is_empty()
    }
}

/// `t` with `params` replaced by `args`.
pub fn substitute(types: &mut Types, t: TypeId, params: &[ParamId], args: &[TypeId]) -> TypeId {
    let mut s = Subst::default();
    for (p, a) in params.iter().zip(args) {
        s.types.insert(*p, *a);
    }
    apply(types, t, &s)
}

pub fn apply(types: &mut Types, t: TypeId, s: &Subst) -> TypeId {
    if s.is_empty() {
        return t;
    }
    match types.get(t).clone() {
        Ty::Param(p) => match s.types.get(&p) {
            Some(&r) => r,
            // A pack parameter used as a single type: its first element.
            None => match s.packs.get(&p) {
                Some(pack) => pack.types.first().copied().unwrap_or(Types::NIL),
                None => t,
            },
        },
        Ty::Union(m) => {
            let m: Vec<TypeId> = m.iter().map(|&x| apply(types, x, s)).collect();
            types.union(&m)
        }
        Ty::Intersection(m) => {
            let m: Vec<TypeId> = m.iter().map(|&x| apply(types, x, s)).collect();
            types.intern(Ty::Intersection(m))
        }
        Ty::Array(e) => {
            let e = apply(types, e, s);
            types.array(e)
        }
        Ty::Map(k, v) => {
            let k = apply(types, k, s);
            let v = apply(types, v, s);
            types.intern(Ty::Map(k, v))
        }
        Ty::Secret(i) => {
            let i = apply(types, i, s);
            types.intern(Ty::Secret(i))
        }
        Ty::Named(d, args) => {
            let args: Vec<TypeId> = args.iter().map(|&x| apply(types, x, s)).collect();
            types.intern(Ty::Named(d, args))
        }
        Ty::Record(mut r) => {
            let tys: Vec<TypeId> = r.fields.values().map(|f| f.ty).collect();
            let applied: Vec<TypeId> = tys.iter().map(|&t| apply(types, t, s)).collect();
            let mut changed = applied != tys;
            if changed {
                let mut fields = FieldMap::default();
                fields.reserve(applied.len());
                for ((name, f), ty) in r.fields.iter().zip(applied) {
                    fields.insert(name.into(), Field { ty, ..*f });
                }
                r.fields = fields.into();
            }
            if let Some((k, v)) = r.indexer {
                let (nk, nv) = (apply(types, k, s), apply(types, v, s));
                changed |= nk != k || nv != v;
                r.indexer = Some((nk, nv));
            }
            if changed { types.record(r) } else { t }
        }
        Ty::Func(f) => {
            let f = apply_func(types, &f, s);
            types.func(f)
        }
        _ => t,
    }
}

pub fn apply_func(types: &mut Types, f: &Func, s: &Subst) -> Func {
    let params = f
        .params
        .iter()
        .map(|p| Param {
            name: p.name.clone(),
            ty: apply(types, p.ty, s),
            optional: p.optional,
        })
        .collect();
    let mut out = Func {
        type_params: f.type_params.clone(),
        params,
        rest: None,
        ret: apply_pack(types, &f.ret, s),
        predicate: f.predicate.map(|(i, t)| (i, apply(types, t, s))),
        unannotated: f.unannotated,
    };
    // A pack parameter as the rest expands into parameters.
    match &f.rest {
        Some(Rest::Pack(p)) if s.packs.contains_key(p) => {
            let pack = s.packs[p].clone();
            for t in pack.types {
                out.params.push(Param {
                    name: None,
                    ty: t,
                    optional: false,
                });
            }
            out.rest = pack.rest;
        }
        Some(Rest::Variadic(t)) => out.rest = Some(Rest::Variadic(apply(types, *t, s))),
        other => out.rest = other.clone(),
    }
    out.type_params
        .retain(|p| !s.types.contains_key(p) && !s.packs.contains_key(p));
    out
}

pub fn apply_pack(types: &mut Types, p: &Pack, s: &Subst) -> Pack {
    let mut out = Pack {
        types: p.types.iter().map(|&t| apply(types, t, s)).collect(),
        rest: None,
    };
    match &p.rest {
        Some(Rest::Pack(id)) if s.packs.contains_key(id) => {
            let pack = s.packs[id].clone();
            out.types.extend(pack.types);
            out.rest = pack.rest;
        }
        Some(Rest::Variadic(t)) => out.rest = Some(Rest::Variadic(apply(types, *t, s))),
        other => out.rest = other.clone(),
    }
    out
}
