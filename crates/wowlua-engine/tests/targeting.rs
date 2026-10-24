//! Targeting from the keyboard, assist and focus, and the player's buffs
//! cancelled (decision T291); an edit box's history (decision T289).

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    // As Blizzard's code (secure): the targeting functions are protected.
    let f = vm.load(src.as_bytes(), "@Secure.lua").expect("compiles");
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
fn tab_targeting_cycles_the_enemies_in_range_nearest_first() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local near = mock.spawnUnit({ name = 'Near', reaction = 'hostile' })
local far = mock.spawnUnit({ name = 'Far', reaction = 'neutral' })
local gone = mock.spawnUnit({ name = 'Gone', reaction = 'hostile' })
local friend = mock.spawnUnit({ name = 'Friend', reaction = 'friendly' })
near:setDistance(5)
far:setDistance(30)
gone:setDistance(60)
friend:setDistance(2)
local seen = {}
for i = 1, 3 do TargetNearestEnemy() seen[#seen + 1] = UnitName('target') end
TargetNearestEnemy(true)
seen[#seen + 1] = UnitName('target')
TargetNearestFriend()
seen[#seen + 1] = UnitName('target')
TargetLastTarget()
seen[#seen + 1] = UnitName('target')
return table.concat(seen, ' ')",
    );
    assert_eq!(r, ["Near Far Near Far Friend Far"]);
}

#[test]
fn assist_and_focus() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local tank = mock.spawnUnit({ name = 'Tank', reaction = 'friendly' })
local boss = mock.spawnUnit({ name = 'Boss', reaction = 'hostile' })
tank:target(boss)
mock.player:target(tank)
local changes = 0
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_FOCUS_CHANGED')
f:SetScript('OnEvent', function() changes = changes + 1 end)
FocusUnit()
FocusUnit('target')
AssistUnit()
return UnitName('target'), UnitName('focus'), changes",
    );
    assert_eq!(r, ["Boss", "Tank", "1"]);
}

#[test]
fn the_players_buffs_can_be_cancelled() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.player:applyAura({ spellId = 1459, name = 'Arcane Intellect', duration = 3600 })
mock.player:applyAura({ spellId = 1243, name = 'Power Word: Fortitude', duration = 1800 })
CancelUnitBuff('player', 1)
local left = C_UnitAuras.GetBuffDataByIndex('player', 1)
CancelSpellByName('Power Word: Fortitude')
return left and left.name, tostring(C_UnitAuras.GetBuffDataByIndex('player', 1))",
    );
    assert_eq!(r, ["Power Word: Fortitude", "nil"]);
}

#[test]
fn the_arrows_step_through_an_edit_boxs_history() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local e = CreateFrame('EditBox', nil, UIParent)
e:SetHistoryLines(2)
e:AddHistoryLine('one')
e:AddHistoryLine('two')
e:AddHistoryLine('three')
e:SetFocus()
local seen = {}
for _, key in ipairs({ 'UP', 'UP', 'UP', 'DOWN', 'DOWN' }) do
    mock.pressKey(key)
    seen[#seen + 1] = '[' .. e:GetText() .. ']'
end
e:SetText('abc')
mock.pressKey('LEFT')
mock.pressKey('LEFT')
e:Insert('-')
seen[#seen + 1] = e:GetText()
e:ClearHistory()
mock.pressKey('UP')
seen[#seen + 1] = e:GetText()
return table.concat(seen, ' ')",
    );
    // Two lines kept (the oldest gone); past the newest, an empty text.
    assert_eq!(r, ["[three] [two] [two] [three] [] a-bc a-bc"]);
}

#[test]
fn auto_attack_assignments_and_the_last_enemy() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local mob = mock.spawnUnit({ name = 'Mob', reaction = 'hostile' })
local pal = mock.spawnUnit({ name = 'Pal', reaction = 'friendly' })
local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_ENTER_COMBAT')
f:RegisterEvent('PLAYER_LEAVE_COMBAT')
f:SetScript('OnEvent', function(_, e) seen[#seen + 1] = e end)
mock.player:target(pal)
StartAttack()
mock.player:target(mob)
StartAttack()
StopAttack()
mock.player:target(pal)
TargetLastEnemy()
seen[#seen + 1] = UnitName('target')
TargetLastFriend()
seen[#seen + 1] = UnitName('target')
SetPartyAssignment('MAINTANK', 'target')
seen[#seen + 1] = tostring(GetPartyAssignment('MAINTANK', 'target'))
ClearPartyAssignment('MAINTANK', 'target')
seen[#seen + 1] = tostring(GetPartyAssignment('MAINTANK', 'target'))
seen[#seen + 1] = Sign(-3) .. Sign(0) .. Sign(2)
return table.concat(seen, ' ')",
    );
    assert_eq!(
        r,
        ["PLAYER_ENTER_COMBAT PLAYER_LEAVE_COMBAT Mob Pal true false -101"]
    );
}

#[test]
fn money_on_the_cursor() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.player:setMoney(500)
local changes = 0
local f = CreateFrame('Frame')
f:RegisterEvent('CURSOR_CHANGED')
f:SetScript('OnEvent', function() changes = changes + 1 end)
PickupPlayerMoney(900)
local tooMuch = GetCursorMoney()
PickupPlayerMoney(200)
local held = GetCursorMoney()
ClearCursor()
return tooMuch, held, GetCursorMoney(), changes",
    );
    assert_eq!(r, ["0", "200", "0", "2"]);
}
