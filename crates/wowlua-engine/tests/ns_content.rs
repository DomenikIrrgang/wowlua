//! Group content the world has little of (`content.rs`): difficulties,
//! roles, and the "nothing here" answers addons now get.

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
        Value::Table(_) => "table".into(),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

#[test]
fn the_dungeon_difficulty_changes() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = 0
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_DIFFICULTY_CHANGED')
f:SetScript('OnEvent', function() seen = seen + 1 end)
local before = GetDungeonDifficultyID()
SetDungeonDifficultyID(2)
SetDungeonDifficultyID(2)
local name, kind, heroic, _, _, _, _, _, _, size = GetDifficultyInfo(9)
return before, GetDungeonDifficultyID(), seen, name, kind, heroic, size, select('#', GetDifficultyInfo(9999))",
    );
    assert_eq!(r, ["1", "2", "1", "40 Player", "raid", "false", "40", "0"]);
}

#[test]
fn roles_and_the_loot_threshold() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "SetLFGRoles(false, true, false, 1)
local leader, tank, healer, dps = GetLFGRoles()
return leader, tank, healer, dps, GetLootThreshold(), C_PartyInfo.IsGUIDInGroup(UnitGUID('player'))",
    );
    assert_eq!(r, ["false", "true", "false", "true", "2", "false"]);
}

#[test]
fn nothing_here_for_addons() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return C_ChallengeMode.IsChallengeModeActive(), #C_ChallengeMode.GetMapTable(),
    select('#', C_LFGList.GetActivityInfoTable(1)), GetBattlefieldStatus(1), GetNumRandomDungeons(),
    C_MythicPlus.GetOwnedKeystoneLevel(), C_PvP.GetTeamInfo(0), GetPVPTimer(),
    type(C_ChallengeMode.GetChallengeCompletionInfo()), select('#', GetLootRollItemInfo(1))",
    );
    assert_eq!(
        r,
        [
            "false", "0", "0", "none", "0", "nil", "nil", "301000", "table", "0"
        ]
    );
}
