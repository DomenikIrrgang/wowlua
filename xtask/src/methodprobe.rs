//! `cargo xtask methodprobe-plan`: the plan of `/stdtest engine methodprobe`
//! (tests/ingame/src/methodprobe.wowlua), every widget method the client
//! has called on a fresh widget of each kind StdTest can make, into
//! `tests/ingame/src/methodprobeplan.lua`.
//!
//! The kinds ([`KINDS`]): every frame type `CreateFrame` makes but the
//! store's and the web's (Browser, Checkout) and those needing Blizzard's
//! templates; the regions a frame makes (Texture, FontString, Line,
//! MaskTexture); the animation group and each animation type; a model
//! scene's actor; a path's control point. Each call gets its own widget
//! under a hidden frame (`P`, itself under a hidden root), so nothing shows
//! and no call sees another's changes; a few kinds again with a font or an
//! owner set ([`Kind::variant_of`]), for their own methods.
//!
//! A kind's methods are the client's (`data/camelot-widget-methods.txt`,
//! the methods its metatable offers) or, for the kinds that list lacks,
//! the documentation's. A kind's own methods (those its parent kind lacks,
//! or documented by a class the parent's documentation lacks) get the full
//! matrix of the full probe ([`crate::fullprobe::matrix_with`]: no
//! arguments, the baseline, each argument through its values, the usage
//! errors; widgets, fonts, textures, script names and counts as a widget
//! takes them, [`method_pool`]); an inherited query gets its call without
//! arguments (the kind's own state), an inherited setter nothing (its
//! parent probes it). A setter's calls read the value back with its getter
//! (`-> GetValue()`; `#1` the call's first argument). The tooltip's
//! methods the documentation lacks have curated calls ([`CURATED`]).
//!
//! Methods marked `IsProtectedFunction` are called: they are refused only
//! on a protected frame in combat, and the probe's frames are its own,
//! insecure ones, called out of combat. Skipped (with the reason): the
//! documentation's restricted methods (`HasRestrictions`), the rolesets,
//! the ones acting beyond the widget ([`DENY`]: a movie, the guild tabard,
//! a file, the keyboard's input language), undocumented methods not named
//! as queries.

use anyhow::{Context, Result};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use crate::fullprobe::{Docs, Pool, base, lua_string, matrix_with, pool, push, words};

/// The plan StdTest loads.
pub const PLAN: &str = "tests/ingame/src/methodprobeplan.lua";

/// The most calls one method of a kind gets.
pub const CAP: usize = 30;

/// Calls between two reloads.
const SEGMENT: usize = 500;

/// A kind of widget the probe makes.
pub struct Kind {
    pub name: &'static str,
    /// Lua making it: a chunk given `P` (its hidden parent frame) and the
    /// makers `F`, `T`, `S`, `M`, `L`, `A` (another frame, texture, font
    /// string, mask texture, line, animation group under `P`).
    pub make: &'static str,
    /// Whether the client's method lists have it
    /// (`camelot-widget-methods.txt`); else the documentation's methods.
    pub client: bool,
    /// The documentation's classes for it (the first's bases follow).
    pub docs: &'static [&'static str],
    /// The kind it extends: its methods are probed there, but the ones
    /// this kind adds or documents anew.
    pub parent: Option<&'static str>,
    /// A kind made again with something set (a font, an owner): probed for
    /// that kind's own methods only.
    pub variant_of: Option<&'static str>,
}

const fn kind(
    name: &'static str,
    make: &'static str,
    docs: &'static [&'static str],
    parent: Option<&'static str>,
) -> Kind {
    Kind {
        name,
        make,
        client: true,
        docs,
        parent,
        variant_of: None,
    }
}

/// A kind the client's lists lack (the documentation's methods).
const fn documented(
    name: &'static str,
    make: &'static str,
    docs: &'static [&'static str],
    parent: Option<&'static str>,
) -> Kind {
    Kind {
        name,
        make,
        client: false,
        docs,
        parent,
        variant_of: None,
    }
}

const fn variant(name: &'static str, make: &'static str, of: &'static str) -> Kind {
    Kind {
        name,
        make,
        client: false,
        docs: &[],
        parent: None,
        variant_of: Some(of),
    }
}

const FRAME: Option<&str> = Some("Frame");

/// The kinds, a kind after the one it extends.
pub const KINDS: &[Kind] = &[
    kind(
        "Frame",
        "return CreateFrame(\"Frame\", nil, P)",
        &["Frame"],
        None,
    ),
    kind("Texture", "return P:CreateTexture()", &["Texture"], FRAME),
    kind(
        "FontString",
        "return P:CreateFontString()",
        &["FontString", "Font"],
        FRAME,
    ),
    kind("Line", "return P:CreateLine()", &["Line"], Some("Texture")),
    kind(
        "MaskTexture",
        "return P:CreateMaskTexture()",
        &["MaskTexture"],
        Some("Texture"),
    ),
    kind(
        "Button",
        "return CreateFrame(\"Button\", nil, P)",
        &["Button"],
        FRAME,
    ),
    kind(
        "CheckButton",
        "return CreateFrame(\"CheckButton\", nil, P)",
        &["CheckButton", "Checkbox"],
        Some("Button"),
    ),
    kind(
        "EditBox",
        "return CreateFrame(\"EditBox\", nil, P)",
        &["EditBox", "Font"],
        FRAME,
    ),
    kind(
        "Slider",
        "return CreateFrame(\"Slider\", nil, P)",
        &["Slider"],
        FRAME,
    ),
    kind(
        "StatusBar",
        "return CreateFrame(\"StatusBar\", nil, P)",
        &["StatusBar"],
        FRAME,
    ),
    kind(
        "ScrollFrame",
        "return CreateFrame(\"ScrollFrame\", nil, P)",
        &["ScrollFrame"],
        FRAME,
    ),
    kind(
        "Cooldown",
        "return CreateFrame(\"Cooldown\", nil, P)",
        &["Cooldown"],
        FRAME,
    ),
    kind(
        "ColorSelect",
        "return CreateFrame(\"ColorSelect\", nil, P)",
        &["ColorSelect"],
        FRAME,
    ),
    kind(
        "MessageFrame",
        "return CreateFrame(\"MessageFrame\", nil, P)",
        &["MessageFrame", "Font"],
        FRAME,
    ),
    kind(
        "SimpleHTML",
        "return CreateFrame(\"SimpleHTML\", nil, P)",
        &["SimpleHTML"],
        FRAME,
    ),
    kind(
        "GameTooltip",
        "return CreateFrame(\"GameTooltip\", nil, P)",
        &["GameTooltip", "Tooltip"],
        FRAME,
    ),
    kind(
        "Model",
        "return CreateFrame(\"Model\", nil, P)",
        &["Model", "ModelFFX"],
        FRAME,
    ),
    kind(
        "PlayerModel",
        "return CreateFrame(\"PlayerModel\", nil, P)",
        &["PlayerModel"],
        Some("Model"),
    ),
    kind(
        "DressUpModel",
        "return CreateFrame(\"DressUpModel\", nil, P)",
        &["DressUpModel"],
        Some("PlayerModel"),
    ),
    kind(
        "CinematicModel",
        "return CreateFrame(\"CinematicModel\", nil, P)",
        &["CinematicModel"],
        Some("PlayerModel"),
    ),
    kind(
        "TabardModel",
        "return CreateFrame(\"TabardModel\", nil, P)",
        &["TabardModel"],
        Some("PlayerModel"),
    ),
    kind(
        "ModelScene",
        "return CreateFrame(\"ModelScene\", nil, P)",
        &["ModelScene", "ModelSceneFrame"],
        FRAME,
    ),
    kind(
        "MovieFrame",
        "return CreateFrame(\"MovieFrame\", nil, P)",
        &["MovieFrame", "Movie"],
        FRAME,
    ),
    kind(
        "OffScreenFrame",
        "return CreateFrame(\"OffScreenFrame\", nil, P)",
        &["OffScreenFrame", "Frame"],
        FRAME,
    ),
    kind(
        "FogOfWarFrame",
        "return CreateFrame(\"FogOfWarFrame\", nil, P)",
        &["FogOfWarFrame"],
        FRAME,
    ),
    kind(
        "UnitPositionFrame",
        "return CreateFrame(\"UnitPositionFrame\", nil, P)",
        &["UnitPositionFrame"],
        FRAME,
    ),
    kind(
        "QuestPOIFrame",
        "return CreateFrame(\"QuestPOIFrame\", nil, P)",
        &["QuestPOIFrame", "QuestPOI"],
        FRAME,
    ),
    kind(
        "ScenarioPOIFrame",
        "return CreateFrame(\"ScenarioPOIFrame\", nil, P)",
        &["ScenarioPOIFrame", "ScenarioPOI"],
        Some("QuestPOIFrame"),
    ),
    kind(
        "ArchaeologyDigSiteFrame",
        "return CreateFrame(\"ArchaeologyDigSiteFrame\", nil, P)",
        &["ArchaeologyDigSiteFrame"],
        Some("QuestPOIFrame"),
    ),
    // Blizzard's intrinsic types: their mixins' methods are on the
    // widget, not in its metatable (the client's lists have the base
    // type's only).
    kind(
        "ScrollingMessageFrame",
        "return CreateFrame(\"ScrollingMessageFrame\", nil, P)",
        &["Frame"],
        FRAME,
    ),
    kind(
        "EventFrame",
        "return CreateFrame(\"EventFrame\", nil, P)",
        &["Frame"],
        FRAME,
    ),
    kind(
        "DropdownButton",
        "return CreateFrame(\"DropdownButton\", nil, P)",
        &["Button"],
        Some("Button"),
    ),
    kind(
        "ContainedAlertFrame",
        "return CreateFrame(\"ContainedAlertFrame\", nil, P)",
        &["Button"],
        Some("Button"),
    ),
    documented(
        "AnimationGroup",
        "return P:CreateAnimationGroup()",
        &["AnimationGroup"],
        None,
    ),
    documented(
        "Animation",
        "return P:CreateAnimationGroup():CreateAnimation(\"Animation\")",
        &["Animation"],
        None,
    ),
    documented(
        "Alpha",
        "return P:CreateAnimationGroup():CreateAnimation(\"Alpha\")",
        &["Alpha"],
        Some("Animation"),
    ),
    documented(
        "Rotation",
        "return P:CreateAnimationGroup():CreateAnimation(\"Rotation\")",
        &["Rotation"],
        Some("Animation"),
    ),
    documented(
        "Scale",
        "return P:CreateAnimationGroup():CreateAnimation(\"Scale\")",
        &["Scale"],
        Some("Animation"),
    ),
    documented(
        "LineScale",
        "return P:CreateAnimationGroup():CreateAnimation(\"LineScale\")",
        &["LineScale"],
        Some("Scale"),
    ),
    documented(
        "Translation",
        "return P:CreateAnimationGroup():CreateAnimation(\"Translation\")",
        &["Translation"],
        Some("Animation"),
    ),
    documented(
        "LineTranslation",
        "return P:CreateAnimationGroup():CreateAnimation(\"LineTranslation\")",
        &["LineTranslation"],
        Some("Translation"),
    ),
    documented(
        "Path",
        "return P:CreateAnimationGroup():CreateAnimation(\"Path\")",
        &["Path"],
        Some("Animation"),
    ),
    documented(
        "ControlPoint",
        "return P:CreateAnimationGroup():CreateAnimation(\"Path\"):CreateControlPoint()",
        &["ControlPoint"],
        None,
    ),
    documented(
        "FlipBook",
        "return P:CreateAnimationGroup():CreateAnimation(\"FlipBook\")",
        &["FlipBook"],
        Some("Animation"),
    ),
    documented(
        "TextureCoordTranslation",
        "return P:CreateAnimationGroup():CreateAnimation(\"TextureCoordTranslation\")",
        &["TextureCoordTranslation"],
        Some("Animation"),
    ),
    documented(
        "VertexColor",
        "return P:CreateAnimationGroup():CreateAnimation(\"VertexColor\")",
        &["VertexColor"],
        Some("Animation"),
    ),
    documented(
        "ModelSceneActor",
        "return CreateFrame(\"ModelScene\", nil, P):CreateActor()",
        &[
            "ModelSceneActor",
            "ModelSceneFrameActor",
            "ModelSceneFrameActorBase",
        ],
        None,
    ),
    variant(
        "FontStringWithFont",
        "local w = P:CreateFontString() w:SetFontObject(GameFontNormal) return w",
        "FontString",
    ),
    variant(
        "EditBoxWithFont",
        "local w = CreateFrame(\"EditBox\", nil, P) w:SetFontObject(GameFontNormal) return w",
        "EditBox",
    ),
    variant(
        "ButtonWithFont",
        "local w = CreateFrame(\"Button\", nil, P) w:SetNormalFontObject(GameFontNormal) return w",
        "Button",
    ),
    variant(
        "MessageFrameWithFont",
        "local w = CreateFrame(\"MessageFrame\", nil, P) w:SetFontObject(GameFontNormal) return w",
        "MessageFrame",
    ),
    variant(
        "SimpleHTMLWithFont",
        "local w = CreateFrame(\"SimpleHTML\", nil, P) w:SetFontObject(GameFontNormal) return w",
        "SimpleHTML",
    ),
    variant(
        "GameTooltipOwned",
        "local w = CreateFrame(\"GameTooltip\", nil, P) w:SetOwner(P, \"ANCHOR_NONE\") return w",
        "GameTooltip",
    ),
];

/// Widget types StdTest does not make (with the reason).
pub const NOT_MADE: &[(&str, &str)] = &[
    (
        "Browser",
        "the in-game web browser (it could load pages from the web)",
    ),
    ("Checkout", "the in-game store's checkout"),
    (
        "AuraContainer",
        "needs Blizzard's template and its addon loaded",
    ),
    ("Minimap", "there is one minimap, Blizzard's"),
    ("Font", "CreateFont makes a named global"),
];

/// Methods never called, on any kind (with the reason).
const DENY: &[(&str, &str)] = &[
    (
        "AddRoleset",
        "the frame's rolesets (protected; what they unlock is not known)",
    ),
    (
        "RemoveRoleset",
        "the frame's rolesets (protected; what they unlock is not known)",
    ),
    (
        "SetRolesets",
        "the frame's rolesets (protected; what they unlock is not known)",
    ),
    ("StartMovie", "plays a movie with its sound"),
    ("StartMovieByName", "plays a movie with its sound"),
    ("Save", "acts: saves the guild tabard"),
    ("TestPrintToFile", "writes a file"),
    ("TakeSnapshot", "renders off screen (untested GPU work)"),
    ("ApplySnapshot", "renders off screen (untested GPU work)"),
    ("Flush", "renders off screen (untested GPU work)"),
    (
        "ToggleInputLanguage",
        "changes the keyboard's input language (the client's, not the widget's)",
    ),
    (
        "ResetInputMode",
        "changes the keyboard's input mode (the client's, not the widget's)",
    ),
    ("SetFrameStack", "the frame stack tooltip, a debugging tool"),
];

/// Undocumented methods' calls (kind, method, read-back, argument lists):
/// the tooltip's, which the documentation lacks.
const CURATED: &[(&str, &str, &str, &[&str])] = &[
    (
        "GameTooltip",
        "SetOwner",
        "GetOwner()",
        &[
            "",
            "F()",
            "F(), \"ANCHOR_NONE\"",
            "F(), \"ANCHOR_RIGHT\"",
            "F(), \"ANCHOR_TOPLEFT\", 10, -10",
            "F(), \"ANCHOR_CURSOR\"",
            "F(), \"ANCHOR_PRESERVE\"",
            "F(), \"x\"",
            "F(), {}",
            "\"x\"",
            "nil",
        ],
    ),
    (
        "GameTooltip",
        "AddLine",
        "NumLines()",
        &[
            "",
            "\"Hello\"",
            "\"Hello\", 1, 0.5, 0",
            "\"Hello\", 1, 0.5, 0, true",
            "\"Hello\", 1, 0.5, 0, 1, true",
            "\"|cffff0000Red|r\"",
            "\"\"",
            "1",
            "{}",
            "nil",
        ],
    ),
    (
        "GameTooltip",
        "AddDoubleLine",
        "NumLines()",
        &[
            "",
            "\"Left\", \"Right\"",
            "\"Left\", \"Right\", 1, 0, 0, 0, 1, 0",
            "\"Left\"",
            "\"Left\", nil",
            "{}, \"Right\"",
        ],
    ),
    ("GameTooltip", "ClearLines", "NumLines()", &[""]),
    (
        "GameTooltip",
        "SetText",
        "NumLines()",
        &[
            "",
            "\"Hello\"",
            "\"Hello\", 1, 0.5, 0",
            "\"Hello\", 1, 0.5, 0, 1, true",
            "\"\"",
            "1",
            "{}",
        ],
    ),
    (
        "GameTooltip",
        "AppendText",
        "NumLines()",
        &["", "\"more\"", "{}"],
    ),
    (
        "GameTooltip",
        "AddTexture",
        "NumLines()",
        &[
            "",
            "134400",
            "\"Interface\\\\Icons\\\\INV_Misc_QuestionMark\"",
            "134400, { width = 16, height = 16 }",
            "\"x\"",
            "{}",
        ],
    ),
    (
        "GameTooltip",
        "AddAtlas",
        "NumLines()",
        &[
            "",
            "\"common-icon-checkmark\"",
            "\"common-icon-checkmark\", { width = 16, height = 16 }",
            "\"NoSuchAtlas\"",
            "{}",
        ],
    ),
    (
        "GameTooltip",
        "SetAnchorType",
        "GetAnchorType()",
        &[
            "",
            "\"ANCHOR_NONE\"",
            "\"ANCHOR_RIGHT\", 5, 5",
            "\"ANCHOR_CURSOR\"",
            "\"x\"",
            "{}",
        ],
    ),
    (
        "GameTooltip",
        "SetCustomLineSpacing",
        "GetCustomLineSpacing()",
        &["", "2", "0", "-1", "\"x\""],
    ),
    (
        "GameTooltip",
        "SetCustomWordWrapMinWidth",
        "",
        &["", "100", "0", "\"x\""],
    ),
    (
        "GameTooltip",
        "SetAllowShowWithNoLines",
        "",
        &["", "true", "false"],
    ),
    (
        "GameTooltip",
        "SetShrinkToFitWrapped",
        "",
        &["", "true", "false"],
    ),
    ("GameTooltip", "FadeOut", "IsShown()", &[""]),
];

/// Words a query's name starts with.
const QUERY: &[&str] = &[
    "Get",
    "Is",
    "Has",
    "Can",
    "Should",
    "Are",
    "Does",
    "Was",
    "Were",
    "Find",
    "Num",
    "Calculate",
];

/// Read-backs not named after their setter.
const READ_BACK: &[(&str, &str)] = &[
    ("Show", "IsShown"),
    ("Hide", "IsShown"),
    ("Enable", "IsEnabled"),
    ("Disable", "IsEnabled"),
    ("ClearAllPoints", "GetNumPoints"),
    ("ClearPoint", "GetNumPoints"),
    ("SetAllPoints", "GetNumPoints"),
    ("SetFormattedText", "GetText"),
    ("Insert", "GetText"),
    ("SetIgnoreParentAlpha", "IsIgnoringParentAlpha"),
    ("SetIgnoreParentScale", "IsIgnoringParentScale"),
    ("AddMessage", "GetNumMessages"),
    ("AddLine", "NumLines"),
    ("AddDoubleLine", "NumLines"),
    ("ClearLines", "NumLines"),
];

/// A call: its argument list and its read-back (or none).
pub type Call = (String, Option<String>);

/// A probed method's calls in one pass of the plan.
type Part<'a> = (&'a Probed, Vec<&'a Call>);

/// A probed method of a kind: its calls.
pub struct Probed {
    pub kind: String,
    pub method: String,
    pub calls: Vec<Call>,
}

/// The plan.
pub struct Plan {
    pub probed: Vec<Probed>,
    /// `Kind:Method` (or a kind) and why it is not called.
    pub skipped: Vec<(String, String)>,
}

/// The client's method lists, by widget type.
fn client_methods(root: &Path) -> Result<HashMap<String, BTreeSet<String>>> {
    let path = root.join("crates/wowlua-engine/data/camelot-widget-methods.txt");
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    Ok(text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(t, names)| {
            (
                t.to_string(),
                names.split_whitespace().map(str::to_string).collect(),
            )
        })
        .collect())
}

/// A kind's documentation classes: its own, then their bases (breadth
/// first).
fn doc_chain(docs: &Docs, kind: &Kind) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut queue: std::collections::VecDeque<String> =
        kind.docs.iter().map(|s| (*s).to_string()).collect();
    while let Some(c) = queue.pop_front() {
        if out.contains(&c) {
            continue;
        }
        for b in docs.bases.get(&c).into_iter().flatten() {
            queue.push_back(b.clone());
        }
        out.push(c);
    }
    out
}

/// A method's documentation for a kind: the class naming it and the
/// function; the kind's classes first, then any class.
fn method_doc<'a>(docs: &'a Docs, chain: &[String], name: &str) -> Option<(String, &'a Json)> {
    for c in chain {
        if let Some(f) = docs.methods.get(c).and_then(|m| m.get(name)) {
            return Some((c.clone(), f));
        }
    }
    docs.methods
        .iter()
        .find_map(|(c, m)| m.get(name).map(|f| (c.clone(), f)))
}

fn is_query(method: &str) -> bool {
    words(method)
        .first()
        .is_some_and(|w| QUERY.contains(&w.as_str()))
}

fn arg_name(a: &Json) -> String {
    a.get("Name")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn arguments(doc: &Json) -> Vec<&Json> {
    doc.get("Arguments")
        .and_then(Json::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn optional(a: &Json) -> bool {
    a.get("Nilable").and_then(Json::as_bool) == Some(true) || a.get("Default").is_some()
}

/// Whether a number argument counts or sizes something the client may
/// allocate for (`SetMaxLines(-1)` could ask for four billion lines):
/// no -1 and no 2^31-1 for it.
fn counts(name: &str, ty: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ty == "uiFontHeight"
        || [
            "max", "num", "count", "lines", "history", "snapshot", "letters", "bytes", "points",
            "height", "size", "level", "frames", "rows", "columns", "digits", "index",
        ]
        .iter()
        .any(|w| n.contains(w))
}

/// Whether 2^31-1 is a boundary the argument may take: a colour, an alpha,
/// an offset, a coordinate.
fn takes_huge(name: &str, ty: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["SingleColorValue", "normalizedValue"].contains(&ty)
        || ["alpha", "offset", "ofs", "color", "red", "green", "blue"]
            .iter()
            .any(|w| n.contains(w))
        || [
            "x", "y", "r", "g", "b", "a", "left", "right", "top", "bottom",
        ]
        .contains(&n.as_str())
}

/// An argument's pool for a method: the full probe's, with widgets made
/// under the hidden parent (never Blizzard's), fonts, textures and script
/// names a widget takes, and no huge or negative counts.
pub fn method_pool(docs: &Docs, method: &str, arg: &Json, others: &[String]) -> Pool {
    let name = arg.get("Name").and_then(Json::as_str).unwrap_or("");
    let ty = arg.get("Type").and_then(Json::as_str).unwrap_or("any");
    let nilable = optional(arg);
    let lname = name.to_ascii_lowercase();
    let is_string = base(docs, ty, 0) == "string";
    let mut p = Pool::default();
    let scripts: &[&str] = &[
        "\"OnShow\"",
        "\"OnEvent\"",
        "\"OnUpdate\"",
        "\"OnClick\"",
        "\"OnValueChanged\"",
        "\"OnNotAScript\"",
        "\"\"",
    ];
    let textures: &[&str] = &[
        "\"Interface\\\\Icons\\\\INV_Misc_QuestionMark\"",
        "134400",
        "\"Interface\\\\Buttons\\\\WHITE8X8\"",
        "\"\"",
        "\"NoSuchFile\"",
        "0",
    ];
    let custom: Option<&[&str]> = match ty {
        "SimpleFont" | "Font" => Some(&["GameFontNormal", "GameFontHighlightSmall"]),
        "FileAsset" | "TextureAsset" | "TextureAssetDisk" => Some(textures),
        "FontAsset" => Some(&[
            "\"Fonts\\\\FRIZQT__.TTF\"",
            "\"Fonts\\\\ARIALN.TTF\"",
            "\"NoSuchFont.ttf\"",
            "\"\"",
        ]),
        "ModelAsset" => Some(&["130738", "0", "-1"]),
        "ScriptTypeName" => Some(scripts),
        "HTMLTextType" => Some(&[
            "\"<html><body><h1>Title</h1><p>Hello</p></body></html>\"",
            "\"Hello\"",
            "\"<html><body><p>|cffff0000Red|r</p></body></html>\"",
            "\"<html>\"",
            "\"\"",
        ]),
        "LuaFunctionReference" | "luaFunction" | "function" => Some(&["function() end"]),
        _ if is_string => match lname.as_str() {
            // A name a Create method gives the new object: a global, so
            // StdTest's own, never one Blizzard's code may use.
            "name" if method.starts_with("Create") => Some(&["\"StdTestMethodProbeObject\""]),
            "templatename" | "template" => Some(&["\"StdTestNoSuchTemplate\""]),
            "objecttype" => Some(&[
                "\"Frame\"",
                "\"Button\"",
                "\"Texture\"",
                "\"Region\"",
                "\"x\"",
            ]),
            "animationtype" => Some(&[
                "\"Alpha\"",
                "\"Scale\"",
                "\"Translation\"",
                "\"Rotation\"",
                "\"Path\"",
                "\"x\"",
            ]),
            "button" => Some(&["\"LeftButton\"", "\"RightButton\"", "\"x\"", "\"\""]),
            "wrapmodehorizontal" | "wrapmodevertical" => Some(&[
                "\"CLAMP\"",
                "\"REPEAT\"",
                "\"MIRROR\"",
                "\"CLAMPTOBLACKADDITIVE\"",
                "\"x\"",
            ]),
            "filtermode" => Some(&["\"LINEAR\"", "\"TRILINEAR\"", "\"NEAREST\"", "\"x\""]),
            "textureasset" | "file" => Some(textures),
            "childkey" | "key" | "parentkey" => Some(&["\"stdtestKey\"", "\"\""]),
            "scriptname" => Some(scripts),
            "handslotname" => Some(&["\"MAINHANDSLOT\"", "\"SECONDARYHANDSLOT\"", "\"x\""]),
            "fontname" => Some(&["\"GameFontNormal\"", "\"x\""]),
            "format" => Some(&["\"|H%s|h[%s]|h\"", "\"x\""]),
            "attributename" => Some(&["\"x\"", "\"type\"", "\"\""]),
            "unittokenstring" => Some(&["\"player\"", "\"party1\"", "\"none\"", "\"x\""]),
            _ => None,
        },
        _ => None,
    };
    if let Some(values) = custom {
        for v in values {
            push(&mut p.values, *v);
        }
        p.wrong = Some(if values.iter().any(|v| v.starts_with('"')) {
            "{}".to_string()
        } else {
            "\"x\"".to_string()
        });
    } else if let Some(maker) = widget_maker(docs, ty) {
        push(&mut p.values, maker);
        if maker != "F()" {
            push(&mut p.values, "F()");
        }
        p.wrong = Some("\"x\"".into());
    } else {
        p = pool(docs, method, arg, others);
        for v in &mut p.values {
            if v == "UIParent" {
                *v = "F()".into();
            }
        }
        if base(docs, ty, 0) == "number" {
            let counted = counts(name, ty);
            let huge = takes_huge(name, ty);
            p.values
                .retain(|v| !(v == "2147483647" && (counted || !huge) || v == "-1" && counted));
            if counted {
                push(&mut p.values, "10");
            } else {
                push(&mut p.values, "100");
            }
        } else if is_string && lname.contains("font") {
            p.values.insert(0, "\"Fonts\\\\FRIZQT__.TTF\"".into());
        }
    }
    if nilable && !p.values.iter().any(|v| v == "nil") {
        push(&mut p.values, "nil");
    }
    p
}

/// The Lua making a widget of the type an argument takes, under the hidden
/// parent (through the declarations' aliases: `SimpleFrame` → `Frame`).
fn widget_maker(docs: &Docs, ty: &str) -> Option<&'static str> {
    let mut name = ty.to_string();
    for _ in 0..6 {
        let made = match name.as_str() {
            "AnimationGroup" => Some("A()"),
            "Path" => Some("A():CreateAnimation(\"Path\")"),
            "Animation"
            | "Alpha"
            | "Rotation"
            | "Scale"
            | "Translation"
            | "FlipBook"
            | "VertexColor"
            | "TextureCoordTranslation"
            | "LineScale"
            | "LineTranslation" => Some("A():CreateAnimation(\"Alpha\")"),
            "MaskTexture" => Some("M()"),
            "Texture" | "TextureBase" => Some("T()"),
            "FontString" => Some("S()"),
            "Line" => Some("L()"),
            "ModelSceneFrameActor" | "ModelSceneActor" => {
                Some("CreateFrame(\"ModelScene\", nil, P):CreateActor()")
            }
            "Frame" | "Region" | "ScriptRegion" | "Object" | "ScriptObject" | "Button"
            | "CheckButton" | "FrameScriptObject" => Some("F()"),
            _ => None,
        };
        if made.is_some() {
            return made;
        }
        match docs.aliases.get(&name) {
            Some(a) if !a.contains('"') && !a.contains('|') => name = a.trim().to_string(),
            _ => break,
        }
    }
    (base(docs, ty, 0) == "widget").then_some("F()")
}

/// A setter's read-back: the getter and the setter's arguments it takes
/// (`GetAttribute(#1)` for `SetAttribute(name, value)`).
fn read_back(
    docs: &Docs,
    chain: &[String],
    methods: &BTreeSet<String>,
    method: &str,
    doc: Option<&Json>,
) -> Option<String> {
    let mut names: Vec<String> = READ_BACK
        .iter()
        .filter(|(s, _)| *s == method)
        .map(|(_, g)| (*g).to_string())
        .collect();
    let rest = method
        .strip_prefix("Set")
        .or_else(|| method.strip_prefix("Clear"));
    if let Some(rest) = rest.filter(|r| !r.is_empty()) {
        for prefix in ["Get", "Is", "Has", ""] {
            names.push(format!("{prefix}{rest}"));
        }
    }
    if let Some(rest) = method.strip_prefix("Enable").filter(|r| !r.is_empty()) {
        names.push(format!("Is{rest}Enabled"));
    }
    let setter_args: Vec<String> = doc
        .map(arguments)
        .unwrap_or_default()
        .iter()
        .map(|a| arg_name(a))
        .collect();
    for getter in names {
        if getter == method || !methods.contains(&getter) {
            continue;
        }
        let required: Vec<String> = match method_doc(docs, chain, &getter) {
            Some((_, g)) => arguments(g)
                .into_iter()
                .filter(|a| !optional(a))
                .map(arg_name)
                .collect(),
            None => Vec::new(),
        };
        if required.is_empty() {
            return Some(format!("{getter}()"));
        }
        if required.len() <= setter_args.len()
            && required.iter().zip(&setter_args).all(|(g, s)| g == s)
        {
            let refs: Vec<String> = (1..=required.len()).map(|i| format!("#{i}")).collect();
            return Some(format!("{getter}({})", refs.join(", ")));
        }
    }
    None
}

impl Plan {
    pub fn build(root: &Path) -> Result<Plan> {
        let docs = Docs::load(root)?;
        let client = client_methods(root)?;
        Ok(Plan::from_parts(&docs, &client))
    }

    pub fn from_parts(docs: &Docs, client: &HashMap<String, BTreeSet<String>>) -> Plan {
        let mut probed = Vec::new();
        let mut skipped: Vec<(String, String)> = NOT_MADE
            .iter()
            .map(|(k, why)| ((*k).to_string(), format!("not made: {why}")))
            .collect();
        // Each kind's documentation classes and methods.
        let mut lists: HashMap<&str, (Vec<String>, BTreeSet<String>)> = HashMap::new();
        for k in KINDS.iter().filter(|k| k.variant_of.is_none()) {
            let chain = doc_chain(docs, k);
            let methods: BTreeSet<String> = match client.get(k.name).filter(|_| k.client) {
                Some(list) => list.clone(),
                None => chain
                    .iter()
                    .flat_map(|c| docs.methods.get(c).into_iter().flatten())
                    .map(|(n, _)| n.clone())
                    .collect(),
            };
            lists.insert(k.name, (chain, methods));
        }
        let empty = (Vec::new(), BTreeSet::new());
        let mut own: HashMap<&str, Vec<String>> = HashMap::new();
        for k in KINDS.iter().filter(|k| k.variant_of.is_none()) {
            let (chain, methods) = &lists[k.name];
            let (parent_chain, parent_methods) =
                k.parent.and_then(|p| lists.get(p)).unwrap_or(&empty);
            for m in methods {
                let label = format!("{}:{m}", k.name);
                let found = method_doc(docs, chain, m);
                let in_chain = found.as_ref().is_some_and(|(c, _)| chain.contains(c));
                let doc = found.as_ref().map(|(_, d)| *d);
                let flag =
                    |key: &str| doc.and_then(|d| d.get(key)).and_then(Json::as_bool) == Some(true);
                // Its own: new on this kind, or documented by a class its
                // parent lacks (`Slider:Enable`).
                let anew = !parent_methods.contains(m)
                    || in_chain
                        && found
                            .as_ref()
                            .is_some_and(|(c, _)| !parent_chain.contains(c));
                if !anew {
                    if is_query(m) && !flag("HasRestrictions") {
                        probed.push(Probed {
                            kind: k.name.to_string(),
                            method: m.clone(),
                            calls: vec![(String::new(), None)],
                        });
                    }
                    continue;
                }
                if let Some((_, why)) = DENY.iter().find(|(n, _)| n == m) {
                    skipped.push((label, (*why).to_string()));
                    continue;
                }
                if flag("HasRestrictions") {
                    skipped.push((label, "restricted (HasRestrictions)".into()));
                    continue;
                }
                let curated = CURATED
                    .iter()
                    .find(|(kn, mn, _, _)| *kn == k.name && mn == m);
                let calls: Vec<Call> = match (curated, doc) {
                    (Some((_, _, rb, list)), _) => {
                        let rb = (!rb.is_empty()).then(|| (*rb).to_string());
                        list.iter()
                            .map(|a| ((*a).to_string(), rb.clone()))
                            .collect()
                    }
                    (None, Some(d)) => {
                        let rb = if is_query(m) {
                            None
                        } else {
                            read_back(docs, chain, methods, m, Some(d))
                        };
                        matrix_with(d, false, CAP, &|arg, names| {
                            method_pool(docs, m, arg, names)
                        })
                        .into_iter()
                        .map(|a| (a, rb.clone()))
                        .collect()
                    }
                    (None, None) if is_query(m) => ["", "1", "\"x\""]
                        .iter()
                        .map(|a| ((*a).to_string(), None))
                        .collect(),
                    (None, None) => {
                        skipped.push((label, "undocumented and not named as a query".into()));
                        continue;
                    }
                };
                own.entry(k.name).or_default().push(m.clone());
                probed.push(Probed {
                    kind: k.name.to_string(),
                    method: m.clone(),
                    calls,
                });
            }
        }
        // The variants: their kind's own methods again.
        for v in KINDS.iter() {
            let Some(of) = v.variant_of else {
                continue;
            };
            let again: Vec<Probed> = probed
                .iter()
                .filter(|p| p.kind == of && own.get(of).is_some_and(|o| o.contains(&p.method)))
                .map(|p| Probed {
                    kind: v.name.to_string(),
                    method: p.method.clone(),
                    calls: p.calls.clone(),
                })
                .collect();
            probed.extend(again);
        }
        Plan { probed, skipped }
    }

    pub fn call_count(&self) -> usize {
        self.probed.iter().map(|p| p.calls.len()).sum()
    }

    /// The plan as StdTest's module `/methodprobeplan`.
    pub fn lua(&self) -> String {
        let boundary = regex::Regex::new(r"(^|[^0-9.\w#])(-1|2147483647|1\.5)($|[^0-9.])")
            .expect("a valid pattern");
        let mut out = String::new();
        let kinds: BTreeSet<&str> = self.probed.iter().map(|p| p.kind.as_str()).collect();
        let _ = writeln!(
            out,
            "-- @generated by `cargo xtask methodprobe-plan` (xtask/src/methodprobe.rs). Do not edit.\n\
             -- The plan of `/stdtest engine methodprobe` (methodprobe.wowlua): {} methods of {} kinds\n\
             -- of widget called {} times in all, each call on a fresh widget under a hidden\n\
             -- frame; {} methods or kinds skipped.\n\
             -- kinds: {{ name, the Lua making one (given P, its hidden parent, and the makers\n\
             -- F, T, S, M, L, A) }}. calls: {{ kind, method, arguments, read-back, … }} (Lua\n\
             -- sources; a read-back \"\" for none, `#1` the call's first argument).",
            self.probed.len(),
            kinds.len(),
            self.call_count(),
            self.skipped.len()
        );
        out.push_str("local _, ns = ...\n\nlocal kinds = {\n");
        for k in KINDS {
            let _ = writeln!(
                out,
                "    {{ {}, {} }},",
                lua_string(k.name),
                lua_string(k.make)
            );
        }
        // Safest first, so a short run gets the most: every method's call
        // without arguments and its baseline, then the other calls; the
        // boundary numbers last (`risky`).
        let mut passes: [Vec<Part>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for p in &self.probed {
            let mut parts: [Vec<&Call>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for (i, c) in p.calls.iter().enumerate() {
                let pass = if i < 2 {
                    0
                } else if boundary.is_match(&c.0) {
                    2
                } else {
                    1
                };
                parts[pass].push(c);
            }
            for (pass, part) in passes.iter_mut().zip(parts) {
                if !part.is_empty() {
                    pass.push((p, part));
                }
            }
        }
        let write_pass = |out: &mut String, pass: &[Part]| {
            for (p, calls) in pass {
                let _ = write!(
                    out,
                    "    {{ {}, {}",
                    lua_string(&p.kind),
                    lua_string(&p.method)
                );
                for (args, rb) in calls {
                    let _ = write!(
                        out,
                        ", {}, {}",
                        lua_string(args),
                        lua_string(rb.as_deref().unwrap_or(""))
                    );
                }
                out.push_str(" },\n");
            }
        };
        out.push_str("}\n\nlocal calls = {\n");
        write_pass(&mut out, &passes[0]);
        write_pass(&mut out, &passes[1]);
        out.push_str("}\n\nlocal risky = {\n");
        write_pass(&mut out, &passes[2]);
        out.push_str("}\n\nlocal skipped = {\n");
        for (name, why) in &self.skipped {
            let _ = writeln!(out, "    {{ {}, {} }},", lua_string(name), lua_string(why));
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in out.bytes() {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        let _ = write!(
            out,
            "}}\n\nns.__wowlua.define(\"methodprobeplan\", {{ kinds = kinds, calls = calls, risky = risky, skipped = skipped, segmentSize = {SEGMENT}, version = \"{hash:016x}\" }})\n"
        );
        out
    }
}

/// `cargo xtask methodprobe-plan [--stats]`.
pub fn generate(root: &Path) -> Result<String> {
    Ok(Plan::build(root)?.lua())
}

pub fn stats(root: &Path) -> Result<()> {
    let plan = Plan::build(root)?;
    println!(
        "{} methods probed, {} calls; {} skipped",
        plan.probed.len(),
        plan.call_count(),
        plan.skipped.len()
    );
    let mut by_kind: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for p in &plan.probed {
        let e = by_kind.entry(&p.kind).or_default();
        e.0 += 1;
        e.1 += p.calls.len();
    }
    for (k, (m, c)) in by_kind {
        println!("  {k:28} {m:4} methods {c:6} calls");
    }
    let mut by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, why) in &plan.skipped {
        by.entry(why).or_default().push(name);
    }
    for (why, names) in by {
        println!("skipped, {why}: {}", names.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
    }

    fn plan() -> Plan {
        Plan::build(root()).unwrap()
    }

    fn calls<'a>(plan: &'a Plan, kind: &str, method: &str) -> &'a [Call] {
        plan.probed
            .iter()
            .find(|p| p.kind == kind && p.method == method)
            .map(|p| p.calls.as_slice())
            .unwrap_or_else(|| panic!("{kind}:{method} is not probed"))
    }

    /// A kind's own method gets the full matrix and its read-back; a kind
    /// extending it its call without arguments if it is a query.
    #[test]
    fn methods_are_probed_on_the_kind_adding_them() {
        let plan = plan();
        let set = calls(&plan, "Frame", "SetFrameStrata");
        assert_eq!(set[0], (String::new(), Some("GetFrameStrata()".into())));
        assert!(set.iter().any(|(a, _)| a == "\"DIALOG\""), "{set:?}");
        assert!(set.iter().any(|(a, _)| a == "{}"), "a usage error: {set:?}");
        let value = calls(&plan, "Slider", "SetValue");
        assert_eq!(value[0].1.as_deref(), Some("GetValue()"));
        let attribute = calls(&plan, "Frame", "SetAttribute");
        assert_eq!(attribute[1].1.as_deref(), Some("GetAttribute(#1)"));
        // Inherited: queries without arguments only.
        assert_eq!(
            calls(&plan, "Button", "GetFrameStrata"),
            [(String::new(), None)]
        );
        assert!(
            !plan
                .probed
                .iter()
                .any(|p| p.kind == "Button" && p.method == "SetFrameStrata")
        );
        // A variant repeats its base kind's own methods.
        assert_eq!(
            calls(&plan, "FontStringWithFont", "SetText").len(),
            calls(&plan, "FontString", "SetText").len()
        );
        // The animations come from the documentation.
        calls(&plan, "Alpha", "SetFromAlpha");
        calls(&plan, "AnimationGroup", "Play");
    }

    /// No call names Blizzard's frames, a numeric string, or a huge or
    /// negative count; the restricted and acting methods are skipped.
    #[test]
    fn the_plan_is_safe() {
        let plan = plan();
        let numeric = regex::Regex::new(r#"(^|[(,{=] ?)"-?[0-9.]+""#).unwrap();
        for p in &plan.probed {
            assert!(p.calls.len() <= CAP, "{}:{}", p.kind, p.method);
            for (args, _) in &p.calls {
                assert!(
                    !args.contains("UIParent"),
                    "{}:{}({args})",
                    p.kind,
                    p.method
                );
                assert!(!numeric.is_match(args), "{}:{}({args})", p.kind, p.method);
            }
        }
        for never in [
            "SetPropagateKeyboardInput",
            "AddRoleset",
            "SetForbidden",
            "StartMovie",
            "AddSecretAspect",
        ] {
            assert!(
                !plan.probed.iter().any(|p| p.method == never),
                "{never} is called"
            );
        }
        for (kind, _) in NOT_MADE {
            assert!(!plan.probed.iter().any(|p| p.kind == *kind), "{kind}");
        }
        for (kind, method) in [
            ("FontString", "SetMaxLines"),
            ("EditBox", "SetHistoryLines"),
            ("EditBox", "SetMaxLetters"),
            ("FontString", "SetTextHeight"),
        ] {
            let list = calls(&plan, kind, method);
            assert!(
                !list
                    .iter()
                    .any(|(a, _)| a.split(", ").any(|v| v == "-1" || v == "2147483647")),
                "{kind}:{method}: {list:?}"
            );
        }
        // Widgets made under the hidden parent, a Create method's name
        // StdTest's own.
        assert!(
            calls(&plan, "Frame", "SetParent")
                .iter()
                .any(|(a, _)| a == "F()")
        );
        assert!(
            calls(&plan, "Frame", "CreateTexture")
                .iter()
                .all(|(a, _)| !a.starts_with("\"x\""))
        );
        assert!(plan.call_count() > 5000, "{}", plan.call_count());
    }
}
