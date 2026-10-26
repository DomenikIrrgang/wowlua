//! The `wow-api` declaration generator (spec package manager §10.9):
//! Blizzard's API documentation per flavor, enriched with LuaLS
//! annotations (Ketho's vscode-wow-api), written as `.d.wowlua`.

pub mod blizzard;
pub mod docs;
pub mod framexml;
pub mod infer;
pub mod luals;
pub mod model;
pub mod wiki;

use model::Decls;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The generated declarations: one file shared by every flavor (from the
/// annotations) and one per flavor (from Blizzard's documentation).
pub struct Output {
    pub shared: String,
    pub flavors: Vec<(String, String)>,
    /// Each flavor's documentation as JSON, every key kept (the engine's
    /// data, spec emulator §17.6).
    pub api: Vec<(String, serde_json::Value)>,
}

/// An argument the client's usage check names: name, check, may be nil.
pub type UsageArg = (String, String, bool);

/// What the client itself told of its undocumented C functions (the
/// engine's recordings of one flavor, decision T294).
#[derive(Default)]
pub struct ClientFacts {
    /// The flavor recorded (`camelot`).
    pub flavor: String,
    /// The client's own argument checks (`camelot-usages.txt`): the
    /// function, then each argument's name, check and whether it may be nil.
    pub usages: Vec<(String, Vec<UsageArg>)>,
    /// The client's answers to a call without arguments (`blanks.rs`):
    /// the function and its answer (`"2: number 0, number 0"`).
    pub answers: Vec<(String, String)>,
}

/// One flavor's input: its ID, a description of the sources (for the
/// header), the documentation files, Blizzard's UI code (Lua and XML) and
/// the names of its global strings.
pub struct FlavorDocs {
    pub id: String,
    pub source: String,
    pub files: Vec<(String, String)>,
    pub ui_lua: Vec<(String, String)>,
    pub ui_xml: Vec<(String, String)>,
    pub strings: Vec<String>,
    /// The global API functions the flavor's client has (`Name`,
    /// `Namespace.Member`), when known.
    pub api_names: Option<HashSet<String>>,
    /// The events the flavor's client has, when known.
    pub event_names: Option<HashSet<String>>,
    /// The UI files only load-on-demand addons load (their globals exist
    /// only once the addon is loaded): path (lowercase, `/`) → addon.
    pub lod_files: std::collections::HashMap<String, String>,
    /// The number globals the flavor's client defines itself (`LE_*`,
    /// `NUM_LE_*`), when recorded.
    pub number_globals: Vec<String>,
}

/// Where the annotations are short: `Frame:GetAttribute` also takes
/// `(prefix, name, suffix)`, the modified lookup Blizzard's
/// `SecureTemplates.lua` uses; `Texture:GetTexture` gives the file ID (a
/// number) of a texture set by one, as the client does, not only a path.
fn widget_corrections(classes: &mut indexmap::IndexMap<String, model::Class>) {
    // What a fresh widget answers (`/stdtest engine`, the engine's
    // `camelot-widget-defaults.txt`): a font string's and a button's
    // `GetText()` nothing, an edit box's and a message frame's `GetFont()`
    // no file (a font object's alike), where the documentation has a
    // string.
    let nilable_first =
        |classes: &mut indexmap::IndexMap<String, model::Class>, class: &str, method: &str| {
            for sig in classes
                .get_mut(class)
                .and_then(|c| c.methods.get_mut(method))
                .into_iter()
                .flatten()
            {
                if sig.returns.first().map(String::as_str) == Some("string") {
                    sig.returns[0] = "string?".into();
                }
            }
        };
    for class in ["FontString", "Button", "DropdownButton"] {
        nilable_first(classes, class, "GetText");
    }
    for class in ["Font", "EditBox", "MessageFrame"] {
        nilable_first(classes, class, "GetFont");
    }
    if let Some(texture) = classes.get_mut("TextureBase")
        && let Some(sigs) = texture.methods.get_mut("GetTexture")
    {
        for sig in sigs.iter_mut() {
            if sig.returns.len() == 1 && sig.returns[0] == "string?" {
                sig.returns = vec!["string | number | nil".into()];
            }
        }
    }
    // A status bar's value and range read back secret after a secret was
    // set (Blizzard's `SecretReturnsForAspect` `BarValue`; the emulator's
    // `bars.rs`).
    if let Some(bar) = classes.get_mut("StatusBar") {
        for method in ["GetValue", "GetInterpolatedValue", "GetMinMaxValues"] {
            for sig in bar.methods.get_mut(method).into_iter().flatten() {
                for ty in sig.returns.iter_mut() {
                    if ty == "number" {
                        *ty = "number | Secret<number>".into();
                    }
                }
            }
        }
    }
    if let Some(frame) = classes.get_mut("Frame")
        && let Some(sigs) = frame.methods.get_mut("GetAttribute")
        && sigs.iter().all(|s| s.params.len() == 1)
    {
        let param = |name: &str, optional| model::ParamDecl {
            name: name.into(),
            ty: "string".into(),
            optional,
        };
        sigs.push(model::Sig {
            params: vec![
                param("prefix", false),
                param("name", false),
                param("suffix", true),
            ],
            returns: vec!["any".into()],
            ..model::Sig::default()
        });
    }
}

/// Frame types the client makes from Blizzard's Lua, not as widgets of
/// their own (`<ScrollingMessageFrame intrinsic="true"
/// mixin="ScrollingMessageFrameMixin">`): what `CreateFrame` gives for
/// them.
const INTRINSIC_FRAMES: &[(&str, &str)] = &[(
    "ScrollingMessageFrame",
    "ScrollingMessageFrameMixin & Frame",
)];

/// A parameter of a hand-typed method: name, type, optional.
type Param = (&'static str, &'static str, bool);
/// A hand-typed method: name, parameters, `...: T`, results.
type Method = (
    &'static str,
    &'static [Param],
    Option<&'static str>,
    &'static [&'static str],
);

const SECRET_TEXT: &str = "string | Secret<string>";
const SECRET_COLOR: &str = "number | Secret<number>";
const MESSAGE: &[Param] = &[
    ("message", SECRET_TEXT, false),
    ("r", SECRET_COLOR, true),
    ("g", SECRET_COLOR, true),
    ("b", SECRET_COLOR, true),
];
const RGBA: &[Param] = &[
    ("r", "number", false),
    ("g", "number", false),
    ("b", "number", false),
    ("a", "number", true),
];
const FOUR_NUMBERS: &[&str] = &["number", "number", "number", "number"];

/// The font methods of a message frame's mixin, typed as the client's
/// `MessageFrame` widget has them (decision T285).
const FONTABLE_METHODS: &[Method] = &[
    (
        "SetFontObject",
        &[("fontObject", "FontObject | nil", false)],
        None,
        &[],
    ),
    ("GetFontObject", &[], None, &["SimpleFont?"]),
    ("HasFontObject", &[], None, &["boolean"]),
    (
        "SetFont",
        &[
            ("font", "FontFile", false),
            ("fontHeight", "uiFontHeight", false),
            ("fontFlags", "TBFFlags", true),
        ],
        None,
        &[],
    ),
    (
        "GetFont",
        &[],
        None,
        &["string", "uiFontHeight", "TBFFlags"],
    ),
    ("SetTextColor", RGBA, None, &[]),
    ("GetTextColor", &[], None, FOUR_NUMBERS),
    ("SetShadowColor", RGBA, None, &[]),
    ("GetShadowColor", &[], None, FOUR_NUMBERS),
    (
        "SetShadowOffset",
        &[("offsetX", "number", false), ("offsetY", "number", false)],
        None,
        &[],
    ),
    ("GetShadowOffset", &[], None, &["number", "number"]),
    ("SetSpacing", &[("spacing", "number", false)], None, &[]),
    ("GetSpacing", &[], None, &["number"]),
    (
        "SetJustifyH",
        &[("justifyH", "JustifyHorizontal", false)],
        None,
        &[],
    ),
    ("GetJustifyH", &[], None, &["JustifyHorizontal"]),
    (
        "SetJustifyV",
        &[("justifyV", "JustifyVertical", false)],
        None,
        &[],
    ),
    ("GetJustifyV", &[], None, &["JustifyVertical"]),
    (
        "SetIndentedWordWrap",
        &[("indentWordWrap", "boolean", false)],
        None,
        &[],
    ),
    ("GetIndentedWordWrap", &[], None, &["boolean"]),
];

/// The message methods of `ScrollingMessageFrameMixin` (and of its secure
/// part): a line and its colour may be secret (decision T285).
const MESSAGE_METHODS: &[Method] = &[
    ("AddMessage", MESSAGE, Some("unknown"), &[]),
    ("BackFillMessage", MESSAGE, Some("unknown"), &[]),
    ("GetNumMessages", &[], None, &["number"]),
    (
        "GetMessageInfo",
        &[("messageIndex", "number", false)],
        None,
        &[
            "(string | Secret<string>)?",
            "(number | Secret<number>)?",
            "(number | Secret<number>)?",
            "(number | Secret<number>)?",
        ],
    ),
    (
        "SetScrollAllowed",
        &[("allowed", "boolean", false)],
        None,
        &[],
    ),
    ("IsScrollAllowed", &[], None, &["boolean"]),
    ("ScrollByAmount", &[("amount", "number", false)], None, &[]),
    ("ScrollUp", &[], None, &[]),
    ("ScrollDown", &[], None, &[]),
    ("PageUp", &[], None, &[]),
    ("PageDown", &[], None, &[]),
    ("ScrollToTop", &[], None, &[]),
    ("ScrollToBottom", &[], None, &[]),
    ("SetScrollOffset", &[("offset", "number", false)], None, &[]),
    ("GetScrollOffset", &[], None, &["number"]),
    ("GetMaxScrollRange", &[], None, &["number"]),
    ("GetNumVisibleLines", &[], None, &["number"]),
    ("AtTop", &[], None, &["boolean"]),
    ("AtBottom", &[], None, &["boolean"]),
    ("SetMaxLines", &[("maxLines", "number", false)], None, &[]),
    ("GetMaxLines", &[], None, &["number"]),
    (
        "SetFading",
        &[("shouldFadeAfterInactivity", "boolean", false)],
        None,
        &[],
    ),
    ("GetFading", &[], None, &["boolean"]),
    (
        "SetTimeVisible",
        &[("timeVisibleSecs", "number", false)],
        None,
        &[],
    ),
    ("GetTimeVisible", &[], None, &["number"]),
    (
        "SetFadeDuration",
        &[("fadeDurationSecs", "number", false)],
        None,
        &[],
    ),
    ("GetFadeDuration", &[], None, &["number"]),
    ("Clear", &[], None, &[]),
    (
        "SetTextCopyable",
        &[("textIsCopyable", "boolean", false)],
        None,
        &[],
    ),
    ("IsTextCopyable", &[], None, &["boolean"]),
    ("ResetAllFadeTimes", &[], None, &[]),
];

/// The UI code's mixins are untyped; a message frame's font and message
/// methods get the types the client gives them, where the mixin defines
/// them (decision T285).
fn mixin_corrections(name: &str, class: &mut model::Class) {
    let methods: &[Method] = match name {
        "FontableFrameMixin" => FONTABLE_METHODS,
        "ScrollingMessageFrameMixin" | "ScrollingMessageFrameSecureMixin" => MESSAGE_METHODS,
        _ => return,
    };
    for (method, params, vararg, returns) in methods {
        let Some(sigs) = class.methods.get_mut(*method) else {
            continue;
        };
        *sigs = vec![model::Sig {
            params: params
                .iter()
                .map(|(n, ty, optional)| model::ParamDecl {
                    name: (*n).into(),
                    ty: (*ty).into(),
                    optional: *optional,
                })
                .collect(),
            vararg: vararg.map(Into::into),
            returns: returns.iter().map(|r| (*r).into()).collect(),
            ..model::Sig::default()
        }];
    }
}

/// What the client passes each script handler after `self` (as the
/// emulator does, wowlua-engine's `script_params`, and the client's
/// documentation of each script).
const SCRIPT_HANDLERS: &[(&str, &str)] = &[
    ("OnLoad", ""),
    ("OnShow", ""),
    ("OnHide", ""),
    ("OnEnable", ""),
    ("OnDisable", ""),
    ("OnUpdate", "elapsed: number"),
    ("OnEvent", "event: string, ...any"),
    ("OnClick", "button: string, down: boolean"),
    ("PreClick", "button: string, down: boolean"),
    ("PostClick", "button: string, down: boolean"),
    ("OnDoubleClick", "button: string"),
    ("OnEnter", "motion: boolean"),
    ("OnLeave", "motion: boolean"),
    ("OnMouseDown", "button: string"),
    ("OnMouseUp", "button: string"),
    ("OnMouseWheel", "delta: number"),
    ("OnDragStart", "button: string"),
    ("OnDragStop", ""),
    ("OnReceiveDrag", ""),
    ("OnSizeChanged", "width: number, height: number"),
    ("OnAttributeChanged", "name: string, value: any"),
    ("OnKeyDown", "key: string"),
    ("OnKeyUp", "key: string"),
    ("OnChar", "text: string"),
    (
        "OnHyperlinkClick",
        "link: string, text: string, button: string, ...any",
    ),
    ("OnHyperlinkEnter", "link: string, text: string, ...any"),
    ("OnHyperlinkLeave", "link: string, text: string, ...any"),
    ("OnValueChanged", "value: number, userInput: boolean"),
    ("OnMinMaxChanged", "min: number, max: number"),
    ("OnTextChanged", "userInput: boolean"),
    ("OnTextSet", ""),
    ("OnEnterPressed", ""),
    ("OnEscapePressed", ""),
    ("OnTabPressed", ""),
    ("OnSpacePressed", ""),
    ("OnEditFocusGained", ""),
    ("OnEditFocusLost", ""),
    ("OnArrowPressed", "key: string"),
    (
        "OnCursorChanged",
        "x: number, y: number, width: number, height: number",
    ),
    ("OnVerticalScroll", "offset: number"),
    ("OnHorizontalScroll", "offset: number"),
    ("OnScrollRangeChanged", "xrange: number, yrange: number"),
    ("OnCooldownDone", ""),
    ("OnColorSelect", "r: number, g: number, b: number"),
    ("OnTooltipCleared", ""),
    ("OnTooltipSetDefaultAnchor", ""),
    ("OnFinished", "requested: boolean"),
    ("OnPlay", ""),
    ("OnPause", ""),
    ("OnStop", "requested: boolean"),
    ("OnLoop", "loopState: string"),
];

/// Where a class's scripts get other arguments than [`SCRIPT_HANDLERS`]
/// say: a status bar's `OnValueChanged` gets the value alone (recorded:
/// `userInput` nil; `userInput` is a slider's), and its scripts get the
/// value and range as its getters read them, secret after a secret was set
/// (Blizzard's `SecretArgumentsAddAspect` `BarValue`; the emulator's
/// `bars.rs`).
const CLASS_SCRIPT_HANDLERS: &[(&str, &str, &str)] = &[
    (
        "StatusBar",
        "OnValueChanged",
        "value: number | Secret<number>",
    ),
    (
        "StatusBar",
        "OnMinMaxChanged",
        "min: number | Secret<number>, max: number | Secret<number>",
    ),
];

/// The script names a script type alias (`ScriptButton`) takes, through
/// the aliases it extends.
fn script_names(
    ty: &str,
    aliases: &indexmap::IndexMap<String, String>,
    out: &mut BTreeSet<String>,
    depth: u32,
) {
    if depth > 8 {
        return;
    }
    for part in ty.split('|').map(str::trim) {
        if let Some(name) = part.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
            out.insert(name.to_string());
        } else if let Some(t) = aliases.get(part) {
            script_names(t, aliases, out, depth + 1);
        }
    }
}

/// `SetScript` and `HookScript` with a script the client calls with known
/// arguments: an overload typing the handler `(self: Self, …) -> ()`, so a
/// function written there gets its parameters' types (spec §2.4.3). The
/// documented form stays last, for any other script name or handler.
/// `ScriptRegion` (`ScriptType`, any name) gets `OnShow` and the scripts
/// every region has.
fn script_handlers(
    classes: &mut indexmap::IndexMap<String, model::Class>,
    aliases: &indexmap::IndexMap<String, String>,
) {
    for (class_name, class) in classes.iter_mut() {
        for method in ["SetScript", "HookScript"] {
            let Some(sigs) = class.methods.get_mut(method) else {
                continue;
            };
            let Some(first) = sigs.first().cloned() else {
                continue;
            };
            let Some(script) = first.params.first() else {
                continue;
            };
            let mut names = BTreeSet::new();
            script_names(&script.ty, aliases, &mut names, 0);
            // Every frame runs `OnUpdate` and `OnLoad` (the annotations'
            // script lists leave them out).
            if names.contains("OnShow") {
                names.extend(["OnUpdate", "OnLoad"].map(String::from));
            }
            if class_name == "ScriptRegion" {
                names.extend(
                    [
                        "OnShow",
                        "OnHide",
                        "OnEnter",
                        "OnLeave",
                        "OnMouseDown",
                        "OnMouseUp",
                        "OnSizeChanged",
                    ]
                    .map(String::from),
                );
            }
            let mut typed = Vec::new();
            for (name, args) in SCRIPT_HANDLERS {
                if !names.contains(*name) {
                    continue;
                }
                let args = CLASS_SCRIPT_HANDLERS
                    .iter()
                    .find(|(c, n, _)| c == class_name && n == name)
                    .map_or(*args, |(_, _, a)| *a);
                let params = if args.is_empty() {
                    "self: Self".to_string()
                } else {
                    format!("self: Self, {args}")
                };
                let handler = format!("(({params}) -> ())");
                let mut sig = first.clone();
                sig.params[0].ty = format!("\"{name}\"");
                if let Some(h) = sig.params.get_mut(1) {
                    h.ty = handler;
                    // SetScript takes nil (no handler); HookScript a function.
                    h.optional = method == "SetScript";
                }
                typed.push(sig);
            }
            if !typed.is_empty() {
                typed.append(sigs);
                *sigs = typed;
            }
        }
    }
}

/// Where Blizzard's documentation is wrong: `C_Timer.NewTimer` and
/// `NewTicker` return a handle with `Cancel()` and `IsCancelled()` (the
/// documentation says the callback), and their callbacks receive it.
/// `TimerHandle` and `TickerHandle` are declared by the compiler.
fn corrections(d: &mut Decls) {
    if d.classes.contains_key("NumericFormatter") {
        for name in ["NumericRuleFormatter", "AbbreviatedNumberFormatter"] {
            if let Some(c) = d.classes.get_mut(name)
                && c.bases.is_empty()
            {
                c.bases.push("NumericFormatter".into());
            }
        }
    }
    if let Some(timer) = d.namespaces.get_mut("C_Timer") {
        for (name, handle) in [("NewTimer", "TimerHandle"), ("NewTicker", "TickerHandle")] {
            for sig in timer.functions.get_mut(name).into_iter().flatten() {
                sig.returns = vec![handle.to_string()];
                if let Some(callback) = sig.params.iter_mut().find(|p| p.name == "callback") {
                    callback.ty = format!("(handle: {handle}) -> ()");
                }
            }
        }
    }
    // Blizzard_SharedTalentUI (loaded at login) replaces
    // `C_Traits.GetConditionInfo` with a wrapper taking
    // `(configID, condID, ignoreFontColor, traitTreeName)` that adds the
    // formatted `tooltipText` (`Blizzard_SharedTalentUtil.lua`; recorded:
    // `C_Traits.GetConditionInfo(…)` answers `tooltipText=string |cffff2020
    // Spend 30 more …`, the trait fixtures).
    for sig in d
        .namespaces
        .get_mut("C_Traits")
        .and_then(|ns| ns.functions.get_mut("GetConditionInfo"))
        .into_iter()
        .flatten()
    {
        if sig.params.len() == 2 {
            sig.params.push(param("ignoreFontColor", "boolean", true));
            sig.params.push(param("traitTreeName", "string", true));
        }
    }
    if let Some(fields) = d.interfaces.get_mut("TraitCondInfo")
        && !fields.iter().any(|(n, _, _)| n == "tooltipText")
    {
        fields.push(("tooltipText".into(), "string".into(), true));
    }
    // A channel message's target is the channel's number: Blizzard's chat
    // edit box sends `C_ChatInfo.SendChatMessage(text, "CHANNEL", language,
    // self:GetChannelTarget())`, the number `GetChannelName` answered
    // (`ChatFrameEditBox.lua`).
    for sig in d
        .namespaces
        .get_mut("C_ChatInfo")
        .and_then(|ns| ns.functions.get_mut("SendChatMessage"))
        .into_iter()
        .flatten()
    {
        channel_target(sig);
    }
    cursor_info(d);
    unit_names(d);
}

/// `GetCursorInfo` is documented without results; the client answers the
/// cursor's kind, then its details, which differ by kind, or nothing with
/// an empty cursor (recorded on camelot: `"spell", slot, "spell",
/// spellID`; `"macro", index`; `"item", itemID, link`; the cursor
/// fixtures).
fn cursor_info(d: &mut Decls) {
    for sig in d.functions.get_mut("GetCursorInfo").into_iter().flatten() {
        if sig.returns.is_empty() {
            sig.returns = vec!["string?".into(), "...any".into()];
        }
    }
}

/// The unit name functions answer nothing for a unit that does not exist,
/// and no realm for one of the player's realm or no player, though the
/// documentation says `Nilable = false` (recorded on camelot:
/// `UnitName("party2")` answers `nil, nil` in a party of two, the group
/// fixtures; `UnitFullName("target")` without a target `nil, nil`;
/// `UnitFullName("none")` and `UnitNameFromGUID` of a creature a name and
/// `nil`, the full probe). Other flavors' clients are documented alike
/// (unverified by a recording).
fn unit_names(d: &mut Decls) {
    for name in [
        "UnitName",
        "UnitFullName",
        "UnitNameUnmodified",
        "UnitNameFromGUID",
    ] {
        for sig in d.functions.get_mut(name).into_iter().flatten() {
            for r in sig.returns.iter_mut() {
                if !r.ends_with('?') {
                    *r = model::optional(r);
                }
            }
        }
    }
}

/// `SendChatMessage`'s `target` also takes a channel number.
fn channel_target(sig: &mut model::Sig) {
    if let Some(target) = sig.params.iter_mut().find(|p| p.name == "target")
        && target.ty == "string"
    {
        target.ty = "string | number".into();
    }
}

fn param(name: &str, ty: &str, optional: bool) -> model::ParamDecl {
    model::ParamDecl {
        name: name.into(),
        ty: ty.into(),
        optional,
    }
}

/// Where the annotations are wrong about functions Blizzard does not
/// document: parameters the client and Blizzard's code leave out.
/// Results the client's recorded answers show and the sources lack (the
/// engine's `camelot-answers.txt`, decision T290), on the final
/// declarations.
fn recorded_corrections(d: &mut Decls) {
    // Results the documentation lacks: `C_PetBattles.IsTrapAvailable()`
    // answers `false, 0` (the reason it is not),
    // `C_TradeSkillUI.IsTradeSkillLinked()` `false, nil` (the linked
    // player's name).
    for (ns, name, extra) in [
        ("C_PetBattles", "IsTrapAvailable", "number"),
        ("C_TradeSkillUI", "IsTradeSkillLinked", "string?"),
    ] {
        for sig in d
            .namespaces
            .get_mut(ns)
            .and_then(|n| n.functions.get_mut(name))
            .into_iter()
            .flatten()
        {
            if sig.returns.len() == 1 && sig.ret_vararg.is_none() {
                sig.returns.push(extra.into());
            }
        }
    }
    // Lists the sources give as their first values: a chat window's
    // channels (name and ID after name and ID) and its message groups.
    for (name, first, rest) in [
        (
            "GetChatWindowChannels",
            &["string", "number"][..],
            "string | number",
        ),
        ("GetChatWindowMessages", &[][..], "string"),
    ] {
        for sig in d.functions.get_mut(name).into_iter().flatten() {
            sig.returns = first.iter().map(|t| t.to_string()).collect();
            sig.ret_vararg = Some(rest.into());
        }
    }
    // The combat log's payload: its 11 base values, then the subevent's
    // own (warcraft.wiki.gg's COMBAT_LOG_EVENT page; Blizzard's
    // documentation gives no results).
    let cleu = || model::Sig {
        returns: [
            "number", "string", "boolean", "string", "string", "number", "number", "string",
            "string", "number", "number",
        ]
        .iter()
        .map(|t| t.to_string())
        .collect(),
        ret_vararg: Some("unknown".into()),
        ..model::Sig::default()
    };
    if let Some(sigs) = d.functions.get_mut("CombatLogGetCurrentEventInfo") {
        *sigs = vec![cleu()];
    }
    if d.globals.contains_key("CombatLogGetCurrentEventInfo") {
        d.globals.insert(
            "CombatLogGetCurrentEventInfo".into(),
            model::fn_type(&cleu()),
        );
    }
    for ns in ["C_CombatLog", "C_CombatLogInternal", "C_CombatLogSecure"] {
        if let Some(sigs) = d
            .namespaces
            .get_mut(ns)
            .and_then(|n| n.functions.get_mut("GetCurrentEventInfo"))
        {
            *sigs = vec![cleu()];
        }
    }
    // The merchant's goods filter (the engine's model, decision T289).
    let filter = |params: &[(&str, &str)], returns: &[&str]| model::Sig {
        params: params
            .iter()
            .map(|(n, t)| model::ParamDecl {
                name: (*n).into(),
                ty: (*t).into(),
                optional: false,
            })
            .collect(),
        returns: returns.iter().map(|t| t.to_string()).collect(),
        ..model::Sig::default()
    };
    for (name, sig) in [
        ("GetMerchantFilter", filter(&[], &["number"])),
        ("SetMerchantFilter", filter(&[("filter", "number")], &[])),
        ("ResetSetMerchantFilter", filter(&[], &[])),
    ] {
        if let Some(sigs) = d.functions.get_mut(name)
            && untyped_sig(sigs)
        {
            *sigs = vec![sig];
        }
    }
    // `GetAchievementInfo`'s `icon` and `earnedBy`: nil for some
    // achievements (recorded: Archmage Arugal kills has no icon, Number of
    // hugs no one who earned it).
    for sig in d
        .functions
        .get_mut("GetAchievementInfo")
        .into_iter()
        .flatten()
    {
        if sig.returns.len() == 15 {
            for i in [9, 13] {
                if !sig.returns[i].ends_with('?') {
                    sig.returns[i] = model::optional(&sig.returns[i]);
                }
            }
        }
    }
}

/// A function no source typed: nothing known of its parameters or results.
fn untyped_sig(sigs: &[model::Sig]) -> bool {
    sigs.iter().all(|s| {
        s.params.is_empty() && s.returns.is_empty() && s.ret_vararg.as_deref() == Some("any")
    })
}

/// A usage check's type (`camelot-usages.txt`'s words).
fn usage_type(check: &str) -> Option<&'static str> {
    Some(match check {
        "number" | "index" | "nonneg" | "nonzero" | "optnumber" => "number",
        "string" | "strictstr" | "guid" => "string",
        "table" => "table",
        "boolean" => "boolean",
        "companion" => "\"CRITTER\" | \"MOUNT\"",
        "questitem" => "\"required\" | \"choice\" | \"reward\"",
        "required" | "any" | "taxislot" => "unknown",
        _ => return None,
    })
}

/// An answer's values' types (`"2: number 0, {…}"`): nil is `unknown`.
fn answer_types(answer: &str) -> Option<Vec<String>> {
    const WORDS: [&str; 7] = [
        "number", "string", "boolean", "nil", "{", "userdata", "function",
    ];
    let (count, rest) = answer.split_once(':')?;
    let count: usize = count.trim().parse().ok()?;
    let rest = rest.trim();
    // the values: split at a top-level comma a type word follows (a
    // string's own commas are not)
    let mut parts = Vec::new();
    let (mut depth, mut from) = (0usize, 0usize);
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let next = rest[i + 1..].trim_start();
                if WORDS.iter().any(|w| next.starts_with(w)) {
                    parts.push(rest[from..i].trim());
                    from = i + 1;
                }
            }
            _ => {}
        }
    }
    if !rest.is_empty() {
        parts.push(rest[from..].trim());
    }
    let types: Vec<String> = parts
        .iter()
        .map(|p| {
            let ty = if p.starts_with('{') {
                "table"
            } else {
                match p.split(' ').next().unwrap_or("") {
                    "number" => "number",
                    "string" => "string",
                    "boolean" => "boolean",
                    _ => "unknown",
                }
            };
            ty.to_string()
        })
        .collect();
    (types.len() == count).then_some(types)
}

/// The undocumented C functions nothing else typed, from what the client
/// told (its usage checks and its answers without arguments, on its own
/// flavor's declarations and the shared ones), else by their names:
/// `Is…`, `Has…`, `Can…`, `Are…`, `Should…`, `Does…` give a boolean,
/// `GetNum…` a number (unverified; decision T294).
fn client_types(d: &mut Decls, client: &ClientFacts, recorded_here: bool) {
    let usages: HashMap<&str, &Vec<(String, String, bool)>> =
        client.usages.iter().map(|(n, a)| (n.as_str(), a)).collect();
    let answers: HashMap<&str, &str> = client
        .answers
        .iter()
        .map(|(n, a)| (n.as_str(), a.as_str()))
        .collect();
    let fill = |name: &str, sigs: &mut Vec<model::Sig>| {
        if !untyped_sig(sigs) {
            return;
        }
        let mut sig = model::Sig {
            vararg: Some("unknown".into()),
            ret_vararg: Some("unknown".into()),
            ..model::Sig::default()
        };
        if recorded_here && let Some(args) = usages.get(name) {
            let typed: Option<Vec<model::ParamDecl>> = args
                .iter()
                .map(|(arg, check, optional)| {
                    Some(model::ParamDecl {
                        name: arg.clone(),
                        ty: usage_type(check)?.to_string(),
                        optional: *optional,
                    })
                })
                .collect();
            if let Some(params) = typed {
                sig.params = params;
                sig.vararg = None;
            }
        }
        if recorded_here
            && let Some(types) = answers.get(name).and_then(|a| answer_types(a))
            && types.iter().any(|t| t != "unknown")
        {
            sig.returns = types;
            sig.ret_vararg = None;
        } else {
            let short = name.rsplit('.').next().unwrap_or(name);
            let upper_after = |prefix: &str| {
                short
                    .strip_prefix(prefix)
                    .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_uppercase()))
            };
            if ["Is", "Has", "Can", "Are", "Should", "Does"]
                .iter()
                .any(|p| upper_after(p))
            {
                sig.returns = vec!["boolean".into()];
                sig.ret_vararg = None;
            } else if upper_after("GetNum") {
                sig.returns = vec!["number".into()];
                sig.ret_vararg = None;
            }
        }
        let typed = !sig.params.is_empty() || !sig.returns.is_empty();
        if typed {
            *sigs = vec![sig];
        }
    };
    for (name, sigs) in d.functions.iter_mut() {
        fill(name, sigs);
    }
    for (ns, space) in d.namespaces.iter_mut() {
        for (member, sigs) in space.functions.iter_mut() {
            fill(&format!("{ns}.{member}"), sigs);
        }
    }
}

/// The forms and nil results of a function as the wiki tells them, over
/// the annotations' (decision T290).
fn answer_corrections(d: &mut Decls, wiki: &Decls) {
    let mut all: Vec<(String, &mut Vec<model::Sig>)> = d
        .functions
        .iter_mut()
        .map(|(n, s)| (n.clone(), s))
        .collect();
    for (ns, space) in d.namespaces.iter_mut() {
        for (n, s) in space.functions.iter_mut() {
            all.push((format!("{ns}.{n}"), s));
        }
    }
    for (name, sigs) in all {
        // An overload the annotations give no results (`---@overload
        // fun(categoryID: number, index: number)` of `GetAchievementInfo`)
        // gives the function's: the forms of one function answer alike.
        if let Some(typed) = sigs
            .iter()
            .find(|s| !s.returns.is_empty() || s.ret_vararg.is_some())
            .cloned()
        {
            for sig in sigs.iter_mut() {
                if sig.returns.is_empty() && sig.ret_vararg.is_none() {
                    sig.returns = typed.returns.clone();
                    sig.ret_vararg = typed.ret_vararg.clone();
                }
            }
        }
        // A result the wiki says can be nil (`month`: "Returns nil if
        // Completed is false") is optional where the results line up.
        let wiki_sigs = match name.split_once('.') {
            Some((ns, member)) => wiki
                .namespaces
                .get(ns)
                .and_then(|n| n.functions.get(member)),
            None => wiki.functions.get(&name),
        };
        let Some(w) = wiki_sigs.and_then(|w| w.first()) else {
            continue;
        };
        for sig in sigs.iter_mut() {
            if sig.returns.len() != w.returns.len() {
                continue;
            }
            for (r, wr) in sig.returns.iter_mut().zip(&w.returns) {
                if wr.ends_with('?') && !r.ends_with('?') && r != "unknown" {
                    *r = model::optional(r);
                }
            }
        }
    }
}

fn annotation_corrections(d: &mut Decls) {
    // `CreateAnimation(type)` gives that animation's class (T280; the method
    // probe records each: `CreateAnimation("Alpha")` is an Alpha), the
    // annotations' generic `table | T` none.
    if let Some(group) = d.classes.get_mut("AnimationGroup") {
        let p = |name: &str, ty: &str, optional: bool| model::ParamDecl {
            name: name.into(),
            ty: ty.into(),
            optional,
        };
        let one = |kind: &str, result: &str| model::Sig {
            params: vec![
                p("animationType", kind, kind == "string"),
                p("name", "string", true),
                p("templateName", "string", true),
            ],
            returns: vec![result.into()],
            ..model::Sig::default()
        };
        let mut sigs: Vec<model::Sig> = [
            "Alpha",
            "Animation",
            "FlipBook",
            "LineScale",
            "LineTranslation",
            "Path",
            "Rotation",
            "Scale",
            "Translation",
            "VertexColor",
        ]
        .iter()
        .map(|k| one(&format!("{k:?}"), k))
        .collect();
        sigs.push(one("string", "Animation"));
        group.methods.insert("CreateAnimation".into(), sigs);
    }
    // ItemTransmogInfoMixin:Init sets them (Blizzard_FrameXMLUtil ItemUtil.lua:
    // `self.appearanceID = appearanceID`, the others `or
    // Constants.Transmog.NoTransmogID`, a number).
    if let Some(info) = d.classes.get_mut("ItemTransmogInfoMixin") {
        for f in ["appearanceID", "secondaryAppearanceID", "illusionID"] {
            info.fields
                .entry(f.into())
                .or_insert(("number".into(), false));
        }
    }
    // (function, parameter), each with its evidence.
    let optional: &[(&str, &str)] = &[
        // Blizzard_TrainerUI calls `SetTrainerServiceTypeFilter(filter,
        // not IsSelected(filter))`.
        ("SetTrainerServiceTypeFilter", "exclusive"),
        // Blizzard's own definitions (Deprecated_SpellBook.lua) read
        // `isPet and Enum.SpellBookSpellBank.Pet or …`.
        ("IsSpellKnown", "isPet"),
        ("IsSpellKnownOrOverridesKnown", "isPet"),
        // Like `JoinPermanentChannel`'s and `JoinTemporaryChannel`'s
        // (Blizzard's ChannelList calls `JoinPermanentChannel(channelName)`);
        // the wiki brackets it too (unverified: no client recording).
        ("JoinChannelByName", "hasVoice"),
    ];
    for (function, name) in optional {
        for sig in d.functions.get_mut(*function).into_iter().flatten() {
            if let Some(p) = sig.params.iter_mut().find(|p| p.name == *name) {
                p.optional = true;
            }
        }
    }
    for sig in d.functions.get_mut("SendChatMessage").into_iter().flatten() {
        channel_target(sig);
    }
    unit_names(d);
}

/// Generates the declarations. `skip`: global names the compiler declares
/// itself (Lua 5.1 and WoW's Lua environment). `annotation_source` goes
/// into the shared file's header.
/// The classes a flavor declares (its mixins, the shared classes, its own).
fn mixins_of_flavor<'a>(
    mixins: &'a indexmap::IndexMap<String, model::Class>,
    shared: &'a Decls,
    d: &'a Decls,
) -> HashSet<&'a String> {
    mixins
        .keys()
        .chain(shared.classes.keys())
        .chain(d.classes.keys())
        .collect()
}

/// A global's type from what its value comes from (decision T288): a mixin
/// table is its mixin class (`table` without one), a call gives what the
/// API says it returns, another global's value its type.
fn global_value_type(
    name: &str,
    hint: &str,
    globals: &indexmap::IndexMap<String, (String, Option<String>)>,
    api: &HashMap<String, model::Sig>,
    classes: &HashSet<&String>,
    dotted: &HashMap<String, String>,
    depth: u32,
) -> String {
    let class_of = |n: &str| classes.iter().any(|c| c.as_str() == n);
    if let Some(callee) = hint.strip_prefix("call:") {
        return match callee {
            "CreateFromMixins" | "Mixin" | "CreateAndInitFromMixin" | "CreateFromMixinsAndInit" => {
                if class_of(name) {
                    name.to_string()
                } else {
                    "table".into()
                }
            }
            "EnumUtil.MakeEnum" => "{ [string]: number }".into(),
            "class" => "table".into(),
            _ => match api.get(callee).and_then(|s| s.returns.first()) {
                Some(r) if !matches!(r.as_str(), "any" | "unknown" | "nil") => r.clone(),
                _ => "unknown".into(),
            },
        };
    }
    if let Some(other) = hint.strip_prefix("alias:") {
        if class_of(name) {
            return name.to_string();
        }
        if class_of(other) {
            return other.to_string();
        }
        if let Some(sig) = api.get(other) {
            return model::fn_type(sig);
        }
        if let Some(ty) = dotted.get(other) {
            return ty.clone();
        }
        if depth < 4
            && let Some((ty, _)) = globals.get(other)
        {
            return if ty.starts_with("call:") || ty.starts_with("alias:") {
                global_value_type(other, ty, globals, api, classes, dotted, depth + 1)
            } else {
                ty.clone()
            };
        }
        return "unknown".into();
    }
    hint.to_string()
}

/// The signature Blizzard's Lua gives a class's function: the class's own,
/// else the nearest base's (through the bases and the parents copied in).
fn defined_in_lua(
    inferred: &HashMap<infer::Owner, model::Sig>,
    parents: &HashMap<String, Vec<String>>,
    class: &str,
    member: &str,
    method: bool,
) -> Option<model::Sig> {
    let mut queue = std::collections::VecDeque::from([class.to_string()]);
    let mut seen = HashSet::new();
    while let Some(c) = queue.pop_front() {
        if !seen.insert(c.clone()) {
            continue;
        }
        let owner = infer::Owner::Table {
            table: c.clone(),
            member: member.to_string(),
            method,
        };
        if let Some(sig) = inferred.get(&owner) {
            return Some(sig.clone());
        }
        // the last mixin's functions win (`CreateFromMixins(A, B)` copies
        // B's over A's): the later parents first
        queue.extend(parents.get(&c).into_iter().flatten().rev().cloned());
    }
    // A function copied in from a parent the classes don't name: the one
    // definition of that name, when there is one.
    let mut found: Option<&model::Sig> = None;
    for (owner, sig) in inferred {
        if let infer::Owner::Table {
            member: m,
            method: k,
            ..
        } = owner
            && m == member
            && *k == method
        {
            if found.is_some_and(|f| f != sig) {
                return None;
            }
            found = Some(sig);
        }
    }
    found.cloned()
}

/// A type the wiki names, as the declarations have it: a name nothing
/// declares is `unknown`; the wiki's own index and string names are
/// `number` and `string`.
fn wiki_type(ty: &str, known: &HashSet<String>) -> String {
    const BUILTIN: &[&str] = &[
        "nil", "number", "string", "boolean", "table", "any", "unknown", "function", "true",
        "false", "thread", "userdata", "never",
    ];
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        let w = word.as_str();
        let mapped = if BUILTIN.contains(&w)
            || w.chars().all(|c| c.is_ascii_digit() || c == '.')
            || known.contains(w)
        {
            w
        } else {
            match w {
                "luaIndex" | "integer" | "float" | "double" => "number",
                "cstring" => "string",
                "bool" => "boolean",
                _ => "unknown",
            }
        };
        out.push_str(mapped);
        word.clear();
    };
    let mut quoted = false;
    for c in ty.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if !quoted && (c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
pub fn generate(
    flavors: &[FlavorDocs],
    annotations: &[(String, String)],
    annotation_source: &str,
    skip: &HashSet<String>,
    compiler_types: &HashSet<String>,
    wiki: &[(String, String)],
    client: &ClientFacts,
    sources: &docs::Sources,
) -> Result<Output, String> {
    let (wiki_docs, written) = (&sources.wiki, &sources.written);
    let mut per_flavor: Vec<(String, String, Decls)> = Vec::new();
    let mut api = Vec::new();
    // what the functions do: the wiki's, completed by each flavor's own
    // documentation
    let mut flavor_docs: HashMap<String, docs::Docs> = HashMap::new();
    for f in flavors {
        let json = blizzard::read_json(&f.files)?;
        let mut d = docs::merge(wiki_docs, &docs::from_blizzard(&json));
        docs::add_written(&mut d, written);
        flavor_docs.insert(f.id.clone(), d);
        api.push((f.id.clone(), json));
        let mut d = blizzard::read(&f.files)?;
        corrections(&mut d);
        d.functions.retain(|n, _| !skip.contains(n));
        d.namespaces.retain(|n, _| !skip.contains(n));
        per_flavor.push((f.id.clone(), f.source.clone(), d));
    }
    // Older flavors' documentation is incomplete (vanilla's lacks
    // `GetTime`): a flavor gets the globals, namespaces and types its own
    // documentation lacks from the other flavors', in the given order.
    // Functions come only where the flavor's client has them (when known).
    let fallback: Vec<Decls> = per_flavor.iter().map(|(_, _, d)| d.clone()).collect();
    for ((_, _, d), f) in per_flavor.iter_mut().zip(flavors) {
        let has = |name: &str| f.api_names.as_ref().is_none_or(|a| a.contains(name));
        for other in &fallback {
            for (name, sigs) in &other.functions {
                if has(name) {
                    d.functions
                        .entry(name.clone())
                        .or_insert_with(|| sigs.clone());
                }
            }
            for (name, ns) in &other.namespaces {
                for (fname, sigs) in &ns.functions {
                    if has(&format!("{name}.{fname}")) {
                        d.namespaces
                            .entry(name.clone())
                            .or_default()
                            .functions
                            .entry(fname.clone())
                            .or_insert_with(|| sigs.clone());
                    }
                }
            }
            for (name, fields) in &other.interfaces {
                d.interfaces
                    .entry(name.clone())
                    .or_insert_with(|| fields.clone());
            }
            for (name, ty) in &other.aliases {
                d.aliases.entry(name.clone()).or_insert_with(|| ty.clone());
            }
            for (name, members) in &other.enums {
                d.enums
                    .entry(name.clone())
                    .or_insert_with(|| members.clone());
            }
            // Events only where the flavor's client has them (when known).
            for (name, params) in &other.events {
                if f.event_names.as_ref().is_none_or(|e| e.contains(name)) {
                    d.events
                        .entry(name.clone())
                        .or_insert_with(|| params.clone());
                }
            }
        }
    }
    let mut shared = luals::convert(annotations);
    // `colorRGB` and `colorRGBA` are ColorMixin objects: Blizzard's
    // `CreateColor` returns `CreateFromMixins(ColorMixin)` (Blizzard_SharedXMLBase
    // Color.lua), which the annotations call `colorRGBA`; one name for one
    // object, so a ColorMixin from anywhere is a colour (decision T288).
    if shared.classes.contains_key("ColorMixin") {
        for name in ["colorRGB", "colorRGBA"] {
            if let Some(c) = shared.classes.shift_remove(name) {
                let mixin = shared.classes.get_mut("ColorMixin").expect("present");
                for (f, (ty, optional)) in c.fields {
                    let optional = optional || !matches!(f.as_str(), "r" | "g" | "b");
                    mixin.fields.entry(f).or_insert((ty, optional));
                }
                shared.aliases.insert(name.into(), "ColorMixin".into());
            }
        }
    }
    // A LuaLS `@class` that is only data (fields, no functions, no parent:
    // `backdropInfo`) is a record, as LuaLS reads it: structural, so a table
    // of those fields is one (decision T288).
    let records: Vec<String> = shared
        .classes
        .iter()
        .filter(|(_, c)| {
            c.methods.is_empty()
                && c.statics.is_empty()
                && c.bases.is_empty()
                && c.also.is_empty()
                && c.widget.is_none()
                && !c.fields.is_empty()
        })
        .map(|(n, _)| n.clone())
        .collect();
    for name in records {
        let c = shared.classes.shift_remove(&name).expect("present");
        let fields = c.fields.into_iter().map(|(f, (t, o))| (f, t, o)).collect();
        shared.interfaces.insert(name, fields);
    }
    annotation_corrections(&mut shared);
    // Blizzard's data wins: drop what any flavor's documentation defines,
    // and what the compiler declares itself.
    let mut bliz_functions: HashSet<String> = HashSet::new();
    let mut bliz_namespaces: HashSet<String> = HashSet::new();
    let mut bliz_types: HashSet<String> = HashSet::new();
    for (_, _, d) in &per_flavor {
        bliz_functions.extend(d.functions.keys().cloned());
        bliz_namespaces.extend(d.namespaces.keys().cloned());
        bliz_types.extend(d.interfaces.keys().cloned());
        bliz_types.extend(d.aliases.keys().cloned());
    }
    shared
        .functions
        .retain(|n, _| !bliz_functions.contains(n) && !skip.contains(n));
    shared.namespaces.retain(|n, _| {
        !bliz_namespaces.contains(n) && !skip.contains(n) && n != "Enum" && n != "Constants"
    });
    shared.aliases.retain(|n, _| !bliz_types.contains(n));
    shared.interfaces.retain(|n, _| !bliz_types.contains(n));
    shared.classes.retain(|n, _| !bliz_types.contains(n));
    shared.globals.retain(|n, _| {
        !skip.contains(n) && !bliz_functions.contains(n) && !bliz_namespaces.contains(n)
    });
    shared.enums.clear();
    shared.constants.clear();
    blizzard::mark_widgets(&mut shared.classes);
    widget_corrections(&mut shared.classes);
    script_handlers(&mut shared.classes, &shared.aliases);
    // `CreateFrame("Button", …)` returns a `Button`: per flavor, with the
    // frame types only some have (`Browser`, `Checkout`); the annotations'
    // generic one goes.
    shared.functions.shift_remove("CreateFrame");
    for (_, _, d) in per_flavor.iter_mut() {
        let mut create = Vec::new();
        for t in blizzard::FRAME_TYPES {
            if shared.classes.contains_key(*t) || d.classes.contains_key(*t) {
                create.push(create_frame_sig(&format!("{t:?}"), t));
            } else if let Some((_, ty)) = INTRINSIC_FRAMES.iter().find(|(n, _)| n == t) {
                create.push(create_frame_sig(&format!("{t:?}"), ty));
            }
        }
        create.push(create_frame_sig("string", "Frame"));
        d.functions.insert("CreateFrame".into(), create);
    }
    // Arguments Blizzard documents as accepting secrets
    // (`SecretArguments`) accept them in the annotated methods too.
    for (_, _, d) in &per_flavor {
        for (class_name, class) in &d.classes {
            let Some(target) = shared.classes.get_mut(class_name) else {
                continue;
            };
            for (method, sigs) in &class.methods {
                let Some(targets) = target.methods.get_mut(method) else {
                    continue;
                };
                // Results the annotations leave out come from Blizzard's
                // documentation (`Minimap:GetZoom()`: `zoomFactor`).
                if let Some(doc) = sigs.first()
                    && !doc.returns.is_empty()
                    && targets
                        .iter()
                        .all(|t| t.returns.is_empty() && t.ret_vararg.is_none())
                {
                    for t in targets.iter_mut() {
                        t.returns = doc.returns.clone();
                        t.ret_vararg = doc.ret_vararg.clone();
                    }
                }
                for sig in sigs {
                    // A method taking secrets takes them in its `...` too
                    // (`FontString:SetFormattedText(format, ...)`: the
                    // documentation names only `text`).
                    if sig.params.iter().any(|p| p.ty.contains("Secret<")) {
                        for t in targets.iter_mut() {
                            if let Some(v) = &t.vararg
                                && !v.contains("Secret<")
                            {
                                let secret: Vec<String> = v
                                    .split('|')
                                    .map(str::trim)
                                    .filter(|m| matches!(*m, "string" | "number" | "boolean"))
                                    .map(|m| format!("Secret<{m}>"))
                                    .collect();
                                if !secret.is_empty() {
                                    t.vararg = Some(format!("{v} | {}", secret.join(" | ")));
                                }
                            }
                        }
                    }
                    for (i, p) in sig.params.iter().enumerate() {
                        if !p.ty.contains("Secret<") {
                            continue;
                        }
                        for t in targets.iter_mut() {
                            if let Some(tp) = t.params.get_mut(i)
                                && tp.name == p.name
                                && !tp.ty.contains("Secret<")
                            {
                                tp.ty = format!("{} | Secret<{}>", tp.ty, tp.ty);
                            }
                        }
                    }
                }
            }
        }
    }
    // Tooltip methods Blizzard's `TooltipDataHandler.lua` defines pass their
    // arguments to a documented `C_TooltipInfo` function and take its
    // parameters (`MailFrame.lua` calls `GameTooltip:SetInboxItem(
    // InboxFrame.openMailID, index)`), where the annotations have fewer or
    // untyped ones.
    if let Some(tooltip) = shared.classes.get_mut("GameTooltip") {
        let accessors: BTreeSet<(String, String)> = flavors
            .iter()
            .flat_map(|f| framexml::tooltip_accessors(&f.ui_lua))
            .collect();
        for (method, getter) in accessors {
            let Some(doc) = per_flavor.iter().find_map(|(_, _, d)| {
                d.namespaces
                    .get("C_TooltipInfo")
                    .and_then(|ns| ns.functions.get(&getter))
                    .and_then(|sigs| sigs.first())
            }) else {
                continue;
            };
            let Some(sigs) = tooltip.methods.get_mut(&method) else {
                continue;
            };
            for sig in sigs.iter_mut() {
                // Annotations with more parameters stay (a flavor's own
                // `SetTalent(talentIndex, isInspect, talentGroup, …)`).
                if sig.params.len() < doc.params.len()
                    || sig.params.len() == doc.params.len()
                        && sig.params.iter().any(|p| p.ty == "any")
                {
                    sig.params = doc.params.clone();
                    sig.vararg = doc.vararg.clone();
                }
            }
            let mut seen = Vec::new();
            sigs.retain(|s| {
                let new = !seen.contains(s);
                seen.push(s.clone());
                new
            });
        }
    }
    // Classes only Blizzard knows stay in the flavor files; methods of
    // classes the annotations declare come from the annotations.
    for (_, _, d) in per_flavor.iter_mut() {
        d.classes.retain(|n, _| !shared.classes.contains_key(n));
        blizzard::mark_widgets(&mut d.classes);
    }
    // Annotated functions some flavor's client lacks move to the flavors
    // that have them.
    let all_have = |name: &str| {
        flavors
            .iter()
            .all(|f| f.api_names.as_ref().is_none_or(|a| a.contains(name)))
    };
    let moved: Vec<String> = shared
        .functions
        .keys()
        .filter(|n| {
            !all_have(n)
                && flavors
                    .iter()
                    .any(|f| f.api_names.as_ref().is_some_and(|a| a.contains(*n)))
        })
        .cloned()
        .collect();
    for name in moved {
        let sigs = shared.functions.shift_remove(&name).expect("present");
        for ((_, _, d), f) in per_flavor.iter_mut().zip(flavors) {
            if f.api_names.as_ref().is_none_or(|a| a.contains(&name)) {
                d.functions
                    .entry(name.clone())
                    .or_insert_with(|| sigs.clone());
            }
        }
    }
    // warcraft.wiki.gg's signatures (decision T288), for the functions
    // nothing above types; its records, where nothing declares them.
    let wiki = luals::convert(wiki);
    let mut known: HashSet<String> = shared
        .aliases
        .keys()
        .chain(shared.interfaces.keys())
        .chain(shared.classes.keys())
        .cloned()
        .collect();
    for (_, _, d) in &per_flavor {
        known.extend(d.aliases.keys().cloned());
        known.extend(d.interfaces.keys().cloned());
        known.extend(d.classes.keys().cloned());
        known.extend(d.enums.keys().map(|e| format!("Enum.{e}")));
    }
    known.extend(compiler_types.iter().cloned());
    type Record = (String, Vec<(String, String, bool)>);
    let records: Vec<Record> = wiki
        .classes
        .iter()
        .filter(|(n, c)| !known.contains(*n) && !c.fields.is_empty())
        .map(|(n, c)| {
            let fields = c
                .fields
                .iter()
                .map(|(f, (t, o))| (f.clone(), t.clone(), *o))
                .collect();
            (n.clone(), fields)
        })
        .collect();
    known.extend(records.iter().map(|(n, _)| n.clone()));
    for (name, fields) in records {
        let fields = fields
            .into_iter()
            .map(|(f, t, o)| (f, wiki_type(&t, &known), o))
            .collect();
        shared.interfaces.insert(name, fields);
    }
    let wiki_sigs = |sigs: &[model::Sig]| -> Vec<model::Sig> {
        sigs.iter()
            .map(|s| {
                let mut s = s.clone();
                for p in &mut s.params {
                    p.ty = wiki_type(&p.ty, &known);
                }
                s.vararg = s.vararg.map(|t| wiki_type(&t, &known));
                s.returns = s.returns.iter().map(|t| wiki_type(t, &known)).collect();
                s.ret_vararg = s.ret_vararg.map(|t| wiki_type(&t, &known));
                s
            })
            .collect()
    };
    let wiki_function = |name: &str| -> Option<Vec<model::Sig>> {
        let sigs = match name.split_once('.') {
            Some((ns, member)) => wiki.namespaces.get(ns)?.functions.get(member)?,
            None => wiki.functions.get(name)?,
        };
        Some(wiki_sigs(sigs))
    };
    // Ketho's untyped functions (a page without annotations) the wiki types.
    let untyped = |sigs: &[model::Sig]| sigs.iter().all(|s| s.ret_vararg.as_deref() == Some("any"));
    let names: Vec<String> = shared.functions.keys().cloned().collect();
    for name in names {
        if untyped(&shared.functions[&name])
            && let Some(sigs) = wiki_function(&name)
        {
            shared.functions.insert(name, sigs);
        }
    }
    let spaces: Vec<String> = shared.namespaces.keys().cloned().collect();
    for ns in spaces {
        let members: Vec<String> = shared.namespaces[&ns].functions.keys().cloned().collect();
        for member in members {
            if untyped(&shared.namespaces[&ns].functions[&member])
                && let Some(sigs) = wiki_function(&format!("{ns}.{member}"))
            {
                shared.namespaces[&ns].functions.insert(member, sigs);
            }
        }
    }
    answer_corrections(&mut shared, &wiki);
    for (_, _, d) in per_flavor.iter_mut() {
        answer_corrections(d, &wiki);
    }
    // Blizzard's UI functions, typed from their Lua (decision T288).
    let mut inferred: Vec<HashMap<infer::Owner, model::Sig>> = Vec::new();
    let mut called: Vec<infer::Inferred> = Vec::new();
    // the mixins' fields as their Lua sets them, per flavor
    let mut lua_fields: Vec<HashMap<(String, String), String>> = Vec::new();
    let mut apis: Vec<HashMap<String, model::Sig>> = Vec::new();
    for ((_, _, d), f) in per_flavor.iter().zip(flavors) {
        let mut api: HashMap<String, model::Sig> = HashMap::new();
        for (n, sigs) in shared.functions.iter().chain(d.functions.iter()) {
            if let Some(s) = sigs.first() {
                api.entry(n.clone()).or_insert_with(|| s.clone());
            }
        }
        for (ns, space) in shared.namespaces.iter().chain(d.namespaces.iter()) {
            for (m, sigs) in &space.functions {
                if let Some(s) = sigs.first() {
                    api.entry(format!("{ns}.{m}")).or_insert_with(|| s.clone());
                }
            }
        }
        // constants and enum values (`=Constants.X.Y`, `=Enum.X.Y`)
        for decls in [&shared, d] {
            for (group, values) in &decls.constants {
                for (name, ty) in values {
                    api.insert(
                        format!("=Constants.{group}.{name}"),
                        model::Sig {
                            returns: vec![ty.clone()],
                            ..model::Sig::default()
                        },
                    );
                }
            }
            for (e, members) in &decls.enums {
                for (m, _) in members {
                    api.insert(
                        format!("=Enum.{e}.{m}"),
                        model::Sig {
                            returns: vec!["number".into()],
                            ..model::Sig::default()
                        },
                    );
                }
            }
        }
        // the classes' own functions (`T:M`, `T.F`), for their Lua's evidence
        for (cname, c) in shared.classes.iter().chain(d.classes.iter()) {
            for (m, sigs) in &c.methods {
                if let Some(s) = sigs.first() {
                    api.entry(format!("{cname}:{m}"))
                        .or_insert_with(|| s.clone());
                }
            }
            for (m, sigs) in &c.statics {
                if let Some(s) = sigs.first() {
                    api.entry(format!("{cname}.{m}"))
                        .or_insert_with(|| s.clone());
                }
            }
        }
        // the records' and classes' declared fields (`T#x`), for a field
        // read of a typed value (`auraData.name`)
        for decls in [&shared, d] {
            for (iname, fields) in &decls.interfaces {
                for (field, ty, optional) in fields {
                    let ty = if *optional {
                        model::optional(ty)
                    } else {
                        ty.clone()
                    };
                    api.entry(format!("{iname}#{field}"))
                        .or_insert_with(|| model::Sig {
                            returns: vec![ty],
                            ..model::Sig::default()
                        });
                }
            }
            for (cname, c) in &decls.classes {
                for (field, (ty, optional)) in &c.fields {
                    let ty = if *optional {
                        model::optional(ty)
                    } else {
                        ty.clone()
                    };
                    api.entry(format!("{cname}#{field}"))
                        .or_insert_with(|| model::Sig {
                            returns: vec![ty],
                            ..model::Sig::default()
                        });
                }
            }
        }
        // passes until nothing changes (at most five): each reads the one
        // before's functions and the mixins' fields as typed API, so
        // wrappers of wrappers get their types (`Settings.X` returns
        // `SettingsInbound.X(…)`)
        let declared: HashSet<String> = api.keys().cloned().collect();
        let mut analysis = infer::analyze(&f.ui_lua, &api);
        for _ in 0..4 {
            let mut changed = false;
            let mut learn =
                |key: String, sig: model::Sig, api: &mut HashMap<String, model::Sig>| {
                    if !declared.contains(&key) && api.get(&key) != Some(&sig) {
                        api.insert(key, sig);
                        changed = true;
                    }
                };
            for (owner, sig) in &analysis.defined {
                match owner {
                    infer::Owner::Global(n) => learn(n.clone(), sig.clone(), &mut api),
                    infer::Owner::Table {
                        table,
                        member,
                        method: false,
                    } => learn(format!("{table}.{member}"), sig.clone(), &mut api),
                    _ => {}
                }
            }
            // the mixins' fields, as `T#field`, for `self.x`
            for ((table, field), ty) in analysis.fields() {
                let sig = model::Sig {
                    returns: vec![ty.clone()],
                    ..model::Sig::default()
                };
                learn(format!("{table}#{field}"), sig, &mut api);
            }
            if !changed {
                break;
            }
            analysis = infer::analyze(&f.ui_lua, &api);
        }
        lua_fields.push(
            analysis
                .fields()
                .map(|(k, t)| (k.clone(), t.clone()))
                .collect(),
        );
        inferred.push(analysis.defined.clone());
        called.push(analysis);
        apis.push(api);
    }
    // Classes whose functions the annotations leave untyped (Ketho's
    // FrameXML mixins): as Blizzard's Lua shows them; a shared class from
    // the first flavor whose Lua defines the function.
    let parents: HashMap<String, Vec<String>> = shared
        .classes
        .iter()
        .chain(per_flavor.iter().flat_map(|(_, _, d)| d.classes.iter()))
        .map(|(n, c)| (n.clone(), c.bases.iter().chain(&c.also).cloned().collect()))
        .collect();
    let typed_from_lua = |class: &str, member: &str, method: bool, flavors: &[usize]| {
        flavors
            .iter()
            .find_map(|&i| defined_in_lua(&inferred[i], &parents, class, member, method))
    };
    let all: Vec<usize> = (0..inferred.len()).collect();
    let untyped_sigs =
        |sigs: &[model::Sig]| sigs.iter().all(|s| s.ret_vararg.as_deref() == Some("any"));
    for (class, c) in shared.classes.iter_mut() {
        for (statics, members) in [(false, &mut c.methods), (true, &mut c.statics)] {
            for (member, sigs) in members.iter_mut() {
                if untyped_sigs(sigs)
                    && let Some(sig) = typed_from_lua(class, member, !statics, &all)
                {
                    *sigs = vec![sig];
                }
            }
        }
    }
    for (class, c) in shared.classes.iter_mut() {
        if c.widget.is_some() {
            continue;
        }
        for fields in &lua_fields {
            let mut fields: Vec<(&(String, String), &String)> = fields.iter().collect();
            fields.sort();
            for ((table, field), ty) in fields {
                if table == class && !c.fields.contains_key(field) && model::is_ident(field) {
                    c.fields.insert(field.clone(), (ty.clone(), false));
                }
            }
        }
    }
    for (fi, (_, _, d)) in per_flavor.iter_mut().enumerate() {
        for (class, c) in d.classes.iter_mut() {
            for (statics, members) in [(false, &mut c.methods), (true, &mut c.statics)] {
                for (member, sigs) in members.iter_mut() {
                    if untyped_sigs(sigs)
                        && let Some(sig) = typed_from_lua(class, member, !statics, &[fi])
                    {
                        *sigs = vec![sig];
                    }
                }
            }
        }
    }
    // The annotations' functions still untyped (no page, no annotation):
    // as Blizzard's calls show them, in the first flavor that calls them.
    let from_calls = |name: &str| called.iter().find_map(|c| c.called(name));
    let names: Vec<String> = shared.functions.keys().cloned().collect();
    for name in names {
        if untyped(&shared.functions[&name])
            && let Some(sig) = from_calls(&name)
        {
            shared.functions.insert(name, vec![sig]);
        }
    }
    let spaces: Vec<String> = shared.namespaces.keys().cloned().collect();
    for ns in spaces {
        let members: Vec<String> = shared.namespaces[&ns].functions.keys().cloned().collect();
        for member in members {
            if untyped(&shared.namespaces[&ns].functions[&member])
                && let Some(sig) = from_calls(&format!("{ns}.{member}"))
            {
                shared.namespaces[&ns].functions.insert(member, vec![sig]);
            }
        }
    }
    // API functions no source documents: the wiki's, Blizzard's Lua's, else
    // untyped.
    for (fi, ((_, _, d), f)) in per_flavor.iter_mut().zip(flavors).enumerate() {
        // defined in Blizzard's Lua, else as its calls there show it
        let from_lua = |name: &str| {
            inferred[fi]
                .get(&infer::Owner::Global(name.to_string()))
                .cloned()
                .or_else(|| called[fi].called(name))
                .map(|s| vec![s])
        };
        let Some(api) = &f.api_names else { continue };
        let mut names: Vec<&String> = api.iter().collect();
        names.sort();
        for name in names {
            match name.split_once('.') {
                None => {
                    if model::is_ident(name)
                        && !skip.contains(name)
                        && !d.functions.contains_key(name)
                        && !shared.functions.contains_key(name)
                        && !d.namespaces.contains_key(name)
                        && !shared.namespaces.contains_key(name)
                        && !shared.classes.contains_key(name)
                    {
                        match wiki_function(name).or_else(|| from_lua(name)) {
                            Some(sigs) => {
                                d.functions.insert(name.clone(), sigs);
                            }
                            None => {
                                d.globals
                                    .entry(name.clone())
                                    .or_insert_with(|| "(...any) -> ...any".into());
                            }
                        }
                    }
                }
                Some((ns, member))
                    if model::is_ident(ns) && model::is_ident(member) && !skip.contains(ns) =>
                {
                    let in_shared = shared
                        .namespaces
                        .get(ns)
                        .is_some_and(|n| n.functions.contains_key(member));
                    if !in_shared && !shared.classes.contains_key(ns) {
                        let target = d.namespaces.entry(ns.to_string()).or_default();
                        let typed = wiki_function(name).or_else(|| from_lua(name));
                        target
                            .functions
                            .entry(member.to_string())
                            .or_insert_with(|| {
                                typed.unwrap_or_else(|| {
                                    vec![model::Sig {
                                        vararg: Some("any".into()),
                                        ret_vararg: Some("any".into()),
                                        ..model::Sig::default()
                                    }]
                                })
                            });
                    }
                }
                _ => {}
            }
        }
    }
    // Globals of Blizzard's UI code and global strings, where nothing else
    // declares them; those of every flavor go into the shared file.
    let widget_classes: HashSet<String> = shared.classes.keys().cloned().collect();
    // Each with the load-on-demand addon that alone defines it, if any.
    let mut ui_overrides: Vec<Vec<(String, String)>> = Vec::new();
    let mut ui_globals: Vec<indexmap::IndexMap<String, (String, Option<String>)>> = Vec::new();
    // Each flavor's mixin classes its named frames are typed with.
    let mut ui_mixins: Vec<indexmap::IndexMap<String, model::Class>> = Vec::new();
    // Each flavor's functions of a Lua table the annotations declare as a
    // namespace but without them (`AuraUtil.UnpackAuraData`).
    let mut ns_extras: Vec<Vec<(String, String, Vec<model::Sig>)>> = Vec::new();
    for (f, (_, _, d)) in flavors.iter().zip(&per_flavor) {
        let lod = framexml::lod_origins(&f.ui_lua, &f.ui_xml, &f.lod_files);
        let ui = framexml::ui_globals(&f.ui_lua, &f.ui_xml, &widget_classes);
        let fi = ui_mixins.len();
        // the mixins' functions as their Lua shows them (a function a base
        // defines, copied in, as the base's Lua shows it)
        let mut mixins = ui.mixins;
        let parents: HashMap<String, Vec<String>> = mixins
            .iter()
            .map(|(n, c)| (n.clone(), c.bases.iter().chain(&c.also).cloned().collect()))
            .chain(
                shared
                    .classes
                    .iter()
                    .map(|(n, c)| (n.clone(), c.bases.iter().chain(&c.also).cloned().collect())),
            )
            .collect();
        for (class, c) in mixins.iter_mut() {
            // functions kept in its fields (`Settings.CreateElementInitializer`)
            for (field, (ty, _)) in c.fields.iter_mut() {
                if ty.contains("->")
                    && let Some(sig) = inferred[fi].get(&infer::Owner::Table {
                        table: class.clone(),
                        member: field.clone(),
                        method: false,
                    })
                {
                    *ty = model::fn_type(sig);
                }
            }
            for (statics, members) in [(false, &mut c.methods), (true, &mut c.statics)] {
                for (member, sigs) in members.iter_mut() {
                    // where the reader found it (a mixin copied in), else
                    // through the bases
                    let origin = ui
                        .origins
                        .get(&(class.clone(), member.clone()))
                        .filter(|_| !statics)
                        .and_then(|o| {
                            inferred[fi].get(&infer::Owner::Table {
                                table: o.clone(),
                                member: member.clone(),
                                method: true,
                            })
                        })
                        .cloned();
                    if let Some(sig) = origin.or_else(|| {
                        defined_in_lua(&inferred[fi], &parents, class, member, !statics)
                    }) {
                        *sigs = vec![sig];
                    }
                }
            }
        }
        // and the fields their Lua sets (`self.r = r`), where not declared
        // (in name order: the map's own order changes from run to run)
        let mut set: Vec<(&(String, String), &String)> = lua_fields[fi].iter().collect();
        set.sort();
        for (class, c) in mixins.iter_mut() {
            for &((table, field), ty) in &set {
                if table == class && !c.fields.contains_key(field) && model::is_ident(field) {
                    c.fields.insert(field.clone(), (ty.clone(), false));
                }
            }
        }
        // a table the annotations declare as a namespace (`AuraUtil`): the
        // functions its Lua defines that they lack (decision T296)
        let mut extras: Vec<(String, String, Vec<model::Sig>)> = Vec::new();
        for (owner, sig) in &inferred[fi] {
            let infer::Owner::Table {
                table,
                member,
                method: false,
            } = owner
            else {
                continue;
            };
            if let Some(ns) = shared.namespaces.get(table)
                && !ns.functions.contains_key(member)
                && !ns.values.contains_key(member)
                && model::is_ident(member)
            {
                extras.push((table.clone(), member.clone(), vec![sig.clone()]));
            }
        }
        // (in name order: the map's own order changes from run to run)
        extras.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        ns_extras.push(extras);
        ui_mixins.push(mixins);
        let mut g: indexmap::IndexMap<String, (String, Option<String>)> = ui
            .globals
            .into_iter()
            .map(|(n, ty)| {
                let addon = lod.get(&n).cloned();
                // a function its Lua defines, typed from it
                let ty = match inferred[fi].get(&infer::Owner::Global(n.clone())) {
                    Some(sig) if ty == "(...any) -> ...any" => model::fn_type(sig),
                    _ => ty,
                };
                (n, (ty, addon))
            })
            .collect();
        // what a value comes from (`call:F`, `alias:X`): typed
        let classes: HashSet<&String> = mixins_of_flavor(&ui_mixins[fi], &shared, d);
        // the types of dotted values: constants, enum members, namespace values
        let mut dotted: HashMap<String, String> = HashMap::new();
        for decls in [&shared, d] {
            for (group, values) in &decls.constants {
                // (their values are their types: `number`, a literal)
                for (name, ty) in values {
                    dotted.insert(format!("Constants.{group}.{name}"), ty.clone());
                }
            }
            for (e, members) in &decls.enums {
                for (m, _) in members {
                    dotted.insert(format!("Enum.{e}.{m}"), "number".into());
                }
            }
            for (ns, space) in &decls.namespaces {
                for (v, ty) in &space.values {
                    dotted.insert(format!("{ns}.{v}"), ty.clone());
                }
            }
        }
        let resolved: Vec<(String, String)> = g
            .iter()
            .filter(|(_, (ty, _))| ty.starts_with("call:") || ty.starts_with("alias:"))
            .map(|(n, (ty, _))| {
                let ty = global_value_type(n, ty, &g, &apis[fi], &classes, &dotted, 0);
                // an alias of a table's function its Lua defines
                let ty = match ty.as_str() {
                    "unknown" => g[n]
                        .0
                        .strip_prefix("alias:")
                        .and_then(|a| a.rsplit_once('.'))
                        .and_then(|(t, m)| {
                            inferred[fi].get(&infer::Owner::Table {
                                table: t.to_string(),
                                member: m.to_string(),
                                method: false,
                            })
                        })
                        .map(model::fn_type)
                        // else as Blizzard's calls of it show it; an alias of
                        // a `C_` namespace's member is a function still
                        .or_else(|| called[fi].called(n).map(|s| model::fn_type(&s)))
                        .or_else(|| {
                            g[n].0
                                .strip_prefix("alias:C_")
                                .filter(|a| a.contains('.'))
                                .map(|_| "(...unknown) -> ...unknown".to_string())
                        })
                        .unwrap_or(ty),
                    _ => ty,
                };
                (n.clone(), ty)
            })
            .collect();
        for (n, ty) in resolved {
            if let Some(slot) = g.get_mut(&n) {
                slot.0 = ty;
            }
        }
        // a table the code puts functions in (`ScrollUtil`): its class,
        // unless the name is another type (`UiMapPoint`, a record too)
        let other_type = |n: &str| {
            shared.interfaces.contains_key(n)
                || shared.aliases.contains_key(n)
                || d.interfaces.contains_key(n)
                || d.aliases.contains_key(n)
        };
        for (n, (ty, _)) in g.iter_mut() {
            if ty == "table" && ui_mixins[fi].contains_key(n) && !other_type(n) {
                *ty = n.clone();
            }
        }
        for s in &f.strings {
            g.entry(s.clone())
                .or_insert_with(|| ("string".into(), None));
        }
        // The number globals the client defines itself.
        for n in &f.number_globals {
            g.entry(n.clone())
                .or_insert_with(|| ("number".into(), None));
        }
        // what Blizzard's Lua makes a global, over another source's untyped
        // declaration of it (`CombatLogGetCurrentEventInfo =
        // C_CombatLog.GetCurrentEventInfo`, an `any` elsewhere)
        let untyped = |t: &str| matches!(t, "any" | "unknown" | "(...any) -> ...any");
        let mut overrides: Vec<(String, String)> = Vec::new();
        for (n, (ty, _)) in g.iter() {
            if untyped(ty) || ty == "table" || ty.starts_with("call:") || ty.starts_with("alias:") {
                continue;
            }
            if d.globals.get(n).is_some_and(|t| untyped(t)) {
                overrides.push((n.clone(), ty.clone()));
            } else if shared.globals.get(n).is_some_and(|t| untyped(t)) {
                shared.globals.insert(n.clone(), ty.clone());
            }
        }
        g.retain(|n, _| {
            !skip.contains(n)
                && !d.functions.contains_key(n)
                && !d.namespaces.contains_key(n)
                && !d.globals.contains_key(n)
                && !shared.functions.contains_key(n)
                && !shared.namespaces.contains_key(n)
                && !shared.globals.contains_key(n)
        });
        // A global table the annotations name a class after is of that
        // class (`ItemLocation = {}` with `ItemLocation:CreateFromBagAndSlot`);
        // a frame keeps its widget type (`NamePlateDriverFrame`).
        for (n, (ty, _)) in g.iter_mut() {
            if shared.classes.contains_key(n) && ty == "table" {
                *ty = n.clone();
            }
        }
        ui_globals.push(g);
        ui_overrides.push(overrides);
    }
    if let Some(first) = ui_globals.first().cloned() {
        for (name, (ty, addon)) in first {
            if ui_globals
                .iter()
                .all(|g| g.get(&name) == Some(&(ty.clone(), addon.clone())))
            {
                shared.globals.insert(name.clone(), ty);
                if let Some(a) = addon {
                    shared.global_addons.insert(name.clone(), a);
                }
                for g in ui_globals.iter_mut() {
                    g.shift_remove(&name);
                }
            }
        }
    }
    for ((((_, _, d), g), over), extras) in per_flavor
        .iter_mut()
        .zip(ui_globals)
        .zip(ui_overrides)
        .zip(ns_extras)
    {
        for (ns, member, sigs) in extras {
            d.namespaces
                .entry(ns)
                .or_default()
                .functions
                .entry(member)
                .or_insert(sigs);
        }
        for (n, ty) in over {
            d.globals.insert(n, ty);
        }
        for (name, (ty, addon)) in g {
            if !d.globals.contains_key(&name)
                && let Some(a) = addon
            {
                d.global_addons.insert(name.clone(), a);
            }
            d.globals.entry(name).or_insert(ty);
        }
    }
    // A UI global or a function nothing typed (one Blizzard's Lua assigns,
    // or the client's own, as camelot's `Clamp` is): the wiki's signature,
    // else the type another flavor gives it, where every type it names
    // exists here (retail's MathUtil.lua types `Clamp`; decision T293).
    let untyped_global = |t: &str| t == "(...any) -> ...any";
    let untyped_sigs = |sigs: &[model::Sig]| {
        sigs.iter().all(|s| {
            s.params.is_empty() && s.returns.is_empty() && s.ret_vararg.as_deref() == Some("any")
        })
    };
    let mut typed_globals: HashMap<String, String> = HashMap::new();
    let mut typed_functions: HashMap<String, Vec<model::Sig>> = HashMap::new();
    for (_, _, d) in &per_flavor {
        for (n, t) in &d.globals {
            if !untyped_global(t) {
                typed_globals.entry(n.clone()).or_insert_with(|| t.clone());
            }
        }
        for (n, sigs) in &d.functions {
            if !untyped_sigs(sigs) {
                typed_functions
                    .entry(n.clone())
                    .or_insert_with(|| sigs.clone());
            }
        }
    }
    let common = shared.defined_types();
    for (_, _, d) in per_flavor.iter_mut() {
        let mut here = d.defined_types();
        here.extend(common.iter().cloned());
        let fits = |ty: &str| {
            let mut names = BTreeSet::new();
            model::collect_type_names(ty, &mut names);
            names.iter().all(|n| here.contains(n))
        };
        for (n, t) in d.globals.iter_mut() {
            if !untyped_global(t) {
                continue;
            }
            if let Some(sigs) = wiki_function(n)
                && let Some(first) = sigs.first()
            {
                *t = model::fn_type(first);
            } else if let Some(other) = typed_globals.get(n)
                && fits(other)
            {
                *t = other.clone();
            }
        }
        for (n, sigs) in d.functions.iter_mut() {
            if !untyped_sigs(sigs) {
                continue;
            }
            if let Some(other) = typed_functions.get(n)
                && other.iter().all(|s| fits(&model::fn_type(s)))
            {
                *sigs = other.clone();
            }
        }
    }
    // The aura filter words each flavor's UI code accepts (wowlua:aura
    // checks literal filters against them); a flavor without the table
    // takes the words every other flavor has.
    let words: Vec<Vec<String>> = flavors
        .iter()
        .map(|f| framexml::aura_filter_words(&f.ui_lua))
        .collect();
    let known: Vec<&Vec<String>> = words.iter().filter(|w| !w.is_empty()).collect();
    let fallback: Vec<String> = known
        .first()
        .map(|first| {
            first
                .iter()
                .filter(|w| known.iter().all(|k| k.contains(w)))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for ((_, _, d), w) in per_flavor.iter_mut().zip(&words) {
        let w = if w.is_empty() { &fallback } else { w };
        if !w.is_empty() {
            let union: Vec<String> = w.iter().map(|x| format!("{x:?}")).collect();
            d.aliases.insert("AuraFilterWord".into(), union.join(" | "));
        }
    }
    let shared_defs = shared.defined_types();
    let shared_refs = shared.referenced_types();
    let mut out_flavors = Vec::new();
    for ((id, source, mut d), mixins) in per_flavor.into_iter().zip(ui_mixins) {
        // The mixin classes of the UI code's named frames, where nothing
        // else declares the name (not the engine's: they are no widgets).
        let mut mixin_names: HashSet<String> = HashSet::new();
        // Only the mixins something refers to (a type in the declarations,
        // a base or a member type of a mixin kept), not every table with
        // functions: they cost every check its parse.
        let mut wanted: BTreeSet<String> = shared_refs.clone();
        wanted.extend(d.referenced_types());
        let mut kept: HashSet<String> = HashSet::new();
        loop {
            let mut grew = false;
            for (name, class) in &mixins {
                if kept.contains(name) || !wanted.contains(name) {
                    continue;
                }
                kept.insert(name.clone());
                grew = true;
                let mut one = Decls::default();
                one.classes.insert(name.clone(), class.clone());
                wanted.extend(one.referenced_types());
                wanted.extend(class.bases.iter().chain(&class.also).cloned());
            }
            if !grew {
                break;
            }
        }
        for (name, mut class) in mixins {
            if !kept.contains(&name) {
                continue;
            }
            mixin_corrections(&name, &mut class);
            if !shared_defs.contains(&name)
                && !d.classes.contains_key(&name)
                && !d.interfaces.contains_key(&name)
                && !d.aliases.contains_key(&name)
            {
                mixin_names.insert(name.clone());
                d.classes.insert(name, class);
            }
        }
        // Names referred to but declared nowhere become `any` (the
        // compiler's own declarations define some).
        let mut defined: BTreeSet<String> = shared_defs.clone();
        defined.extend(d.defined_types());
        defined.extend(compiler_types.iter().cloned());
        let mut refs = shared_refs.clone();
        refs.extend(d.referenced_types());
        for name in refs.difference(&defined) {
            if model::is_ident(name) {
                // Blizzard's documentation names an object's type without
                // `Object` where it returns one (`DurationTextBinding`)
                let object = format!("{name}Object");
                let ty = if defined.contains(&object) {
                    object
                } else {
                    "any".into()
                };
                d.aliases.entry(name.clone()).or_insert(ty);
            }
        }
        let header = format!(
            "wow-api declarations for the `{id}` flavor, generated by wowlua-types-gen from\nBlizzard's API documentation ({source}). Do not edit."
        );
        // The engine's data: the documentation, and each class's bases and
        // `CreateFrame` type (from the annotations; the documentation has
        // no inheritance).
        if let Some((_, json)) = api.iter_mut().find(|(f, _)| *f == id) {
            let systems = std::mem::take(json);
            let classes: serde_json::Map<String, serde_json::Value> = shared
                .classes
                .iter()
                .chain(d.classes.iter())
                .filter(|(name, _)| !mixin_names.contains(*name))
                .map(|(name, c)| {
                    (
                        name.clone(),
                        serde_json::json!({
                            "bases": c.bases.iter().chain(c.also.iter()).collect::<Vec<_>>(),
                            "widget": c.widget,
                        }),
                    )
                })
                .collect();
            // Every event the client knows, documented or not.
            let mut events: Vec<&String> = d.events.keys().collect();
            if let Some(names) = flavors
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| f.event_names.as_ref())
            {
                events.extend(names.iter());
            }
            events.sort();
            events.dedup();
            // The UI code's templates, for `CreateFrame` without the code
            // loaded.
            let templates = flavors
                .iter()
                .find(|f| f.id == id)
                .map(|f| framexml::templates(&f.ui_xml))
                .unwrap_or_default();
            *json = serde_json::json!({
                "systems": systems,
                "classes": classes,
                "events": events,
                "templates": templates,
            });
        }
        recorded_corrections(&mut d);
        client_types(&mut d, client, id == client.flavor);
        d.without_any();
        d.docs = flavor_docs.remove(&id).unwrap_or_else(|| wiki_docs.clone());
        out_flavors.push((id, d.emit(&header)));
    }
    let header = format!(
        "wow-api declarations shared by every flavor, generated by wowlua-types-gen from\n{annotation_source}. Do not edit."
    );
    recorded_corrections(&mut shared);
    client_types(&mut shared, client, true);
    shared.without_any();
    shared.docs = wiki_docs.clone();
    docs::add_written(&mut shared.docs, written);
    Ok(Output {
        shared: shared.emit(&header),
        flavors: out_flavors,
        api,
    })
}

fn create_frame_sig(frame_type: &str, result: &str) -> model::Sig {
    let p = |name: &str, ty: &str, optional: bool| model::ParamDecl {
        name: name.into(),
        ty: ty.into(),
        optional,
    };
    model::Sig {
        params: vec![
            p("frameType", frame_type, false),
            p("name", "string", true),
            p("parent", "any", true),
            p("template", "string", true),
            p("id", "number", true),
        ],
        returns: vec![result.into()],
        ..model::Sig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn untyped(params: &[&str]) -> Vec<model::Sig> {
        vec![model::Sig {
            params: params
                .iter()
                .map(|n| model::ParamDecl {
                    name: (*n).into(),
                    ty: "any".into(),
                    optional: false,
                })
                .collect(),
            ret_vararg: Some("any".into()),
            ..model::Sig::default()
        }]
    }

    /// A status bar's value and range read back secret, its scripts get
    /// them so, and its `OnValueChanged` no `userInput`; a slider's keeps it.
    #[test]
    fn a_status_bars_value_may_be_secret() {
        let sig = |params: Vec<model::ParamDecl>, returns: &[&str]| model::Sig {
            params,
            returns: returns.iter().map(|r| r.to_string()).collect(),
            ..model::Sig::default()
        };
        let script = |ty: &str| {
            vec![
                model::ParamDecl {
                    name: "scriptType".into(),
                    ty: ty.into(),
                    optional: false,
                },
                model::ParamDecl {
                    name: "handler".into(),
                    ty: "function".into(),
                    optional: true,
                },
            ]
        };
        let mut classes = indexmap::IndexMap::new();
        for (name, alias) in [("StatusBar", "ScriptStatusBar"), ("Slider", "ScriptSlider")] {
            let mut c = model::Class::default();
            c.methods
                .insert("GetValue".into(), vec![sig(vec![], &["number"])]);
            c.methods.insert(
                "GetMinMaxValues".into(),
                vec![sig(vec![], &["number", "number"])],
            );
            c.methods
                .insert("SetScript".into(), vec![sig(script(alias), &[])]);
            classes.insert(name.to_string(), c);
        }
        let mut aliases = indexmap::IndexMap::new();
        for alias in ["ScriptStatusBar", "ScriptSlider"] {
            aliases.insert(
                alias.to_string(),
                "\"OnValueChanged\" | \"OnMinMaxChanged\"".to_string(),
            );
        }
        widget_corrections(&mut classes);
        script_handlers(&mut classes, &aliases);
        let text = |class: &str, m: &str, i: usize| model::sig_text(&classes[class].methods[m][i]);
        assert_eq!(
            text("StatusBar", "GetValue", 0),
            "(): number | Secret<number>"
        );
        assert_eq!(
            text("StatusBar", "GetMinMaxValues", 0),
            "(): (number | Secret<number>, number | Secret<number>)"
        );
        assert_eq!(
            text("StatusBar", "SetScript", 0),
            "(scriptType: \"OnValueChanged\", handler: (((self: Self, value: number | Secret<number>) -> ()))?)"
        );
        assert_eq!(
            text("StatusBar", "SetScript", 1),
            "(scriptType: \"OnMinMaxChanged\", handler: (((self: Self, min: number | Secret<number>, max: number | Secret<number>) -> ()))?)"
        );
        assert_eq!(text("Slider", "GetValue", 0), "(): number");
        assert_eq!(
            text("Slider", "SetScript", 0),
            "(scriptType: \"OnValueChanged\", handler: (((self: Self, value: number, userInput: boolean) -> ()))?)"
        );
    }

    #[test]
    fn message_frame_mixins_are_typed() {
        let mut class = model::Class::default();
        class
            .methods
            .insert("AddMessage".into(), untyped(&["message", "r", "g", "b"]));
        class.methods.insert("GetScrollOffset".into(), untyped(&[]));
        class.methods.insert("RefreshLayout".into(), untyped(&[]));
        mixin_corrections("ScrollingMessageFrameMixin", &mut class);
        let text = |m: &str| model::sig_text(&class.methods[m][0]);
        assert_eq!(
            text("AddMessage"),
            "(message: string | Secret<string>, r: (number | Secret<number>)?, g: (number | Secret<number>)?, b: (number | Secret<number>)?, ...: unknown)"
        );
        assert_eq!(text("GetScrollOffset"), "(): number");
        // its other functions stay as the Lua defines them, and none is added
        assert_eq!(text("RefreshLayout"), "(): (...any)");
        assert!(!class.methods.contains_key("ScrollUp"));
        // the font methods on the font mixin; another mixin is left alone
        let mut fontable = model::Class::default();
        fontable.methods.insert(
            "SetFont".into(),
            untyped(&["font", "fontHeight", "fontFlags"]),
        );
        mixin_corrections("FontableFrameMixin", &mut fontable);
        assert_eq!(
            model::sig_text(&fontable.methods["SetFont"][0]),
            "(font: FontFile, fontHeight: uiFontHeight, fontFlags: TBFFlags?)"
        );
        let mut other = model::Class::default();
        other.methods.insert("GetFont".into(), untyped(&[]));
        mixin_corrections("ChatFrameMixin", &mut other);
        assert_eq!(
            model::sig_text(&other.methods["GetFont"][0]),
            "(): (...any)"
        );
    }

    #[test]
    fn the_clients_answers_type_undocumented_functions() {
        assert_eq!(
            answer_types("2: number 0, string a, b").unwrap(),
            ["number", "string"]
        );
        assert_eq!(
            answer_types("3: {1=number 1,2=string x}, nil nil, boolean true").unwrap(),
            ["table", "unknown", "boolean"]
        );
        assert_eq!(answer_types("0: ").unwrap(), Vec::<String>::new());
        let untyped = || model::Sig {
            vararg: Some("any".into()),
            ret_vararg: Some("any".into()),
            ..model::Sig::default()
        };
        let mut d = Decls::default();
        for name in [
            "GetTradeThing",
            "IsChatAFK",
            "GetNumFlyouts",
            "StartAutoRun",
            "HasInboxItem",
        ] {
            d.functions.insert(name.into(), vec![untyped()]);
        }
        let client = ClientFacts {
            flavor: "camelot".into(),
            usages: vec![(
                "HasInboxItem".into(),
                vec![
                    ("messageIndex".into(), "required".into(), false),
                    ("attachIndex".into(), "number".into(), true),
                ],
            )],
            answers: vec![("GetTradeThing".into(), "2: number 0, nil nil".into())],
        };
        client_types(&mut d, &client, true);
        let text = |n: &str| model::sig_text(&d.functions[n][0]);
        assert_eq!(text("GetTradeThing"), "(...: unknown): (number, unknown)");
        assert_eq!(text("IsChatAFK"), "(...: unknown): boolean");
        assert_eq!(text("GetNumFlyouts"), "(...: unknown): number");
        assert_eq!(text("StartAutoRun"), "(...: any): (...any)");
        assert_eq!(
            text("HasInboxItem"),
            "(messageIndex: unknown, attachIndex: number?): boolean"
        );
    }
}
