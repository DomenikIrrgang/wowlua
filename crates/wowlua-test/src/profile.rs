//! World profiles (spec Tooling §16.1.4, decision T213): JSON files under
//! `tests/profiles/` with the whole situation a test starts in, applied
//! through the engine's controls before the addon loads. A test file names
//! its profile with `config({ profile = "name" })` (read from its syntax,
//! since the profile comes first), else `test.profile` of the manifest.

use std::path::Path;
use wowlua_syntax::view::*;
use wowlua_vm::{TableRef, Taint, Value, Vm};

/// The profile a test file's `config({ profile = "…" })` names.
pub fn named_in(source: &[u8]) -> Option<String> {
    config_text(source, "profile")
}

/// A literal text a test file's `config({ … })` gives `key` (read from its
/// syntax, before anything loads: `profile`, `platform`).
pub fn config_text(source: &[u8], key: &str) -> Option<String> {
    wowlua_driver::profiles::config_text(source, key).map(|(text, _)| text)
}

/// The number literals of the table a test file's `config({ … })` gives
/// `key` (read from its syntax before anything loads: `screen = { width =
/// 1600, height = 900 }`).
pub fn config_numbers(source: &[u8], key: &str) -> Option<Vec<(String, f64)>> {
    let file = wowlua_span::SourceDb::new().add("", "");
    let parse = wowlua_syntax::parse(source, file);
    parse.view().block().stmts().iter().find_map(|s| {
        let StmtKind::Call(call) = s.kind() else {
            return None;
        };
        let ExprKind::Call { callee, args } = call.kind() else {
            return None;
        };
        if !matches!(callee.kind(), ExprKind::Name(n) if n.text() == "config") {
            return None;
        }
        let ExprKind::Table(fields) = args.exprs().first()?.kind() else {
            return None;
        };
        fields.iter().find_map(|f| match f {
            TableField::Named { name, value, .. } if name.text() == key => match value.kind() {
                ExprKind::Table(inner) => Some(
                    inner
                        .iter()
                        .filter_map(|f| match f {
                            TableField::Named { name, value, .. } => match value.kind() {
                                ExprKind::Number(n) => Some((name.text().to_string(), n.value()?)),
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            },
            _ => None,
        })
    })
}

/// A JSON value as Lua's: objects and arrays become tables (object keys
/// that are whole numbers become number keys), `null` nil; an array with
/// nulls keeps its length in `n` (a recorded answer).
fn to_lua(vm: &mut Vm, v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => vm.str(s),
        serde_json::Value::Array(list) => {
            let t = vm.new_table();
            for (i, item) in list.iter().enumerate() {
                let v = to_lua(vm, item);
                let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), v);
            }
            if list.iter().any(serde_json::Value::is_null) {
                let n = vm.str("n");
                let _ = vm.heap.table_mut(t).set(n, Value::Num(list.len() as f64));
            }
            Value::Table(t)
        }
        serde_json::Value::Object(map) => {
            let t = vm.new_table();
            for (k, item) in map {
                let key = match k.parse::<i64>() {
                    Ok(n) => Value::Num(n as f64),
                    Err(_) => vm.str(k),
                };
                let v = to_lua(vm, item);
                let _ = vm.heap.table_mut(t).set(key, v);
            }
            Value::Table(t)
        }
    }
}

/// The world of a test that names no profile: the flavor's fresh
/// character (`wowlua_manifest::world::fresh_character`).
pub fn default_for(flavor: &str) -> Option<&'static str> {
    wowlua_manifest::world::fresh_character(flavor)
}

/// A profile a package names: its own `tests/profiles/<name>.json`, else
/// wowlua's built-in of that name for the flavor (decision T533); the
/// text and how to call it in an error.
pub fn named(root: &std::path::Path, flavor: &str, name: &str) -> Result<(String, String), String> {
    let path = root
        .join(wowlua_manifest::world::PROFILES_DIR)
        .join(format!("{name}.json"));
    match wowlua_host::read_to_string(&path) {
        Ok(text) => Ok((text, path.display().to_string())),
        Err(e) => wowlua_manifest::world::built_in(flavor, name)
            .map(|text| (text, format!("built in `{name}`")))
            .ok_or_else(|| format!("the world profile {}: {e}", path.display())),
    }
}

/// The screen a profile's `screen` part sets (decision T482), when it has
/// one (the part applies before the addon loads, not with the rest).
pub fn screen_of(text: &str, label: &str) -> Result<Option<wowlua_engine::Screen>, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("the world profile {label}: {e}"))?;
    let Some(part) = json.get("screen") else {
        return Ok(None);
    };
    let s: wowlua_manifest::model::TestScreen = serde_json::from_value(part.clone())
        .map_err(|e| format!("the world profile {label}: screen: {e}"))?;
    Ok(Some(wowlua_engine::Screen {
        width: s.width,
        height: s.height,
        dpi_scale: s.dpi_scale.unwrap_or(1.0),
        ui_scale: s.ui_scale,
    }))
}

/// Applies the profile in `path` with the engine's `controls`.
pub fn apply(vm: &mut Vm, controls: TableRef, path: &Path) -> Result<(), String> {
    let text = wowlua_host::read_to_string(path)
        .map_err(|e| format!("the world profile {}: {e}", path.display()))?;
    apply_text(vm, controls, &text, &path.display().to_string())
}

/// Applies a profile given as JSON text (`label` names it in errors).
pub fn apply_text(vm: &mut Vm, controls: TableRef, text: &str, label: &str) -> Result<(), String> {
    let path = label;
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("the world profile {path}: {e}"))?;
    if !json.is_object() {
        return Err(format!("the world profile {path} is not a JSON object"));
    }
    let profile = to_lua(vm, &json);
    crate::widgets_keep(vm, profile);
    // The world as it was: set untainted, as the client's own state (a
    // tainted SetCVar of a secure one is forbidden: Blizzard's popup).
    let saved = *vm.taint();
    {
        let t = vm.taint_mut();
        t.stack = Taint::SECURE;
        t.fixed = Taint::SECURE;
        t.new_object = Taint::SECURE;
        t.new_closure = Taint::SECURE;
    }
    let f = match vm.load(include_bytes!("profile.lua"), "=[world profile]") {
        Ok(f) => f,
        Err(e) => {
            *vm.taint_mut() = saved;
            return Err(e.to_string());
        }
    };
    let ran = vm.call(f, &[Value::Table(controls), profile]);
    *vm.taint_mut() = saved;
    ran.map(|_| ()).map_err(|e| {
        let msg = match e.value {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => "?".into(),
        };
        format!("the world profile {path}: {msg}")
    })
}
