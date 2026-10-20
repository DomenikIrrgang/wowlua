//! Collections: the recorded wardrobe, mounts, statistics and titles, and
//! the journals' filters reading back what was set.

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    let text = |vm: &Vm, v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

#[test]
fn recorded_collections_answer() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local source = C_TransmogCollection.GetSourceInfo(21598)
local _, _, statID = GetStatistic(122, 1)
return #C_MountJournal.GetMountIDs(), source.itemID, source.visualID,
    C_TransmogCollection.GetSourceItemID(21598), statID, GetNumTitles(), GetCurrentTitle(),
    HasCompletedAnyAchievement(), PlayerHasToy(1), pcall(C_ToyBox.GetToyInfo)",
    );
    // The three mounts whose info was recorded (the answers battery's two,
    // the full probe's first of the journal) are listed.
    assert_eq!(
        &r[..9],
        [
            "3", "44803", "11257", "44803", "60", "111", "-1", "true", "false"
        ]
    );
    assert_eq!(r[9], "false");
    assert!(r[10].contains("Usage: GetToyInfo(itemID)"), "{}", r[10]);
}

#[test]
fn journal_filters_read_back() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = C_ToyBoxInfo.IsUsingDefaultFilters()
C_ToyBox.SetCollectedShown(false)
local shown, changed = C_ToyBox.GetCollectedShown(), C_ToyBoxInfo.IsUsingDefaultFilters()
C_ToyBoxInfo.SetDefaultFilters()
C_PetJournal.SetSearchFilter('cat')
return before, shown, changed, C_ToyBox.GetCollectedShown(), C_PetJournal.GetSearchFilter(),
    C_PetBattles.GetAttackModifier(1, 2), C_TransmogOutfitInfo.IsSlotWeaponSlot(12)",
    );
    // The recorded character's toy box starts away from the defaults (the
    // full probe, 1.60.1: unusable toys shown).
    assert_eq!(r, ["false", "false", "false", "true", "cat", "1.5", "true"]);
}
