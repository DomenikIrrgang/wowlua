//! `wowlua:meta` (spec chapter 6): the values of one build configuration
//! and flavor, constant access paths into them, and static conditions.

use crate::constant::{self, Const};
use indexmap::IndexMap;
use std::collections::HashMap;
use wowlua_syntax::view::{ArgsKind, Decorator, Expr, ExprKind};

/// A value of `wowlua:meta`.
#[derive(Clone, Debug, PartialEq)]
pub enum MetaValue {
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
    List(Vec<MetaValue>),
    Table(IndexMap<String, MetaValue>),
}

impl MetaValue {
    pub fn str(s: impl Into<String>) -> MetaValue {
        MetaValue::String(s.into())
    }

    pub fn opt_str(s: Option<impl Into<String>>) -> MetaValue {
        s.map_or(MetaValue::Nil, |s| MetaValue::String(s.into()))
    }

    pub fn strings<S: Into<String>>(items: impl IntoIterator<Item = S>) -> MetaValue {
        MetaValue::List(
            items
                .into_iter()
                .map(|s| MetaValue::String(s.into()))
                .collect(),
        )
    }

    /// The scalar as a constant (tables and lists are not constants).
    pub fn as_const(&self) -> Option<Const> {
        Some(match self {
            MetaValue::Nil => Const::Nil,
            MetaValue::Bool(b) => Const::Bool(*b),
            MetaValue::Number(n) => Const::Number(*n),
            MetaValue::String(s) => Const::String(s.clone().into_bytes()),
            MetaValue::List(_) | MetaValue::Table(_) => return None,
        })
    }

    fn get(&self, key: &str) -> Option<&MetaValue> {
        match self {
            MetaValue::Table(t) => t.get(key),
            MetaValue::List(items) => key
                .parse::<usize>()
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| items.get(i)),
            _ => None,
        }
    }

    /// The value as a Lua expression (runtime tables).
    pub fn to_lua(&self, out: &mut String) {
        match self {
            MetaValue::Nil => out.push_str("nil"),
            MetaValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            MetaValue::Number(n) => out.push_str(&constant::format_number(*n)),
            MetaValue::String(s) => {
                out.push_str(&String::from_utf8_lossy(&constant::quote(s.as_bytes())))
            }
            MetaValue::List(items) => {
                out.push('{');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.to_lua(out);
                }
                out.push('}');
            }
            MetaValue::Table(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push('[');
                    out.push_str(&String::from_utf8_lossy(&constant::quote(k.as_bytes())));
                    out.push_str("] = ");
                    v.to_lua(out);
                }
                out.push('}');
            }
        }
    }
}

/// The exports of `wowlua:meta`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MetaKind {
    Addon,
    Build,
    Flavor,
    Config,
    Features,
    Asset,
}

impl MetaKind {
    pub fn from_export(name: &str) -> Option<MetaKind> {
        Some(match name {
            "addon" => MetaKind::Addon,
            "build" => MetaKind::Build,
            "flavor" => MetaKind::Flavor,
            "config" => MetaKind::Config,
            "features" => MetaKind::Features,
            "asset" => MetaKind::Asset,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            MetaKind::Addon => "addon",
            MetaKind::Build => "build",
            MetaKind::Flavor => "flavor",
            MetaKind::Config => "config",
            MetaKind::Features => "features",
            MetaKind::Asset => "asset",
        }
    }
}

/// The `wowlua:meta` values of one build configuration and flavor.
#[derive(Clone, Debug)]
pub struct Meta {
    pub addon: MetaValue,
    pub build: MetaValue,
    pub config: MetaValue,
    pub features: MetaValue,
    /// The flavor in `split` mode (a compile-time constant); `None` in
    /// `shared` mode, where it is a runtime value.
    pub flavor: Option<String>,
    /// `asset()`: package-relative asset path → in-game path.
    pub assets: HashMap<String, String>,
}

/// Where a constant access path into `wowlua:meta` leads.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    /// A scalar: folded to a literal.
    Const(Const),
    /// A table, list or runtime-only value: read from the runtime tables.
    Runtime,
    /// No such field (E2003).
    Missing(String),
}

impl Meta {
    fn root(&self, kind: MetaKind) -> Option<&MetaValue> {
        match kind {
            MetaKind::Addon => Some(&self.addon),
            MetaKind::Build => Some(&self.build),
            MetaKind::Config => Some(&self.config),
            MetaKind::Features => Some(&self.features),
            MetaKind::Flavor | MetaKind::Asset => None,
        }
    }

    /// Looks up `kind.path…`.
    pub fn lookup(&self, kind: MetaKind, path: &[String]) -> Lookup {
        if kind == MetaKind::Flavor {
            return match (&self.flavor, path.is_empty()) {
                (Some(f), true) => Lookup::Const(Const::String(f.clone().into_bytes())),
                (None, true) => Lookup::Runtime,
                (_, false) => Lookup::Missing(format!(
                    "`flavor` is a string; it has no field `{}`",
                    path[0]
                )),
            };
        }
        let Some(mut value) = self.root(kind) else {
            return Lookup::Runtime;
        };
        for (i, key) in path.iter().enumerate() {
            match value.get(key) {
                Some(v) => value = v,
                None => {
                    return match value {
                        // Missing list elements and table keys read as nil
                        // from open tables (`addon.meta`); records are closed.
                        MetaValue::List(_) => Lookup::Const(Const::Nil),
                        _ if kind == MetaKind::Addon
                            && path.first().is_some_and(|p| p == "meta")
                            && i == 1 =>
                        {
                            Lookup::Const(Const::Nil)
                        }
                        _ => {
                            let shown: Vec<&str> = std::iter::once(kind.name())
                                .chain(path[..=i].iter().map(String::as_str))
                                .collect();
                            Lookup::Missing(format!(
                                "`wowlua:meta` has no field `{}`",
                                shown.join(".")
                            ))
                        }
                    };
                }
            }
        }
        match value.as_const() {
            Some(c) => Lookup::Const(c),
            None => Lookup::Runtime,
        }
    }
}

/// A constant access path: `meta.a.b`, `meta["a"]`. `kind_of` gives the
/// `wowlua:meta` export a name is bound to.
pub fn access_path(
    e: Expr<'_>,
    kind_of: &dyn Fn(&str) -> Option<MetaKind>,
) -> Option<(MetaKind, Vec<String>)> {
    match e.kind() {
        ExprKind::Name(n) => kind_of(n.text()).map(|k| (k, Vec::new())),
        ExprKind::Field { obj, name } => {
            let (k, mut path) = access_path(obj, kind_of)?;
            path.push(name.text().to_string());
            Some((k, path))
        }
        ExprKind::Index { obj, key } => {
            let key = match key.kind() {
                ExprKind::String(s) => String::from_utf8_lossy(&s.value()).into_owned(),
                ExprKind::Number(n) => {
                    let v = n.value()?;
                    if v.fract() != 0.0 {
                        return None;
                    }
                    format!("{}", v as i64)
                }
                _ => return None,
            };
            let (k, mut path) = access_path(obj, kind_of)?;
            path.push(key);
            Some((k, path))
        }
        ExprKind::Paren(inner) => access_path(inner, kind_of),
        _ => None,
    }
}

/// The value of a static condition (spec §6.4.1): `wowlua:meta` scalar
/// paths (and `flavor` in split mode), folded constants and literals,
/// combined with the allowed operators.
pub fn static_value(
    e: Expr<'_>,
    meta: &Meta,
    kind_of: &dyn Fn(&str) -> Option<MetaKind>,
    consts: &dyn Fn(&str) -> Option<Const>,
) -> Option<Const> {
    constant::eval_with(e, &|leaf| {
        if let Some((kind, path)) = access_path(leaf, kind_of) {
            return match meta.lookup(kind, &path) {
                Lookup::Const(c) => Some(c),
                _ => None,
            };
        }
        match leaf.kind() {
            ExprKind::Name(n) => consts(n.text()),
            _ => None,
        }
    })
}

/// The expression uses `flavor` (E1039 in shared mode).
pub fn uses_flavor(e: Expr<'_>, kind_of: &dyn Fn(&str) -> Option<MetaKind>) -> bool {
    let mut found = false;
    wowlua_syntax::visit::walk_expr(e, &mut |x| {
        if let Some((MetaKind::Flavor, _)) = access_path(x, kind_of) {
            found = true;
        }
    });
    found
}

/// The `@if(condition)` decorator's condition, if `d` is one.
pub fn if_condition(d: Decorator<'_>) -> Option<Option<Expr<'_>>> {
    let path = d.path();
    if !(path.len() == 1 && path.at(0).text() == "if") {
        return None;
    }
    Some(match d.args().map(|a| a.kind()) {
        Some(ArgsKind::Parens { args, .. }) if args.len() == 1 => Some(args.at(0)),
        _ => None,
    })
}
