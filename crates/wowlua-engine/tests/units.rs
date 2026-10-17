//! Units, tokens, combat and restrictions, driven by the controls.

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
            "false",
            "true",
            "true",
            "true",
            "true",
            "false",
            "false"
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
    assert_eq!(
        r,
        [
            "true", "false", "Friend", "false", "false", "true", "2", "true"
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
/// the player's `SENT` target is secret in combat (`/stdtest engine
/// record aura` and `combat`).
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
return table.concat(seen, ' | ')",
    );
    assert_eq!(
        r,
        [
            "target true true false | sent false | player false false false | sent true | player false false false"
        ]
    );
}

/// A secure CVar is refused to addons (`ADDON_ACTION_FORBIDDEN`, recorded
/// for `SoftTargetEnemy`); others change, and `GetCVarInfo` gives the flags.
#[test]
fn secure_cvars_refuse_addons() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local refused = C_CVar.SetCVar('SoftTargetEnemy', 3)
local soft = C_CVar.GetCVar('SoftTargetEnemy')
C_CVar.SetCVar('autoLootDefault', 1)
local r = mock.refusals()
local _, default, _, _, _, secure, readOnly = C_CVar.GetCVarInfo('SoftTargetEnemy')
return tostring(refused), soft, C_CVar.GetCVar('autoLootDefault'), #r, r[1].event, r[1].addon, default, tostring(secure), tostring(readOnly)",
    );
    assert_eq!(
        r,
        [
            "nil",
            "1",
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

/// A kill as the client recorded the player's (`/stdtest engine record
/// combat`), and the target's `UNIT_TARGET("player")` echo 0.4 s later.
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
mock.player:target(hogger)
Seen[#Seen + 1] = '|'
mock.player:enterCombat()
hogger:kill({ by = mock.player, lootable = true })
Seen[#Seen + 1] = '|'
wolf:kill()
return table.concat(Seen, ' ')",
    );
    assert_eq!(
        r,
        [
            "PLAYER_TARGET_CHANGED(nil) | PARTY_KILL(secret, secret) UNIT_COMBAT(target, WOUND) UNIT_LOOT(guid, true) \
          UNIT_DIED(secret) PLAYER_TARGET_DIED(nil) PLAYER_TARGET_CHANGED(nil) | UNIT_DIED(secret)"
        ]
    );
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(&mut vm, "return Seen[#Seen]");
    assert_eq!(r, ["UNIT_TARGET(player)"]);
}
