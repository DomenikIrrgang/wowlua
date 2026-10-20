//! Blizzard's API documentation (`Blizzard_APIDocumentationGenerated`):
//! the documentation tables are executed in a sandboxed Lua VM and turned
//! into declarations (spec package manager §10.9).

use crate::model::{Class, Decls, ParamDecl, Sig, optional};
use mlua::{Lua, LuaOptions, StdLib, Table, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Reads documentation files (name, Lua source) into declarations.
pub fn read(files: &[(String, String)]) -> Result<Decls, String> {
    let (_lua, docs) = load(files)?;
    let docs: Vec<Table> = docs
        .sequence_values::<Table>()
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    // The structures, each with its fields documented `NeverSecret`.
    let mut structures = Structures::default();
    for doc in &docs {
        for t in list(doc, "Tables") {
            if s(&t, "Type").as_deref() != Some("Structure") {
                continue;
            }
            let Some(name) = s(&t, "Name") else { continue };
            let never = list(&t, "Fields")
                .iter()
                .filter(|f| b(f, "NeverSecret"))
                .filter_map(|f| s(f, "Name"))
                .collect();
            structures.never_secret.insert(name, never);
        }
    }
    let mut out = Decls::default();
    for doc in &docs {
        read_doc(doc, &mut out, &mut structures);
    }
    // A structure a possibly secret function returns stays a readable
    // table: its fields are secret but those documented `NeverSecret`
    // (`C_ActionBar.GetActionCooldown(1)` in combat answers `{duration=
    // secret number, isActive=boolean false, …}`, the quest scenario's
    // recording; the engine answers the same).
    // Lists stay readable, their structures' fields secret (the damage
    // meter's sessions: Details! reads `issecretvalue(session.
    // combatSources[1].name)`; unverified by a recording).
    let mut pending: Vec<String> = structures.secret_fields.iter().cloned().collect();
    let mut done: HashSet<String> = HashSet::new();
    while let Some(name) = pending.pop() {
        if !done.insert(name.clone()) {
            continue;
        }
        let never = &structures.never_secret[&name];
        let Some(fields) = out.interfaces.get_mut(&name) else {
            continue;
        };
        for (field, ty, _) in fields.iter_mut() {
            if never.contains(field) || ty.contains("Secret<") {
                continue;
            }
            if let Some(element) = ty.strip_suffix("[]") {
                if structures.never_secret.contains_key(element) {
                    pending.push(element.to_string());
                } else if !element.contains(['(', '|']) {
                    *ty = format!("({element} | Secret<{element}>)[]");
                }
                continue;
            }
            *ty = format!("{ty} | Secret<{ty}>");
        }
    }
    Ok(out)
}

/// What the documentation says about its structures' secrecy.
#[derive(Default)]
struct Structures {
    /// Each structure's fields documented `NeverSecret`.
    never_secret: HashMap<String, HashSet<String>>,
    /// The structures possibly secret functions return.
    secret_fields: BTreeSet<String>,
}

/// The documentation tables as they are, every key kept (for the engine,
/// spec emulator §17.6): a list of systems, each with its `Functions`,
/// `Events` and `Tables`. Object keys are sorted; references to enum values
/// the sandbox does not know become null.
pub fn read_json(files: &[(String, String)]) -> Result<serde_json::Value, String> {
    let (_lua, docs) = load(files)?;
    let mut json = to_json(&Value::Table(docs))?;
    // Each widget system names the class its functions are methods of.
    if let Some(systems) = json.as_array_mut() {
        for s in systems {
            let is_widget = s.get("Type").and_then(|t| t.as_str()) == Some("ScriptObject");
            let name = s
                .get("Name")
                .and_then(|n| n.as_str())
                .map(script_object_class);
            if let (true, Some(class), Some(obj)) = (is_widget, name, s.as_object_mut()) {
                obj.insert("Class".into(), serde_json::Value::String(class));
            }
        }
    }
    Ok(json)
}

fn to_json(v: &Value) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    Ok(match v {
        Value::Nil => J::Null,
        Value::Boolean(b) => J::Bool(*b),
        Value::Integer(i) => J::from(*i),
        Value::Number(n) => serde_json::Number::from_f64(*n).map_or(J::Null, J::Number),
        Value::String(s) => J::String(s.to_string_lossy()),
        // An enum value the sandbox stands in for.
        Value::Table(t) if t.metatable().is_some() => J::Null,
        Value::Table(t) => {
            let len = t.raw_len();
            let mut entries: Vec<(String, J)> = Vec::new();
            let mut count = 0;
            for pair in t.clone().pairs::<Value, Value>() {
                let (k, v) = pair.map_err(|e| e.to_string())?;
                count += 1;
                let key = match &k {
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                entries.push((key, to_json(&v)?));
            }
            if len > 0 && count == len {
                let mut items = Vec::with_capacity(len);
                for i in 1..=len {
                    items.push(to_json(&t.raw_get::<Value>(i).map_err(|e| e.to_string())?)?);
                }
                J::Array(items)
            } else {
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                J::Object(entries.into_iter().collect())
            }
        }
        _ => J::Null,
    })
}

/// Runs the documentation files in a sandbox: the Lua state and the list
/// of documentation tables they added.
fn load(files: &[(String, String)]) -> Result<(Lua, Table), String> {
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    lua.load(
        r#"
        __docs = {}
        APIDocumentation = { AddDocumentationTable = function(_, t) __docs[#__docs + 1] = t end }
        -- Documentation tables refer to enum values (`Enum.SecretAspect.X`).
        -- Unknown values: tables that also take part in arithmetic.
        local function number() return 0 end
        local meta
        local function names(prefix)
            return setmetatable({}, meta)
        end
        meta = {
            __index = function(t, k)
                local v = names()
                rawset(t, k, v)
                return v
            end,
            __add = number, __sub = number, __mul = number, __div = number,
            __mod = number, __unm = number, __pow = number,
            __concat = function() return "" end,
        }
        Enum = names("Enum")
        Constants = names("Constants")
        "#,
    )
    .exec()
    .map_err(|e| e.to_string())?;
    for (name, source) in files {
        lua.load(source.as_str())
            .set_name(name.as_str())
            .exec()
            .map_err(|e| format!("{name}: {e}"))?;
    }
    let docs: Table = lua.globals().get("__docs").map_err(|e| e.to_string())?;
    Ok((lua, docs))
}

fn s(t: &Table, key: &str) -> Option<String> {
    match t.get::<Value>(key).ok()? {
        Value::String(v) => Some(v.to_string_lossy()),
        _ => None,
    }
}

fn b(t: &Table, key: &str) -> bool {
    matches!(t.get::<Value>(key), Ok(Value::Boolean(true)))
}

fn list(t: &Table, key: &str) -> Vec<Table> {
    match t.get::<Value>(key) {
        Ok(Value::Table(l)) => l
            .sequence_values::<Table>()
            .filter_map(Result::ok)
            .collect(),
        _ => Vec::new(),
    }
}

fn read_doc(doc: &Table, out: &mut Decls, structures: &mut Structures) {
    let kind = s(doc, "Type").unwrap_or_default();
    let namespace = s(doc, "Namespace");
    let name = s(doc, "Name").unwrap_or_default();
    for f in list(doc, "Functions") {
        let Some(fname) = s(&f, "Name") else { continue };
        let sig = function_sig(&f, structures);
        match (&namespace, kind.as_str()) {
            (_, "ScriptObject") => {
                let class = script_object_class(&name);
                out.classes
                    .entry(class)
                    .or_default()
                    .methods
                    .entry(fname)
                    .or_default()
                    .push(sig);
            }
            (Some(ns), _) => {
                out.namespaces
                    .entry(ns.clone())
                    .or_default()
                    .functions
                    .entry(fname)
                    .or_default()
                    .push(sig);
            }
            (None, _) => out.functions.entry(fname).or_default().push(sig),
        }
    }
    for e in list(doc, "Events") {
        let Some(literal) = s(&e, "LiteralName") else {
            continue;
        };
        let params = list(&e, "Payload").iter().map(param).collect();
        out.events.insert(literal, params);
    }
    for t in list(doc, "Tables") {
        let Some(tname) = s(&t, "Name") else { continue };
        match s(&t, "Type").as_deref() {
            Some("Structure") => {
                let fields = list(&t, "Fields")
                    .iter()
                    .filter_map(|f| {
                        let n = s(f, "Name")?;
                        let optional = b(f, "Nilable") || has(f, "Default");
                        Some((n, field_type(f), optional))
                    })
                    .collect();
                out.interfaces.insert(tname, fields);
            }
            Some("Enumeration") => {
                let members: Vec<(String, f64)> = list(&t, "Fields")
                    .iter()
                    .filter_map(|f| {
                        let n = s(f, "Name")?;
                        let v: f64 = f.get("EnumValue").ok()?;
                        Some((n, v))
                    })
                    .collect();
                out.enums.insert(tname.clone(), members);
                out.aliases.insert(tname, "number".into());
            }
            Some("Constants") => {
                let values: Vec<(String, String)> = list(&t, "Values")
                    .iter()
                    .filter_map(|v| {
                        let n = s(v, "Name")?;
                        // The type grammar has no negative literals.
                        let ty = match v.get::<Value>("Value") {
                            Ok(Value::Integer(i)) if i >= 0 => i.to_string(),
                            Ok(Value::Number(x)) if x.is_finite() && x >= 0.0 => format!("{x}"),
                            Ok(Value::Integer(_) | Value::Number(_)) => "number".into(),
                            Ok(Value::String(st)) => format!("{:?}", st.to_string_lossy()),
                            _ => map_type(&s(v, "Type").unwrap_or_default()),
                        };
                        Some((n, ty))
                    })
                    .collect();
                out.constants.insert(tname, values);
            }
            Some("CallbackType") => {
                let params: Vec<String> = list(&t, "Arguments")
                    .iter()
                    .map(|a| param_text(&param(a)))
                    .collect();
                out.aliases
                    .insert(tname, format!("({}) -> ()", params.join(", ")));
            }
            _ => {}
        }
    }
}

fn has(t: &Table, key: &str) -> bool {
    !matches!(t.get::<Value>(key), Ok(Value::Nil) | Err(_))
}

/// `FrameAPICooldown` → `Cooldown`, `SimpleFrameAPI` → `Frame`.
pub fn script_object_class(name: &str) -> String {
    let n = name.strip_suffix("API").unwrap_or(name);
    if let Some(rest) = n.strip_prefix("FrameAPI") {
        return rest.to_string();
    }
    let n = n.strip_prefix("Simple").unwrap_or(n);
    let n = n.strip_prefix("Lua").unwrap_or(n);
    match n {
        "AnimGroup" => "AnimationGroup".into(),
        "Anim" => "Animation".into(),
        "AnimAlpha" => "Alpha".into(),
        "AnimRotation" => "Rotation".into(),
        "AnimTranslation" => "Translation".into(),
        "AnimScale" => "Scale".into(),
        "AnimFlipBook" => "FlipBook".into(),
        "AnimPath" => "Path".into(),
        "AnimLineScale" => "LineScale".into(),
        "AnimLineTranslation" => "LineTranslation".into(),
        "AnimTextureCoordTranslation" => "TextureCoordTranslation".into(),
        "AnimVertexColor" => "VertexColor".into(),
        "ControlPoint" => "ControlPoint".into(),
        "FontString" => "FontString".into(),
        "Texture" => "Texture".into(),
        "MaskTexture" => "MaskTexture".into(),
        "Line" => "Line".into(),
        "Font" => "Font".into(),
        "EditBox" => "EditBox".into(),
        "MessageFrame" => "MessageFrame".into(),
        "HTML" => "SimpleHTML".into(),
        "MinimapFrame" | "Minimap" => "Minimap".into(),
        other => other.to_string(),
    }
}

fn function_sig(f: &Table, structures: &mut Structures) -> Sig {
    // Entries with `StrideIndex` repeat: they become the variadic tail.
    let args = list(f, "Arguments");
    let fixed_args: Vec<&Table> = args.iter().take_while(|a| !has(a, "StrideIndex")).collect();
    // `SecretArguments = "AllowedWhenTainted"`: addon code may pass
    // secrets (to the arguments not marked `NeverSecret`).
    let secret_args = s(f, "SecretArguments").as_deref() == Some("AllowedWhenTainted");
    let accept = |a: &Table, ty: String| {
        // `any` takes secrets already.
        if secret_args && !b(a, "NeverSecret") && ty != "any" {
            format!("{ty} | Secret<{ty}>")
        } else {
            ty
        }
    };
    let stride_args: Vec<String> = args
        .iter()
        .skip(fixed_args.len())
        .map(|a| accept(a, field_type(a)))
        .collect();
    let params: Vec<ParamDecl> = fixed_args
        .iter()
        .map(|a| {
            let mut p = param(a);
            p.ty = accept(a, p.ty);
            p
        })
        .collect();
    let vararg = (!stride_args.is_empty()).then(|| stride_args.join(" | "));
    let secret = b(f, "SecretReturns")
        || has(f, "SecretReturnsForAspect")
        || f.pairs::<String, Value>()
            .filter_map(Result::ok)
            .any(|(k, v)| k.starts_with("SecretWhen") && matches!(v, Value::Boolean(true)));
    let may_return_nothing = b(f, "MayReturnNothing");
    let all_returns = list(f, "Returns");
    let ret_vararg = all_returns.iter().any(|r| has(r, "StrideIndex")).then(|| {
        all_returns
            .iter()
            .filter(|r| has(r, "StrideIndex"))
            .map(field_type)
            .collect::<Vec<_>>()
            .join(" | ")
    });
    let returns: Vec<String> = all_returns
        .iter()
        .take_while(|r| !has(r, "StrideIndex"))
        .map(|r| {
            let mut ty = field_type(r);
            if secret && structures.never_secret.contains_key(&ty) {
                structures.secret_fields.insert(ty.clone());
            } else if secret && ty != "any" {
                ty = format!("{ty} | Secret<{ty}>");
            }
            if b(r, "Nilable") || may_return_nothing {
                ty = optional(&ty);
            }
            ty
        })
        .collect();
    Sig {
        params,
        vararg,
        returns,
        ret_vararg,
        ..Sig::default()
    }
}

fn param(a: &Table) -> ParamDecl {
    ParamDecl {
        name: s(a, "Name").unwrap_or_else(|| "arg".into()),
        ty: field_type(a),
        optional: b(a, "Nilable") || has(a, "Default"),
    }
}

fn param_text(p: &ParamDecl) -> String {
    format!(
        "{}: {}",
        crate::model::ident(&p.name),
        if p.optional {
            optional(&p.ty)
        } else {
            p.ty.clone()
        }
    )
}

/// The type of an argument, return or field entry.
fn field_type(t: &Table) -> String {
    if let Some(m) = s(t, "Mixin") {
        return m;
    }
    let ty = s(t, "Type").unwrap_or_else(|| "any".into());
    if ty == "table"
        && let Some(inner) = s(t, "InnerType")
    {
        let inner = map_type(&inner);
        return if inner.contains('|') {
            format!("({inner})[]")
        } else {
            format!("{inner}[]")
        };
    }
    map_type(&ty)
}

/// Blizzard type names → wowlua types.
pub fn map_type(ty: &str) -> String {
    match ty {
        // Big integers are numbers too (`C_AuctionHouse.GetCancelCost(1)`
        // answers `number 0`, the full probe), and so are UI sizes
        // (`uiUnit`: `SetPoint`'s offsets).
        "number" | "luaIndex" | "time_t" | "size_t" | "uint32" | "int32" | "uint64" | "int"
        | "double" | "float" | "fileID" | "SingleColorValue" | "BigUInteger" | "BigInteger"
        | "uiUnit" => "number".into(),
        "bool" | "boolean" => "boolean".into(),
        "cstring"
        | "string"
        | "mouseButton"
        | "kstringClubId"
        | "kstringLfgListApplicant"
        | "kstringLfgListSearch"
        | "ModelAsset"
        | "textureAtlas"
        | "textureKit"
        | "WOWGUID"
        | "UnitToken"
        | "UnitTokenVariant"
        | "UnitTokenType"
        | "UnitTokenPvPRestrictedForAddOns" => "string".into(),
        "table" => "table".into(),
        "function" => "function".into(),
        // Any Lua value (`issecrettable(table)`, `mapvalues(func, ...)`).
        "LuaValueReference" => "any".into(),
        "userdata" => "userdata".into(),
        "" => "any".into(),
        other => {
            // Widget references: `SimpleFrame` → `Frame`.
            if let Some(rest) = other.strip_prefix("Simple")
                && rest.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            {
                return script_object_class(&format!("Simple{rest}API"));
            }
            other.to_string()
        }
    }
}

/// Marks the classes of every frame type `CreateFrame` creates as widgets.
pub fn mark_widgets(classes: &mut indexmap::IndexMap<String, Class>) {
    for name in FRAME_TYPES {
        if let Some(c) = classes.get_mut(*name) {
            c.widget = Some((*name).to_string());
        }
    }
}

/// Frame types `CreateFrame` accepts.
pub const FRAME_TYPES: &[&str] = &[
    "Frame",
    "ArchaeologyDigSiteFrame",
    "Browser",
    "Button",
    "CheckButton",
    "Checkout",
    "CinematicModel",
    "ColorSelect",
    "Cooldown",
    "DressUpModel",
    "EditBox",
    "FogOfWarFrame",
    "GameTooltip",
    "ItemButton",
    "MessageFrame",
    "Minimap",
    "Model",
    "ModelScene",
    "MovieFrame",
    "OffScreenFrame",
    "PlayerModel",
    "QuestPOIFrame",
    "ScenarioPOIFrame",
    "ScrollFrame",
    "ScrollingMessageFrame",
    "SimpleHTML",
    "Slider",
    "StatusBar",
    "TabardModel",
    "UnitPositionFrame",
];

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"
APIDocumentation:AddDocumentationTable({
    Name = "Spell", Type = "System", Namespace = "C_Spell",
    Functions = {
        { Name = "GetSpellCooldown", Type = "Function", MayReturnNothing = true,
          SecretWhenCooldownsRestricted = true,
          Arguments = { { Name = "spellID", Type = "number", Nilable = false } },
          Returns = { { Name = "info", Type = "SpellCooldownInfo", Nilable = false } } },
        { Name = "GetSpellName", Type = "Function", SecretReturns = true,
          Arguments = { { Name = "spellID", Type = "number", Nilable = false } },
          Returns = { { Name = "name", Type = "cstring", Nilable = false } } },
        { Name = "GetCost", Type = "Function",
          Arguments = { { Name = "width", Type = "uiUnit", Nilable = false } },
          Returns = { { Name = "cost", Type = "BigUInteger", Nilable = false } } },
        { Name = "IsTable", Type = "Function",
          Arguments = { { Name = "table", Type = "LuaValueReference", Nilable = false } },
          Returns = { { Name = "yes", Type = "bool", Nilable = false } } },
    },
    Tables = {
        { Name = "SpellCooldownInfo", Type = "Structure", Fields = {
            { Name = "startTime", Type = "number", Nilable = false },
            { Name = "isActive", Type = "bool", Nilable = false, NeverSecret = true },
        } },
    },
})
"#;

    /// A possibly secret function's structure is a readable table with
    /// secret fields, but those documented `NeverSecret`; other results
    /// are wholly secret. Big integers and UI sizes are numbers, a Lua
    /// value reference any value.
    #[test]
    fn secret_structures_and_number_types() {
        let d = read(&[("doc.lua".into(), DOC.into())]).unwrap();
        let ns = &d.namespaces["C_Spell"].functions;
        assert_eq!(ns["GetSpellCooldown"][0].returns, ["SpellCooldownInfo?"]);
        assert_eq!(ns["GetSpellName"][0].returns, ["string | Secret<string>"]);
        assert_eq!(
            d.interfaces["SpellCooldownInfo"],
            [
                (
                    "startTime".to_string(),
                    "number | Secret<number>".to_string(),
                    false
                ),
                ("isActive".to_string(), "boolean".to_string(), false),
            ]
        );
        assert_eq!(ns["GetCost"][0].params[0].ty, "number");
        assert_eq!(ns["GetCost"][0].returns, ["number"]);
        assert_eq!(ns["IsTable"][0].params[0].ty, "any");
    }
}
