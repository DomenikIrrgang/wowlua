//! Questions about types: display in messages, members of named types
//! (through inheritance), and assignability (spec §2.6).

use super::env::{DefKind, Env, Member, MemberKind, Visibility};
use super::subst;
use super::types::*;
use rustc_hash::FxHashSet;

/// The checking mode of a file (spec §2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Basic,
    Strict,
}

impl Env<'_> {
    /// A type as messages show it.
    pub fn display(&self, t: TypeId) -> String {
        self.display_depth(t, 0)
    }

    fn display_depth(&self, t: TypeId, depth: usize) -> String {
        if depth > 4 {
            return "…".into();
        }
        let d = |x: TypeId| self.display_depth(x, depth + 1);
        match self.types.get(t) {
            Ty::Any => "any".into(),
            Ty::Unknown => "unknown".into(),
            Ty::Never => "never".into(),
            Ty::Error => "any".into(),
            Ty::Nil => "nil".into(),
            Ty::Boolean => "boolean".into(),
            Ty::Number => "number".into(),
            Ty::String => "string".into(),
            Ty::Table => "table".into(),
            Ty::Function => "function".into(),
            Ty::Thread => "thread".into(),
            Ty::Userdata => "userdata".into(),
            Ty::Bool(b) => b.to_string(),
            Ty::Num(n) => number_text(*n),
            Ty::Str(s) => format!("\"{s}\""),
            Ty::Union(m) => {
                let non_nil: Vec<&TypeId> = m.iter().filter(|&&x| x != Types::NIL).collect();
                if non_nil.len() == 1 && m.len() == 2 {
                    let inner = d(*non_nil[0]);
                    return if matches!(self.types.get(*non_nil[0]), Ty::Func(_) | Ty::Union(_)) {
                        format!("({inner})?")
                    } else {
                        format!("{inner}?")
                    };
                }
                // A long union (`EventName`) shows its first members.
                const SHOWN: usize = 8;
                if m.len() > SHOWN + 4 {
                    let mut parts: Vec<String> = m.iter().take(SHOWN).map(|&x| d(x)).collect();
                    parts.push(format!("… ({} more)", m.len() - SHOWN));
                    return parts.join(" | ");
                }
                m.iter().map(|&x| d(x)).collect::<Vec<_>>().join(" | ")
            }
            Ty::Intersection(m) => m.iter().map(|&x| d(x)).collect::<Vec<_>>().join(" & "),
            Ty::Array(e) => match self.types.get(*e) {
                Ty::Union(_) | Ty::Func(_) => format!("({})[]", d(*e)),
                _ => format!("{}[]", d(*e)),
            },
            Ty::Map(k, v) => format!("{{ [{}]: {} }}", d(*k), d(*v)),
            Ty::Record(r) => {
                let mut parts: Vec<String> = r
                    .fields
                    .iter()
                    .take(4)
                    .map(|(n, f)| {
                        // `x?: T` (the type includes `nil`; not shown twice).
                        let ty = match (f.optional, self.types.get(f.ty)) {
                            (true, Ty::Union(m)) => {
                                let rest: Vec<String> = m
                                    .iter()
                                    .filter(|&&x| !matches!(self.types.get(x), Ty::Nil))
                                    .map(|&x| d(x))
                                    .collect();
                                rest.join(" | ")
                            }
                            _ => d(f.ty),
                        };
                        format!("{n}{}: {ty}", if f.optional { "?" } else { "" })
                    })
                    .collect();
                if r.fields.len() > 4 {
                    parts.push("…".into());
                }
                if let Some((k, v)) = r.indexer {
                    parts.push(format!("[{}]: {}", d(k), d(v)));
                }
                if parts.is_empty() {
                    "{}".into()
                } else {
                    format!("{{ {} }}", parts.join(", "))
                }
            }
            Ty::Func(f) => self.display_func(f, depth),
            Ty::Named(def, args) => {
                let name = self.view(*def).name();
                if args.is_empty() {
                    name.to_string()
                } else {
                    format!(
                        "{name}<{}>",
                        args.iter().map(|&a| d(a)).collect::<Vec<_>>().join(", ")
                    )
                }
            }
            Ty::ClassOf(def) => {
                let v = self.view(*def);
                match v.kind() {
                    DefKind::Class => format!("ClassOf<{}>", v.name()),
                    _ => format!("typeof {}", v.name()),
                }
            }
            Ty::EnumMember(def, i) => {
                let e = self.view(*def);
                match e.enum_member_at(*i) {
                    Some((m, _)) => format!("{}.{m}", e.name()),
                    None => e.name().to_string(),
                }
            }
            Ty::Param(p) => self.params[p.0 as usize].name.to_string(),
            Ty::ClassOfParam(p) => format!("ClassOf<{}>", self.params[p.0 as usize].name),
            Ty::EventArgs(e) => format!("EventArgs<{}>", d(*e)),
            Ty::KeyOf(t) => format!("keyof {}", d(*t)),
            Ty::IndexOf(o, k) => format!("{}[{}]", d(*o), d(*k)),
            Ty::Secret(i) => format!("Secret<{}>", d(*i)),
            Ty::Module(_) => "module".into(),
        }
    }

    fn display_func(&self, f: &Func, depth: usize) -> String {
        let d = |x: TypeId| self.display_depth(x, depth + 1);
        let mut params: Vec<String> = f
            .params
            .iter()
            .map(|p| match &p.name {
                Some(n) => format!("{n}: {}", d(p.ty)),
                None => d(p.ty),
            })
            .collect();
        match &f.rest {
            Some(Rest::Variadic(t)) => params.push(format!("...{}", d(*t))),
            Some(Rest::Pack(p)) => params.push(format!("...{}...", self.params[p.0 as usize].name)),
            None => {}
        }
        format!(
            "({}) -> {}",
            params.join(", "),
            self.display_pack(&f.ret, depth)
        )
    }

    pub fn display_pack(&self, p: &Pack, depth: usize) -> String {
        let d = |x: TypeId| self.display_depth(x, depth + 1);
        let mut parts: Vec<String> = p.types.iter().map(|&t| d(t)).collect();
        match &p.rest {
            Some(Rest::Variadic(t)) => parts.push(format!("...{}", d(*t))),
            Some(Rest::Pack(id)) => parts.push(format!("{}...", self.params[id.0 as usize].name)),
            None => {}
        }
        if parts.len() == 1 && p.rest.is_none() {
            parts.remove(0)
        } else {
            format!("({})", parts.join(", "))
        }
    }

    /// The superclass or extended interfaces of a named type, with type
    /// arguments substituted.
    pub fn bases_of(&mut self, def: DefId, args: &[TypeId]) -> Vec<TypeId> {
        let params = self.view(def).params().to_vec();
        let bases = self.view(def).bases().to_vec();
        bases
            .into_iter()
            .map(|b| subst::substitute(&mut self.types, b, &params, args))
            .collect()
    }

    /// An instance member of a class or interface, found through its
    /// superclasses, with type arguments substituted. Returns the member
    /// and the class that declares it.
    pub fn instance_member(
        &mut self,
        def: DefId,
        args: &[TypeId],
        name: &str,
    ) -> Option<(Member, DefId)> {
        let mut seen = FxHashSet::default();
        let found = self.instance_member_inner(def, args, name, &mut seen);
        // A declared class's `Self` is the receiver (decision L46).
        match found {
            Some((m, at)) => match self.view(at).self_param() {
                Some(sp) => {
                    let receiver = self.types.intern(Ty::Named(def, args.to_vec()));
                    let ty = subst::substitute(&mut self.types, m.ty, &[sp], &[receiver]);
                    Some((Member { ty, ..m }, at))
                }
                None => Some((m, at)),
            },
            None => None,
        }
    }

    fn instance_member_inner(
        &mut self,
        def: DefId,
        args: &[TypeId],
        name: &str,
        seen: &mut FxHashSet<DefId>,
    ) -> Option<(Member, DefId)> {
        if !seen.insert(def) {
            return None;
        }
        if let Some(m) = self.view(def).member(name).cloned() {
            let params = self.view(def).params().to_vec();
            let ty = subst::substitute(&mut self.types, m.ty, &params, args);
            return Some((Member { ty, ..m }, def));
        }
        for b in self.bases_of(def, args) {
            if let Ty::Named(bd, bargs) = self.types.get(b).clone()
                && let Some(found) = self.instance_member_inner(bd, &bargs, name, seen)
            {
                return Some(found);
            }
        }
        None
    }

    /// A static member of a class (inherited statics included).
    pub fn static_member(&mut self, def: DefId, name: &str) -> Option<Member> {
        let mut current = Some(def);
        let mut guard = 0;
        while let Some(d) = current {
            if let Some(m) = self.view(d).static_member(name) {
                return Some(m.clone());
            }
            guard += 1;
            if guard > 64 {
                return None;
            }
            current = self
                .view(d)
                .bases()
                .first()
                .and_then(|&b| match self.types.get(b) {
                    Ty::Named(bd, _) => Some(*bd),
                    _ => None,
                });
        }
        None
    }

    /// The indexer of a named type (`AddonNamespace` has `[string]: any`;
    /// frames are open tables: XML templates add children by `parentKey`).
    pub fn indexer_of(&self, def: DefId) -> Option<(TypeId, TypeId)> {
        let mut current = Some(def);
        let mut guard = 0;
        while let Some(d) = current {
            let v = self.view(d);
            if let Some(i) = v.indexer() {
                return Some(i);
            }
            // Declared (native) classes are open: frames and mixins get
            // fields at run time, and the declarations are incomplete.
            if v.widget().is_some() || (v.native() && v.kind() == DefKind::Class) {
                return Some((Types::STRING, Types::ANY));
            }
            guard += 1;
            if guard > 64 {
                return None;
            }
            current = v.bases().first().and_then(|&b| match self.types.get(b) {
                Ty::Named(bd, _) => Some(*bd),
                _ => None,
            });
        }
        None
    }

    /// A class of the project's (a widget class) open only through the game's
    /// frames it is built on: its own fields are declared, so an unknown one
    /// is `any` nobody wrote (W2063, L51).
    pub fn open_through_native(&self, def: DefId) -> bool {
        if self.view(def).native() {
            return false;
        }
        let mut current = Some(def);
        let mut guard = 0;
        while let Some(d) = current {
            let v = self.view(d);
            if v.indexer().is_some() {
                return false;
            }
            if v.native() && (v.widget().is_some() || v.kind() == DefKind::Class) {
                return true;
            }
            guard += 1;
            if guard > 64 {
                return false;
            }
            current = v.bases().first().and_then(|&b| match self.types.get(b) {
                Ty::Named(bd, _) => Some(*bd),
                _ => None,
            });
        }
        false
    }

    /// `sub` is `sup` or one of its subclasses / sub-interfaces.
    pub fn derives(&self, sub: DefId, sup: DefId) -> bool {
        let mut stack = vec![sub];
        let mut seen = FxHashSet::default();
        while let Some(d) = stack.pop() {
            if d == sup {
                return true;
            }
            if !seen.insert(d) {
                continue;
            }
            for &b in self.view(d).bases() {
                if let Ty::Named(bd, _) = self.types.get(b) {
                    stack.push(*bd);
                }
            }
        }
        false
    }

    /// Is `s` assignable to `t`?
    pub fn assignable(&mut self, s: TypeId, t: TypeId, mode: Mode) -> bool {
        let mut assuming = FxHashSet::default();
        self.assignable_in(s, t, mode, &mut assuming)
    }

    fn assignable_in(
        &mut self,
        s: TypeId,
        t: TypeId,
        mode: Mode,
        assuming: &mut FxHashSet<(TypeId, TypeId)>,
    ) -> bool {
        if s == t {
            return true;
        }
        if !assuming.insert((s, t)) {
            return true;
        }
        let result = self.assignable_step(s, t, mode, assuming);
        assuming.remove(&(s, t));
        result
    }

    fn assignable_step(
        &mut self,
        s: TypeId,
        t: TypeId,
        mode: Mode,
        assuming: &mut FxHashSet<(TypeId, TypeId)>,
    ) -> bool {
        // A recursive record alias matches as its record does (L58).
        if let Some(s2) = self.unfold_alias(s) {
            return self.assignable_in(s2, t, mode, assuming);
        }
        if let Some(t2) = self.unfold_alias(t) {
            return self.assignable_in(s, t2, mode, assuming);
        }
        let st = self.types.get(s).clone();
        let tt = self.types.get(t).clone();
        match (&st, &tt) {
            (_, Ty::Any | Ty::Error | Ty::Unknown | Ty::EventArgs(_) | Ty::IndexOf(..))
            | (Ty::Any | Ty::Error | Ty::Never | Ty::EventArgs(_) | Ty::IndexOf(..), _) => {
                return true;
            }
            // `unknown` fits only a union that takes anything (`pairs` over
            // a record with an `unknown` indexer builds one).
            // `keyof T` of a type parameter: field names, so strings.
            (Ty::KeyOf(_), _) => return self.assignable_in(Types::STRING, t, mode, assuming),
            (Ty::Unknown, Ty::Union(members)) => {
                return members
                    .iter()
                    .any(|&m| matches!(self.types.get(m), Ty::Any | Ty::Unknown | Ty::Error));
            }
            (Ty::Unknown, _) => return false,
            (Ty::Nil, _) if mode == Mode::Basic => return true,
            (Ty::Union(members), _) => {
                return members
                    .clone()
                    .iter()
                    .all(|&m| self.assignable_in(m, t, mode, assuming));
            }
            (_, Ty::Union(members)) => {
                if members
                    .clone()
                    .iter()
                    .any(|&m| self.assignable_in(s, m, mode, assuming))
                {
                    return true;
                }
                // `E extends EventName` where `EventName` (a union) is
                // expected: its constraint fits as a whole.
                return match st {
                    Ty::Param(p) => self.params[p.0 as usize]
                        .constraint
                        .is_some_and(|c| self.assignable_in(c, t, mode, assuming)),
                    _ => false,
                };
            }
            (Ty::Intersection(members), _) => {
                return members
                    .clone()
                    .iter()
                    .any(|&m| self.assignable_in(m, t, mode, assuming));
            }
            (_, Ty::Intersection(members)) => {
                // An array for `T[] & { name?: X }`: the record part needs
                // only optional fields (spec §2.6).
                let members = members.clone();
                let with_array = matches!(st, Ty::Array(_))
                    && members
                        .iter()
                        .any(|&m| matches!(self.types.get(m), Ty::Array(_)));
                return members.iter().all(|&m| match self.types.get(m).clone() {
                    Ty::Record(r) if with_array && r.indexer.is_none() => {
                        r.fields.values().all(|f| f.optional)
                    }
                    _ => self.assignable_in(s, m, mode, assuming),
                });
            }
            _ => {}
        }
        match (st, tt) {
            (Ty::Bool(_), Ty::Boolean) | (Ty::Num(_), Ty::Number) | (Ty::Str(_), Ty::String) => {
                true
            }
            // WoW uppercases enum strings (`"topleft"` is `"TOPLEFT"`).
            (Ty::Str(a), Ty::Str(b)) => {
                b.chars().any(|c| c.is_ascii_uppercase())
                    && !b.chars().any(|c| c.is_ascii_lowercase())
                    && a.to_ascii_uppercase() == *b.as_ref()
            }
            (Ty::Secret(a), Ty::Secret(b)) => self.assignable_in(a, b, mode, assuming),
            (Ty::Secret(_), _) | (_, Ty::Secret(_)) => false,
            (Ty::EnumMember(d, i), _) => {
                let member = self.def(d).enum_members.get(i as usize).map(|m| m.1);
                match self.types.get(t).clone() {
                    Ty::Named(td, _) if td == d => true,
                    _ => member.is_some_and(|m| self.assignable_in(m, t, mode, assuming)),
                }
            }
            (Ty::Named(d, _), _) if self.def(d).kind == DefKind::Enum => {
                let members: Vec<TypeId> = self.def(d).enum_members.iter().map(|m| m.1).collect();
                !members.is_empty()
                    && members
                        .iter()
                        .all(|&m| self.assignable_in(m, t, mode, assuming))
            }
            // `table` and `function` accept every value of that kind.
            (
                Ty::Record(_)
                | Ty::Array(_)
                | Ty::Map(..)
                | Ty::ClassOf(_)
                | Ty::ClassOfParam(_)
                | Ty::Module(_)
                | Ty::Table,
                Ty::Table,
            ) => true,
            (Ty::Named(d, _), Ty::Table) => self.def(d).kind != DefKind::Enum,
            (Ty::Func(_) | Ty::Function, Ty::Function) => true,
            // Basic mode: values typed only `table` / `function` are trusted.
            // `{ [any]: any }` is every table: class tables, namespaces and
            // enum tables (`pairs(Enum.X)`) and instances included.
            (Ty::Table | Ty::ClassOf(_) | Ty::ClassOfParam(_) | Ty::Module(_), Ty::Map(k, v))
                if k == Types::ANY && v == Types::ANY =>
            {
                true
            }
            // `table` (a value `type(x) == "table"` narrowed) is any map whose
            // values are `any`: `{ [string]: any }`, `{ [number]: any }`.
            (Ty::Table, Ty::Map(_, v)) if v == Types::ANY => true,
            (Ty::Named(d, _), Ty::Map(k, v))
                if k == Types::ANY
                    && v == Types::ANY
                    && matches!(self.def(d).kind, DefKind::Class | DefKind::Interface) =>
            {
                true
            }
            (Ty::Table, Ty::Record(_) | Ty::Array(_) | Ty::Map(..) | Ty::Named(..)) => {
                mode == Mode::Basic
            }
            // `(...any) -> ...any` is every function.
            (Ty::Function, Ty::Func(g))
                if g.params.is_empty()
                    && matches!(g.rest, Some(Rest::Variadic(t)) if t == Types::ANY)
                    && g.ret.types.is_empty()
                    && matches!(g.ret.rest, Some(Rest::Variadic(t)) if t == Types::ANY) =>
            {
                true
            }
            (Ty::Function, Ty::Func(_)) => mode == Mode::Basic,
            (Ty::Array(a), Ty::Array(b)) => self.element(a, b, mode, assuming),
            (Ty::Array(a), Ty::Map(k, v)) => {
                self.assignable_in(Types::NUMBER, k, mode, assuming)
                    && self.element(a, v, mode, assuming)
            }
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
                self.element(k1, k2, mode, assuming) && self.element(v1, v2, mode, assuming)
            }
            (Ty::Record(r), Ty::Map(k, v)) => {
                // Every field must fit the map; an indexer too.
                let fields_ok = r.fields.iter().all(|(n, f)| {
                    let key = self.types.str(n);
                    self.assignable_in(key, k, mode, assuming)
                        && self.assignable_in(f.ty, v, mode, assuming)
                });
                let indexer_ok = r.indexer.is_none_or(|(rk, rv)| {
                    self.assignable_in(rk, k, mode, assuming)
                        && self.assignable_in(rv, v, mode, assuming)
                });
                fields_ok && indexer_ok
            }
            // A table used both as an array and a record (`{ ..., n = … }`,
            // spec §2.2.4) is an array of its numeric indexer's values.
            (Ty::Record(r), Ty::Array(e)) => match r.indexer {
                Some((rk, rv)) => {
                    self.assignable_in(rk, Types::NUMBER, mode, assuming)
                        && self.assignable_in(rv, e, mode, assuming)
                }
                None => r.fields.is_empty(),
            },
            // An array is a record whose named fields are all optional and
            // whose numeric indexer takes its elements (without an indexer,
            // only beside an array type in an intersection, above).
            (Ty::Array(e), Ty::Record(target)) => {
                target.fields.values().all(|f| f.optional)
                    && target.indexer.is_some_and(|(k, v)| {
                        self.assignable_in(Types::NUMBER, k, mode, assuming)
                            && self.element(e, v, mode, assuming)
                    })
            }
            (_, Ty::Record(target)) => self.satisfies_record(s, &target, mode, assuming),
            (Ty::Named(sd, sargs), Ty::Named(td, targs)) => {
                if sd == td {
                    return sargs
                        .iter()
                        .zip(&targs)
                        .all(|(&a, &b)| self.element(a, b, mode, assuming));
                }
                let tkind = self.def(td).kind;
                if self.derives(sd, td) {
                    // Through the superclass chain with its arguments.
                    for b in self.bases_of(sd, &sargs) {
                        if self.assignable_in(b, t, mode, assuming) {
                            return true;
                        }
                    }
                    return targs.is_empty() || self.def(td).params.is_empty();
                }
                // Interfaces are structural; classes nominal.
                tkind == DefKind::Interface
                    && self.satisfies_interface(s, td, &targs, mode, assuming)
            }
            (Ty::Record(_), Ty::Named(td, targs)) => {
                self.def(td).kind == DefKind::Interface
                    && self.satisfies_interface(s, td, &targs, mode, assuming)
            }
            (Ty::ClassOf(a), Ty::ClassOf(b)) => self.derives(a, b),
            (Ty::Func(f), Ty::Func(g)) => self.func_assignable(&f, &g, mode, assuming),
            (Ty::Param(p), _) => match self.params[p.0 as usize].constraint {
                Some(c) => self.assignable_in(c, t, mode, assuming),
                None => false,
            },
            _ => false,
        }
    }

    /// A recursive record alias's body with its type arguments (L58).
    pub fn unfold_alias(&mut self, t: TypeId) -> Option<TypeId> {
        let Ty::Named(d, args) = self.types.get(t).clone() else {
            return None;
        };
        let def = self.def(d);
        if !def.recursive_alias {
            return None;
        }
        let body = def.alias?;
        if args.is_empty() {
            return Some(body);
        }
        let params = def.params.clone();
        Some(super::subst::substitute(
            &mut self.types,
            body,
            &params,
            &args,
        ))
    }

    /// Element types of arrays, maps and type arguments: covariant in basic
    /// mode, invariant in strict mode (spec §2.2.6).
    fn element(
        &mut self,
        a: TypeId,
        b: TypeId,
        mode: Mode,
        assuming: &mut FxHashSet<(TypeId, TypeId)>,
    ) -> bool {
        match mode {
            Mode::Basic => self.assignable_in(a, b, mode, assuming),
            Mode::Strict => {
                self.assignable_in(a, b, mode, assuming) && self.assignable_in(b, a, mode, assuming)
            }
        }
    }

    /// The type of field `name` of `s` for structural checks.
    pub fn field_of(&mut self, s: TypeId, name: &str) -> Option<(TypeId, bool)> {
        match self.types.get(s).clone() {
            Ty::Record(r) => match r.fields.get(name) {
                Some(f) => Some((f.ty, f.optional)),
                None => r.indexer.and_then(|(k, v)| {
                    let key = self.types.str(name);
                    self.assignable(key, k, Mode::Basic).then_some((v, true))
                }),
            },
            Ty::Named(d, args) => match self.instance_member(d, &args, name) {
                Some((m, _)) if m.visibility == Visibility::Public => {
                    let ty = match m.kind {
                        // A method seen as a field takes `self` first.
                        MemberKind::Method => self.with_self(m.ty, s),
                        _ => m.ty,
                    };
                    Some((ty, m.optional))
                }
                Some(_) => None,
                None => self.indexer_of(d).map(|(_, v)| (v, true)),
            },
            _ => None,
        }
    }

    /// A method's type as a plain function with `self` as first parameter.
    pub fn with_self(&mut self, method: TypeId, receiver: TypeId) -> TypeId {
        match self.types.get(method).clone() {
            Ty::Func(f) => {
                let mut f = std::sync::Arc::unwrap_or_clone(f);
                f.params.insert(
                    0,
                    Param {
                        name: Some("self".into()),
                        ty: receiver,
                        optional: false,
                    },
                );
                if let Some((i, t)) = f.predicate {
                    f.predicate = Some((i + 1, t));
                }
                self.types.func(f)
            }
            Ty::Intersection(m) => {
                let m: Vec<TypeId> = m.iter().map(|&x| self.with_self(x, receiver)).collect();
                self.types.intern(Ty::Intersection(m))
            }
            _ => method,
        }
    }

    fn satisfies_record(
        &mut self,
        s: TypeId,
        target: &Record,
        mode: Mode,
        assuming: &mut FxHashSet<(TypeId, TypeId)>,
    ) -> bool {
        match self.types.get(s) {
            Ty::Record(_) | Ty::Named(..) => {}
            _ => return false,
        }
        for (name, f) in target.fields.iter() {
            match self.field_of(s, name) {
                Some((ty, _)) => {
                    if !self.assignable_in(ty, f.ty, mode, assuming) {
                        return false;
                    }
                }
                None => {
                    if !(f.optional || (mode == Mode::Basic && self.types.is_optional(f.ty))) {
                        return false;
                    }
                }
            }
        }
        if let (Some((k, v)), Ty::Record(sr)) = (target.indexer, self.types.get(s).clone()) {
            for (name, f) in sr.fields.iter() {
                let key = self.types.str(name);
                if self.assignable_in(key, k, mode, assuming)
                    && !self.assignable_in(f.ty, v, mode, assuming)
                {
                    return false;
                }
            }
        }
        true
    }

    fn satisfies_interface(
        &mut self,
        s: TypeId,
        iface: DefId,
        args: &[TypeId],
        mode: Mode,
        assuming: &mut FxHashSet<(TypeId, TypeId)>,
    ) -> bool {
        // All members, inherited ones included.
        let mut names: Vec<Box<str>> = Vec::new();
        let mut stack = vec![iface];
        let mut seen = FxHashSet::default();
        while let Some(d) = stack.pop() {
            if !seen.insert(d) {
                continue;
            }
            names.extend(self.def(d).members.keys().cloned());
            for &b in &self.def(d).bases.clone() {
                if let Ty::Named(bd, _) = self.types.get(b) {
                    stack.push(*bd);
                }
            }
        }
        for name in names {
            let Some((m, _)) = self.instance_member(iface, args, &name) else {
                continue;
            };
            let want = match m.kind {
                MemberKind::Method => self.with_self(m.ty, s),
                _ => m.ty,
            };
            match self.field_of(s, &name) {
                Some((ty, _)) => {
                    if !self.assignable_in(ty, want, mode, assuming) {
                        return false;
                    }
                }
                None => {
                    if !(m.optional || (mode == Mode::Basic && self.types.is_optional(m.ty))) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn func_assignable(
        &mut self,
        f: &Func,
        g: &Func,
        mode: Mode,
        assuming: &mut FxHashSet<(TypeId, TypeId)>,
    ) -> bool {
        // Generic sources are compared with their parameters as `any`.
        let f = if f.type_params.is_empty() {
            f.clone()
        } else {
            let mut s = subst::Subst::default();
            for p in &f.type_params {
                if self.params[p.0 as usize].pack {
                    s.packs.insert(*p, Pack::any());
                } else {
                    s.types.insert(*p, Types::ANY);
                }
            }
            subst::apply_func(&mut self.types, f, &s)
        };
        // Parameters: the target's arguments must be accepted.
        for (i, gp) in g.params.iter().enumerate() {
            let fp = match f.params.get(i) {
                Some(p) => p.ty,
                None => match &f.rest {
                    Some(Rest::Variadic(t)) => *t,
                    Some(Rest::Pack(_)) => Types::ANY,
                    // `S` may take fewer parameters than `T`.
                    None => continue,
                },
            };
            let ok = match mode {
                Mode::Strict => self.assignable_in(gp.ty, fp, mode, assuming),
                Mode::Basic => {
                    self.assignable_in(gp.ty, fp, mode, assuming)
                        || self.assignable_in(fp, gp.ty, mode, assuming)
                }
            };
            if !ok {
                return false;
            }
        }
        // Required parameters of `f` beyond `g`'s are a mismatch.
        if f.params.len() > g.params.len() && g.rest.is_none() {
            let extra_required = f.params[g.params.len()..].iter().any(|p| !p.optional);
            if extra_required && mode == Mode::Strict {
                return false;
            }
        }
        // Results compare covariantly; a target with no results accepts any.
        for (i, &gr) in g.ret.types.iter().enumerate() {
            let fr = match f.ret.types.get(i) {
                Some(&t) => t,
                None => match &f.ret.rest {
                    Some(Rest::Variadic(t)) => *t,
                    Some(Rest::Pack(_)) => Types::ANY,
                    None => Types::NIL,
                },
            };
            if !self.assignable_in(fr, gr, mode, assuming) {
                return false;
            }
        }
        true
    }
}
