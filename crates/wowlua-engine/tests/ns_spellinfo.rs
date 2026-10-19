//! The spells' and specializations' remaining answers (`spellinfo`).

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
fn specializations_by_id() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local id, name, _, icon, role, file, className = GetSpecializationInfoByID(1491)
return id, name, icon, role, file, className, GetSpecializationNameForSpecID(1484),
    GetSpecializationRoleEnumByID(1482), GetSpecializationInfoByID(9999)",
    );
    assert_eq!(
        r,
        ["1491", "Warrior", "626008", "DAMAGER", "WARRIOR", "Warrior", "Druid", "2"]
    );
}

#[test]
fn a_defined_spell_by_name_and_its_range() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineSpell(133, { name = 'Fireball', iconID = 135812, known = true, maxRange = 35 })
return C_Spell.GetSpellIDForSpellIdentifier('Fireball'), C_Spell.SpellHasRange(133),
    C_Spell.IsSpellInRange(133, 'player'), C_SpellBook.FindSpellOverrideByID(133),
    C_Spell.GetSpellCooldownDuration(133):IsZero()",
    );
    assert_eq!(r, ["133", "true", "true", "133", "true"]);
}

#[test]
fn the_client_s_base_cooldowns_and_queue_window() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local cd, gcd = GetSpellBaseCooldown(1058)
return cd, gcd, GetSpellQueueWindow(), GetTotemTimeLeft(1), C_Spell.IsSpellHarmful(1058)",
    );
    assert_eq!(r, ["0", "1500", "400", "0", "false"]);
}

#[test]
fn the_cooldown_viewer_keeps_its_layout() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = C_CooldownViewer.GetLayoutData()
C_CooldownViewer.SetLayoutData('1|x')
return before ~= '', C_CooldownViewer.GetLayoutData()",
    );
    assert_eq!(r, ["true", "1|x"]);
}
