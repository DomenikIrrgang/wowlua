//! The chat channels as the client lists them (recorded in Thunder Bluff
//! by `/stdtest snapshot`).

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

/// The recording's channels, joined as its profile has them.
const JOINED: &str = "_G.CHANNEL_CATEGORY_GROUP = 'Group'
_G.CHANNEL_CATEGORY_WORLD = 'World'
_G.CHANNEL_CATEGORY_CUSTOM = 'Custom'
local W = 'CHANNEL_CATEGORY_WORLD'
mock.setChannels({ groupChannels = 1, joined = {
    { name = 'LookingForGroup', fullName = 'LookingForGroup', category = W },
    { name = 'General', fullName = 'General - Thunder Bluff', category = W },
    { name = 'Trade', fullName = 'Trade - English', category = W },
    { name = 'LocalDefense', fullName = 'LocalDefense - Thunder Bluff', category = W },
    { name = 'Services', fullName = 'Trade (Services) - English', category = W },
    { name = 'TradeLocal', fullName = 'Trade (Local) - Thunder Bluff', category = W },
} })
local function row(...)
    local out = {}
    for i = 1, select('#', ...) do out[i] = tostring((select(i, ...))) end
    return table.concat(out, ' ')
end
";

#[test]
fn the_pane_lists_headers_and_channels_as_recorded() {
    let mut vm = vm();
    let src = format!(
        "{JOINED}
local rows = {{ GetNumDisplayChannels() }}
for i = 1, GetNumDisplayChannels() do rows[#rows + 1] = row(GetChannelDisplayInfo(i)) end
return unpack(rows)"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            "10",
            "Group true false nil nil nil CHANNEL_CATEGORY_GROUP 0",
            "World true false nil 6 nil CHANNEL_CATEGORY_WORLD 0",
            "LookingForGroup false nil 1 nil true CHANNEL_CATEGORY_WORLD 1",
            "General false nil 2 nil true CHANNEL_CATEGORY_WORLD 1",
            "Trade false nil 3 nil true CHANNEL_CATEGORY_WORLD 1",
            "LocalDefense false nil 4 nil true CHANNEL_CATEGORY_WORLD 1",
            "Services false nil 5 nil true CHANNEL_CATEGORY_WORLD 1",
            "TradeLocal false nil 6 nil true CHANNEL_CATEGORY_WORLD 1",
            "Custom true false nil nil nil CHANNEL_CATEGORY_CUSTOM 0",
            " true false nil nil nil CHANNEL_CATEGORY_DISCORD 0",
        ]
    );
}

#[test]
fn lists_names_and_general_as_recorded() {
    let mut vm = vm();
    let src = format!(
        "{JOINED}
return row(GetChannelList()), row(GetChannelName(2)), row(GetChannelName(7)), row(GetChannelName('trade')),
    GetNumGroupChannels(), C_ChatInfo.GetGeneralChannelLocalID(), C_ChatInfo.GetGeneralChannelID()"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            "1 LookingForGroup false 2 General false 3 Trade false 4 LocalDefense false 5 Services false 6 TradeLocal false",
            "2 General - Thunder Bluff 0 false",
            "0 nil 0 false",
            "3 Trade - English 0 false",
            "1",
            "2",
            "1",
        ]
    );
}
