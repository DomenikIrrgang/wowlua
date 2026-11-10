//! The game's click bindings and the modifier keys as they write them
//! (decision T625): Blizzard's unit frames read them before a click's
//! `type`, so a target or menu click the profile does not bind there does
//! nothing.

#![allow(clippy::disallowed_methods)] // (tests read the UI code on the disk)

use std::path::{Path, PathBuf};

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A VM with camelot's UI code (Blizzard's secure templates), or nothing
/// when it is not on disk.
fn client() -> Option<Vm> {
    let interface = repo().join("target/types-src/camelot/Interface");
    if !interface.exists() {
        eprintln!("skipped: the UI code is missing");
        return None;
    }
    let xtask = std::fs::read_to_string(repo().join("xtask/src/types.rs")).unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings = repo().join(format!("target/types-src/strings/{commit}.lua"));
    let mut vm = vm();
    wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(Path::new(&strings)),
    );
    Some(vm)
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

/// A unit frame on a friendly unit that targets it with any left click
/// (`*type1`), as an addon's unit frames do; `Frame` and `Friend` globals.
const FRAME: &str = "Friend = mock.spawnUnit({ name = 'Friend', reaction = 'friendly' })
mock.player:target(nil)
Frame = CreateFrame('Button', 'MyUnitFrame', UIParent, 'SecureUnitButtonTemplate')
Frame:SetSize(40, 40)
Frame:SetPoint('CENTER')
Frame:RegisterForClicks('AnyUp')
Frame:SetAttribute('unit', 'party1')
Frame:SetAttribute('*type1', 'target')
Friend:joinGroup()
local function targeted()
    return UnitName('target') or 'none'
end
function ClickWith(mods, button)
    mock.player:target(nil)
    mock.setModifiers(mods)
    mock.click(Frame, button or 'LeftButton')
    mock.setModifiers({})
    return targeted()
end
";

/// `MakeModifiers` holds both bits of each modifier key held (probed: one
/// Shift held, 3), `GetStringFromModifiers` names each bit as probed.
#[test]
fn make_modifiers_follows_the_keys_held() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local out = { MakeModifiers() }
for _, k in ipairs({ 'shift', 'ctrl', 'alt', 'meta' }) do
    mock.setModifiers({ [k] = true })
    out[#out + 1] = MakeModifiers()
end
mock.setModifiers({ shift = true, alt = true })
out[#out + 1] = MakeModifiers()
mock.setModifiers({})
out[#out + 1] = MakeModifiers()
local names = {}
for _, n in ipairs({ 0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 3 + 48 }) do
    names[#names + 1] = n .. '=' .. GetStringFromModifiers(n)
end
return table.concat(out, ' '), table.concat(names, ' ')",
    );
    assert_eq!(
        r,
        [
            "0 3 12 48 192 51 0",
            "0= 1=SHIFT 2=SHIFT 4=CTRL 8=CTRL 16=ALT 32=ALT 64=META 128=META 256= 512= 51=ALT-SHIFT"
        ]
    );
}

/// The profile: the two interactions by default (as Forever answered),
/// copies each time; `SetProfileByInfo` replaces it, `ResetCurrentProfile`
/// restores it; `GetBindingType` and `GetEffectiveInteractionButton` read
/// it for exactly a button and its modifiers.
#[test]
fn the_profile_is_set_read_and_reset() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local function show()
    local out = {}
    for _, b in ipairs(C_ClickBindings.GetProfileInfo()) do
        out[#out + 1] = b.type .. ':' .. b.actionID .. ':' .. b.button .. ':' .. b.modifiers
    end
    return table.concat(out, ' ')
end
local function ask(button, mods)
    return C_ClickBindings.GetBindingType(button, mods) .. ' '
        .. tostring(C_ClickBindings.GetEffectiveInteractionButton(button, mods))
end
local first = C_ClickBindings.GetProfileInfo()
first[1].modifiers = 48
local default = show()
local plain, shifted, right = ask('LeftButton', 0), ask('LeftButton', 3), ask('RightButton', 0)
C_ClickBindings.SetProfileByInfo({
    { type = 3, actionID = 1, button = 'RightButton', modifiers = 0 },
    { type = 3, actionID = 2, button = 'LeftButton', modifiers = 0 },
    { type = 1, actionID = 139, button = 'MiddleButton', modifiers = 12 },
})
local set = show()
local swapped = ask('RightButton', 0) .. ', ' .. ask('LeftButton', 0) .. ', ' .. ask('MiddleButton', 12)
    .. ', ' .. ask('MiddleButton', 0)
C_ClickBindings.ResetCurrentProfile()
return default, plain, shifted, right, set, swapped, show()",
    );
    assert_eq!(
        r,
        [
            "3:1:LeftButton:0 3:2:RightButton:0",
            "3 LeftButton",
            "0 nil",
            "3 RightButton",
            "3:1:RightButton:0 3:2:LeftButton:0 1:139:MiddleButton:12",
            "3 LeftButton, 3 RightButton, 1 nil, 0 nil",
            "3:1:LeftButton:0 3:2:RightButton:0",
        ]
    );
}

/// A unit frame's target on any left click (`*type1`) targets on a plain
/// left click only by default: Shift-Left and Alt-Left have no interaction
/// in the game's click bindings, so Blizzard's `SecureUnitButton_OnClick`
/// stops there (seen in game). Target moved to Shift-Left, Shift-Left
/// targets and a plain left click does nothing.
#[test]
fn a_unit_frame_targets_only_where_the_click_bindings_say() {
    let Some(mut vm) = client() else {
        return;
    };
    let r = run(
        &mut vm,
        &format!(
            "{FRAME}
local plain, shift, alt = ClickWith({{}}), ClickWith({{ shift = true }}), ClickWith({{ alt = true }})
C_ClickBindings.SetProfileByInfo({{
    {{ type = 3, actionID = 1, button = 'LeftButton', modifiers = 3 }},
    {{ type = 3, actionID = 2, button = 'RightButton', modifiers = 0 }},
}})
return plain, shift, alt, ClickWith({{ shift = true }}), ClickWith({{}})"
        ),
    );
    assert_eq!(r, ["Friend", "none", "none", "Friend", "none"]);
}

/// A friend's spell on Ctrl-Left (`ctrl-helpbutton1`) without a
/// `ctrl-type1`: the click's type is the frame's `*type1` target, which
/// the click bindings lack for Ctrl-Left, so nothing happens (seen in
/// game); with its own type set the click goes on to the spell.
#[test]
fn a_help_button_without_its_own_type_does_nothing() {
    let Some(mut vm) = client() else {
        return;
    };
    let r = run(
        &mut vm,
        &format!(
            "{FRAME}
Cast = {{}}
hooksecurefunc('CastSpellByID', function(id, unit) Cast[#Cast + 1] = id .. ' ' .. tostring(unit) end)
Frame:SetAttribute('ctrl-helpbutton1', 'heal')
Frame:SetAttribute('*type-heal', 'spell')
Frame:SetAttribute('*spell-heal', 2050)
local without = ClickWith({{ ctrl = true }}) .. ' ' .. #Cast
Frame:SetAttribute('ctrl-type1', '')
local with = ClickWith({{ ctrl = true }}) .. ' ' .. table.concat(Cast, ',')
return without, with"
        ),
    );
    assert_eq!(r, ["none 0", "none 2050 party1"]);
}

/// A spell bound in the click bindings (`Enum.ClickBindingType.Spell`):
/// Blizzard's unit frame hands the click to `ExecuteBinding`, which
/// presses the spell as the player does (`CURRENT_SPELL_CAST_CHANGED`),
/// whatever the frame's attributes say.
#[test]
fn a_spell_binding_presses_its_spell() {
    let Some(mut vm) = client() else {
        return;
    };
    let r = run(
        &mut vm,
        &format!(
            "{FRAME}
local seen = 0
local f = CreateFrame('Frame')
f:RegisterEvent('CURRENT_SPELL_CAST_CHANGED')
f:SetScript('OnEvent', function() seen = seen + 1 end)
ClickWith({{ ctrl = true }})
local before = seen
C_ClickBindings.SetProfileByInfo({{
    {{ type = 3, actionID = 1, button = 'LeftButton', modifiers = 0 }},
    {{ type = 1, actionID = 2050, button = 'LeftButton', modifiers = 12 }},
}})
local target = ClickWith({{ ctrl = true }})
return before, tostring(seen > 0), target"
        ),
    );
    assert_eq!(r, ["0", "true", "none"]);
}
