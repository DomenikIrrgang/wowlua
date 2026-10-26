//! Addon messages, slash commands, keybindings and test-defined addons.

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
fn addon_messages_register_send_and_echo() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Got = {}
local f = CreateFrame('Frame')
f:RegisterEvent('CHAT_MSG_ADDON')
f:SetScript('OnEvent', function(_, _, prefix, msg, channel, sender) Got[#Got + 1] = prefix .. ':' .. msg .. ':' .. channel .. ':' .. sender end)
local ok = C_ChatInfo.RegisterAddonMessagePrefix('MyPrefix')
local again = C_ChatInfo.RegisterAddonMessagePrefix('MyPrefix')
local long = C_ChatInfo.RegisterAddonMessagePrefix(string.rep('x', 17))
local party = C_ChatInfo.SendAddonMessage('MyPrefix', 'hi', 'PARTY')
local whisper = C_ChatInfo.SendAddonMessage('MyPrefix', 'me', 'WHISPER', UnitName('player'))
local big = C_ChatInfo.SendAddonMessage('MyPrefix', string.rep('x', 256), 'WHISPER', 'Other')
mock.receiveAddonMessage('MyPrefix', 'from them', 'GUILD', 'Friend-Realm')
mock.receiveAddonMessage('Unregistered', 'x', 'GUILD', 'Friend-Realm')
return ok, again, long, party, whisper, big, #mock.sentAddonMessages(), tostring(C_ChatInfo.IsAddonMessagePrefixRegistered('MyPrefix'))",
    );
    // A 256-byte message goes out (`/stdtest engine comm`).
    assert_eq!(r, ["0", "1", "2", "5", "0", "0", "2", "true"]);
    wowlua_engine::clock::advance_frames(&mut vm, 2, 1000.0 / 60.0);
    let got = run(&mut vm, "return table.concat(Got, ' | ')");
    assert_eq!(
        got,
        ["MyPrefix:from them:GUILD:Friend-Realm | MyPrefix:me:WHISPER:Player-Realm"]
    );
}

#[test]
fn slash_commands_bindings_and_addons() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "SlashCmdList = SlashCmdList or {}
Ran = {}
SLASH_MYADDON1, SLASH_MYADDON2 = '/myaddon', '/ma'
SlashCmdList.MYADDON = function(msg) Ran[#Ran + 1] = 'slash ' .. msg end
local took = mock.slash('/MA hello world')
local unknown = mock.slash('/nope')
mock.defineBinding('MYADDON_TOGGLE', function(state) Ran[#Ran + 1] = 'binding ' .. state end, { header = 'MYADDON' })
SetBinding('CTRL-K', 'MYADDON_TOGGLE')
mock.pressKey('CTRL-K')
local command, header, key = GetBinding(1)
mock.defineAddon('MyAddon_Options', { metadata = { Title = 'My Options', Version = '1.2' }, loadOnDemand = true,
    load = function() Ran[#Ran + 1] = 'options loaded' end })
local loaded = C_AddOns.LoadAddOn('MyAddon_Options')
return table.concat(Ran, ', '), tostring(took), tostring(unknown), GetBindingAction('CTRL-K'), command, header, key,
    tostring(loaded), C_AddOns.GetAddOnMetadata('myaddon_options', 'version'), tostring(C_AddOns.IsAddOnLoaded('MyAddon_Options')),
    tostring(C_AddOns.IsAddOnLoadOnDemand('MyAddon_Options'))",
    );
    assert_eq!(
        r,
        [
            "slash hello world, binding down, options loaded",
            "true",
            "false",
            "MYADDON_TOGGLE",
            "MYADDON_TOGGLE",
            "MYADDON",
            "CTRL-K",
            "true",
            "1.2",
            "true",
            "true"
        ]
    );
}

#[test]
fn forever_characters_are_whispered_by_first_and_last_name() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Got = {}
local f = CreateFrame('Frame')
f:RegisterEvent('CHAT_MSG_ADDON')
f:SetScript('OnEvent', function(_, _, _, msg, _, sender) Got[#Got + 1] = msg .. ' from ' .. sender end)
C_ChatInfo.RegisterAddonMessagePrefix('P')
mock.player:setName('Tamsin', 'Ashford')
C_ChatInfo.SendAddonMessage('P', 'first', 'WHISPER', 'Tamsin')
C_ChatInfo.SendAddonMessage('P', 'full', 'WHISPER', 'Tamsin Ashford')
local first, last = UnitName('player')
local full, second = UnitFullName('player')
return first, last, full, second",
    );
    assert_eq!(r, ["Tamsin", "Ashford", "Tamsin", "Ashford"]);
    wowlua_engine::clock::advance_frames(&mut vm, 2, 1000.0 / 60.0);
    let got = run(&mut vm, "return table.concat(Got, ' | ')");
    assert_eq!(got, ["full from Tamsin Ashford"]);
}

/// A `CLICK` binding clicks its button as a mouse click would: only the
/// clicks it registered for (by default the left button's release), so a
/// button left as it was clicks once per key press (decision T234).
#[test]
fn click_bindings_click_as_registered() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Clicks = {}
local owner = CreateFrame('Frame')
local plain = CreateFrame('Button', 'TestPlainButton', UIParent)
plain:SetScript('OnClick', function(_, button, down) Clicks[#Clicks + 1] = 'plain ' .. button .. ' ' .. tostring(down) end)
local early = CreateFrame('Button', 'TestEarlyButton', UIParent)
early:RegisterForClicks('AnyDown')
early:SetScript('OnClick', function(_, button, down) Clicks[#Clicks + 1] = 'early ' .. button .. ' ' .. tostring(down) end)
SetOverrideBindingClick(owner, false, 'K', 'TestPlainButton')
SetOverrideBindingClick(owner, false, 'L', 'TestEarlyButton')
mock.pressKey('K')
mock.pressKey('L')
return table.concat(Clicks, ', ')",
    );
    assert_eq!(r, ["plain LeftButton false, early LeftButton true"]);
}

/// The machine the client runs on (decision T236): the answers that differ
/// by machine are the platform's (recorded on a Mac and on Windows), and
/// `IsMacClient` and `IsWindowsClient` follow it; the Mac by default.
#[test]
fn the_platform_decides_the_machines_answers() {
    let mut vm = vm();
    // Blizzard's global strings for the two keys (GlobalStrings.lua).
    run(
        &mut vm,
        "KEY_DELETE, KEY_DELETE_MAC, KEY_ENTER, KEY_ENTER_MAC = 'Delete', 'Del', 'Enter', 'Return'",
    );
    let ask = "return tostring(IsMacClient()), tostring(IsWindowsClient()), GetBindingText('DELETE'), GetBindingText('META-ENTER')";
    assert_eq!(run(&mut vm, ask), ["true", "false", "Del", "CMD-Return"]);
    wowlua_engine::set_platform(&mut vm, wowlua_engine::Platform::Windows);
    assert_eq!(run(&mut vm, ask), ["false", "true", "Delete", "META-Enter"]);
}

/// The action bars' hotkeys: `GetBindingText(key, 1)` shortens the
/// modifiers with Blizzard's `<MODIFIER>_KEY_TEXT_ABBR` strings (`s-1`),
/// the older `(key, prefix, abbreviate)` too; without the strings, or
/// without `abbreviate`, they stay whole.
#[test]
fn abbreviated_binding_text_shortens_the_modifiers() {
    let mut vm = vm();
    let whole = run(&mut vm, "return GetBindingText('SHIFT-1', 1)");
    assert_eq!(whole, ["SHIFT-1"]);
    run(
        &mut vm,
        "SHIFT_KEY_TEXT_ABBR, CTRL_KEY_TEXT_ABBR, ALT_KEY_TEXT_ABBR, CMD_KEY_TEXT_ABBR = 's', 'c', 'a', 'm'
KEY_MOUSEWHEELUP = 'Mouse Wheel Up'",
    );
    let r = run(
        &mut vm,
        "return GetBindingText('SHIFT-1', 1), GetBindingText('CTRL-ALT-2', true), GetBindingText('META-E', 'KEY_', 1), GetBindingText('SHIFT-MOUSEWHEELUP', 1), GetBindingText('SHIFT-1')",
    );
    assert_eq!(r, ["s-1", "c-a-2", "m-E", "s-Mouse Wheel Up", "SHIFT-1"]);
}

/// `Settings.OpenToCategory`'s client call asks the settings panel to open
/// (`SETTINGS_PANEL_OPEN`, its category and element), at once (T400).
#[test]
fn opening_the_settings_panel_fires_its_event() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local got = {}
local f = CreateFrame('Frame')
f:RegisterEvent('SETTINGS_PANEL_OPEN')
f:SetScript('OnEvent', function(_, event, category, element) got[#got + 1] = tostring(category) .. ':' .. tostring(element) end)
C_SettingsUtil.OpenSettingsPanel(12, 'Sound')
C_SettingsUtil.OpenSettingsPanel()
return got[1], got[2]",
    );
    assert_eq!(r, ["12:Sound", "nil:nil"]);
}
