//! Where the engine's answer to each function and widget method comes
//! from, for the coverage report (spec emulator §17.8, `cargo xtask
//! coverage-report`). Read from the tables [`crate::install`] builds the
//! engine from, in the order it lets them take each other's place, so the
//! report follows the code.

use std::collections::{BTreeSet, HashMap, HashSet};

use wowlua_vm::{Value, Vm};

use crate::{CAMELOT_ABSENT, OVER_RECORDED, api, extras, facts, protection, widgets};

/// Where an answer comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// A model or rule of an engine module, or the VM's runtime.
    Model,
    /// The client's recorded answers.
    Recorded,
    /// A fill-in module's answer, from the documentation, Blizzard's UI
    /// code and the function's name (`fill_ins`; unverified).
    FillIn,
    /// The documentation's generated stand-in alone: "not mocked" for an
    /// addon, the documented types' empty values for Blizzard's code.
    Generated,
}

/// Which list a function comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// The flavor's API documentation.
    Documented,
    /// The client's C functions the documentation lacks (camelot's
    /// inventory, `data/camelot-functions.txt`).
    Undocumented,
    /// Functions the engine gives behaviour that neither the documentation
    /// nor the client's inventory lists.
    EngineOnly,
}

/// A function or widget method and where its answer comes from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// `C_Timer.After`, `GetTime`; a method's class and name
    /// (`Frame:Show`).
    pub name: String,
    pub group: Group,
    pub source: Source,
    /// The module that answers (`clock`, `vm`), empty for a generated one.
    pub module: &'static str,
}

/// Whether the VM's runtime defines `name` before the engine is installed
/// (`issecretvalue`, `strsplit`): the engine leaves it.
fn defined(vm: &mut Vm, name: &str) -> bool {
    let (table, short) = match name.split_once('.') {
        None => (vm.globals, name),
        Some((ns, short)) => match vm.get_global(ns) {
            Value::Table(t) => (t, short),
            _ => return false,
        },
    };
    let key = vm.heap.intern(short.as_bytes());
    !vm.heap.table(table).get_str(key).is_nil()
}

/// Every global and namespaced function of a flavor: the documented ones,
/// the client's undocumented ones and the engine's own, by name.
pub fn functions(flavor: &str) -> Result<Vec<Entry>, String> {
    let api = api::api(flavor).ok_or_else(|| format!("no API documentation for `{flavor}`"))?;
    let mut vm = Vm::wow();
    let mut fill: HashMap<&'static str, &'static str> = HashMap::new();
    for m in crate::fill_ins() {
        for (name, _) in m.list {
            fill.insert(name, m.module);
        }
    }
    let mut own: HashMap<&'static str, (&'static str, bool)> = HashMap::new();
    for m in crate::own_behaviours(flavor) {
        for (name, _) in m.list {
            if m.recorded && OVER_RECORDED.contains(&name) {
                continue;
            }
            own.insert(name, (m.module, m.recorded));
        }
    }
    let recorded: HashSet<&'static str> = facts::recorded_names(flavor).into_iter().collect();
    // The behaviours as `install` lets them take each other's place: the
    // engine's own over the recorded facts over the fill-ins, but for the
    // fill-ins a setter changes.
    let behaviour = |name: &str| -> Option<(Source, &'static str)> {
        if let Some(&(module, rec)) = own.get(name) {
            let source = if rec { Source::Recorded } else { Source::Model };
            return Some((source, module));
        }
        if let Some(&module) = fill.get(name) {
            if !OVER_RECORDED.contains(&name) && recorded.contains(name) {
                return Some((Source::Recorded, "facts"));
            }
            return Some((Source::FillIn, module));
        }
        recorded
            .contains(name)
            .then_some((Source::Recorded, "facts"))
    };
    let protected =
        |name: &str| protection::PROTECTED.contains(&name) || protection::HARDWARE.contains(&name);
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for f in &api.functions {
        if !seen.insert(f.name) {
            continue;
        }
        let (source, module) = if defined(&mut vm, f.name) {
            (Source::Model, "vm")
        } else if extras::silent_unchecked(&f.doc) || extras::silent_checked(&f.doc) {
            // Answered nothing by the documentation's flags, before any
            // behaviour (the full probe).
            (Source::Model, "extras")
        } else {
            behaviour(f.name).unwrap_or((Source::Generated, ""))
        };
        out.push(Entry {
            name: f.name.to_string(),
            group: Group::Documented,
            source,
            module,
        });
    }
    let inventory: BTreeSet<&'static str> = if flavor == "camelot" {
        include_str!("../data/camelot-functions.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .collect()
    } else {
        BTreeSet::new()
    };
    for &name in &inventory {
        if !seen.insert(name) {
            continue;
        }
        let (source, module) = if defined(&mut vm, name) {
            (Source::Model, "vm")
        } else if name == "CreateFrame" {
            (Source::Model, "widgets")
        } else if let Some(found) = behaviour(name) {
            found
        } else if protected(name) {
            (Source::Model, "protection")
        } else {
            (Source::Generated, "")
        };
        out.push(Entry {
            name: name.to_string(),
            group: Group::Undocumented,
            source,
            module,
        });
    }
    let mut extra: BTreeSet<&'static str> = own.keys().copied().collect();
    extra.extend(fill.keys().copied());
    extra.extend(recorded.iter().copied());
    extra.extend(protection::PROTECTED.iter().copied());
    extra.extend(protection::HARDWARE.iter().filter(|n| !n.contains('.')));
    for name in extra {
        if seen.contains(name) || defined(&mut vm, name) {
            continue;
        }
        // Not defined in camelot's environment (`install`).
        if flavor == "camelot" && CAMELOT_ABSENT.contains(&name) {
            continue;
        }
        seen.insert(name);
        let (source, module) = behaviour(name).unwrap_or((Source::Model, "protection"));
        out.push(Entry {
            name: name.to_string(),
            group: Group::EngineOnly,
            source,
            module,
        });
    }
    out.sort();
    Ok(out)
}

/// Every documented widget method of a flavor (`Frame:Show`), by class.
pub fn methods(flavor: &str) -> Result<Vec<Entry>, String> {
    let mut vm = Vm::wow();
    crate::install(&mut vm, flavor)?;
    let e = crate::engine(&vm);
    let mut modules: HashMap<&'static str, (&'static str, bool)> = HashMap::new();
    for m in crate::method_behaviours() {
        let fill_in = matches!(m.module, "methods2" | "methods3");
        for (name, _) in m.list {
            modules.insert(name, (m.module, fill_in));
        }
    }
    let mut out = Vec::new();
    for f in &e.api.methods {
        let class = f.namespace.unwrap_or("");
        let returns = f
            .doc
            .get("Returns")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        // As `widgets::method` answers: a behaviour (by the full name,
        // then the name), the stored setter, a fresh widget's recorded
        // getter, the stored flag, nothing.
        let (source, module) = match modules.get(f.name).or_else(|| modules.get(f.short)) {
            Some(&(module, true)) => (Source::FillIn, module),
            Some(&(module, false)) => (Source::Model, module),
            None if f.short.starts_with("Set") || f.short.starts_with("Enable") => {
                (Source::Model, "widgets")
            }
            None if widgets::recorded_default(e, class, f.short).is_some() => {
                (Source::Recorded, "widgetdata")
            }
            None if f.short.starts_with("Is") && returns == 1 => (Source::Model, "widgets"),
            None => (Source::Generated, ""),
        };
        out.push(Entry {
            name: f.name.to_string(),
            group: Group::Documented,
            source,
            module,
        });
    }
    out.sort();
    out.dedup_by(|a, b| a.name == b.name);
    Ok(out)
}
