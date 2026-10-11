//! Blizzard's API documentation (`Blizzard_APIDocumentationGenerated`):
//! the documentation tables are executed in a sandboxed Lua VM and turned
//! into declarations (spec package manager §10.9).

use crate::model::{Class, Decls, ParamDecl, Sig, optional};
use mlua::{Lua, LuaOptions, StdLib, Table, Value};

/// Reads documentation files (name, Lua source) into declarations.
pub fn read(files: &[(String, String)]) -> Result<Decls, String> {
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
    let mut out = Decls::default();
    for doc in docs.sequence_values::<Table>() {
        let doc = doc.map_err(|e| e.to_string())?;
        read_doc(&doc, &mut out);
    }
    Ok(out)
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

fn read_doc(doc: &Table, out: &mut Decls) {
    let kind = s(doc, "Type").unwrap_or_default();
    let namespace = s(doc, "Namespace");
    let name = s(doc, "Name").unwrap_or_default();
    for f in list(doc, "Functions") {
        let Some(fname) = s(&f, "Name") else { continue };
        let sig = function_sig(&f);
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

fn function_sig(f: &Table) -> Sig {
    // Entries with `StrideIndex` repeat: they become the variadic tail.
    let args = list(f, "Arguments");
    let fixed_args: Vec<&Table> = args.iter().take_while(|a| !has(a, "StrideIndex")).collect();
    let stride_args: Vec<String> = args.iter().skip(fixed_args.len()).map(field_type).collect();
    let params: Vec<ParamDecl> = fixed_args.iter().map(|a| param(a)).collect();
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
            if secret {
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
        "number" | "luaIndex" | "time_t" | "size_t" | "uint32" | "int32" | "uint64" | "int"
        | "double" | "float" | "fileID" | "SingleColorValue" => "number".into(),
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
        | "uiUnit"
        | "UnitTokenVariant"
        | "UnitTokenType"
        | "UnitTokenPvPRestrictedForAddOns"
        | "BigUInteger"
        | "BigInteger" => "string".into(),
        "table" => "table".into(),
        "function" | "LuaValueReference" => "function".into(),
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
