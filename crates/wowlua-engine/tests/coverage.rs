//! The coverage report's view of the engine (`wowlua_engine::coverage`,
//! `cargo xtask coverage-report`) agrees with the engine installed: a
//! function it says has behaviour has it, a generated one has none, and
//! every function it lists exists in the environment.

use wowlua_engine::coverage::{Group, Source};
use wowlua_vm::{Value, Vm};

/// Whether `name` (`GetTime`, `C_Timer.After`) is defined.
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

#[test]
fn coverage_agrees_with_the_installed_engine() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let entries = wowlua_engine::coverage::functions("camelot").expect("camelot");
    assert!(entries.len() > 7000, "{}", entries.len());
    let mut wrong = Vec::new();
    for entry in &entries {
        let has = wowlua_engine::engine(&vm).behaviour(&entry.name).is_some();
        let ok = match (entry.source, entry.module) {
            // Rules outside the behaviour table: the VM's runtime, the
            // documentation's flags, `CreateFrame`, the protected
            // functions; a recorded fact of an undocumented function is a
            // native of its own.
            (Source::Model, "vm" | "extras" | "widgets" | "protection")
            | (Source::Recorded, "facts") => true,
            (Source::Generated, _) => !has,
            _ => has,
        };
        if !ok {
            wrong.push(format!(
                "{} ({:?}, {})",
                entry.name, entry.source, entry.module
            ));
        }
        // A documented function may be Blizzard's secure code's alone (its
        // namespace taken out of the addons' environment); the others are
        // all there.
        if entry.group != Group::Documented && !defined(&mut vm, &entry.name) {
            wrong.push(format!("{} is not defined", entry.name));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let methods = wowlua_engine::coverage::methods("camelot").expect("camelot");
    assert!(methods.len() > 1000, "{}", methods.len());
    assert!(
        methods
            .iter()
            .any(|m| m.name == "Frame:Show" && m.source == Source::Model)
    );
}
