//! The engine's API from the documentation.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use wowlua_vm::{Value, Vm};

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "=test").expect("compiles");
    let r = vm.call(f, &[]).unwrap_or_else(|e| match e.value {
        Value::Str(s) => panic!("{}", String::from_utf8_lossy(vm.str_bytes(s))),
        _ => panic!("an error"),
    });
    r.into_iter()
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn documented_functions_exist_and_are_not_mocked() {
    // Every camelot function has behaviour (decision T295); a flavor with
    // gaps shows the stand-in: a function without behaviour fails for
    // addon code, and gives its documented empty values to Blizzard's.
    let gap = wowlua_engine::coverage::functions("vanilla")
        .expect("vanilla")
        .into_iter()
        .find(|e| {
            e.source == wowlua_engine::coverage::Source::Generated
                && e.group == wowlua_engine::coverage::Group::Documented
                && !e.name.contains('.')
                && !wowlua_engine::protection::is_protected(&e.name, None)
        })
        .expect("a function without behaviour on vanilla")
        .name;
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "vanilla").expect("installs");
    let src =
        format!("return type({gap}), select(2, pcall({gap})), type(GetTime), type(secretwrap)");
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    let r: Vec<String> = vm
        .call(f, &[])
        .expect("runs")
        .into_iter()
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        r,
        [
            "function".to_string(),
            format!("{gap} is not mocked"),
            "function".to_string(),
            "function".to_string()
        ]
    );
    // On camelot it has an answer: its documented empty values.
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(&mut vm, "return IsCollisionEnabled()");
    assert_eq!(r, ["Bool(false)"]);
}

#[test]
fn enums_and_constants() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(
        &mut vm,
        "return tostring(Enum.AddOnRestrictionType.Combat), tostring(Enum.AddOnRestrictionType.Map), tostring(Enum.TableSecurityOption.SecretWrapContents)",
    );
    assert_eq!(r, ["0", "4", "2"]);
}

#[test]
fn every_flavor_installs() {
    for flavor in &wowlua_flavors::table().flavors {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, &flavor.id).expect("installs");
    }
}

/// The `C_` functions' old global names camelot's client lacks (its
/// inventory, forever-camelot-functions.tsv) are not defined there, the
/// `C_` functions are; Blizzard's deprecation files' action functions
/// stand in as the engine's until its UI code loads; another flavor keeps
/// the old names (unverified).
#[test]
fn camelot_lacks_the_old_global_names() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(
        &mut vm,
        "return type(GetItemCount), type(GetMouseFocus), type(InviteUnit), type(PickupContainerItem),
            type(PickupSpell), type(PickupSpellBookItem), type(RunMacroText), type(SplitContainerItem),
            type(UseContainerItem), type(UseItemByName),
            type(C_Item.GetItemCount), type(GetMouseFoci), type(C_PartyInfo.InviteUnit),
            type(C_Container.PickupContainerItem), type(C_Spell.PickupSpell), type(C_Container.UseContainerItem),
            type(HasAction), type(GetActionText)",
    );
    let mut expected = vec!["nil"; 10];
    expected.extend(["function"; 8]);
    assert_eq!(r, expected);
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "vanilla").expect("installs");
    let r = run(
        &mut vm,
        "return type(GetItemCount), type(PickupContainerItem)",
    );
    assert_eq!(r, ["function", "function"]);
}

/// A client's C functions its documentation lacks exist on its flavor
/// (its inventory, Ketho's dump for the unrecorded ones) and not on a
/// flavor whose client lacks them.
#[test]
fn each_clients_undocumented_functions_exist_on_its_flavor() {
    let names = "type(GetQuestLogTitle), type(GetGlyphSocketInfo), type(GetReadyCheckStatus)";
    for (flavor, expected) in [
        ("standard", ["nil", "nil", "function"]),
        ("mists", ["function", "function", "function"]),
        ("tbc", ["function", "nil", "function"]),
        ("vanilla", ["function", "nil", "function"]),
        ("camelot", ["nil", "nil", "function"]),
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
        assert_eq!(
            run(&mut vm, &format!("return {names}")),
            expected,
            "{flavor}"
        );
    }
}

/// What every client answers with nothing there, and its expansion; the
/// classic clients' hardcore rule, which standard's lacks.
#[test]
fn every_client_answers_with_nothing_there() {
    let src = "return tostring(IsTargetLoose()), tostring(C_ActionBar.GetBonusBarOffset()),
        tostring(UnitIsTapDenied('target')), tostring(GetRaidTargetIndex('target')),
        tostring(select('#', GetReadyCheckStatus('player'))), tostring(GetExpansionLevel()),
        type(C_GameRules.IsHardcoreActive)";
    for (flavor, level, hardcore) in [
        ("standard", "11", "nil"),
        ("mists", "4", "function"),
        ("tbc", "1", "function"),
        ("vanilla", "0", "function"),
        ("camelot", "0", "function"),
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
        assert_eq!(
            run(&mut vm, src),
            ["false", "0", "false", "nil", "0", level, hardcore],
            "{flavor}"
        );
    }
}

/// On a classic flavor `C_CombatLog.GetCurrentEventInfo` answers the
/// payload of the combat log event last fired; nothing before one.
#[test]
fn the_classic_combat_log_tells_the_current_event() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "vanilla").expect("installs");
    assert!(run(&mut vm, "return C_CombatLog.GetCurrentEventInfo()").is_empty());
    let args = [Value::Num(12.5), vm.str("SPELL_DAMAGE")];
    wowlua_engine::widgets::fire_event(&mut vm, "COMBAT_LOG_EVENT_UNFILTERED", &args);
    let r = run(
        &mut vm,
        "local t, e = C_CombatLog.GetCurrentEventInfo() return tostring(t), e",
    );
    assert_eq!(r, ["12.5", "SPELL_DAMAGE"]);
}

#[test]
fn the_classes_have_their_clients_colours() {
    for (flavor, expected) in [
        (
            "standard",
            ["0.77647066116333", "0.43921571969986", "0.59607845544815"],
        ),
        (
            "mists",
            ["0.77647066116333", "0.43921571969986", "0.59607845544815"],
        ),
        ("tbc", ["0.77647066116333", "0.43921571969986", "none"]),
        ("vanilla", ["0.77647066116333", "0.54901963472366", "none"]),
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
        let src = "local function g(class) local c = C_ClassColor.GetClassColor(class) return c and tostring(c.r == nil and c or (class == 'SHAMAN' and c.g or class == 'MONK' and c.b or c.r)) or 'none' end
            return g('WARRIOR'), g('SHAMAN'), g('MONK')";
        assert_eq!(run(&mut vm, src), expected, "{flavor}");
    }
}

#[test]
fn the_classic_clients_refuse_what_they_no_longer_support() {
    let era = "GetTalentGroupRole GetNumSpecGroups GetTalentTierInfo";
    for (flavor, refused) in [
        (
            "mists",
            "GetTalentGroupRole GetNumTalentTabs GetNumTalentGroups GetUnspentTalentPoints",
        ),
        ("tbc", era),
        ("vanilla", era),
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
        let src = "local out = {}
            for _, name in ipairs({ 'GetTalentGroupRole', 'GetNumTalentTabs', 'GetNumTalentGroups',
                'GetUnspentTalentPoints', 'GetNumSpecGroups', 'GetTalentTierInfo' }) do
                local ok, err = pcall(_G[name], 1)
                if not ok and err == 'Script_' .. name .. ': API unsupported in this version of World of Warcraft.' then
                    out[#out + 1] = name
                end
            end
            local runes = {}
            for i = 1, 6 do runes[i] = tostring(GetRuneType(i)) end
            runes[7] = select('#', GetRuneType(7))
            return table.concat(out, ' '), table.concat(runes, ' ')";
        assert_eq!(run(&mut vm, src), [refused, "1 1 3 3 2 2 0"], "{flavor}");
    }
}

#[test]
fn the_functions_the_dumps_lack_exist() {
    for (flavor, age) in [
        ("standard", "function"),
        ("mists", "nil"),
        ("vanilla", "nil"),
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
        let src = "return type(Kiosk.GetCharacterTemplateSetIndex), type(C_SocialRestrictions and C_SocialRestrictions.IsAgeVerificationRestricted)";
        assert_eq!(run(&mut vm, src), ["function", age], "{flavor}");
    }
}

#[test]
fn a_test_sets_a_hardcore_realm() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "vanilla").expect("installs");
    wowlua_engine::controls::install(&mut vm, "engine");
    let src = "local before = C_GameRules.IsHardcoreActive()
        engine.setHardcore(true)
        return tostring(before), tostring(C_GameRules.IsHardcoreActive())";
    assert_eq!(run(&mut vm, src), ["false", "true"]);
    for flavor in ["standard", "mists", "tbc"] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
        wowlua_engine::controls::install(&mut vm, "engine");
        let src = "local ok, e = pcall(engine.setHardcore, true) return tostring(ok), e";
        assert_eq!(
            run(&mut vm, src),
            [
                "false",
                "mock.setHardcore: this flavor has no hardcore realms"
            ],
            "{flavor}"
        );
    }
}

#[test]
fn the_classic_clients_tell_their_expansions_and_weapon_enchants() {
    // The server's expansion only after login (the wiki).
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "mists").expect("installs");
    assert_eq!(
        run(&mut vm, "return tostring(GetServerExpansionLevel())"),
        ["0"]
    );
    for (flavor, expected) in [
        ("standard", "11 11 11 11 10 0"),
        ("mists", "4 4 0 4 3 8"),
        ("tbc", "1 1 0 1 5 12"),
        ("vanilla", "0 0 0 0 5 12"),
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
        wowlua_engine::widgets::fire_event(&mut vm, "PLAYER_LOGIN", &[]);
        let src = "local function n(name) local f = _G[name] return f and tostring(f()) or '-' end
            local enchants = GetWeaponEnchantInfo and pcall(GetWeaponEnchantInfo) and select('#', GetWeaponEnchantInfo()) or 0
            return table.concat({ n('GetServerExpansionLevel'), n('GetClassicExpansionLevel'), n('GetAccountExpansionLevel'),
                n('GetMaximumExpansionLevel'), n('GetMinimumExpansionLevel'), tostring(enchants) }, ' ')";
        assert_eq!(run(&mut vm, src), [expected], "{flavor}");
    }
}
