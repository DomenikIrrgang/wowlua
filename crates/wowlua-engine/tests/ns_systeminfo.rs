//! The client's answers about itself (`systeminfo.rs`): math helpers,
//! encodings, keys held, CVar bitfields, add-on states.

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
fn math_helpers_as_blizzards_lua() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return Round(2.5), Round(-2.5), Clamp(5, 0, 3), Lerp(0, 10, 0.25), Saturate(1.5),
    RoundToSignificantDigits(1.2345, 2)",
    );
    assert_eq!(r, ["3", "-3", "3", "2.5", "1", "1.23"]);
}

#[test]
fn encodings_round_trip() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local E = C_EncodingUtil
local b64 = E.EncodeBase64('hello')
local t = E.DeserializeCBOR(E.SerializeCBOR({ 1, 2, { a = 'x' } }))
local j = E.DeserializeJSON(E.SerializeJSON({ a = 1 }))
local packed = E.CompressString(string.rep('ab', 50))
return b64, E.DecodeBase64(b64), E.EncodeHex('AZ'), E.DecodeHex('415a'), t[2], t[3].a, j.a,
    E.DecompressString(packed) == string.rep('ab', 50)",
    );
    assert_eq!(
        r,
        ["aGVsbG8=", "hello", "415a", "AZ", "2", "x", "1", "true"]
    );
}

#[test]
fn keys_held_and_bitfields() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = IsKeyDown('LSHIFT')
mock.setModifiers({ shift = true })
local ok = C_CVar.SetCVarBitfield('closedInfoFrames', 3, true)
return before, IsKeyDown('LSHIFT'), IsKeyDown('LCTRL'), ok,
    C_CVar.GetCVarBitfield('closedInfoFrames', 3), C_CVar.GetCVarBitfield('closedInfoFrames', 4)",
    );
    assert_eq!(r, ["false", "true", "false", "true", "true", "false"]);
}

#[test]
fn addons_disabled_and_enabled() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineAddon('Other', { metadata = { Title = 'The Other', Dependencies = 'A, B' } })
local exists = C_AddOns.DoesAddOnExist('Other')
C_AddOns.DisableAddOn('Other')
local off = C_AddOns.GetAddOnEnableState('Other')
C_AddOns.EnableAddOn('Other')
local a, b = C_AddOns.GetAddOnDependencies('Other')
return exists, off, C_AddOns.GetAddOnEnableState('Other'), C_AddOns.GetAddOnTitle('Other'), a, b,
    C_AddOns.DoesAddOnExist('Missing')",
    );
    assert_eq!(r, ["true", "0", "2", "The Other", "A", "B", "false"]);
}
