//! Units as NPCs (decision T530): a vendor's window when the player talks
//! to it, a unit's drops on its corpse, its attack's damage.

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "@Test.lua").expect("compiles");
    let text = |vm: &Vm, v: Value| match vm.hidden(v) {
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
fn a_vendor_opens_its_window_when_talked_to() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        "Seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('MERCHANT_SHOW')
f:SetScript('OnEvent', function(_, e) Seen[#Seen + 1] = e end)
local inn = mock.spawnUnit({ name = 'Innkeeper', reaction = 'friendly', distance = 3 })
mock.defineItem(159, { name = 'Refreshing Spring Water', quality = 1 })
mock.defineVendor(inn, { name = 'Innkeeper', items = { { id = 159, price = 25, stack = 5 } }, canRepair = true })
mock.player:interact(inn)
local before = #Seen
mock.advanceFrames(60, 1000 / 60)
return before, table.concat(Seen, ','), GetMerchantNumItems(), tostring(CanMerchantRepair())",
    );
    // (the server's answer a round trip later)
    assert_eq!(out, ["0", "MERCHANT_SHOW", "1", "true"]);
}

#[test]
fn a_units_drops_are_its_corpses_and_its_attack_hits() {
    let mut vm = vm();
    run(
        &mut vm,
        "mock.setPlayer({ health = 1000, maxHealth = 1000 })
Wolf = mock.spawnUnit({ name = 'Wolf', reaction = 'hostile', distance = 3,
    loot = { { id = 159, count = 2 } }, attack = { damage = 7, speed = 1 },
    actions = { { at = 0, ['do'] = 'attack' } } })
mock.advanceFrames(90, 1000 / 60)
Wolf:kill({ by = mock.player })",
    );
    let e = wowlua_engine::engine(&vm);
    let wolf = e
        .units
        .list
        .iter()
        .find(|u| u.name == "Wolf")
        .expect("the wolf");
    assert_eq!(wolf.loot, vec![(159, 2.0)]);
    let player = &e.units.list[0];
    // (a swing at once and one a second later: 7 each)
    assert_eq!(player.health, 1000.0 - 14.0);
}
