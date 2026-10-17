//! Addon messages, slash commands, keybindings and test-defined addons.

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
