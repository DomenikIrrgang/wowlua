//! The collections a test sets (decision T235): the mount journal, the toy
//! box and the wardrobe answer from `mock.setMounts`, `mock.setToys` and
//! `mock.setAppearances`, and as recorded until then.

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

/// What a script returns, joined.
fn joined(vm: &mut Vm, src: &str) -> String {
    run(vm, src).join(" ")
}

/// Until a test sets them, the recorded account's: 139 mounts, none
/// displayed; 21 toys, none shown; the recorded wardrobe.
#[test]
fn recorded_until_set() {
    let mut vm = vm();
    assert_eq!(
        joined(
            &mut vm,
            "return C_MountJournal.GetNumMounts(), C_MountJournal.GetNumDisplayedMounts(),
                C_ToyBox.GetNumToys(), C_ToyBox.GetNumFilteredToys(), C_ToyBox.GetToyFromIndex(1),
                tostring(C_ToyBoxInfo.HasAnyToy()), tostring(PlayerHasToy(54452)),
                C_TransmogCollection.GetCategoryCollectedCount(1)"
        ),
        "139 0 21 0 -1 false false 1"
    );
}

const MOUNTS: &str = "Events = {}
local f = CreateFrame('Frame')
f:RegisterEvent('MOUNT_JOURNAL_SEARCH_UPDATED')
f:SetScript('OnEvent', function(_, e) Events[#Events + 1] = e end)
mock.defineSpell(900001, { name = 'Swift Test Horse', iconID = 132261 })
mock.setMounts({
    [11] = { spellID = 900001, display = 2404, sourceType = 1, source = 'Vendor', description = 'A horse.' },
    [12] = { name = 'Brown Ram', spellID = 900002, icon = 132248, favorite = true, faction = 1 },
    [13] = { name = 'Zebra', spellID = 900003, collected = false },
    [14] = { name = 'Hidden Steed', spellID = 900004, hidden = true },
    [15] = { name = 'Night Saber', spellID = 900005, usable = false },
})
function names()
    local out = {}
    for i = 1, C_MountJournal.GetNumDisplayedMounts() do
        out[#out + 1] = (C_MountJournal.GetDisplayedMountInfo(i))
    end
    return table.concat(out, ', ')
end
";

/// The journal's mounts: their info, the displayed list (not hidden,
/// through the filters and the search; favourites first, then collected
/// ones, by name), a favourite set, usability.
#[test]
fn mounts_from_the_test() {
    let mut vm = vm();
    run(&mut vm, MOUNTS);
    assert_eq!(
        joined(
            &mut vm,
            "local ids = C_MountJournal.GetMountIDs()
            return #ids, ids[1], ids[5], C_MountJournal.GetNumMounts(), table.concat(Events, ',')"
        ),
        "5 11 15 5 "
    );
    // The name and icon from the spell; the 13 values and the 9.
    assert_eq!(
        joined(&mut vm, "return C_MountJournal.GetMountInfoByID(11)"),
        "Swift Test Horse 900001 132261 false true 1 false false nil false true 11 false"
    );
    assert_eq!(
        joined(&mut vm, "return C_MountJournal.GetMountInfoExtraByID(11)"),
        "2404 A horse. Vendor false 230 4 91 0 false"
    );
    assert_eq!(
        joined(
            &mut vm,
            "return select(7, C_MountJournal.GetMountInfoByID(12)), select(5, C_MountJournal.GetMountInfoByID(13)),
                select(5, C_MountJournal.GetMountInfoByID(15)), C_MountJournal.GetMountInfoByID(99)"
        ),
        "true false false"
    );
    // The recorded character's filters: not collected ones left out.
    assert_eq!(
        joined(
            &mut vm,
            "return names(), C_MountJournal.GetDisplayedMountID(1), C_MountJournal.GetDisplayedMountID(9)"
        ),
        "Brown Ram, Night Saber, Swift Test Horse 12 0"
    );
    assert_eq!(
        joined(
            &mut vm,
            "C_MountJournal.SetCollectedFilterSetting(2, true)
            local all = names()
            C_MountJournal.SetCollectedFilterSetting(3, false)
            local usable = names()
            C_MountJournal.SetCollectedFilterSetting(3, true)
            C_MountJournal.SetSearch('RA')
            local search = names()
            C_MountJournal.SetSearch('')
            return all, '|', usable, '|', search"
        ),
        "Brown Ram, Night Saber, Swift Test Horse, Zebra | Brown Ram, Swift Test Horse, Zebra | Brown Ram, Zebra"
    );
    // A favourite by its displayed index: a collected one, the list again.
    assert_eq!(
        joined(
            &mut vm,
            "Events = {}
            local before = { C_MountJournal.GetIsFavorite(3) }
            C_MountJournal.SetIsFavorite(3, true)
            C_MountJournal.SetIsFavorite(4, true)
            return tostring(before[1]), tostring(before[2]), names(), select(2, C_MountJournal.GetIsFavorite(4)), #Events"
        ),
        "false true Brown Ram, Swift Test Horse, Night Saber, Zebra true 1"
    );
    assert_eq!(
        joined(
            &mut vm,
            "return C_MountJournal.GetMountFromSpell(900002), tostring(C_MountJournal.GetMountFromSpell(1)),
                C_MountJournal.GetMountUsabilityByID(11, false), C_MountJournal.GetMountUsabilityByID(13, false),
                C_MountJournal.GetMountUsabilityByID(15, false), tostring(C_MountJournal.HasDisplayableMount())"
        ),
        "12 nil true false false true"
    );
    assert_eq!(
        joined(
            &mut vm,
            "return C_MountJournal.GetMountUsabilityByID(13, false)"
        ),
        "false You have not collected this mount."
    );
    assert_eq!(
        joined(
            &mut vm,
            "return C_MountJournal.GetMountUsabilityByID(15, false)"
        ),
        "false nil"
    );
}

const TOYS: &str = "Events = {}
local f = CreateFrame('Frame')
f:RegisterEvent('TOYS_UPDATED')
f:SetScript('OnEvent', function(_, e, item) Events[#Events + 1] = e .. ':' .. tostring(item) end)
mock.defineItem(801, { name = 'Toy Drum', icon = 134001, quality = 1 }, { cached = true })
mock.defineItem(802, { name = 'Bubble Wand', icon = 134002, quality = 2 }, { cached = true })
mock.defineItem(803, { name = 'Abacus', icon = 134003, quality = 3 }, { cached = true })
mock.defineItem(804, { name = 'Later Toy', icon = 134004, quality = 1 }, { cached = false })
mock.setToys({
    [801] = { favorite = true },
    [802] = { usable = false },
    [803] = { collected = false },
    [804] = true,
})
function shown()
    local out = {}
    for i = 1, C_ToyBox.GetNumFilteredToys() do
        out[#out + 1] = C_ToyBox.GetToyFromIndex(i)
    end
    return table.concat(out, ',')
end
";

/// The toy box's toys: their info from their items, the box's list through
/// its filters and search (favourites first, then collected ones, by
/// name), a favourite set, the counts.
#[test]
fn toys_from_the_test() {
    let mut vm = vm();
    run(&mut vm, TOYS);
    assert_eq!(
        joined(
            &mut vm,
            "return C_ToyBox.GetNumToys(), table.concat(Events, ','), tostring(C_ToyBoxInfo.HasAnyToy()),
                tostring(C_ToyBox.HasFavorites())"
        ),
        "4  true true"
    );
    // A loaded item's name and quality; one not loaded yet, nil.
    assert_eq!(
        joined(&mut vm, "return C_ToyBox.GetToyInfo(801)"),
        "801 Toy Drum 134001 true false 1"
    );
    assert_eq!(
        joined(&mut vm, "return C_ToyBox.GetToyInfo(804)"),
        "804 nil 134004 false false nil"
    );
    assert_eq!(
        joined(
            &mut vm,
            "return tostring(PlayerHasToy(803)), tostring(PlayerHasToy(801)), tostring(C_ToyBox.IsToyUsable(802)),
                tostring(PlayerHasToy(999)), C_ToyBox.GetToyLink(801)"
        ),
        "false true false false |cnIQ1:|Hitem:801::::::::60::::::::::|h[Toy Drum]|h|r"
    );
    // The recorded character's filters: uncollected hidden, unusable shown.
    assert_eq!(
        joined(
            &mut vm,
            "local first = shown()
            C_ToyBox.SetUncollectedShown(true)
            local all = shown()
            C_ToyBox.SetUnusableShown(false)
            local usable = shown()
            C_ToyBox.SetUnusableShown(true)
            C_ToyBox.SetFilterString('wand')
            local search = shown()
            C_ToyBox.SetFilterString('')
            return first, all, usable, search, C_ToyBox.GetNumLearnedDisplayedToys(), C_ToyBox.GetNumTotalDisplayedToys(),
                C_ToyBox.GetToyFromIndex(9)"
        ),
        "801,802,804 801,802,804 801,804 802 3 3 -1"
    );
    assert_eq!(
        joined(
            &mut vm,
            "Events = {}
            C_ToyBox.SetIsFavorite(802, true)
            C_ToyBox.SetIsFavorite(803, true)
            return shown(), tostring(C_ToyBox.GetIsFavorite(802)), tostring(C_ToyBox.GetIsFavorite(803)), table.concat(Events, ',')"
        ),
        "802,801,804 true false TOYS_UPDATED:802"
    );
}

/// The wardrobe's collected sources and favourites over the recorded
/// catalogue: every answer that tells whether a source or an appearance is
/// collected, and the favourites.
#[test]
fn appearances_from_the_test() {
    let mut vm = vm();
    // Source 21598 is appearance 11257's (category 1, item 44803).
    let ask = "local function inCategory(cat, visual)
        for _, a in ipairs(C_TransmogCollection.GetCategoryAppearances(cat)) do
            if a.visualID == visual then return a end
        end
    end
    local a = inCategory(1, 11257)
    local collected = 0
    for _, e in ipairs(C_TransmogCollection.GetCategoryAppearances(1)) do
        if e.isCollected then collected = collected + 1 end
    end
    return tostring(C_TransmogCollection.PlayerKnowsSource(21598)),
        tostring(C_TransmogCollection.GetSourceInfo(21598).isCollected),
        tostring(C_TransmogCollection.GetAppearanceSources(11257, 1)[1].isCollected),
        tostring(a.isCollected), tostring(a.isFavorite or false), collected,
        C_TransmogCollection.GetCategoryCollectedCount(1),
        tostring(C_TransmogCollection.PlayerHasTransmog(44803)),
        tostring(C_TransmogCollection.GetAppearanceInfoBySource(21598).appearanceIsCollected),
        tostring(C_TransmogCollection.GetIsAppearanceFavorite(11257)),
        tostring(C_TransmogCollection.HasFavorites())";
    let recorded = joined(&mut vm, ask);
    assert_eq!(
        recorded,
        "false false false false false 1 1 false false false false"
    );
    run(
        &mut vm,
        "Events = 0
        local f = CreateFrame('Frame')
        f:RegisterEvent('TRANSMOG_COLLECTION_UPDATED')
        f:SetScript('OnEvent', function() Events = Events + 1 end)
        mock.setAppearances({ collected = { 21598 }, favorites = { 11257 } })",
    );
    assert_eq!(
        joined(&mut vm, ask),
        "true true true true true 1 1 true true true true"
    );
    // The collected ones replace the recorded ones; favourites the game
    // sets on top of a test's.
    assert_eq!(
        joined(
            &mut vm,
            "C_TransmogCollection.SetIsAppearanceFavorite(11257, false)
            local unset = C_TransmogCollection.GetIsAppearanceFavorite(11257)
            mock.setAppearances({ collected = {} })
            return Events, tostring(unset), C_TransmogCollection.GetCategoryCollectedCount(1),
                tostring(C_TransmogCollection.PlayerKnowsSource(21598))"
        ),
        "1 false 0 false"
    );
}

/// An appearance's sources asked without its category (as addons ask):
/// every recorded source of it, collected as set; none for one not
/// recorded.
#[test]
fn appearance_sources_without_a_category() {
    let mut vm = vm();
    assert_eq!(
        joined(
            &mut vm,
            "local s = C_TransmogCollection.GetAppearanceSources(11257)
            mock.setAppearances({ collected = { 21598 } })
            return #s, s[1].sourceID, s[1].itemID, tostring(s[1].isCollected),
                tostring(C_TransmogCollection.GetAppearanceSources(11257)[1].isCollected)"
        ),
        "1 21598 44803 false true"
    );
}

/// What the client sends when the journals change (recorded, `/stdtest
/// engine journals` and `journals2`): a mount filter's change tells the
/// list and its CVar at once (1 collected, 2 not collected, 4 unusable),
/// nothing when set as it was; the toy box's checks one `TOYS_UPDATED`
/// (three nils) on the next frame; an uncollected mount is offered as a
/// favourite but not taken.
#[test]
fn the_journals_events() {
    let mut vm = vm();
    run(&mut vm, MOUNTS);
    assert_eq!(
        joined(
            &mut vm,
            "Log = {}
            local f = CreateFrame('Frame')
            for _, e in ipairs({ 'MOUNT_JOURNAL_SEARCH_UPDATED', 'CVAR_UPDATE', 'TOYS_UPDATED' }) do f:RegisterEvent(e) end
            f:SetScript('OnEvent', function(_, e, ...)
                local args = {}
                for i = 1, select('#', ...) do args[i] = tostring((select(i, ...))) end
                Log[#Log + 1] = e .. '(' .. table.concat(args, ',') .. ')'
            end)
            C_MountJournal.SetCollectedFilterSetting(2, true)
            C_MountJournal.SetCollectedFilterSetting(2, true)
            C_MountJournal.SetCollectedFilterSetting(3, false)
            local cvar = C_CVar.GetCVar('mountJournalGeneralFilters')
            C_ToyBox.SetUncollectedShown(false)
            C_ToyBox.SetUnusableShown(false)
            C_ToyBox.SetUnusableShown(false)
            return table.concat(Log, ' '), cvar"
        ),
        "MOUNT_JOURNAL_SEARCH_UPDATED() CVAR_UPDATE(mountJournalGeneralFilters,7) \
         MOUNT_JOURNAL_SEARCH_UPDATED() CVAR_UPDATE(mountJournalGeneralFilters,3) 3"
            .replace("         ", "")
    );
    wowlua_engine::clock::advance_frames(&mut vm, 2, 1000.0 / 60.0);
    assert_eq!(
        joined(&mut vm, "return table.concat(Log, ' ', 5)"),
        "TOYS_UPDATED(nil,nil,nil)"
    );
    // Zebra, not collected, at the end of the list: offered, not taken.
    assert_eq!(
        joined(
            &mut vm,
            "Log = {}
            C_MountJournal.SetCollectedFilterSetting(3, true)
            Log = {}
            local n = C_MountJournal.GetNumDisplayedMounts()
            local name = C_MountJournal.GetDisplayedMountInfo(n)
            local before = { C_MountJournal.GetIsFavorite(n) }
            C_MountJournal.SetIsFavorite(n, true)
            return name, tostring(before[1]), tostring(before[2]), tostring((C_MountJournal.GetIsFavorite(n))), #Log"
        ),
        "Zebra false true false 0"
    );
}

/// An appearance's favourite (recorded in four categories):
/// `TRANSMOG_COLLECTION_UPDATED(category + 1, its collected source, the
/// appearance, "favorite")` at once, `TRANSMOG_COLLECTION_ITEM_FAVORITE_UPDATE`
/// when it is no longer one, and `TRANSMOG_COLLECTION_UPDATED` with four
/// nils 0.6 s later; set as it was, nothing.
#[test]
fn an_appearance_favourites_events() {
    let mut vm = vm();
    assert_eq!(
        joined(
            &mut vm,
            "Log = {}
            local f = CreateFrame('Frame')
            for _, e in ipairs({ 'TRANSMOG_COLLECTION_UPDATED', 'TRANSMOG_COLLECTION_ITEM_FAVORITE_UPDATE' }) do f:RegisterEvent(e) end
            f:SetScript('OnEvent', function(_, e, ...)
                local args = {}
                for i = 1, select('#', ...) do args[i] = tostring((select(i, ...))) end
                Log[#Log + 1] = e .. '(' .. table.concat(args, ',') .. ')'
            end)
            mock.setAppearances({ collected = { 21598 } })
            C_TransmogCollection.SetIsAppearanceFavorite(11257, true)
            C_TransmogCollection.SetIsAppearanceFavorite(11257, true)
            local on = table.concat(Log, ' ')
            Log = {}
            C_TransmogCollection.SetIsAppearanceFavorite(11257, false)
            return on, '|', table.concat(Log, ' ')"
        ),
        "TRANSMOG_COLLECTION_UPDATED(2,21598,11257,favorite) | \
         TRANSMOG_COLLECTION_UPDATED(2,21598,11257,favorite) \
         TRANSMOG_COLLECTION_ITEM_FAVORITE_UPDATE(11257,false)"
            .replace("         ", "")
    );
    run(&mut vm, "Log = {}");
    wowlua_engine::clock::advance(&mut vm, 0.7);
    assert_eq!(
        joined(&mut vm, "return table.concat(Log, ' ')"),
        "TRANSMOG_COLLECTION_UPDATED(nil,nil,nil,nil) TRANSMOG_COLLECTION_UPDATED(nil,nil,nil,nil)"
    );
}

/// A source's `name` and `quality` are its item's while the client has the
/// item loaded (the documentation's optional fields), nil before.
#[test]
fn a_sources_item_name_and_quality() {
    let mut vm = vm();
    assert_eq!(
        joined(
            &mut vm,
            "local before = C_TransmogCollection.GetAppearanceSources(11257)[1]
            mock.defineItem(44803, { name = 'Plain Cloak', quality = 2 }, { cached = true })
            local s = C_TransmogCollection.GetAppearanceSources(11257, 1)[1]
            local info = C_TransmogCollection.GetSourceInfo(21598)
            return tostring(before.name), s.name, s.quality, info.name, info.quality"
        ),
        "nil Plain Cloak 2 Plain Cloak 2"
    );
}
