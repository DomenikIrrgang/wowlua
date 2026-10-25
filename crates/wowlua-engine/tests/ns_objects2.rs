//! The userdata script objects of `objects2.rs`: manual clocks driving
//! duration objects, locale contexts, abbreviation configs, heal
//! prediction values and housing catalog searchers.

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
fn a_manual_clock_drives_a_duration() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local clock = C_DurationUtil.CreateManualClock()
local d = C_DurationUtil.CreateDuration()
d:SetClock(clock)
d:SetTimeFromStart(10, 20)
local before = d:GetRemainingDuration()
clock:SetTime(15)
local during = d:GetRemainingDuration()
mock.advanceTime(100)
local still = d:GetRemainingDuration()
clock:AdvanceTime(10)
return type(clock), before, during, still, d:GetRemainingDuration(), clock:GetTime(),
    d:GetClock() == clock, d:Copy():GetClockTime()",
    );
    assert_eq!(r, ["userdata", "20", "15", "15", "5", "25", "true", "25"]);
}

#[test]
fn locale_contexts_format_per_locale() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local de = C_Intl.CreateLocaleContext('deDE')
local en = C_Intl.CreateLocaleContext('enUS')
return de:GetLocale(), de:FormatNumber(1234.5, 0), en:FormatNumber(1234.5, 0),
    en:SelectPlural(1, 0), en:SelectPlural(2, 0), de:TransformLocale(3), de:TransformLocale(5),
    en:ToUpper('abc'), de:GetDisplayName('enUS')",
    );
    assert_eq!(
        r,
        [
            "deDE",
            "1.234,5",
            "1,234.5",
            "one",
            "other",
            "de",
            "DE",
            "ABC",
            "German (Germany)"
        ]
    );
}

#[test]
fn abbreviate_configs_keep_their_breakpoints() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local data = {
    { breakpoint = 1000, abbreviation = 'k', significandDivisor = 100, fractionDivisor = 10, abbreviationIsGlobal = false },
}
local config = CreateAbbreviateConfig(data)
return #config:GetAbbreviateNumberData(), AbbreviateNumbers(1234, { config = config }),
    AbbreviateNumbers(999, { config = config })",
    );
    assert_eq!(r, ["1", "1.2k", "999"]);
}

#[test]
fn heal_calculators_keep_predicted_values() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local calc = CreateUnitHealPredictionCalculator()
calc:SetPredictedValues({ health = 50, healthMax = 200, totalIncomingHeals = 10 })
local v = calc:GetPredictedValues()
local health = calc:GetCurrentHealth()
calc:ResetPredictedValues()
return v.health, v.healthMax, v.totalIncomingHeals, health, calc:GetPredictedValues().health",
    );
    assert_eq!(r, ["50", "200", "10", "50", "0"]);
}

#[test]
fn catalog_searchers_find_nothing() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local s = C_HousingCatalog.CreateCatalogSearcher()
local collected = s:IsCollectedActive()
s:ToggleCollected()
s:SetSearchText('chair')
return collected, s:IsCollectedActive(), s:GetSearchText(), s:GetNumSearchItems(),
    #s:GetCatalogSearchResults(), s:GetSortType(), s:GetFilterTagStatus(1, 2)",
    );
    assert_eq!(r, ["true", "false", "chair", "0", "0", "0", "true"]);
}
