//! Ranges and facing from the positions (decision T519): the action bars'
//! range told as the target comes and goes, a spell pressed out of range
//! or at a target behind the player refused.

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

/// The player at (0, 0) facing north, a Fireball that needs its target in
/// front (30 yards), an ogre 20 yards north targeted; what the events
/// tell, in `Log`.
const START: &str = "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })
mock.defineSpell(133, { name = 'Fireball', known = true, maxRange = 30, facing = true })
Log = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ACTION_RANGE_CHECK_UPDATE')
f:RegisterEvent('UI_ERROR_MESSAGE')
f:SetScript('OnEvent', function(_, e, a, b, c)
    Log[#Log + 1] = e == 'UI_ERROR_MESSAGE' and (a .. ' ' .. b)
        or (a .. ' ' .. tostring(b) .. ' ' .. tostring(c))
end)
local ogre = mock.spawnUnit({ name = 'Ogre', reaction = 'hostile', world = { y = 20, x = 0, z = 0 } })
mock.player:target(ogre)
";

#[test]
fn the_bars_are_told_their_range_as_the_target_comes_and_goes() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}mock.setAction(1, {{ type = 'spell', id = 133 }})
mock.setAction(2, {{ type = 'macro', id = 1 }})
C_ActionBar.EnableActionRangeCheck(1, true)
C_ActionBar.EnableActionRangeCheck(2, true)
mock.advanceFrames(1, 1000 / 60)
mock.moveTo({{ world = {{ y = -20, x = 0, z = 0 }} }})
mock.advanceFrames(1, 1000 / 60)
mock.player:target(nil)
mock.advanceFrames(1, 1000 / 60)
C_ActionBar.EnableActionRangeCheck(1, false)
mock.moveTo({{ world = {{ y = 0, x = 0, z = 0 }} }})
mock.advanceFrames(1, 1000 / 60)
return table.concat(Log, ', ')"
        ),
    );
    // (in range, then 40 yards away, then no target; the macro never
    // checks a range; slot 1 no more told)
    assert_eq!(out, ["1 true true, 1 false true, 1 false false"]);
}

#[test]
fn a_spell_out_of_range_or_at_a_target_behind_is_refused() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}mock.defineSpell(2061, {{ name = 'Flash Heal', known = true, maxRange = 40, helpful = true }})
mock.moveTo({{ world = {{ y = -20, x = 0, z = 0 }} }})
mock.player:useSpell(133)
mock.player:useSpell(2061)
mock.advanceFrames(60, 1000 / 60)
mock.moveTo({{ world = {{ y = 10, x = 0, z = 0 }}, facing = math.pi }})
mock.player:useSpell(133)
mock.advanceFrames(60, 1000 / 60)
mock.moveTo({{ facing = 0 }})
mock.player:useSpell(133)
mock.advanceFrames(60, 1000 / 60)
return table.concat(Log, ', ')"
        ),
    );
    // (out of range; the heal goes on; behind; then in front, cast)
    assert_eq!(
        out,
        ["402 Out of range., 57 Target needs to be in front of you."]
    );
}
