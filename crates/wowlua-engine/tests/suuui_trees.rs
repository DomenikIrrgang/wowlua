//! The frame trees of Blizzard's windows that SuuUI's tree probe recorded
//! in WoW Forever 1.60.1 (70450), against the engine's: sizes, anchors,
//! shown state, layers, vertex colors, fonts, levels.
//!
//! The probe (`/suu probe tree`, SuuUI's Diagnostics/TreeProbe.lua) walks
//! a window: each frame, its regions, then its children, a line each. The
//! engine runs the same walk on the same windows; the lines are matched by
//! their path (type and name or key, counted among siblings) and compared
//! field by field. The client's texts are left out of the fixtures (they
//! held the character's name). The player's state differs (gear, spells,
//! reputations), so not every line can match; the counts below may only go
//! up.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

/// The engine with Blizzard's UI loaded and logged in, if `cargo xtask
/// types` fetched the UI code.
fn ui_vm() -> Option<Vm> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let interface = repo.join("target/types-src/camelot/Interface");
    if !interface.exists() {
        eprintln!("skipped: {} is missing", interface.display());
        return None;
    }
    let xtask = std::fs::read_to_string(repo.join("xtask/src/types.rs")).unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings = repo.join(format!("target/types-src/strings/{commit}.lua"));
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let report = wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(strings.as_path()),
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    wowlua_engine::controls::install(&mut vm, "mock");
    wowlua_engine::login::login(&mut vm, "MyAddon", false, false);
    wowlua_engine::clock::advance_frames(&mut vm, 60, 1000.0 / 60.0);
    Some(vm)
}

fn run(vm: &mut Vm, src: &str) -> String {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    let text = |vm: &Vm, v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// SuuUI's TreeProbe.lua walk, its texts left out as in the fixtures.
const WALK: &str = r#"
local isSecret = issecretvalue or function() return false end
local STATES = { "Normal", "Pushed", "Disabled", "Highlight", "Checked", "DisabledChecked" }
local function show(v)
    if isSecret(v) then return "secret" end
    if v == nil then return "nil" end
    if type(v) == "number" then return (v == math.floor(v)) and tostring(v) or ("%.1f"):format(v) end
    return tostring(v)
end
local function ask(f, method, ...)
    if type(f[method]) ~= "function" then return nil end
    local res = { pcall(f[method], f, ...) }
    if res[1] then return unpack(res, 2) end
end
local function keyOf(f)
    local parent = ask(f, "GetParent")
    if parent then
        for k, v in pairs(parent) do
            if v == f and type(k) == "string" then return k end
        end
    end
end
local function anchor(f)
    local p, rel, rp, x, y = ask(f, "GetPoint", 1)
    if not p then return "-" end
    local relName = rel and (ask(rel, "GetName") or keyOf(rel) or "?") or "nil"
    return ("%s>%s.%s(%s,%s)"):format(show(p), show(relName), show(rp), show(x), show(y))
end
local function describe(f)
    local t = ask(f, "GetObjectType") or "?"
    local w, h = ask(f, "GetSize")
    local parts = { t, (ask(f, "GetName") or keyOf(f) or "-"), ("%sx%s"):format(show(w), show(h)), "shown=" .. show(ask(f, "IsShown")) }
    if ask(f, "IsProtected") then parts[#parts + 1] = "PROTECTED" end
    parts[#parts + 1] = "at " .. anchor(f)
    if t == "Texture" or t == "MaskTexture" then
        local atlas = ask(f, "GetAtlas")
        parts[#parts + 1] = atlas and ("atlas=" .. atlas) or ("tex=" .. show(ask(f, "GetTexture")))
        local layer, sub = ask(f, "GetDrawLayer")
        parts[#parts + 1] = "layer=" .. show(layer) .. ":" .. show(sub)
        local r, g, b, a = ask(f, "GetVertexColor")
        if r then parts[#parts + 1] = ("rgba=%s,%s,%s,%s"):format(show(r), show(g), show(b), show(a)) end
        -- The engine's alone (not compared): the button state it is of.
        local parent = ask(f, "GetParent")
        for _, s in ipairs(STATES) do
            if parent and ask(parent, "Get" .. s .. "Texture") == f then
                parts[#parts + 1] = ("state=%s/%s/%s"):format(s, show(ask(parent, "IsEnabled")), show(ask(parent, "GetChecked")))
            end
        end
    elseif t == "FontString" then
        local file, size = ask(f, "GetFont")
        parts[#parts + 1] = "font=" .. show(file and file:match("[^\\/]+$")) .. " " .. show(size)
        local fo = ask(f, "GetFontObject")
        if fo then parts[#parts + 1] = "object=" .. show(ask(fo, "GetName")) end
    else
        local scripts = {}
        for _, s in ipairs({ "OnClick", "OnEnter", "OnShow" }) do
            if ask(f, "HasScript", s) and ask(f, "GetScript", s) then scripts[#scripts + 1] = s end
        end
        if #scripts > 0 then parts[#parts + 1] = "scripts=" .. table.concat(scripts, ",") end
        parts[#parts + 1] = "level=" .. show(ask(f, "GetFrameLevel"))
    end
    return table.concat(parts, " ")
end
local function walk(f, depth, maxDepth, out, seen)
    if #out > 1500 or seen[f] then return end
    seen[f] = true
    out[#out + 1] = string.rep("  ", depth) .. describe(f)
    if depth >= maxDepth then return end
    for _, r in ipairs({ ask(f, "GetRegions") }) do
        if #out > 1500 then return end
        out[#out + 1] = string.rep("  ", depth + 1) .. "· " .. describe(r)
    end
    for _, c in ipairs({ ask(f, "GetChildren") }) do
        if not ask(c, "IsForbidden") then walk(c, depth + 1, maxDepth, out, seen) end
    end
end
local name, depth = ...
local f = _G
for part in name:gmatch("[^%.]+") do f = type(f) == "table" and f[part] or nil end
if type(f) ~= "table" or not f.GetObjectType then return "missing" end
local out = { "at" }
walk(f, 0, depth, out, {})
table.remove(out, 1)
return table.concat(out, "\n")
"#;

/// One line of a tree: its path and its fields.
struct Node {
    path: String,
    fields: Vec<(&'static str, String)>,
}

/// A number as the probe shows it, rounded to a tenth.
fn num(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().map(|n| (n * 10.0).round() / 10.0)
}

/// Two field values alike: numbers within half a pixel (the client's
/// sizes are a hair off whole pixels).
fn alike(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let split = |s: &str| -> Vec<String> {
        s.split(['x', ',', '(', ')', ':', ' '])
            .map(str::to_string)
            .collect()
    };
    let (pa, pb) = (split(a), split(b));
    pa.len() == pb.len()
        && pa.iter().zip(&pb).all(|(x, y)| match (num(x), num(y)) {
            (Some(x), Some(y)) => (x - y).abs() <= 0.51,
            _ => x == y,
        })
}

/// The lines of a tree with their paths: a line's parent is the frame
/// above it one indent less; siblings of one type and name are counted.
fn parse(text: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = (line.len() - trimmed.len()) / 2;
        let region = trimmed.starts_with("· ");
        let body = trimmed.trim_start_matches("· ");
        let mut words = body.split(' ');
        let (Some(kind), Some(name), Some(size), Some(shown)) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            continue;
        };
        while stack.last().is_some_and(|(d, _)| *d >= indent) {
            stack.pop();
        }
        let parent = stack.last().map(|(_, p)| p.as_str()).unwrap_or("");
        let base = format!("{parent}/{}{kind}:{name}", if region { "·" } else { "" });
        let n = seen.entry(base.clone()).or_default();
        *n += 1;
        let path = format!("{base}#{n}");
        let rest: Vec<&str> = words.collect();
        let mut fields = vec![
            ("size", size.to_string()),
            ("shown", shown.trim_start_matches("shown=").to_string()),
        ];
        let mut protected = "no".to_string();
        let mut i = 0;
        while i < rest.len() {
            let w = rest[i];
            if w == "PROTECTED" {
                protected = "yes".into();
            } else if w == "at" {
                i += 1;
                fields.push(("anchor", rest.get(i).unwrap_or(&"").to_string()));
            } else if let Some(v) = w.strip_prefix("font=") {
                let size = rest.get(i + 1).unwrap_or(&"");
                i += 1;
                fields.push(("font", format!("{v} {size}")));
            } else if let Some((k, v)) = w.split_once('=') {
                let key = match k {
                    "atlas" | "tex" => "texture",
                    "layer" => "layer",
                    "rgba" => "rgba",
                    "object" => "object",
                    "scripts" => "scripts",
                    "level" => "level",
                    _ => "other",
                };
                let v = if k == "atlas" {
                    format!("atlas {v}")
                } else {
                    v.to_string()
                };
                fields.push((key, v));
            }
            i += 1;
        }
        fields.push(("protected", protected));
        if !region {
            stack.push((indent, path.clone()));
        }
        nodes.push(Node { path, fields });
    }
    nodes
}

/// What differs between the client's tree and the engine's: lines the
/// engine lacks, and per field the lines that disagree.
#[derive(Default)]
struct Diff {
    lines: usize,
    missing: Vec<String>,
    fields: usize,
    agree: usize,
    by_field: HashMap<&'static str, Vec<String>>,
}

fn compare(client: &str, engine: &str, diff: &mut Diff) {
    let ours: HashMap<String, Node> = parse(engine)
        .into_iter()
        .map(|n| (n.path.clone(), n))
        .collect();
    for node in parse(client) {
        diff.lines += 1;
        let Some(mine) = ours.get(&node.path) else {
            diff.missing.push(node.path);
            continue;
        };
        for (key, value) in &node.fields {
            diff.fields += 1;
            let theirs = mine
                .fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or("(none)");
            if alike(value, theirs) {
                diff.agree += 1;
            } else {
                diff.by_field
                    .entry(key)
                    .or_default()
                    .push(format!("{}: client {value}, engine {theirs}", node.path));
            }
        }
    }
}

/// The windows as the probe found them: what was open, then each tree.
const SCENES: &[(&str, &[&str])] = &[
    // The character window open on its statistics tab (the probe's
    // `/suu probe tree character`, run on that tab).
    (
        "ToggleCharacter('PaperDollFrame') ToggleCharacter('StatisticsFrame')",
        &[
            "CharacterFrame",
            "PaperDollFrame",
            "PaperDollItemsFrame",
            "CharacterStatsPane",
            "PaperDollSidebarTabs",
            "CharacterModelScene",
            "ReputationFrame",
            "SkillsFrame",
            "PVPRankFrame",
            "TokenFrame",
            "StatisticsFrame",
        ],
    ),
    // The talents and spell book window (the probe's on its talents tab; the
    // engine's character, whose talents tab does not open, gets its spell book).
    (
        "HideUIPanel(CharacterFrame) PlayerSpellsUtil.TogglePlayerSpellsFrame()",
        &["PlayerSpellsFrame", "PlayerSpellsFrame.TabSystem"],
    ),
    // The legacy system window, its track's renown set as the legacy scenario
    // sets it.
    (
        "mock.setRenown(2802, 16) HideUIPanel(PlayerSpellsFrame) ToggleLegacySystemUI()",
        &["LegacySystemFrame"],
    ),
];

fn dump(vm: &mut Vm, name: &str, client: &str) -> String {
    let depth = client
        .lines()
        .filter(|l| !l.trim_start().starts_with('·'))
        .map(|l| (l.len() - l.trim_start().len()) / 2)
        .max()
        .unwrap_or(4);
    run(
        vm,
        &format!("return (function(...) {WALK} end)({name:?}, {depth})"),
    )
}

/// Lines the engine has of the client's, and fields alike (they may only
/// go up).
const TREE_LINES_MATCHED: usize = 4451;
const TREE_FIELDS_MATCHED: usize = 27246;

#[test]
fn blizzard_windows_lay_out_as_the_client_does() {
    let Some(mut vm) = ui_vm() else { return };
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/client/suuui-trees");
    let mut diff = Diff::default();
    let mut report = String::new();
    let out = std::env::var_os("WOWLUA_TREE_DUMP").map(PathBuf::from);
    for (setup, trees) in SCENES {
        run(&mut vm, &format!("{setup} return ''"));
        wowlua_engine::clock::advance_frames(&mut vm, 2, 1000.0 / 60.0);
        for name in *trees {
            let client = std::fs::read_to_string(dir.join(format!("{name}.txt"))).expect("fixture");
            let engine = dump(&mut vm, name, &client);
            if let Some(out) = &out {
                std::fs::create_dir_all(out).ok();
                std::fs::write(out.join(format!("{name}-engine.txt")), &engine).ok();
                std::fs::write(out.join(format!("{name}-client.txt")), &client).ok();
            }
            compare(&client, &engine, &mut diff);
        }
    }
    let matched = diff.lines - diff.missing.len();
    writeln!(
        report,
        "tree lines matched: {matched} of {}; fields alike: {} of {}",
        diff.lines, diff.agree, diff.fields
    )
    .ok();
    let mut keys: Vec<_> = diff.by_field.keys().copied().collect();
    keys.sort();
    for key in keys {
        let lines = &diff.by_field[key];
        writeln!(report, "  {key}: {} differ", lines.len()).ok();
        for l in lines.iter().take(8) {
            writeln!(report, "    {l}").ok();
        }
    }
    writeln!(report, "  missing: {}", diff.missing.len()).ok();
    for l in diff.missing.iter().take(12) {
        writeln!(report, "    {l}").ok();
    }
    if let Some(out) = &out {
        let mut all = String::new();
        for (k, v) in &diff.by_field {
            for l in v {
                writeln!(all, "{k} {l}").ok();
            }
        }
        for l in &diff.missing {
            writeln!(all, "missing {l}").ok();
        }
        std::fs::write(out.join("diffs.txt"), all).ok();
    }
    println!("{report}");
    assert!(matched >= TREE_LINES_MATCHED, "{report}");
    assert!(diff.agree >= TREE_FIELDS_MATCHED, "{report}");
}

/// The engine alone, without Blizzard's UI.
fn bare_vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

#[test]
fn a_buttons_state_textures_show_as_its_state() {
    let mut vm = bare_vm();
    let r = run(
        &mut vm,
        "local b = CreateFrame('CheckButton', nil, UIParent)
local t = {}
for _, s in ipairs({ 'Normal', 'Pushed', 'Disabled', 'Highlight', 'Checked', 'DisabledChecked' }) do
    b['Set' .. s .. 'Texture'](b, 'Interface/Buttons/WHITE8X8')
end
local function states()
    local out = {}
    for _, s in ipairs({ 'Normal', 'Pushed', 'Disabled', 'Highlight', 'Checked', 'DisabledChecked' }) do
        out[#out + 1] = b['Get' .. s .. 'Texture'](b):IsShown() and '1' or '0'
    end
    return table.concat(out)
end
local fresh = states()
b:SetChecked(true)
local checked = states()
b:Disable()
local disabled = states()
b:Enable() b:SetChecked(false) b:SetButtonState('PUSHED')
local pushed = states()
return fresh .. ' ' .. checked .. ' ' .. disabled .. ' ' .. pushed .. ' '
    .. (b:GetHighlightTexture():GetDrawLayer()) .. ' ' .. (b:GetCheckedTexture():GetDrawLayer())",
    );
    assert_eq!(r, "100100 100110 001101 010100 HIGHLIGHT OVERLAY");
}

#[test]
fn an_unsized_font_string_takes_its_texts_size() {
    let mut vm = bare_vm();
    let r = run(
        &mut vm,
        "local fs = UIParent:CreateFontString()
fs:SetFont('Fonts/FRIZQT__.TTF', 12, '')
local w0, h0 = fs:GetSize()
fs:SetText('|cffff0000Attack|r')
local w1, h1 = fs:GetSize()
fs:SetWidth(18)
local w2, h2 = fs:GetSize()
fs:SetNonSpaceWrap(true)
local w3, h3 = fs:GetSize()
local t = UIParent:CreateTexture()
t:SetPoint('LEFT', fs, 'RIGHT')
fs:SetPoint('LEFT', UIParent, 'LEFT', 10, 0)
return ('%d %d %d %d %d %d %d %d'):format(w0, h0, w1, h1, w2, h2, h3, t:GetLeft())",
    );
    // A word wider than the string is cut between its characters, its
    // height every line, with or without `SetNonSpaceWrap` (without, it is
    // truncated: the wrap probe, decision T282).
    assert_eq!(r, "1 1 42 12 18 36 36 28");
}

#[test]
fn regions_on_frames_without_a_rectangle_have_none() {
    let mut vm = bare_vm();
    let r = run(
        &mut vm,
        "local b = CreateFrame('Button', nil, UIParent)
b:SetSize(26, 26)
local t = b:CreateTexture()
t:SetPoint('TOPLEFT', -5, 5)
t:SetPoint('BOTTOMRIGHT', 5, -5)
local w, h = t:GetSize()
local before = tostring(t:GetLeft())
b:SetPoint('CENTER')
local w2 = t:GetWidth()
t:SetToDefaults()
return ('%d %d %s %d %d'):format(w, h, before, w2, t:GetNumPoints())",
    );
    assert_eq!(r, "0 0 nil 36 0");
}
