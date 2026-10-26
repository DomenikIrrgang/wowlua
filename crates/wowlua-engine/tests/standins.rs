//! What Blizzard's UI code defines that addons use as the client's, the
//! engine's until that code loads (`standins.rs`); and the fixes found
//! while writing the documentation: a bar's scripts after a secret value,
//! a bag item without data, an event payload's enumerations.

use wowlua_vm::{Value, Vm};

fn vm(flavor: &str) -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, flavor).expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    }
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// The error a chunk raises.
fn fails(vm: &mut Vm, src: &str) -> String {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    match vm.call(f, &[]) {
        Ok(_) => panic!("no error"),
        Err(e) => text(vm, e.value),
    }
}

#[test]
fn seconds_to_time_writes_the_units_as_timeutil_does() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "return SecondsToTime(3900), SecondsToTime(3900, false, true), SecondsToTime(45),
            SecondsToTime(90061, false, false, 3), SecondsToTime(3725, true),
            SecondsToTime(5000, false, false, 1), SecondsToTime(3000, false, false, 1, true),
            SecondsToTime(0)",
    );
    assert_eq!(
        r,
        [
            "1 |4Hr:Hr; 5 |4Min:Min;",
            "1 |4Hour:Hours; 5 |4Minute:Minutes;",
            "45 |4Sec:Sec;",
            "1 |4Day:Days; 1 |4Hr:Hr; 1 |4Min:Min;",
            "1 |4Hr:Hr; 2 |4Min:Min;",
            // One unit: from 1.5 of it, so 83 minutes are minutes.
            "83 |4Min:Min;",
            "50 |4Min:Min;",
            "",
        ]
    );
}

#[test]
fn the_short_forms_and_clocks_are_timeutils() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "local f1, n1 = SecondsToTimeAbbrev(95)
local f2, n2 = SecondsToTimeAbbrev(30)
local f3, n3 = SecondsToTimeAbbrev(7200)
local f4, n4 = SecondsToTimeAbbrev(100, 1)
return f1, n1, f2, n2, f3, n3, f4, n4,
    SecondsToClock(187), SecondsToClock(3787), SecondsToClock(5, true), SecondsToClock(-4),
    FormatShortDate(26, 10, 2026), FormatShortDate(5, 3),
    FormatLargeNumber(1234567), FormatLargeNumber(123), FormatLargeNumber(1234)",
    );
    assert_eq!(
        r,
        [
            "%d m",
            "2",
            "%d s",
            "30",
            "%d h",
            "2",
            "%d m",
            "2",
            "03:07",
            "01:03:07",
            "00:00:05",
            "00:00",
            "10/26/2026",
            "3/05",
            "1,234,567",
            "123",
            "1,234",
        ]
    );
}

#[test]
fn money_strings_have_the_coins_icons() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "return GetMoneyString(123456), GetMoneyString(5), GetMoneyString(0), GetMoneyString(0, false, false, true),
            GetMoneyString(12345600, true), COPPER_PER_SILVER, SILVER_PER_GOLD, COPPER_PER_GOLD,
            GOLD_AMOUNT, SILVER_AMOUNT, COPPER_AMOUNT",
    );
    let gold = "|TInterface\\MoneyFrame\\UI-GoldIcon:0:0:2:0|t";
    let silver = "|TInterface\\MoneyFrame\\UI-SilverIcon:0:0:2:0|t";
    let copper = "|TInterface\\MoneyFrame\\UI-CopperIcon:0:0:2:0|t";
    assert_eq!(
        r,
        [
            format!("12{gold} 34{silver} 56{copper}"),
            format!("5{copper}"),
            format!("0{copper}"),
            format!("0{gold}"),
            format!("1,234{gold} 56{silver}"),
            "100".into(),
            "100".into(),
            "10000".into(),
            "%d Gold".into(),
            "%d Silver".into(),
            "%d Copper".into(),
        ]
    );
    // Colorblind mode: the coins' letters.
    let r = run(
        &mut vm,
        "C_CVar.SetCVar('colorblindMode', '1') return GetMoneyString(123456)",
    );
    assert_eq!(r, ["12g 34s 56c"]);
}

#[test]
fn a_seconds_argument_must_be_a_number() {
    let mut vm = vm("camelot");
    let e = fails(&mut vm, "SecondsToTime(secretwrap(5))");
    assert!(e.contains("secret"), "{e}");
    let e = fails(&mut vm, "SecondsToClock('x')");
    assert!(e.contains("number expected"), "{e}");
}

/// `SendChatMessage` and the emotes are Blizzard's deprecation file's:
/// they call the `C_ChatInfo` functions as they are then.
#[test]
fn send_chat_message_calls_c_chat_info() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "local seen = {}
local send = C_ChatInfo.SendChatMessage
C_ChatInfo.SendChatMessage = function(m, t, l, w)
    seen[#seen + 1] = strjoin(',', tostring(m), tostring(t), tostring(l), tostring(w))
end
SendChatMessage('hello', 'PARTY', nil, nil)
C_ChatInfo.SendChatMessage = send
C_ChatInfo.PerformEmote = function(name) seen[#seen + 1] = 'emote ' .. name return true end
C_ChatInfo.CancelEmote = function() seen[#seen + 1] = 'cancel' end
local danced = DoEmote('DANCE')
local stopped = DoEmote(nil)
CancelEmote()
return table.concat(seen, ';'), danced, stopped, select('#', SendChatMessage('hi'))",
    );
    assert_eq!(
        r,
        [
            "hello,PARTY,nil,nil;emote DANCE;cancel;cancel",
            "true",
            "false",
            "0"
        ]
    );
}

/// Blizzard's `Constants.lua` makes the container constants from
/// `Constants.InventoryConstants` (camelot's, recorded: 4 bags and the
/// reagent bag; the client's `NUM_TOTAL_EQUIPPED_BAG_SLOTS` 5).
#[test]
fn the_container_constants_come_from_the_inventory_constants() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "return NUM_BAG_SLOTS, NUM_REAGENTBAG_SLOTS, NUM_TOTAL_EQUIPPED_BAG_SLOTS, NUM_CHARACTER_BANK_SLOTS",
    );
    assert_eq!(r, ["4", "1", "5", "9"]);
}

/// A status bar's scripts get what its getters read: a secret after a
/// secret value or range, and no `userInput` (recorded: `40:nil`).
#[test]
fn a_bars_scripts_pass_its_value_as_its_getters_read_it() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "local bar = CreateFrame('StatusBar')
local seen = {}
bar:SetScript('OnValueChanged', function(_, v, user)
    seen[#seen + 1] = 'value ' .. (issecretvalue(v) and 'secret' or tostring(v)) .. ' ' .. tostring(user)
end)
bar:SetScript('OnMinMaxChanged', function(_, min, max)
    seen[#seen + 1] = 'range ' .. (issecretvalue(min) and 'secret' or tostring(min))
        .. ' ' .. (issecretvalue(max) and 'secret' or tostring(max))
end)
bar:SetMinMaxValues(0, 100)
bar:SetValue(40)
local plain = bar:HasSecretAspect(Enum.SecretAspect.BarValue)
bar:SetValue(secretwrap(30))
bar:SetMinMaxValues(0, secretwrap(50))
return table.concat(seen, ';'), plain, bar:HasSecretAspect(Enum.SecretAspect.BarValue),
    bar:HasAnySecretAspect(), issecretvalue(bar:GetValue())",
    );
    assert_eq!(
        r,
        [
            "range 0 100;value 40 nil;value secret nil;range secret secret",
            "false",
            "true",
            "true",
            "true"
        ]
    );
}

/// An item the client has no data for reads as no item info, as the
/// client answers while an item's data has not arrived; one with data has
/// its name, link and icon.
#[test]
fn a_bag_item_without_data_has_no_info() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "mock.defineItem(2589, { name = 'Linen Cloth', icon = 132889, quality = 1 })
mock.setBag(0, { slots = 4, items = { [1] = 2589, [2] = 987654 } })
local known = C_Container.GetContainerItemInfo(0, 1)
return known.itemName, known.iconFileID, known.hyperlink ~= nil,
    C_Container.GetContainerItemInfo(0, 2), C_Container.GetContainerItemID(0, 2)",
    );
    assert_eq!(r, ["Linen Cloth", "132889", "true", "nil", "987654"]);
}

/// `mock.fireEvent` checks an enumeration's payload value as a number.
#[test]
fn fire_event_checks_an_enumerations_payload() {
    let mut vm = vm("camelot");
    for bad in ["true", "'x'", "{}"] {
        let e = fails(&mut vm, &format!("mock.fireEvent('BAG_UPDATE', {bad})"));
        assert!(e.contains("BAG_UPDATE's `bagID` is BagIndex"), "{e}");
    }
    let r = run(
        &mut vm,
        "local got
local f = CreateFrame('Frame')
f:RegisterEvent('BAG_UPDATE')
f:SetScript('OnEvent', function(_, _, bag) got = bag end)
mock.fireEvent('BAG_UPDATE', Enum.BagIndex.Backpack)
return got",
    );
    assert_eq!(r, ["0"]);
}
