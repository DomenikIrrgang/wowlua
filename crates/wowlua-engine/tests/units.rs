//! Units, tokens, combat and restrictions, driven by the controls.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

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
fn targeting_an_enemy_and_reading_it() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_TARGET_CHANGED')
f:RegisterUnitEvent('UNIT_HEALTH', 'target')
f:SetScript('OnEvent', function(_, e, unit) seen[#seen + 1] = e .. (unit and (':' .. unit) or '') end)
local hogger = mock.spawnUnit({ name = 'Hogger', level = 11, reaction = 'hostile', health = 666 })
mock.player:target(hogger)
local name = UnitName('target')
hogger:damage(100)
return table.concat(seen, ' '), name, UnitLevel('target'), UnitExists('target'), UnitCanAttack('player', 'target'),
    UnitIsEnemy('player', 'target'), issecretvalue(UnitHealth('target')), UnitIsPlayer('target')",
    );
    assert_eq!(
        r,
        [
            "PLAYER_TARGET_CHANGED UNIT_HEALTH:target",
            "Hogger",
            "11",
            "true",
            "true",
            "true",
            "true",
            "false"
        ]
    );
}

#[test]
fn combat_restricts_and_locks_down() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_REGEN_DISABLED')
f:RegisterEvent('ADDON_RESTRICTION_STATE_CHANGED')
f:SetScript('OnEvent', function(_, e, t, s) seen[#seen + 1] = e .. (t and ('(' .. t .. ',' .. s .. ')') or '') .. ':' .. tostring(InCombatLockdown()) end)
local mob = mock.spawnUnit({ name = 'Mob', power = 50 })
mock.player:target(mob)
local before = issecretvalue(UnitPower('target'))
mock.player:enterCombat()
return table.concat(seen, ' '), before, issecretvalue(UnitPower('target')), InCombatLockdown(),
    C_RestrictedActions.IsAddOnRestrictionActive(Enum.AddOnRestrictionType.Combat), C_Secrets.ShouldAurasBeSecret(),
    issecretvalue(UnitName('target')), C_RestrictedActions.CheckAllowProtectedFunctions(UIParent, true)",
    );
    assert_eq!(
        r,
        [
            "PLAYER_REGEN_DISABLED:false ADDON_RESTRICTION_STATE_CHANGED(0,1):false",
            // Another unit's power: secret out of combat too (scenarios).
            "true",
            "true",
            "true",
            "true",
            "true",
            "false",
            "false"
        ]
    );
}

/// `CheckAllowProtectedFunctions(object)` refuses tainted code in combat
/// only on a protected object: an addon's plain frame and a tooltip
/// stay its to use (a tooltip shown in a fight), a secure frame and
/// UIParent don't (decision T245).
#[test]
fn protected_functions_are_refused_on_protected_objects() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local plain = CreateFrame('Frame')
local tip = CreateFrame('GameTooltip', 'MyTooltip', UIParent)
local secure = CreateFrame('Button', nil, UIParent, 'SecureActionButtonTemplate')
local check = C_RestrictedActions.CheckAllowProtectedFunctions
local before = tostring(check(secure, true))
mock.player:enterCombat()
return before, tostring(check(plain, true)), tostring(check(tip, true)), tostring(check(secure, true)),
    tostring(check(UIParent, true))",
    );
    assert_eq!(r, ["true", "true", "true", "false", "false"]);
}

#[test]
fn a_threat_list_with_the_group_on_it() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local mob = mock.spawnUnit({ name = 'Mob', reaction = 'hostile', distance = 3 })
local tank = mock.spawnUnit({ name = 'Tank', player = true, reaction = 'friendly' })
mock.addToGroup(tank)
local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('UNIT_THREAT_SITUATION_UPDATE')
f:SetScript('OnEvent', function(_, _, unit) seen[#seen + 1] = unit end)
local function row(unit)
    local tanking, status, scaled, raw, value = UnitDetailedThreatSituation(unit, 'target')
    return table.concat({ tostring(tanking), tostring(status), tostring(scaled and math.floor(scaled + 0.5)),
        tostring(raw and math.floor(raw + 0.5)), tostring(value) }, ' ')
end
mock.player:target(mob)
mob:setThreat(1000, tank)
mob:target(tank)
local alone = row('party1') .. ' | ' .. tostring(UnitThreatSituation('player', 'target'))
mob:setThreat(1050)
local over = row('player') .. ' | ' .. row('party1')
mob:setThreat(1200)
local past = row('player') .. ' | ' .. row('party1') .. ' | ' .. tostring(UnitThreatSituation('party1'))
mob:setThreat(nil, tank)
local gone = row('player') .. ' | ' .. tostring(UnitThreatSituation('party1', 'target'))
-- no target: the highest holds it
mob:setThreat(900, tank)
mob:target(nil)
local highest = row('player') .. ' | ' .. row('party1')
return alone, over, past, gone, highest, table.concat(seen, ' ')",
    );
    assert_eq!(
        r,
        [
            "true 3 100 255 1000 | nil",
            "false 1 95 105 1050 | true 2 100 100 1000",
            "false 1 100 120 1200 | true 2 100 100 1000 | 2",
            "true 3 100 255 1200 | nil",
            "true 3 100 100 1200 | false 0 68 75 900",
            "party1 player targettarget party1 player targettarget party1 player targettarget party1 player party1"
        ]
    );
}

#[test]
fn a_member_logging_out_tells_its_connection() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local friend = mock.spawnUnit({ name = 'Friend', player = true, reaction = 'friendly' })
local stranger = mock.spawnUnit({ name = 'Stranger', player = true, reaction = 'friendly' })
mock.addToGroup(friend)
mock.player:target(friend)
local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('UNIT_CONNECTION')
f:RegisterEvent('GROUP_ROSTER_UPDATE')
f:SetScript('OnEvent', function(_, e, unit, on) seen[#seen + 1] = e .. (unit and ('(' .. unit .. ',' .. tostring(on) .. ')') or '') end)
friend:setOnline(false)
local out = table.concat(seen, ' ') .. ' ' .. tostring(UnitIsConnected('party1'))
seen = {}
friend:setOnline(false)
stranger:setOnline(false)
return out, table.concat(seen, ' ')",
    );
    assert_eq!(
        r,
        [
            "UNIT_CONNECTION(target,false) UNIT_CONNECTION(party1,false) GROUP_ROSTER_UPDATE false",
            "GROUP_ROSTER_UPDATE GROUP_ROSTER_UPDATE"
        ]
    );
}

#[test]
fn identity_is_secret_on_restricted_maps() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local mob = mock.spawnUnit({ name = 'Mob' })
local friend = mock.spawnUnit({ name = 'Friend', player = true, reaction = 'friendly' })
mock.addToGroup(friend)
mock.player:target(mob)
mock.enterInstance()
return issecretvalue(UnitName('target')), issecretvalue(UnitName('party1')), UnitName('party1'), issecretvalue(UnitGUID('player')),
    issecretvalue(UnitHealthMax('party1')), issecretvalue(UnitHealthMax('target')), GetNumGroupMembers(), IsInGroup()",
    );
    // A party member's maximum health is secret too (recorded through
    // `target`, scenario group).
    assert_eq!(
        r,
        [
            "true", "false", "Friend", "false", "true", "true", "2", "true"
        ]
    );
}

#[test]
fn nameplates_and_compound_tokens() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local added = {}
local f = CreateFrame('Frame')
f:RegisterEvent('NAME_PLATE_UNIT_ADDED')
f:SetScript('OnEvent', function(_, _, unit) added[#added + 1] = unit end)
local a = mock.spawnUnit({ name = 'A' })
local b = mock.spawnUnit({ name = 'B' })
a:showNameplate()
b:showNameplate()
a:target(mock.player)
mock.player:target(a)
return table.concat(added, ','), UnitName('nameplate2'), UnitName('targettarget'), UnitIsUnit('target', 'nameplate1'),
    UnitExists('nameplate3'), (UnitName('targettargettarget'))",
    );
    assert_eq!(
        r,
        ["nameplate1,nameplate2", "B", "Player", "true", "false", "A"]
    );
}

#[test]
fn nameplates_are_placed_over_the_world() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local a = mock.spawnUnit({ name = 'A' })
local b = mock.spawnUnit({ name = 'B' })
a:showNameplate()
b:showNameplate()
local pa, pb = C_NamePlate.GetNamePlateForUnit('nameplate1'), C_NamePlate.GetNamePlateForUnit('nameplate2')
local point, rel, _, xa, ya = pa:GetPoint(1)
local _, _, _, xb, yb = pb:GetPoint(1)
local w, h = pa:GetSize()
a:hideNameplate()
a:showNameplate()
local _, _, _, again = pa:GetPoint(1)
return point, rel == UIParent, pa:GetParent() == WorldFrame, w, h, tostring(xa ~= xb or ya ~= yb), tostring(again == xa),
    tostring(math.abs(xa) <= 350 and ya >= -50 and ya <= 200)",
    );
    assert_eq!(
        r,
        [
            "CENTER", "true", "true", "237.5", "85", "true", "true", "true"
        ]
    );
}

/// The player's casts as the client recorded them (`/stdtest engine record
/// cast`, Forever 70425): GUIDs and cast bar ids by first appearance.
#[test]
fn casts_as_the_client_recorded() {
    let mut vm = vm();
    run(
        &mut vm,
        "CastLog = {}
local names, n = {}, 0
local function show(v)
    if type(v) == 'string' and (v:match('^Cast%-') or v:match('^CastBar%-')) then
        if not names[v] then
            n = n + 1
            names[v] = (v:match('^CastBar') and 'bar' or 'guid') .. n
        end
        return names[v]
    end
    return tostring(v)
end
local f = CreateFrame('Frame')
for _, e in ipairs({ 'UNIT_SPELLCAST_SENT', 'UNIT_SPELLCAST_START', 'UNIT_SPELLCAST_STOP', 'UNIT_SPELLCAST_SUCCEEDED',
    'UNIT_SPELLCAST_INTERRUPTED', 'UNIT_SPELLCAST_FAILED_QUIET', 'UNIT_SPELLCAST_CHANNEL_START',
    'UNIT_SPELLCAST_CHANNEL_STOP', 'CURRENT_SPELL_CAST_CHANGED', 'UI_ERROR_MESSAGE' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, ...)
    local args = {}
    for i = 1, select('#', ...) do args[#args + 1] = show((select(i, ...))) end
    CastLog[#CastLog + 1] = e:gsub('^UNIT_SPELLCAST_', '') .. '(' .. table.concat(args, '; ') .. ')'
end)
mock.player:castSpell(647, { castTime = 2.5 })
mock.player:castSpell(647, { castTime = 2.5 })",
    );
    wowlua_engine::clock::advance(&mut vm, 3.0);
    run(
        &mut vm,
        "mock.player:castSpell(647, { castTime = 2.5 })
mock.player:interruptCast()
mock.player:castSpell(19750, { castTime = 0 })
mock.player:castSpell(5143, { castTime = 1, channel = true })",
    );
    wowlua_engine::clock::advance(&mut vm, 2.0);
    let log = run(&mut vm, "return table.concat(CastLog, '\\n')");
    let expected = [
        "SENT(player; nil; guid1; 647)",
        "START(player; guid1; 647; bar2)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        // A cast while casting fails; the running one goes on.
        "SENT(player; nil; guid3; 647)",
        "FAILED_QUIET(player; guid3; 647; nil)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        "UI_ERROR_MESSAGE(280; Another action is in progress)",
        "SUCCEEDED(player; guid1; 647; bar2)",
        "STOP(player; guid1; 647; bar2)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        "SENT(player; nil; guid4; 647)",
        "START(player; guid4; 647; bar5)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        "INTERRUPTED(player; guid4; 647; nil; bar5)",
        "STOP(player; guid4; 647; bar5)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        // Instant.
        "SENT(player; nil; guid6; 19750)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        "SUCCEEDED(player; guid6; 19750; nil)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        // A channel: no cast GUID.
        "SENT(player; nil; guid7; 5143)",
        "CHANNEL_START(player; nil; 5143; bar8)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        // The server's late answer to the interrupt (0.4 s).
        "INTERRUPTED(player; guid4; 647; nil; nil)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        "INTERRUPTED(player; guid4; 647; nil; nil)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        "INTERRUPTED(player; guid4; 647; nil; nil)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
        "UI_ERROR_MESSAGE(57; Interrupted)",
        "CHANNEL_STOP(player; nil; 5143; nil; bar8)",
        "CURRENT_SPELL_CAST_CHANGED(false)",
    ];
    assert_eq!(log[0].lines().collect::<Vec<_>>(), expected);
}

/// Duration objects: a span on the clock, secret when set from secrets
/// (in secure code, as Blizzard's aura buttons do).
#[test]
fn duration_objects() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Span = C_DurationUtil.CreateDuration()
local d = Span
local zero = d:IsZero()
d:SetTimeFromEnd(GetTime() + 10, 20)
local copy = d:Copy()
return tostring(zero), d:GetTotalDuration(), d:GetRemainingDuration(), d:GetElapsedPercent(),
    tostring(d:IsActive()), tostring(d:HasSecretValues()), copy:GetEndTime() - GetTime()",
    );
    assert_eq!(r, ["true", "20", "10", "0.5", "true", "false", "10"]);
    wowlua_engine::clock::advance(&mut vm, 15.0);
    let r = run(
        &mut vm,
        "return Span:GetRemainingDuration(), tostring(Span:HasExpired()), tostring(Span:IsActive())",
    );
    assert_eq!(r, ["0", "true", "false"]);
}

/// Another unit's cast GUID and spell ID are secret, its cast bar id not;
/// the player's `SENT` target is secret in an instance, not in the open
/// world's combat (`/stdtest engine record aura` and `combat` in Ragefire
/// Chasm; scenario combat).
#[test]
fn cast_payload_secrecy() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('UNIT_SPELLCAST_START')
f:RegisterEvent('UNIT_SPELLCAST_SENT')
f:SetScript('OnEvent', function(_, e, unit, a, b, c)
    if e == 'UNIT_SPELLCAST_START' then
        seen[#seen + 1] = ('%s %s %s %s'):format(unit, tostring(issecretvalue(a)), tostring(issecretvalue(b)), tostring(issecretvalue(c)))
    else
        seen[#seen + 1] = ('sent %s'):format(tostring(issecretvalue(a)))
    end
end)
local hogger = mock.spawnUnit({ name = 'Hogger', level = 11, reaction = 'hostile' })
mock.player:target(hogger)
hogger:castSpell(133, { castTime = 1 })
mock.player:castSpell(647, { castTime = 1 })
mock.player:interruptCast()
mock.player:enterCombat()
mock.player:castSpell(647, { castTime = 1 })
mock.player:interruptCast()
mock.enterInstance('party')
mock.player:castSpell(647, { castTime = 1 })
return table.concat(seen, ' | ')",
    );
    assert_eq!(
        r,
        [
            "target true true false | sent false | player false false false | sent false | player false false false \
          | sent true | player false false false"
        ]
    );
}

/// A secure CVar is refused to addons in combat (`ADDON_ACTION_FORBIDDEN`,
/// recorded for `SoftTargetEnemy`), changed out of combat (the user's
/// word); others change, and `GetCVarInfo` gives the flags.
#[test]
fn secure_cvars_refuse_addons() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "C_CVar.SetCVar('SoftTargetEnemy', 3)
local outside = C_CVar.GetCVar('SoftTargetEnemy')
mock.player:enterCombat()
local refused = C_CVar.SetCVar('SoftTargetEnemy', 1)
local soft = C_CVar.GetCVar('SoftTargetEnemy')
C_CVar.SetCVar('autoLootDefault', 1)
local r = mock.refusals()
local _, default, _, _, _, secure, readOnly = C_CVar.GetCVarInfo('SoftTargetEnemy')
return outside, tostring(refused), soft, C_CVar.GetCVar('autoLootDefault'), #r, r[1].event, r[1].addon, default, tostring(secure), tostring(readOnly)",
    );
    assert_eq!(
        r,
        [
            "3",
            "nil",
            "3",
            "1",
            "1",
            "ADDON_ACTION_FORBIDDEN",
            "MyAddon",
            "1",
            "true",
            "false"
        ]
    );
}

/// Kills as the client recorded the player's: in the open world (scenario
/// combat: plain GUIDs, `UNIT_LOOT` last) and in an instance (`/stdtest
/// engine record combat` in Ragefire Chasm: GUIDs secret, `UNIT_LOOT`
/// before `UNIT_DIED`); the target's `UNIT_TARGET("player")` echo a round
/// trip later.
#[test]
fn kills_and_the_target_echo() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'PARTY_KILL', 'UNIT_COMBAT', 'UNIT_LOOT', 'UNIT_DIED', 'PLAYER_TARGET_DIED',
    'PLAYER_TARGET_CHANGED', 'UNIT_HEALTH', 'UNIT_TARGET' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, a, b)
    local function show(v) return issecretvalue(v) and 'secret' or (type(v) == 'string' and v:match('^Creature') and 'guid') or tostring(v) end
    Seen[#Seen + 1] = e .. '(' .. show(a) .. (b ~= nil and (', ' .. show(b)) or '') .. ')'
end)
local hogger = mock.spawnUnit({ name = 'Hogger', level = 11, reaction = 'hostile', health = 70 })
local wolf = mock.spawnUnit({ name = 'Wolf', level = 5, reaction = 'hostile', health = 30 })
local boar = mock.spawnUnit({ name = 'Boar', level = 5, reaction = 'hostile', health = 30 })
mock.player:target(hogger)
Seen[#Seen + 1] = '|'
mock.player:enterCombat()
hogger:kill({ by = mock.player, lootable = true })
Seen[#Seen + 1] = '|'
wolf:kill()
Seen[#Seen + 1] = '|'
mock.enterInstance('party')
mock.player:target(boar)
boar:kill({ by = mock.player, lootable = true })
return table.concat(Seen, ' ')",
    );
    assert_eq!(
        r,
        [
            "PLAYER_TARGET_CHANGED(nil) | PARTY_KILL(Player-4621-0A000001, guid) UNIT_COMBAT(target, WOUND) \
          UNIT_DIED(guid) PLAYER_TARGET_DIED(nil) PLAYER_TARGET_CHANGED(nil) UNIT_LOOT(guid, true) | UNIT_DIED(guid) \
          | PLAYER_TARGET_CHANGED(nil) PARTY_KILL(Player-4621-0A000001, secret) UNIT_COMBAT(target, WOUND) \
          UNIT_LOOT(guid, true) UNIT_DIED(secret) PLAYER_TARGET_DIED(nil) PLAYER_TARGET_CHANGED(nil)"
        ]
    );
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(&mut vm, "return Seen[#Seen]");
    assert_eq!(r, ["UNIT_TARGET(player)"]);
}

/// Soft targets (`/stdtest engine record soft`, `softinteract`): their
/// tokens, `PLAYER_SOFT_*_CHANGED`, and unit events for them.
#[test]
fn soft_targets() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'PLAYER_SOFT_ENEMY_CHANGED', 'PLAYER_SOFT_FRIEND_CHANGED', 'PLAYER_SOFT_INTERACT_CHANGED', 'UNIT_HEALTH' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, a, b)
    local function show(v) return type(v) == 'string' and v:match('^Creature') and 'guid' or tostring(v) end
    seen[#seen + 1] = e:gsub('PLAYER_SOFT_', '') .. '(' .. show(a) .. ', ' .. show(b) .. ')'
end)
local swoop = mock.spawnUnit({ name = 'Swoop', level = 4, reaction = 'hostile', health = 50 })
local vendor = mock.spawnUnit({ name = 'Harn Longcast', level = 30, reaction = 'friendly' })
mock.player:softTarget(swoop)
local name, exists, same = UnitName('softenemy'), UnitExists('softenemy'), UnitIsUnit('softenemy', 'target')
swoop:damage(5)
mock.player:softTarget(vendor)
mock.player:softTarget(vendor, 'interact')
mock.player:softTarget(nil, 'interact')
mock.player:softTarget(nil, 'enemy')
return table.concat(seen, ' '), name, tostring(exists), tostring(same), UnitName('softfriend'), tostring(UnitExists('softenemy'))",
    );
    assert_eq!(
        r,
        [
            "ENEMY_CHANGED(nil, nil) UNIT_HEALTH(softenemy, nil) FRIEND_CHANGED(nil, nil) \
             INTERACT_CHANGED(nil, guid) INTERACT_CHANGED(guid, nil) ENEMY_CHANGED(nil, nil)",
            "Swoop",
            "true",
            "false",
            "Harn Longcast",
            "false"
        ]
    );
}

/// A vendor as the client recorded one (`/stdtest engine record merchant`,
/// `/stdtest engine merchant`): opening, buying, selling, buying back,
/// closing, and the merchant API's answers.
#[test]
fn merchants() {
    let mut vm = vm();
    run(
        &mut vm,
        "Seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'MERCHANT_SHOW', 'MERCHANT_CLOSED', 'MERCHANT_UPDATE', 'MERCHANT_FILTER_ITEM_UPDATE',
    'PLAYER_INTERACTION_MANAGER_FRAME_SHOW', 'PLAYER_INTERACTION_MANAGER_FRAME_HIDE', 'ITEM_PUSH',
    'BAG_NEW_ITEMS_UPDATED', 'BAG_UPDATE', 'UNIT_INVENTORY_CHANGED', 'PLAYER_MONEY', 'BAG_UPDATE_DELAYED',
    'ITEM_COUNT_CHANGED', 'ITEM_LOCK_CHANGED', 'ITEM_LOCKED', 'ITEM_UNLOCKED' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, a, b)
    Seen[#Seen + 1] = e .. (a ~= nil and ('(' .. tostring(a) .. (b ~= nil and (',' .. tostring(b)) or '') .. ')') or '')
end)
mock.defineItem(6256, { name = 'Fishing Pole', icon = 132932, quality = 1 })
mock.defineItem(6529, { name = 'Shiny Bauble', icon = 134335, quality = 1, sellPrice = 12, stackCount = 20 })
mock.player:setMoney(1000)
mock.openMerchant({ name = 'Harn Longcast', items = { { id = 6256, price = 23 }, { id = 6529, price = 50 } } })",
    );
    wowlua_engine::clock::advance_frames(&mut vm, 2, 1000.0 / 60.0);
    let r = run(
        &mut vm,
        "local info = C_MerchantFrame.GetItemInfo(1)
local out = { table.concat(Seen, ' '), GetMerchantNumItems(), info.name, info.price, info.numAvailable,
    GetMerchantItemLink(2), GetMerchantItemMaxStack(2), select('#', CanAffordMerchantItem(1)),
    UnitName('npc'), select('#', C_MerchantFrame.GetItemInfo(9)) }
Seen = {}
BuyMerchantItem(2, 1)
return unpack(out)",
    );
    assert_eq!(
        r,
        [
            "PLAYER_MONEY MERCHANT_SHOW PLAYER_INTERACTION_MANAGER_FRAME_SHOW(5)",
            "2",
            "Fishing Pole",
            "23",
            "-1",
            "|cnIQ1:|Hitem:6529::::::::60:::14:::::::|h[Shiny Bauble]|h|r",
            "20",
            "0",
            "Harn Longcast",
            "0"
        ]
    );
    // The server's answer to the purchase, the sale, the buyback.
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(
        &mut vm,
        "local bought = table.concat(Seen, ' ')
Seen = {}
mock.sell(0, 1)
local locked = table.concat(Seen, ' ')
Seen = {}
return bought, locked, GetMoney()",
    );
    assert_eq!(
        r,
        [
            "MERCHANT_UPDATE ITEM_PUSH(0,134335) BAG_NEW_ITEMS_UPDATED BAG_UPDATE(0) BAG_UPDATE(-1) \
             UNIT_INVENTORY_CHANGED(player) PLAYER_MONEY BAG_UPDATE(0) BAG_UPDATE(-1) BAG_UPDATE_DELAYED \
             ITEM_COUNT_CHANGED(6529)",
            "ITEM_LOCK_CHANGED(0,1) ITEM_LOCKED(0,1)",
            "950"
        ]
    );
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(
        &mut vm,
        "local sold = table.concat(Seen, ' ')
local name, _, price, quantity, available, usable, bound = GetBuybackItemInfo(1)
local money = GetMoney()
Seen = {}
BuybackItem(1)
return sold, money, GetNumBuybackItems(), name, price, quantity, available, tostring(usable), tostring(bound)",
    );
    assert_eq!(
        r,
        [
            "PLAYER_MONEY BAG_UPDATE(0) BAG_UPDATE(-1) MERCHANT_UPDATE BAG_UPDATE_DELAYED ITEM_COUNT_CHANGED(6529)",
            "962",
            "1",
            "Shiny Bauble",
            "12",
            "1",
            "0",
            "true",
            "false"
        ]
    );
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(
        &mut vm,
        "local back = table.concat(Seen, ' ')
Seen = {}
mock.closeMerchant()
return back, GetMoney(), GetNumBuybackItems(), C_Container.GetContainerItemID(0, 1), table.concat(Seen, ' '),
    GetMerchantNumItems(), tostring(UnitExists('npc'))",
    );
    assert_eq!(
        r,
        [
            "PLAYER_MONEY ITEM_LOCK_CHANGED(0,1) ITEM_UNLOCKED(0,1) BAG_UPDATE(0) BAG_UPDATE(-1) MERCHANT_UPDATE \
             BAG_UPDATE_DELAYED ITEM_COUNT_CHANGED(6529)",
            "950",
            "0",
            "6529",
            "MERCHANT_CLOSED PLAYER_INTERACTION_MANAGER_FRAME_HIDE(5) PLAYER_INTERACTION_MANAGER_FRAME_HIDE(0)",
            "0",
            "false"
        ]
    );
}

/// Repairs and full bags at a vendor (`/stdtest engine record vendor2`).
#[test]
fn repairs_and_full_bags() {
    let mut vm = vm();
    run(
        &mut vm,
        "Seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'UPDATE_INVENTORY_DURABILITY', 'PLAYER_MONEY', 'UI_ERROR_MESSAGE', 'ITEM_PUSH', 'CHAT_MSG_LOOT' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, a, b) Seen[#Seen + 1] = e .. '(' .. tostring(a) .. ', ' .. tostring(b) .. ')' end)
mock.defineItem(2375, { name = 'Battered Leather Gloves', icon = 132952, quality = 1 })
mock.player:setMoney(1000)
mock.setBag(0, { slots = 1, items = { [1] = 6948 } })
mock.setBag(1, { slots = 1, items = {} })
mock.openMerchant({ name = 'Varg Windwhisper', items = { { id = 2375, price = 10 } }, canRepair = true, repairCost = 100, damaged = 4 })
Seen = {}
BuyMerchantItem(1)",
    );
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(
        &mut vm,
        "local bought = table.concat(Seen, ' ')
Seen = {}
BuyMerchantItem(1)
return bought",
    );
    assert_eq!(
        r,
        [
            "ITEM_PUSH(31, 132952) CHAT_MSG_LOOT(You receive item: |cnIQ1:|Hitem:2375::::::::60:::14:::::::|h[Battered Leather Gloves]|h|r, Player) PLAYER_MONEY(nil, nil)"
        ]
    );
    wowlua_engine::clock::advance(&mut vm, 0.5);
    run(
        &mut vm,
        "Full = table.concat(Seen, ' ')
Cost, Needed = GetRepairAllCost()
Seen = {}
RepairAllItems()",
    );
    // The repair's answer a round trip later (recorded: `/stdtest engine
    // scenario vendor`).
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(
        &mut vm,
        "return Full, Cost, tostring(Needed), table.concat(Seen, ' '), GetMoney(), select('#', GetRepairAllCost())",
    );
    assert_eq!(
        r,
        [
            "UI_ERROR_MESSAGE(3, Inventory is full.)",
            "100",
            "true",
            "UPDATE_INVENTORY_DURABILITY(nil, nil) UPDATE_INVENTORY_DURABILITY(nil, nil) \
             UPDATE_INVENTORY_DURABILITY(nil, nil) UPDATE_INVENTORY_DURABILITY(nil, nil) PLAYER_MONEY(nil, nil)",
            "890",
            "2"
        ]
    );
}

#[test]
fn item_cooldowns_run_out() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.setBag(0, { slots = 3, items = { [1] = { id = 6948, cooldown = { remaining = 600, duration = 3600 } }, [2] = 117 } })
local now = GetTime()
local s, d, e = C_Container.GetContainerItemCooldown(0, 1)
local _, d2, e2 = C_Container.GetContainerItemCooldown(0, 2)
local _, d3, e3 = C_Container.GetContainerItemCooldown(0, 3)
local _, d4 = C_Container.GetItemCooldown(6948)
mock.setItemCooldown(117, 10)
local _, d5 = C_Container.GetContainerItemCooldown(0, 2)
mock.advanceTime(11)
local _, d6 = C_Container.GetContainerItemCooldown(0, 2)
return now - s, d, e, d2, e2, d3, e3, d4, d5, d6",
    );
    // The Hearthstone an hour long, 600 s left: started 3000 s ago; an
    // item without one enabled; an empty slot not (recorded).
    assert_eq!(
        r,
        ["3000", "3600", "1", "0", "1", "0", "0", "3600", "10", "0"]
    );
}

/// The engine's groups are the player's own (home): in the instance
/// category (`LE_PARTY_CATEGORY_INSTANCE`, 2) there is none.
#[test]
fn a_home_group_is_no_instance_group() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local friend = mock.spawnUnit({ name = 'Friend', player = true, reaction = 'friendly' })
mock.addToGroup(friend, true)
return tostring(IsInGroup()), tostring(IsInGroup(1)), tostring(IsInGroup(2)), tostring(IsInRaid(2)), GetNumGroupMembers(), GetNumGroupMembers(2)",
    );
    assert_eq!(r, ["true", "true", "false", "false", "2", "0"]);
}
