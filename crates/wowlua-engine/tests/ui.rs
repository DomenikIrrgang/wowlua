//! Blizzard's UI code loaded as secure code: a small UI tree, and the
//! flavor's own when it is on disk (`cargo xtask types` fetches it).

use std::path::{Path, PathBuf};

use wowlua_vm::{Value, Vm};

fn vm(flavor: &str) -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, flavor).expect("installs");
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

/// Writes `files` under a fresh directory; returns its `Interface` folder.
fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("wowlua-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in files {
        let p = root.join("Interface").join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    root.join("Interface")
}

#[test]
fn templates_mixins_scripts_and_load_order() {
    let interface = tree(
        "basic",
        &[
            (
                "AddOns/Blizzard_Base/Blizzard_Base.toc",
                "## Title: Base\n## LoadFirst: 1\nMixin.lua\nTemplates.xml\n[Family]\\Family.lua\n",
            ),
            (
                "AddOns/Blizzard_Base/Mixin.lua",
                "local name = ...\nLoadLog = { name }\nMyMixin = {}\nfunction MyMixin:OnLoad() LoadLog[#LoadLog + 1] = 'mixin ' .. (self:GetName() or '?') end\nfunction MyMixin:Hello() return 'hello' end\n",
            ),
            (
                "AddOns/Blizzard_Base/Mainline/Family.lua",
                "FamilyLoaded = true\n",
            ),
            (
                "AddOns/Blizzard_Base/Templates.xml",
                r#"<Ui xmlns="http://www.blizzard.com/wow/ui/">
  <Frame name="MyTemplate" virtual="true" mixin="MyMixin">
    <Size x="100" y="40"/>
    <KeyValues><KeyValue key="kind" value="templated"/><KeyValue key="count" value="3" type="number"/></KeyValues>
    <Layers><Layer level="ARTWORK"><FontString parentKey="Label" inherits="MyFont" text="MY_TEXT"/></Layer></Layers>
    <Scripts><OnLoad method="OnLoad"/></Scripts>
  </Frame>
  <Font name="MyFont" font="Fonts\FRIZQT__.TTF" height="14" outline="NORMAL" virtual="true"/>
  <Frame name="MyEventFrame" intrinsic="true">
    <Scripts><OnLoad>LoadLog[#LoadLog + 1] = 'intrinsic'</OnLoad></Scripts>
  </Frame>
</Ui>"#,
            ),
            (
                "AddOns/Blizzard_Later/Blizzard_Later.toc",
                "## Dep: Blizzard_Base\nLater.lua\nLater.xml\n",
            ),
            (
                "AddOns/Blizzard_Later/Later.lua",
                "MY_TEXT = 'Shown text'\nLoadLog[#LoadLog + 1] = 'later'\n",
            ),
            (
                "AddOns/Blizzard_Later/Later.xml",
                r#"<Ui>
  <Frame name="MyFrame" parent="UIParent" inherits="MyTemplate" hidden="true">
    <Anchors><Anchor point="TOPLEFT" x="10" y="-20"/></Anchors>
    <Frames>
      <Button name="$parentButton" parentKey="Button">
        <Scripts>
          <OnLoad>LoadLog[#LoadLog + 1] = 'button ' .. self:GetName()</OnLoad>
          <OnClick>Clicked = button</OnClick>
        </Scripts>
      </Button>
    </Frames>
    <Scripts><OnLoad inherit="append">LoadLog[#LoadLog + 1] = 'own'</OnLoad></Scripts>
  </Frame>
</Ui>"#,
            ),
            (
                "AddOns/Blizzard_Lazy/Blizzard_Lazy.toc",
                "## LoadOnDemand: 1\nLazy.lua\n",
            ),
            ("AddOns/Blizzard_Lazy/Lazy.lua", "LazyLoaded = true\n"),
            (
                "AddOns/Blizzard_Glue/Blizzard_Glue.toc",
                "## AllowLoad: Glue\nGlue.lua\n",
            ),
            ("AddOns/Blizzard_Glue/Glue.lua", "GlueLoaded = true\n"),
        ],
    );
    let mut vm = vm("camelot");
    let report = wowlua_engine::ui::load(&mut vm, &interface, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.addons, ["Blizzard_Base", "Blizzard_Later"]);
    let r = run(
        &mut vm,
        "local f = MyFrame
f.Button:Click()
local made = CreateFrame('Frame', 'Made', UIParent, 'MyTemplate')
local intrinsic = CreateFrame('MyEventFrame')
return table.concat(LoadLog, ','), tostring(FamilyLoaded), tostring(LazyLoaded), tostring(GlueLoaded),
    f:GetWidth(), f.kind, f.count, f.Label:GetText(), select(2, f.Label:GetFont()), tostring(f:IsShown()),
    select(4, f:GetPoint(1)), f:Hello(), tostring(Clicked), made:GetHeight(), made.Label:GetText(),
    intrinsic:GetObjectType(), tostring(issecurevariable('MyFrame'))",
    );
    assert_eq!(
        r,
        [
            "Blizzard_Base,later,button MyFrameButton,own,mixin MyFrame,mixin Made,intrinsic",
            "true",
            "nil",
            "nil",
            "100",
            "templated",
            "3",
            "Shown text",
            "13.999999046326", // 14 as camelot keeps it
            "false",
            "10",
            "hello",
            "LeftButton",
            "40",
            "Shown text",
            "Frame",
            "true",
        ]
    );
}

/// A secure addon's API for addons, as Blizzard_AuraContainer builds it: a
/// template under `allowUntaintedCreation` is built untainted for an
/// addon (its scripts and `OnLoad` Blizzard's), and its inbound mixin's
/// `secureDelegates` run untainted for a tainted caller, unless it passes
/// a secret; `CreateSecureDelegate` elevates even then.
#[test]
fn untainted_creation_and_secure_delegates() {
    let interface = tree(
        "delegates",
        &[
            (
                "AddOns/Blizzard_Box/Blizzard_Box.toc",
                "## UseSecureEnvironment: 1\nBox.lua\nBox.xml [LoadIntoEnvironment global]\nExport.lua [LoadIntoEnvironment global]\n",
            ),
            (
                "AddOns/Blizzard_Box/Box.lua",
                "local _, addonTable = ...
local Shared = {}
function Shared:SetValue(v) self.value = v self.setSecure = issecure() end
function Shared:IsOwn() return GetForbiddenObjectTable(self) == self end
BoxInbound = { SetValue = Shared.SetValue, IsOwn = Shared.IsOwn }
BoxPrivate = { SetValue = Shared.SetValue, IsOwn = Shared.IsOwn }
function BoxPrivate:OnLoad() self.loadSecure = issecure() end
function BoxPrivate:OnUpdate() self.updateSecure = issecure() end
addonTable.Delegate = CreateSecureDelegate(function(x) return issecure(), x ~= nil end)
",
            ),
            (
                "AddOns/Blizzard_Box/Export.lua",
                "local _, addonTable = ...\nBoxDelegate = addonTable.Delegate\n",
            ),
            (
                "AddOns/Blizzard_Box/Box.xml",
                r#"<Ui>
  <ScopedModifier allowUntaintedCreation="true">
    <Frame name="BoxTemplate" virtual="true">
      <Mixins>
        <Mixin key="BoxInbound" source="secure" secureDelegates="true"/>
        <Mixin key="BoxPrivate" source="secure"/>
      </Mixins>
      <Scripts><OnLoad method="OnLoad"/><OnUpdate method="OnUpdate"/></Scripts>
    </Frame>
  </ScopedModifier>
  <Frame name="PlainBoxTemplate" virtual="true">
    <Mixins><Mixin key="BoxPrivate" source="secure"/></Mixins>
    <Scripts><OnLoad method="OnLoad"/></Scripts>
  </Frame>
</Ui>"#,
            ),
        ],
    );
    let mut vm = vm("camelot");
    let report = wowlua_engine::ui::load(&mut vm, &interface, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let r = run(
        &mut vm,
        "Box = CreateFrame('Frame', nil, UIParent, 'BoxTemplate')
local plain = CreateFrame('Frame', nil, UIParent, 'PlainBoxTemplate')
Box:SetValue({})
local set, value = tostring(Box.setSecure), tostring(issecurevariable(Box, 'value'))
local after = tostring(issecure())
Box:SetValue(secretwrap(1))
local elevated, given = BoxDelegate(secretwrap(1))
return tostring(Box.loadSecure), tostring(plain.loadSecure), set, value, after,
    tostring(Box.setSecure), tostring(elevated), tostring(given), tostring(Box:IsOwn()),
    tostring(GetForbiddenObjectTable)",
    );
    assert_eq!(
        r,
        [
            "true", "false", "true", "true", "false", "false", "true", "true", "true", "nil"
        ]
    );
    wowlua_engine::clock::advance_frames(&mut vm, 1, 16.0);
    assert_eq!(run(&mut vm, "return tostring(Box.updateSecure)"), ["true"]);
}

/// A frame whose untrusted layout scripts are forbidden (ElvUI's aura
/// containers inherit `DisableUntrustedLayoutScriptsTemplate`): an addon's
/// `SetSize` hook does not see the secrets Blizzard's layout sizes it with
/// (the user's client: no error from ElvUI's hook), but sees the addon's
/// own sizes; its tainted `OnSizeChanged` does not run, a secure one does.
#[test]
fn forbidden_untrusted_layout_scripts() {
    let interface = tree(
        "aspects",
        &[
            (
                "AddOns/Blizzard_Aspects/Blizzard_Aspects.toc",
                "Layout.lua\nTemplates.xml\n",
            ),
            (
                "AddOns/Blizzard_Aspects/Layout.lua",
                "function LayoutIt(f) f:SetSize(secretwrap(30, 40)) end\nfunction SecureSized(f) f.secureSized = true end\n",
            ),
            (
                "AddOns/Blizzard_Aspects/Templates.xml",
                r#"<Ui>
  <Frame name="DisableUntrustedLayoutScriptsTemplate" virtual="true">
    <ForbiddenAspects><ForbiddenAspect aspect="UntrustedLayoutScriptExecution"/></ForbiddenAspects>
  </Frame>
</Ui>"#,
            ),
        ],
    );
    let mut vm = vm("camelot");
    let report = wowlua_engine::ui::load(&mut vm, &interface, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    run(
        &mut vm,
        "Kept = CreateFrame('Frame', nil, UIParent, 'DisableUntrustedLayoutScriptsTemplate')
Plain = CreateFrame('Frame', nil, UIParent)
local mover = CreateFrame('Frame', nil, UIParent)
Seen, Errors = {}, {}
seterrorhandler(function(e) Errors[#Errors + 1] = e end)
for _, f in ipairs({ Kept, Plain }) do
    f:SetPoint('CENTER')
    hooksecurefunc(f, 'SetSize', function(self, w, h)
        Seen[#Seen + 1] = self == Kept and 'kept' or 'plain'
        mover:SetSize(w, h)
    end)
    f:SetScript('OnSizeChanged', function(self) Seen[#Seen + 1] = 'size' end)
    f:HookScript('OnSizeChanged', SecureSized)
end",
    );
    for name in ["Kept", "Plain"] {
        let layout = vm.get_global("LayoutIt");
        let frame = vm.get_global(name);
        vm.call_from_engine(layout, &[frame]);
    }
    wowlua_engine::clock::advance_frames(&mut vm, 1, 16.0);
    let r = run(
        &mut vm,
        "Kept:SetSize(1, 2)
return table.concat(Seen, ','), #Errors, tostring(Kept.secureSized), tostring(Plain.secureSized)",
    );
    assert_eq!(r, ["plain,size,kept", "1", "true", "true"]);
}

/// The flavor's own UI code, if `cargo xtask types` fetched it: loads, and
/// says how many errors remain.
#[test]
fn the_flavor_ui_code_loads() {
    let interface =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/types-src/camelot/Interface");
    if !interface.exists() {
        eprintln!("skipped: {} is missing", interface.display());
        return;
    }
    // The flavor's global strings, as `cargo xtask types` names them.
    let xtask = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xtask/src/types.rs"),
    )
    .unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings = interface.join(format!("../../strings/{commit}.lua"));
    let mut vm = vm("camelot");
    let start = std::time::Instant::now();
    let report = wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(strings.as_path()),
    );
    eprintln!(
        "{} addons, {} errors in {:?}",
        report.addons.len(),
        report.errors.len(),
        start.elapsed()
    );
    let limit = if std::env::var("WOWLUA_UI_ERRORS").is_ok() {
        usize::MAX
    } else {
        40
    };
    for e in report.errors.iter().take(limit) {
        eprintln!("  {e}");
    }
    // The startup set WoW Forever had (`/stdtest engine`).
    let fixture = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/client/forever-camelot-engine.lua"),
    )
    .expect("the fixture");
    let line = fixture
        .lines()
        .find(|l| l.contains("[\"probe.blizzardAtStart\"]"))
        .expect("the startup set");
    let list = line
        .split(" = \"")
        .nth(1)
        .expect("a value")
        .trim_end_matches("\",");
    let mut client: Vec<&str> = list.split(' ').collect();
    client.sort_unstable();
    let mut ours: Vec<&str> = report.addons.iter().map(String::as_str).collect();
    ours.sort_unstable();
    assert_eq!(ours, client, "the startup set");
    if let Ok(path) = std::env::var("WOWLUA_UI_ADDONS") {
        std::fs::write(path, report.addons.join("\n")).expect("writes");
    }
    assert!(!report.addons.is_empty());
}
