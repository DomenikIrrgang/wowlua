//! What `/stdtest engine` recorded of each widget type on WoW Forever
//! (camelot): the methods its metatable offers, and a fresh widget's
//! getter results (`data/camelot-widget-*.txt`, from the fixture
//! `tests/client/forever-camelot-engine.lua`).

use std::collections::HashMap;
use std::sync::OnceLock;

use wowlua_vm::{Value, Vm};

/// A recorded result value.
#[derive(Clone, Debug)]
pub enum Recorded {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
}

impl Recorded {
    pub fn value(&self, vm: &mut Vm) -> Value {
        match self {
            Recorded::Nil => Value::Nil,
            Recorded::Bool(b) => Value::Bool(*b),
            Recorded::Num(n) => Value::Num(*n),
            Recorded::Str(s) => vm.str(s),
        }
    }
}

pub struct WidgetData {
    /// Widget type → its methods.
    pub methods: HashMap<String, Vec<String>>,
    /// (widget type, getter) → a fresh widget's results.
    pub defaults: HashMap<(String, String), Vec<Recorded>>,
    /// (widget type, script) → `HasScript`'s answer.
    pub scripts: HashMap<(String, String), bool>,
}

/// The probe's text of results: `2: string OVERLAY, number 0`.
fn parse_results(text: &str) -> Option<Vec<Recorded>> {
    let (count, rest) = text.split_once(": ").or_else(|| {
        text.strip_suffix(':')
            .map(|c| (c, ""))
            .or_else(|| text.strip_suffix(": ").map(|c| (c, "")))
    })?;
    let count: usize = count.trim().parse().ok()?;
    if count == 0 {
        return Some(Vec::new());
    }
    let parts: Vec<&str> = rest.splitn(count, ", ").collect();
    let mut out = Vec::new();
    for p in parts {
        let (ty, value) = p.split_once(' ').unwrap_or((p, ""));
        out.push(match ty {
            "nil" => Recorded::Nil,
            "boolean" => Recorded::Bool(value == "true"),
            "number" => Recorded::Num(value.parse().ok()?),
            "string" => Recorded::Str(value.to_string()),
            _ => return None,
        });
    }
    (out.len() == count).then_some(out)
}

fn parse() -> WidgetData {
    let mut methods = HashMap::new();
    for line in include_str!("../data/camelot-widget-methods.txt").lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some((ty, names)) = line.split_once('\t') {
            methods.insert(
                ty.to_string(),
                names.split(' ').map(str::to_string).collect(),
            );
        }
    }
    let mut defaults = HashMap::new();
    for line in include_str!("../data/camelot-widget-defaults.txt").lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let (Some(ty), Some(getter), Some(results)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if let Some(values) = parse_results(results) {
            defaults.insert((ty.to_string(), getter.to_string()), values);
        }
    }
    let mut scripts = HashMap::new();
    for line in include_str!("../data/camelot-widget-scripts.txt").lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some((ty, answers)) = line.split_once('\t') {
            for a in answers.split(' ') {
                if let Some((script, yes)) = a.split_once('=') {
                    scripts.insert((ty.to_string(), script.to_string()), yes == "true");
                }
            }
        }
    }
    WidgetData {
        methods,
        defaults,
        scripts,
    }
}

/// The recorded data, for flavors that have it (camelot).
pub fn data(flavor: &str) -> Option<&'static WidgetData> {
    static CAMELOT: OnceLock<WidgetData> = OnceLock::new();
    (flavor == "camelot").then(|| CAMELOT.get_or_init(parse))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_text() {
        let r = parse_results("2: string OVERLAY, number 0").unwrap();
        assert!(matches!(&r[0], Recorded::Str(s) if s == "OVERLAY"));
        assert!(matches!(r[1], Recorded::Num(n) if n == 0.0));
        assert!(parse_results("0: ").unwrap().is_empty());
        assert!(
            matches!(parse_results("1: string ").unwrap()[0], Recorded::Str(ref s) if s.is_empty())
        );
        assert!(matches!(
            parse_results("1: nil nil").unwrap()[0],
            Recorded::Nil
        ));
    }
}
