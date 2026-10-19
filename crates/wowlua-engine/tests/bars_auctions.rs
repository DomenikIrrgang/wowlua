//! What Bartender4 (action bars) and Auctionator (the auction house) need
//! of the engine: frames built shown from a template, bindings told,
//! drops onto action slots, a slot's spell pressed, the paladin's forms,
//! an item's expansion; the house's searches by name, the player's
//! auctions and their cancelling.

use std::path::PathBuf;

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
    call(vm, f)
}

/// Runs `src` as Blizzard's secure code runs (a secure action button's
/// click).
fn run_secure(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "=secure").expect("compiles");
    call(vm, f)
}

fn call(vm: &mut Vm, f: Value) -> Vec<String> {
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

/// Writes `files` under a fresh directory; returns its `Interface` folder.
fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("wowlua-bars-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in files {
        let p = root.join("Interface").join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    root.join("Interface")
}

#[test]
fn a_frame_made_shown_from_a_template_runs_its_on_show() {
    let interface = tree(
        "onshow",
        &[
            (
                "AddOns/Blizzard_Base/Blizzard_Base.toc",
                "## Title: Base\nTemplates.xml\n",
            ),
            (
                "AddOns/Blizzard_Base/Templates.xml",
                r#"<Ui>
  <Frame name="ShownTemplate" virtual="true">
    <Scripts>
      <OnLoad>Seen = (Seen or '') .. 'load ' .. self:GetName() .. ','</OnLoad>
      <OnShow>Seen = (Seen or '') .. 'show ' .. self:GetName() .. ','</OnShow>
    </Scripts>
  </Frame>
</Ui>"#,
            ),
        ],
    );
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let report = wowlua_engine::ui::load(&mut vm, &interface, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let r = run(
        &mut vm,
        "local hidden = CreateFrame('Frame', 'Hidden', UIParent)
hidden:Hide()
CreateFrame('Frame', 'Shown', UIParent, 'ShownTemplate')
CreateFrame('Frame', 'Unseen', hidden, 'ShownTemplate')
hidden:Show()
return Seen",
    );
    // Shown at once under the shown screen; under a hidden parent, once it
    // shows.
    assert_eq!(r, ["load Shown,show Shown,load Unseen,show Unseen,"]);
}

#[test]
fn bindings_set_or_saved_tell_update_bindings() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineBinding('MYADDON_TOGGLE', 'print(1)')
local n = 0
local f = CreateFrame('Frame')
f:RegisterEvent('UPDATE_BINDINGS')
f:SetScript('OnEvent', function() n = n + 1 end)
local counts = {}
SetBinding('F', 'MYADDON_TOGGLE') counts[#counts + 1] = n
SetBinding('f', 'MYADDON_TOGGLE') counts[#counts + 1] = n
SetBindingClick('G', 'MyButton') counts[#counts + 1] = n
SaveBindings(GetCurrentBindingSet()) counts[#counts + 1] = n
SetOverrideBinding(f, false, 'H', 'MYADDON_TOGGLE') counts[#counts + 1] = n
return table.concat(counts, ',')",
    );
    // A lower-case key binds nothing (and tells nothing); overrides tell
    // nothing.
    assert_eq!(r, ["1,1,2,3,3"]);
}

#[test]
fn an_action_picked_up_with_something_on_the_cursor_swaps() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.setAction(4, { type = 'spell', id = 20288, subType = 'spell', texture = 132325 })
C_Spell.PickupSpell(1044)
PickupAction(5)
local five = select(2, GetActionInfo(5))
local held = select(2, GetCursorInfo())
PickupAction(4)
local four, held2 = select(2, GetActionInfo(4)), select(4, GetCursorInfo())
PickupAction(5)
return five, tostring(held), four, tostring(held2), select(2, GetActionInfo(5)), tostring(GetCursorInfo())",
    );
    // Onto the empty slot 5: placed, the cursor empty; slot 4 picked up
    // with nothing held (Hammer of Justice held, the slot empty); onto 5:
    // swapped, Blessing of Freedom held.
    assert_eq!(r, ["1044", "nil", "nil", "20288", "20288", "spell"]);
}

#[test]
fn an_action_used_presses_its_spell() {
    let mut vm = vm();
    let r = run_secure(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('UNIT_SPELLCAST_SENT')
f:RegisterEvent('UNIT_SPELLCAST_SUCCEEDED')
f:SetScript('OnEvent', function(_, e, unit, a, b, c) seen[#seen + 1] = e .. ' ' .. tostring(e == 'UNIT_SPELLCAST_SENT' and c or b) end)
mock.setAction(2, { type = 'spell', id = 1311656, subType = 'spell', texture = 135957 })
mock.setAction(3, { type = 'macro', id = 1, text = 'Test', texture = 841219 })
UseAction(3)
UseAction(2)
mock.advanceTime(1)
return table.concat(seen, ',')",
    );
    assert_eq!(
        r,
        ["UNIT_SPELLCAST_SENT 1311656,UNIT_SPELLCAST_SUCCEEDED 1311656"]
    );
}

#[test]
fn the_shapeshift_forms_are_the_characters() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = GetNumShapeshiftForms()
mock.player:setStats({ shapeshiftForms = { 2 }, shapeshiftFormInfo = { [1] = { 135893, false, true, 643 } } })
return before, GetNumShapeshiftForms(), select(4, GetShapeshiftFormInfo(1))",
    );
    assert_eq!(r, ["0", "2", "643"]);
}

#[test]
fn an_items_expansion_is_classics_unless_its_data_says() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineItem(2375, { name = 'Battered Leather Gloves', quality = 1, classID = 4 }, { cached = true })
mock.defineItem(1, { name = 'Later', expansionID = 2 }, { cached = true })
return (select(15, C_Item.GetItemInfo(2375))), (select(15, C_Item.GetItemInfo(1)))",
    );
    assert_eq!(r, ["0", "2"]);
}

/// A house of two rows (the Light Shot, the Heavy Shot) at an auctioneer,
/// open; the player's gloves in the backpack.
const HOUSE: &str = "mock.defineItem(8067, { name = 'Crafted Light Shot', quality = 1, stackCount = 200 })
mock.defineItem(8068, { name = 'Crafted Heavy Shot', quality = 1, stackCount = 200 })
mock.defineItem(2375, { name = 'Battered Leather Gloves', quality = 1, stackCount = 1 })
mock.setBag(0, { slots = 16, items = { [1] = { id = 2375 } } })
mock.player:setMoney(1000)
local stampi = mock.spawnUnit({ name = 'Auctioneer Stampi', npcID = 8674, reaction = 'neutral' })
local key = function(id) return { itemID = id, itemLevel = 1, itemSuffix = 0, battlePetSpeciesID = 0 } end
mock.defineAuctionHouse(stampi, {
    browse = {
        { itemKey = key(8067), minPrice = 1, totalQuantity = 70, containsOwnerItem = false },
        { itemKey = key(8068), minPrice = 4, totalQuantity = 10, containsOwnerItem = false },
    },
    deposit = 100, nextAuctionID = 500,
})
stampi:setDistance(3)
mock.player:interact(stampi)
mock.advanceTime(1)
local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'AUCTION_HOUSE_BROWSE_RESULTS_UPDATED', 'AUCTION_HOUSE_BROWSE_RESULTS_ADDED',
    'OWNED_AUCTIONS_UPDATED', 'AUCTION_CANCELED', 'AUCTION_HOUSE_AUCTION_CREATED' }) do f:RegisterEvent(e) end
f:SetScript('OnEvent', function(_, e, a) seen[#seen + 1] = e .. (type(a) == 'number' and (' ' .. a) or '') end)
";

#[test]
fn a_browse_search_finds_its_items_by_name() {
    let mut vm = vm();
    let src = format!(
        "{HOUSE}
C_AuctionHouse.SendBrowseQuery({{ searchString = '', sorts = {{}}, filters = {{}}, itemClassFilters = {{}} }})
mock.advanceTime(1)
local all, full = #C_AuctionHouse.GetBrowseResults(), C_AuctionHouse.HasFullBrowseResults()
C_AuctionHouse.RequestMoreBrowseResults()
mock.advanceTime(1)
local fullAfter = C_AuctionHouse.HasFullBrowseResults()
C_AuctionHouse.SendBrowseQuery({{ searchString = 'light SHOT', sorts = {{}}, filters = {{}}, itemClassFilters = {{}} }})
mock.advanceTime(1)
local rows = C_AuctionHouse.GetBrowseResults()
return all, tostring(full), tostring(fullAfter), #rows, rows[1].itemKey.itemID,
    tostring(C_AuctionHouse.HasFullBrowseResults()), table.concat(seen, ',')"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            "2",
            "false",
            "true",
            "1",
            "8067",
            "true",
            "AUCTION_HOUSE_BROWSE_RESULTS_UPDATED,AUCTION_HOUSE_BROWSE_RESULTS_ADDED,AUCTION_HOUSE_BROWSE_RESULTS_UPDATED",
        ]
    );
}

#[test]
fn the_players_auctions_told_and_one_cancelled() {
    let mut vm = vm();
    let src = format!(
        "{HOUSE}
C_AuctionHouse.PostItem({{ bagID = 0, slotIndex = 1 }}, 1, 1, nil, 250)
mock.advanceTime(2)
local before = C_AuctionHouse.GetNumOwnedAuctions()
C_AuctionHouse.QueryOwnedAuctions({{}})
mock.advanceTime(1)
local n, info = C_AuctionHouse.GetNumOwnedAuctions(), C_AuctionHouse.GetOwnedAuctionInfo(1)
local can, cost = C_AuctionHouse.CanCancelAuction(500), C_AuctionHouse.GetCancelCost(500)
C_AuctionHouse.CancelAuction(500)
mock.advanceTime(1)
return before, n, info.auctionID, info.itemKey.itemID, info.buyoutAmount, info.status, tostring(can), cost,
    C_AuctionHouse.GetNumOwnedAuctions(), tostring(C_AuctionHouse.CanCancelAuction(500)), tostring(HasNewMail() and true),
    GetMoney(), table.concat(seen, ',')"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            // None told before the query (recorded: 0 after a post).
            "0",
            "1",
            "500",
            "2375",
            "250",
            "0",
            "true",
            "0",
            "0",
            "false",
            "true",
            // The deposit paid at the post, nothing back at the cancel.
            "900",
            "AUCTION_HOUSE_BROWSE_RESULTS_UPDATED,AUCTION_HOUSE_AUCTION_CREATED 500,OWNED_AUCTIONS_UPDATED,AUCTION_CANCELED 500,OWNED_AUCTIONS_UPDATED",
        ]
    );
}
