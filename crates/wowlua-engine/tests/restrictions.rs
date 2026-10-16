//! `/stdtest restrictions` (fixture
//! `wowlua-vm/tests/client/forever-camelot-restrictions.lua`): in each
//! recorded state (none, Combat, Map, Combat+Map, Map without a target) the
//! engine's world is set up as it was (a level 14 enemy targeted, a buff
//! on the player) and the probe's calls give what WoW Forever gave.

use std::collections::BTreeMap;
use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The probe's recording code (tests/ingame/src/restrictions.wowlua), but
/// the conformance cases (checked on their own) and the calls that depend
/// on the character's action bar, threat, or Blizzard's ActionButton1.
const PROBE: &str = r##"
local function tostringall(...)
    local out = {}
    for i = 1, select("#", ...) do out[i] = tostring((select(i, ...))) end
    return unpack(out, 1, select("#", ...))
end
local function show(v)
    if issecretvalue(v) then return "secret " .. type(v) end
    local t = type(v)
    if t == "table" then
        local keys = {}
        for k, field in pairs(v) do
            keys[#keys + 1] = tostring(k) .. ":" .. (issecretvalue(field) and "secret " or "") .. type(field)
        end
        table.sort(keys)
        return "table" .. (issecrettable(v) and " (secret table)" or "") .. " {" .. table.concat(keys, ",") .. "}"
    elseif t == "string" or t == "number" or t == "boolean" or t == "nil" then
        return t .. " " .. tostring(v)
    end
    return t
end
local function results(f, ...)
    local packed = { pcall(f, ...) }
    if not packed[1] then
        return "error: " .. (issecretvalue(packed[2]) and "<secret>" or tostring(packed[2]))
    end
    local out = {}
    for i = 2, select("#", unpack(packed)) do out[#out + 1] = show(packed[i]) end
    return table.concat(out, ", ")
end
local function api(path)
    local value = _G
    for part in path:gmatch("[^.]+") do value = type(value) == "table" and value[part] or nil end
    return value
end
local CALLS = {
    { "UnitHealth", "player" }, { "UnitHealth", "target" }, { "UnitHealthMax", "target" },
    { "UnitPower", "target" }, { "UnitPowerMax", "target" },
    { "UnitName", "target" }, { "UnitGUID", "target" }, { "UnitLevel", "target" },
    { "UnitClass", "target" }, { "UnitExists", "target" },
    { "UnitIsUnit", "target", "player" }, { "UnitCanAttack", "player", "target" },
    { "UnitAffectingCombat", "target" }, { "UnitAffectingCombat", "player" },
    { "UnitCastingInfo", "target" }, { "UnitChannelInfo", "target" },
    { "C_UnitAuras.GetAuraDataByIndex", "player", 1, "HELPFUL" },
    { "C_UnitAuras.GetAuraDataByIndex", "target", 1, "HARMFUL" },
    { "C_RestrictedActions.CheckAllowProtectedFunctions", "UIParent", true },
}
local out = {}
out.inCombatLockdown = tostring(InCombatLockdown())
out.target = results(UnitExists, "target")
for name, value in pairs(Enum.AddOnRestrictionType) do
    out["restrictionTypes." .. name] = results(C_RestrictedActions.GetAddOnRestrictionState, value)
        .. "; active " .. results(C_RestrictedActions.IsAddOnRestrictionActive, value)
end
for name, f in pairs(C_Secrets) do
    if type(f) == "function" then
        out["secrets." .. name] = results(f)
        out["secrets." .. name .. "(target)"] = results(f, "target")
    end
end
for _, call in ipairs(CALLS) do
    local args = { select(2, unpack(call)) }
    for i, a in ipairs(args) do if a == "UIParent" then args[i] = _G[a] end end
    local text = call[1] .. "(" .. table.concat({ tostringall(select(2, unpack(call))) }, ", ") .. ")"
    local f = api(call[1])
    out["calls." .. text] = f and results(f, unpack(args)) or "absent"
end
return out
"##;

fn strings(vm: &Vm, t: Value) -> BTreeMap<String, String> {
    let Value::Table(t) = t else {
        return BTreeMap::new();
    };
    vm.heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(vm.str_bytes(v)).into_owned(),
            )),
            (Value::Str(k), Value::Bool(b)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                b.to_string(),
            )),
            _ => None,
        })
        .collect()
}

/// The recording of a state, flattened as the probe above flattens ours.
fn recorded(state: &str) -> BTreeMap<String, String> {
    let src = std::fs::read(
        repo().join("crates/wowlua-vm/tests/client/forever-camelot-restrictions.lua"),
    )
    .expect("the fixture");
    let mut vm = Vm::new();
    let f = vm.load(&src, "=fixture").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let flatten = vm
        .load(
            br#"local root, state = ...
local s = root[state]
local out = { inCombatLockdown = tostring(s.inCombatLockdown), target = s.target }
for _, part in ipairs({ "restrictionTypes", "secrets", "calls" }) do
    for k, v in pairs(s[part] or {}) do out[part .. "." .. k] = v end
end
return out"#,
            "=flatten",
        )
        .expect("compiles");
    let name = vm.str(state);
    let out = vm.call(flatten, &[r[0], name]).expect("flattens");
    strings(&vm, out[0])
}

/// The world as it was: in or out of combat, on a restricted map or not,
/// with the recorded enemy targeted (`mob`: its spawn data) or none.
fn ours(combat: bool, map: bool, mob: Option<&str>) -> BTreeMap<String, String> {
    let target = mob.is_some();
    let mob = mob.unwrap_or("name = 'Mob'");
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    let setup = format!(
        "mock.player:applyAura({{ spellId = 1459, name = 'Arcane Intellect', duration = 3600, source = mock.player }})
local mob = mock.spawnUnit({{ {mob}, reaction = 'hostile', health = 300, power = 100 }})
if {target} then mock.player:target(mob) end
if {map} then mock.enterInstance() end
if {combat} then mock.player:enterCombat() mob:enterCombat() end"
    );
    let f = vm.load(setup.as_bytes(), "=setup").expect("compiles");
    vm.call(f, &[]).expect("sets up");
    let f = vm
        .load_as(
            PROBE.as_bytes(),
            "@Interface/AddOns/StdTest/restrictions.lua",
            "StdTest",
        )
        .expect("compiles");
    let r = match vm.call(f, &[]) {
        Ok(r) => r,
        Err(e) => match e.value {
            Value::Str(s) => panic!("{}", String::from_utf8_lossy(vm.str_bytes(s))),
            _ => panic!("the probe fails"),
        },
    };
    strings(&vm, r[0])
}

#[test]
fn each_restriction_state_answers_as_recorded() {
    let mut differences = Vec::new();
    // The enemies the recordings had targeted.
    const PLAINSTRIDER: &str = "name = 'Plainstrider', level = 1, class = 'WARRIOR'";
    const DUNGEON_MOB: &str = "name = 'Mob', level = 14";
    for (state, combat, map, mob) in [
        ("none", false, false, None),
        ("Combat", true, false, Some(PLAINSTRIDER)),
        ("Map", false, true, Some(DUNGEON_MOB)),
        ("Combat+Map", true, true, Some(DUNGEON_MOB)),
        ("Map (no target)", false, true, None),
    ] {
        let theirs = recorded(state);
        let mine = ours(combat, map, mob);
        for (key, expected) in &theirs {
            // The character's action bar, threat and Blizzard's frames.
            if key.contains("GetAction")
                || key.contains("Threat")
                || key.contains("ActionButton1")
                || key.contains("GetSpellCooldown(")
            {
                continue;
            }
            // A creature's GUID differs by spawn; its kind is compared.
            let guid = |s: &str| s.starts_with("string Creature-");
            match mine.get(key) {
                Some(m) if m == expected => {}
                Some(m) if guid(m) && guid(expected) => {}
                m => differences.push(format!(
                    "[{state}] {key}\n    client: {expected}\n    ours:   {}",
                    m.map_or("<missing>", String::as_str)
                )),
            }
        }
    }
    assert!(
        differences.is_empty(),
        "{} differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
