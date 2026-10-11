//! Runtime type checks (spec §7.5): which checks a static type allows at
//! run time, for lowering to emit at function entry and on `::`.

use super::body::Checker;
use super::env::DefKind;
use super::types::{Ty, TypeId, number_text};

/// A check of one value.
#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeCheck {
    /// `type(v)` is one of these.
    Types(Vec<&'static str>),
    /// `v` equals one of these Lua literals (source text).
    Literals(Vec<String>),
    /// An instance of the class with this name (`isInstance`).
    Instance(String),
    /// A table whose required fields are present, with their `type()` name
    /// when it is a primitive.
    Table(Vec<(String, Option<&'static str>)>),
    /// Any of these.
    AnyOf(Vec<RuntimeCheck>),
}

/// The check of a parameter or a `::` assertion.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueCheck {
    pub check: RuntimeCheck,
    /// `nil` passes.
    pub optional: bool,
    /// The static type as the error message names it.
    pub expected: String,
}

fn primitive(ty: &Ty) -> Option<&'static str> {
    Some(match ty {
        Ty::Boolean | Ty::Bool(_) => "boolean",
        Ty::Number | Ty::Num(_) => "number",
        Ty::String | Ty::Str(_) => "string",
        Ty::Table | Ty::Array(_) | Ty::Map(..) | Ty::Record(_) => "table",
        Ty::Function | Ty::Func(_) => "function",
        Ty::Thread => "thread",
        Ty::Userdata => "userdata",
        Ty::Nil => "nil",
        _ => return None,
    })
}

impl Checker<'_> {
    /// The runtime check of `t`, or `None` when it is not checkable (`any`,
    /// generics, …).
    pub(crate) fn value_check(&mut self, t: TypeId) -> Option<ValueCheck> {
        let members = self.env.types.members(t);
        let secret = members
            .iter()
            .any(|&m| matches!(self.env.types.get(m), Ty::Secret(_)));
        let mut optional = false;
        let mut types: Vec<&'static str> = Vec::new();
        let mut literals: Vec<String> = Vec::new();
        let mut others: Vec<RuntimeCheck> = Vec::new();
        for m in members {
            let ty = match self.env.types.get(m).clone() {
                Ty::Secret(inner) => self.env.types.get(inner).clone(),
                ty => ty,
            };
            match ty {
                Ty::Nil => optional = true,
                Ty::Any | Ty::Unknown | Ty::Error | Ty::Param(_) | Ty::Never => return None,
                Ty::Bool(b) if !secret => literals.push(b.to_string()),
                Ty::Num(bits) if !secret => literals.push(number_text(bits)),
                Ty::Str(s) if !secret => literals.push(lua_quote(&s)),
                Ty::Record(r) if !secret => {
                    let fields = r
                        .fields
                        .iter()
                        .filter(|(_, f)| !f.optional && !self.env.types.is_optional(f.ty))
                        .map(|(name, f)| (name.to_string(), primitive(self.env.types.get(f.ty))))
                        .collect();
                    others.push(RuntimeCheck::Table(fields));
                }
                Ty::Named(d, _) if !secret => {
                    let def = self.env.def(d).clone();
                    match def.kind {
                        DefKind::Class if !def.native => {
                            others.push(RuntimeCheck::Instance(def.name.clone()))
                        }
                        DefKind::Enum => {
                            for (_, value) in &def.enum_members {
                                match self.env.types.get(*value) {
                                    Ty::Num(bits) => literals.push(number_text(*bits)),
                                    Ty::Str(s) => literals.push(lua_quote(s)),
                                    _ => return None,
                                }
                            }
                        }
                        DefKind::Interface => {
                            let fields = def
                                .members
                                .iter()
                                .filter(|(_, mem)| {
                                    mem.kind == super::env::MemberKind::Field
                                        && !mem.optional
                                        && !self.env.types.is_optional(mem.ty)
                                })
                                .map(|(name, mem)| {
                                    (name.to_string(), primitive(self.env.types.get(mem.ty)))
                                })
                                .collect();
                            others.push(RuntimeCheck::Table(fields));
                        }
                        // Native classes (frames) and aliases: a table.
                        _ => types.push("table"),
                    }
                }
                Ty::EnumMember(d, i) if !secret => {
                    let value = self.env.def(d).enum_members.get(i as usize).map(|m| m.1)?;
                    match self.env.types.get(value) {
                        Ty::Num(bits) => literals.push(number_text(*bits)),
                        Ty::Str(s) => literals.push(lua_quote(s)),
                        _ => return None,
                    }
                }
                ty => match primitive(&ty) {
                    Some(p) => types.push(p),
                    None => match ty {
                        Ty::Named(..) | Ty::Intersection(_) | Ty::ClassOf(_) | Ty::Module(_) => {
                            types.push("table")
                        }
                        _ => return None,
                    },
                },
            }
        }
        types.sort_unstable();
        types.dedup();
        let mut parts = Vec::new();
        if !types.is_empty() {
            parts.push(RuntimeCheck::Types(types));
        }
        if !literals.is_empty() {
            parts.push(RuntimeCheck::Literals(literals));
        }
        parts.extend(others);
        let check = match parts.len() {
            0 => return None,
            1 => parts.pop().expect("a check"),
            _ => RuntimeCheck::AnyOf(parts),
        };
        Some(ValueCheck {
            check,
            optional,
            expected: self.env.display(t),
        })
    }
}

/// A Lua string literal.
fn lua_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
