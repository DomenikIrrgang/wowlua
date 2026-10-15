//! Auras on the world's units, read through C_UnitAuras.

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
fn applied_auras_fire_unit_aura_and_read_back() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterUnitEvent('UNIT_AURA', 'player', 'target')
f:SetScript('OnEvent', function(_, e, unit, info)
    seen[#seen + 1] = unit .. ':' .. (info.addedAuras and info.addedAuras[1].name or ('-' .. info.removedAuraInstanceIDs[1]))
end)
local mob = mock.spawnUnit({ name = 'Mob', reaction = 'hostile' })
mock.player:target(mob)
local buff = mock.player:applyAura({ spellId = 1459, name = 'Arcane Intellect', duration = 3600, source = mock.player })
mob:applyAura({ spellId = 116, name = 'Frostbolt', harmful = true, duration = 8, source = 'player', dispelName = 'Magic' })
mob:applyAura({ spellId = 1, name = 'Other', harmful = true })
local a = C_UnitAuras.GetAuraDataByIndex('player', 1, 'HELPFUL')
local d = C_UnitAuras.GetAuraDataByIndex('target', 1, 'HARMFUL|PLAYER')
local mine = C_UnitAuras.GetDebuffDataByIndex('target', 2)
mock.player:removeAura(buff)
return table.concat(seen, ','), a.name, a.spellId, a.isHelpful, a.sourceUnit, a.expirationTime, d.name, d.dispelName,
    mine.name, tostring((C_UnitAuras.GetAuraDataByIndex('player', 1, 'HELPFUL'))), select('#', C_UnitAuras.GetAuraSlots('target', 'HARMFUL'))",
    );
    assert_eq!(
        r,
        [
            "player:Arcane Intellect,target:Frostbolt,target:Other,player:-1",
            "Arcane Intellect",
            "1459",
            "true",
            "player",
            "3600",
            "Frostbolt",
            "Magic",
            "Other",
            "nil",
            "3"
        ]
    );
}

#[test]
fn tainted_reads_in_combat_fail_as_recorded() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.player:applyAura({ spellId = 1459, name = 'Arcane Intellect' })
mock.enterInstance()
local onMap = type(C_UnitAuras.GetAuraDataByIndex('player', 1, 'HELPFUL'))
mock.leaveInstance()
mock.player:enterCombat()
return onMap, select(2, pcall(C_UnitAuras.GetAuraDataByIndex, 'player', 1, 'HELPFUL'))",
    );
    assert_eq!(
        r,
        [
            "table",
            "GetAuraDataByIndex(): Auras cannot be accessed when secret while tainted by 'MyAddon'\nLua Taint: MyAddon"
        ]
    );
}
