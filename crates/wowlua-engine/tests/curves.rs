//! The client's script objects that keep secrets: curves, duration objects
//! and numeric formatters, as Blizzard's documentation describes them.

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

/// A result as text; a secret as `secret:` and its hidden value.
fn text(vm: &Vm, v: Value) -> String {
    let plain = |v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match v {
        Value::Secret(_) => format!("secret:{}", plain(vm.hidden(v))),
        v => plain(v),
    }
}

fn call(vm: &mut Vm, f: Value) -> Vec<String> {
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// Runs code as an addon's.
fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    call(vm, f)
}

/// Runs code as Blizzard's (secure: it may use secrets).
fn secure(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "@Secure.lua").expect("compiles");
    call(vm, f)
}

#[test]
fn curves_evaluate_by_their_type() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local c = C_CurveUtil.CreateCurve()
local empty = c:Evaluate(5)
c:AddPoint(1, 10)
c:AddPoint(0, 0)
local linear, below, above = c:Evaluate(0.5), c:Evaluate(-1), c:Evaluate(2)
c:SetType(Enum.LuaCurveType.Step)
local step, onPoint = c:Evaluate(0.5), c:Evaluate(1)
c:SetType(Enum.LuaCurveType.Cosine)
local cosine = ('%.4f'):format(c:Evaluate(0.25))
return empty, linear, below, above, step, onPoint, cosine, c:GetType(), c:GetPointCount()",
    );
    assert_eq!(r, ["0", "5", "0", "10", "0", "10", "1.4645", "2", "2"]);
}

#[test]
fn curve_points_are_read_copied_and_replaced() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local c = C_CurveUtil.CreateCurve()
c:AddPoint(1, 10)
c:AddPoint(0, 3)
local p, missing = c:GetPoint(1), c:GetPoint(3)
local all = c:GetPoints()
local copy = c:Copy()
c:RemovePoint(1)
local bad = pcall(c.RemovePoint, c, 5)
local after = c:GetPointCount()
copy:SetPoints({ { x = 0, y = 1 }, { x = 2, y = 3 } })
local mid = copy:Evaluate(1)
copy:SetType(Enum.LuaCurveType.Step)
copy:SetToDefaults()
return p.x, p.y, tostring(missing), #all, all[2].y, tostring(bad), after, mid, copy:GetPointCount(),
    copy:GetType(), tostring(copy:HasSecretValues())",
    );
    assert_eq!(
        r,
        [
            "0", "3", "nil", "2", "10", "false", "1", "2", "0", "0", "false"
        ]
    );
}

#[test]
fn color_curves_give_colors() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local cc = C_CurveUtil.CreateColorCurve()
cc:AddPoint(0, { r = 1, g = 0, b = 0, a = 1 })
cc:AddPoint(10, { r = 0, g = 0, b = 1 })
local c = cc:Evaluate(5)
local r, g, b, a = cc:EvaluateUnpacked(5)
local p = cc:GetPoint(2)
return c.r, c.b, c.a, r, g, b, a, p.x, p.y.b, cc:GetType()",
    );
    assert_eq!(
        r,
        ["0.5", "0.5", "1", "0.5", "0", "0.5", "1", "10", "1", "0"]
    );
}

#[test]
fn secret_values_make_secret_results() {
    let mut vm = vm();
    let r = secure(
        &mut vm,
        "local c = C_CurveUtil.CreateCurve()
c:AddPoint(0, 0)
c:AddPoint(1, secretwrap(10))
local plain = C_CurveUtil.CreateCurve()
plain:AddPoint(0, 0)
plain:AddPoint(1, 1)
local copy = c:Copy()
local before = c:HasSecretValues()
c:SetToDefaults()
return before, c:Evaluate(0.5), copy:Evaluate(0.5), copy:HasSecretValues(), plain:Evaluate(secretwrap(0.5)),
    plain:Evaluate(0.5), plain:HasSecretValues()",
    );
    assert_eq!(
        r,
        [
            "true",
            "0",
            "secret:5",
            "true",
            "secret:0.5",
            "0.5",
            "false"
        ]
    );
}

#[test]
fn colors_from_booleans() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local yes, no = { r = 1, g = 1, b = 1, a = 1 }, { r = 0, g = 0, b = 0, a = 0.5 }
local c = C_CurveUtil.EvaluateColorFromBoolean(false, yes, no)
return c.a, C_CurveUtil.EvaluateColorValueFromBoolean(true, 0.25, 1)",
    );
    assert_eq!(r, ["0.5", "0.25"]);
    let r = secure(
        &mut vm,
        "local c = C_CurveUtil.EvaluateColorFromBoolean(secretwrap(true), { r = 1, g = 1, b = 1 }, { r = 0, g = 0, b = 0 })
return c.r, C_CurveUtil.EvaluateColorValueFromBoolean(secretwrap(false), 0.25, 1)",
    );
    assert_eq!(r, ["secret:1", "secret:1"]);
}

/// SuuUI's aura duration format: whole seconds under a minute, m:ss under
/// ten minutes, then minutes, then hours.
const RULES: &str = "local Up, Down = Enum.NumericRuleFormatRounding.Up, Enum.NumericRuleFormatRounding.Down
local f = C_StringUtil.CreateNumericRuleFormatter()
f:SetBreakpoints({
    { threshold = 0, format = '%d', step = 1, rounding = Up },
    { threshold = 60, format = '%d:%02d', step = 1, rounding = Up, components = { { div = 60 }, { mod = 60 } } },
    { threshold = 600, format = '%dm', step = 1, rounding = Down, components = { { div = 60 } } },
    { threshold = 3600, format = '%dh', step = 1, rounding = Down, components = { { div = 3600 } } },
})
";

#[test]
fn numeric_rule_formatters_pick_a_rule_by_threshold() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        &format!(
            "{RULES}
local bps = f:GetBreakpoints()
local bad = pcall(f.AddBreakpoint, f, {{ threshold = 1 }})
local d = C_DurationUtil.CreateDuration()
d:SetTimeFromStart(GetTime(), 100)
return f:FormatNumber(42.3), f:FormatNumber(90), f:FormatNumber(600), f:FormatNumber(3700), #bps,
    bps[2].components[2].mod, bps[3].rounding, tostring(bad), f:Copy():FormatNumber(61.5),
    d:FormatRemainingDuration(f)"
        ),
    );
    assert_eq!(
        r,
        [
            "43", "1:30", "10m", "1h", "4", "60", "2", "false", "1:02", "1:40"
        ]
    );
}

#[test]
fn abbreviated_number_formatters() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = C_StringUtil.CreateAbbreviatedNumberFormatter()
local a, b, c, d = f:FormatNumber(1234), f:FormatNumber(12345), f:FormatNumber(999), f:FormatNumber(1500000)
local bad = pcall(f.SetBreakpoints, f, { { breakpoint = 1000, abbreviation = 'k', significandDivisor = 0, fractionDivisor = 10 } })
f:SetBreakpoints({ { breakpoint = 1000, abbreviation = 'k', significandDivisor = 100, fractionDivisor = 10, abbreviationIsGlobal = false } })
return a, b, c, d, tostring(bad), f:FormatNumber(123456), #C_StringUtil.GetDefaultAbbreviationBreakpoints()",
    );
    assert_eq!(r, ["1.2K", "12K", "999", "1.5M", "false", "123.4k", "8"]);
}

#[test]
fn seconds_formatters() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local OneLetter = Enum.SecondsFormatterAbbreviation.OneLetter
local f = C_StringUtil.CreateSecondsFormatter()
local plain, short = f:Format(93), f:Format(93, OneLetter)
-- Blizzard's aura duration formatter (Blizzard_AuraContainerShared).
local aura = C_StringUtil.CreateSecondsFormatter()
local curve = C_CurveUtil.CreateCurve()
curve:SetType(Enum.LuaCurveType.Step)
curve:AddPoint(0, Enum.SecondsFormatterInterval.Seconds)
curve:AddPoint(1 + 1.5 * 60, Enum.SecondsFormatterInterval.Minutes)
curve:AddPoint(1 + 1.5 * 3600, Enum.SecondsFormatterInterval.Hours)
aura:SetDefaultAbbreviation(OneLetter)
aura:SetRounding(Enum.SecondsFormatterRounding.Truncate)
aura:SetCanRoundUpLastUnit(true)
aura:SetMinInterval(Enum.SecondsFormatterInterval.Seconds)
aura:SetMaxIntervalCurve(curve)
aura:SetDesiredUnitCount(1)
local approx = C_StringUtil.CreateSecondsFormatter()
approx:SetApproximationSeconds(7200)
local promote = C_StringUtil.CreateSecondsFormatter()
promote:SetDesiredUnitCount(1)
promote:SetCanRoundUpLastUnit(true)
promote:SetCanRoundUpIntervals(true)
promote:SetDefaultAbbreviation(OneLetter)
return plain, short, aura:Format(90.5), aura:Format(125), aura:FormatZero(), tostring(aura:GetMaxInterval()),
    tostring(aura:GetMaxIntervalCurve() == curve), aura:EvaluateMaxInterval(125), approx:Format(5000),
    tostring(approx:CanApproximate(10)), promote:Format(3590), promote:Format(119)",
    );
    assert_eq!(
        r,
        [
            "1 |4Minute:Minutes; 33 |4Second:Seconds;",
            "1 m 33 s",
            "90 s",
            "3 m",
            "0 s",
            "nil",
            "true",
            "1",
            "< 2 |4Hour:Hours;",
            "true",
            "1 h",
            "2 m"
        ]
    );
}

#[test]
fn string_helpers() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local link = '|cff0070dd|Hitem:1234::::|h[Sword]|h|r and [this]'
return C_StringUtil.StripHyperlinks(link), C_StringUtil.StripHyperlinks(link, true, true),
    C_StringUtil.EscapeDecimalNonPrintables('a\\1b\\127\\255'), C_StringUtil.EscapeLuaFormatString('5%'),
    C_StringUtil.EscapeLuaPatterns('a.b'), C_StringUtil.EscapeQuotedCodes('|cff'),
    C_StringUtil.TruncateWhenZero(0.5), C_StringUtil.TruncateWhenZero(7.9), C_StringUtil.WrapString('x', '(', ')'),
    C_StringUtil.WrapString('', '(', ')'), C_StringUtil.RemoveContiguousSpaces('a    b', 1),
    C_StringUtil.RoundToNearestString(2.5), C_StringUtil.FloorToNearestString(2.5)",
    );
    assert_eq!(
        r,
        [
            "Sword and [this]",
            "|cff0070dd[Sword]|r and [this]",
            "a\\1b\\127\\255",
            "5%%",
            "a%.b",
            "||cff",
            "",
            "7",
            "(x)",
            "",
            "a b",
            "3",
            "2"
        ]
    );
}

/// A unit's cast as a duration object: readable by addons, its values
/// secret (`UnitCastingDuration` is documented `SecretReturns`); a channel
/// as a plain one; nothing without either.
#[test]
fn casts_as_duration_objects() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local hogger = mock.spawnUnit({ name = 'Hogger', level = 11, reaction = 'hostile' })
mock.player:target(hogger)
local none = UnitCastingDuration('target')
hogger:castSpell(133, { castTime = 2 })
local d = UnitCastingDuration('target')
return tostring(none), tostring(issecretvalue(d)), tostring(d:HasSecretValues()), d:GetTotalDuration(),
    tostring((UnitChannelDuration('target')))",
    );
    assert_eq!(r, ["nil", "false", "true", "secret:2", "nil"]);
    let r = run(
        &mut vm,
        "local wolf = mock.spawnUnit({ name = 'Wolf', level = 10, reaction = 'hostile' })
mock.player:target(wolf)
wolf:castSpell(5143, { castTime = 3, channel = true })
local d = UnitChannelDuration('target')
return tostring(d:HasSecretValues()), d:GetTotalDuration(), tostring((UnitCastingDuration('target')))",
    );
    assert_eq!(r, ["false", "3", "nil"]);
}

/// The documentation's userdata script objects are userdata: methods
/// through their metatable, fields kept (Blizzard's `TimerUtil` defines a
/// method on its signal map), `securecopy` passing them on.
#[test]
fn script_objects_are_userdata() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local c = C_CurveUtil.CreateCurve()
local objects = { c, C_CurveUtil.CreateColorCurve(), C_StringUtil.CreateNumericRuleFormatter(),
    C_StringUtil.CreateAbbreviatedNumberFormatter(), C_StringUtil.CreateSecondsFormatter(),
    C_DurationUtil.CreateDuration(), C_DurationUtil.CreateDurationTextBinding(),
    C_Timer.NewTimedSignalMap(function() end) }
local types = {}
for i, o in ipairs(objects) do types[i] = type(o) end
c:AddPoint(0, 1)
c.note = 'kept'
local copy = securecopy({ curve = c, list = objects })
local f = CreateFrame('Frame')
local refused = pcall(securecopy, { frame = f })
return table.concat(types, ' '), c:GetPointCount(), c.note, tostring(copy.curve == c),
    tostring(copy.list[8] == objects[8]), tostring(refused), tostring(c.Missing)",
    );
    assert_eq!(
        r,
        [
            "userdata userdata userdata userdata userdata userdata userdata userdata",
            "1",
            "kept",
            "true",
            "true",
            "false",
            "nil"
        ]
    );
}

/// A duration text binding writes its duration's text into its font
/// string each frame while enabled: its formatter's remaining time, a text
/// format's components, the zero-duration and expired texts.
#[test]
fn duration_text_bindings() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local fs = CreateFrame('Frame'):CreateFontString()
fs:SetFont('Fonts\\\\FRIZQT__.TTF', 12, '')
local f = C_StringUtil.CreateSecondsFormatter()
f:SetDefaultAbbreviation(Enum.SecondsFormatterAbbreviation.OneLetter)
local b = C_DurationUtil.CreateDurationTextBinding()
local fresh = { tostring(b:IsEnabled()), tostring(b:CanFormatText()), b:GetFormattedText() }
b:SetZeroDurationText('none')
local zero = b:GetFormattedText()
local d = C_DurationUtil.CreateDuration()
d:SetTimeFromStart(GetTime(), 90)
b:SetFormatter(f)
b:SetDuration(d)
b:SetFontString(fs)
mock.nextFrame()
local disabled = tostring(fs:GetText())
b:SetEnabled(true)
mock.nextFrame()
local shown = fs:GetText()
b:SetTextFormat('{} of {}', { { property = Enum.DurationTextBindingProperty.ElapsedDuration, formatter = f },
    { property = Enum.DurationTextBindingProperty.TotalDuration, formatter = f } })
mock.advanceTime(30)
mock.nextFrame()
local formatted = fs:GetText()
b:SetExpiredText('done')
mock.advanceTime(61)
mock.nextFrame()
local copy = b:Copy()
b:SetToDefaults()
return fresh[1], fresh[2], fresh[3], zero, disabled, shown, formatted, fs:GetText(),
    tostring(copy:GetDuration() == d), tostring(b:GetDuration()), tostring(b:CanUpdateFontString()),
    tostring(copy:CanUpdateFontString())",
    );
    assert_eq!(
        r,
        [
            "false",
            "false",
            "",
            "none",
            "nil",
            "1 m 30 s",
            // The frames' time too: past 30 s, rounded up.
            "31 s of 1 m 30 s",
            "done",
            "true",
            "nil",
            "false",
            "true"
        ]
    );
}
