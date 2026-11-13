//! The traits' state and recorded values (wowlua-engine's `traits` answers from them).

use std::collections::BTreeMap;

use wowlua_vm::{Value, Vm};

use crate::hooks;

pub fn owner_answer(call: &str) -> Option<&'static &'static str> {
    (hooks::get().traits_answer)(call)
}

pub fn player_config(vm: &Vm, config: f64) -> f64 {
    (hooks::get().player_config)(vm, config)
}

/// A recorded value (`string x`, `number 1`, `boolean true`, `nil nil`,
/// `{k=v,…}`).
#[derive(Clone, Debug, PartialEq)]
pub enum Rv {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Table(Vec<(String, Rv)>),
}

impl Rv {
    pub fn parse(text: &str) -> Rv {
        let text = text.trim_start();
        if let Some(inner) = text.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
            let mut fields = Vec::new();
            for entry in crate::recorded::split_top(inner, ',') {
                if let Some((k, v)) = entry.split_once('=') {
                    fields.push((k.to_string(), Rv::parse(v)));
                }
            }
            return Rv::Table(fields);
        }
        match text.split_once(' ') {
            Some(("string", s)) => Rv::Str(s.to_string()),
            Some(("number", n)) => n.parse().map_or(Rv::Nil, Rv::Num),
            Some(("boolean", b)) => Rv::Bool(b == "true"),
            _ => Rv::Nil,
        }
    }

    /// An answer's values (`n: v1, v2`).
    pub fn answer(text: &str) -> Vec<Rv> {
        let Some((count, rest)) = text.split_once(": ") else {
            return Vec::new();
        };
        if count == "0" {
            return Vec::new();
        }
        let starts = crate::recorded::regex_starts(rest);
        (0..starts.len())
            .map(|i| {
                let end = starts.get(i + 1).map_or(rest.len(), |&e| e - 2);
                Rv::parse(&rest[starts[i]..end])
            })
            .collect()
    }

    pub fn get(&self, key: &str) -> Option<&Rv> {
        match self {
            Rv::Table(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        match self.get(key) {
            Some(Rv::Num(n)) => Some(*n),
            _ => None,
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Rv::Bool(true)))
    }

    /// A list's values in order (`{1=…,2=…}`, keys as numbers).
    pub fn list(&self) -> Vec<&Rv> {
        let Rv::Table(fields) = self else {
            return Vec::new();
        };
        let mut items: Vec<(f64, &Rv)> = fields
            .iter()
            .filter_map(|(k, v)| k.parse::<f64>().ok().map(|i| (i, v)))
            .collect();
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        items.into_iter().map(|(_, v)| v).collect()
    }

    pub fn nums(&self, key: &str) -> Vec<i64> {
        self.get(key)
            .map(|l| {
                l.list()
                    .into_iter()
                    .filter_map(|v| match v {
                        Rv::Num(n) => Some(*n as i64),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set(&mut self, key: &str, value: Rv) {
        if let Rv::Table(fields) = self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => {
                    fields.push((key.to_string(), value));
                    fields.sort_by(|a, b| a.0.cmp(&b.0));
                }
            }
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Rv::Table(fields) = self {
            fields.retain(|(k, _)| k != key);
        }
    }

    pub fn list_of(items: Vec<Rv>) -> Rv {
        Rv::Table(
            items
                .into_iter()
                .enumerate()
                .map(|(i, v)| ((i + 1).to_string(), v))
                .collect(),
        )
    }

    pub fn to_value(&self, vm: &mut Vm) -> Value {
        match self {
            Rv::Nil => Value::Nil,
            Rv::Bool(b) => Value::Bool(*b),
            Rv::Num(n) => Value::Num(*n),
            Rv::Str(s) => vm.str(s),
            Rv::Table(fields) => {
                let t = vm.new_table();
                for (k, v) in fields {
                    let key = match k.parse::<f64>() {
                        Ok(n) => Value::Num(n),
                        Err(_) => vm.str(k),
                    };
                    let value = v.to_value(vm);
                    let _ = vm.heap.table_mut(t).set(key, value);
                }
                Value::Table(t)
            }
        }
    }
}

/// The ranks the engine keeps: by (config, node).
#[derive(Default)]
pub struct Traits {
    /// Committed ranks set by the test or a commit (else the recording's).
    pub committed: BTreeMap<(i64, i64), i64>,
    /// Ranks bought and not applied yet.
    pub staged: BTreeMap<(i64, i64), i64>,
    /// The ranks committed when the session began (set by the test).
    pub start: BTreeMap<(i64, i64), i64>,
    /// Renown by major faction (`e.setRenown`).
    pub renown: BTreeMap<i64, f64>,
    /// The recorded character playing (`e.setTraitCharacter`).
    pub character: Option<String>,
}
