//! The engine's API from the documentation.

use wowlua_vm::{Value, Vm};

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "=test").expect("compiles");
    let r = vm.call(f, &[]).unwrap_or_else(|e| match e.value {
        Value::Str(s) => panic!("{}", String::from_utf8_lossy(vm.str_bytes(s))),
        _ => panic!("an error"),
    });
    r.into_iter()
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn documented_functions_exist_and_are_not_mocked() {
    // Every camelot function has behaviour (decision T295); a flavor with
    // gaps shows the stand-in: a function without behaviour fails for
    // addon code, and gives its documented empty values to Blizzard's.
    let gap = wowlua_engine::coverage::functions("mists")
        .expect("mists")
        .into_iter()
        .find(|e| {
            e.source == wowlua_engine::coverage::Source::Generated
                && e.group == wowlua_engine::coverage::Group::Documented
                && !e.name.contains('.')
                && !wowlua_engine::protection::is_protected(&e.name, None)
        })
        .expect("a function without behaviour on mists")
        .name;
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "mists").expect("installs");
    let src =
        format!("return type({gap}), select(2, pcall({gap})), type(GetTime), type(secretwrap)");
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    let r: Vec<String> = vm
        .call(f, &[])
        .expect("runs")
        .into_iter()
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        r,
        [
            "function".to_string(),
            format!("{gap} is not mocked"),
            "function".to_string(),
            "function".to_string()
        ]
    );
    // On camelot it has an answer: its documented empty values.
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(&mut vm, "return IsCollisionEnabled()");
    assert_eq!(r, ["Bool(false)"]);
}

#[test]
fn enums_and_constants() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(
        &mut vm,
        "return tostring(Enum.AddOnRestrictionType.Combat), tostring(Enum.AddOnRestrictionType.Map), tostring(Enum.TableSecurityOption.SecretWrapContents)",
    );
    assert_eq!(r, ["0", "4", "2"]);
}

#[test]
fn every_flavor_installs() {
    for flavor in [
        "standard", "camelot", "mists", "cata", "wrath", "tbc", "vanilla",
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
    }
}

/// The `C_` functions' old global names camelot's client lacks (its
/// inventory, forever-camelot-functions.tsv) are not defined there, the
/// `C_` functions are; Blizzard's deprecation files' action functions
/// stand in as the engine's until its UI code loads; another flavor keeps
/// the old names (unverified).
#[test]
fn camelot_lacks_the_old_global_names() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(
        &mut vm,
        "return type(GetItemCount), type(GetMouseFocus), type(InviteUnit), type(PickupContainerItem),
            type(PickupSpell), type(PickupSpellBookItem), type(RunMacroText), type(SplitContainerItem),
            type(UseContainerItem), type(UseItemByName),
            type(C_Item.GetItemCount), type(GetMouseFoci), type(C_PartyInfo.InviteUnit),
            type(C_Container.PickupContainerItem), type(C_Spell.PickupSpell), type(C_Container.UseContainerItem),
            type(HasAction), type(GetActionText)",
    );
    let mut expected = vec!["nil"; 10];
    expected.extend(["function"; 8]);
    assert_eq!(r, expected);
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "vanilla").expect("installs");
    let r = run(
        &mut vm,
        "return type(GetItemCount), type(PickupContainerItem)",
    );
    assert_eq!(r, ["function", "function"]);
}
