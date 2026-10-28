//! `cargo xtask profile <StdTest.lua> <out.json>`: the world `/stdtest
//! snapshot` recorded (`StdTestState.snapshot` in the saved variables) as
//! a world profile for `wowlua test` (decision T213). The character's name
//! and realm, a player target's name, and CVars that may identify the
//! account are scrubbed.

use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;

/// A Lua value as JSON: a table with the keys 1..n is an array (as is one
/// with its length in `n`, its missing values null: a recorded answer),
/// other tables objects (sorted keys).
fn to_json(v: &mlua::Value) -> serde_json::Value {
    match v {
        mlua::Value::Nil => serde_json::Value::Null,
        mlua::Value::Boolean(b) => serde_json::Value::Bool(*b),
        mlua::Value::Integer(n) => serde_json::json!(n),
        mlua::Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                serde_json::json!(*n as i64)
            } else {
                serde_json::json!(n)
            }
        }
        mlua::Value::String(s) => serde_json::Value::String(s.to_string_lossy().to_string()),
        mlua::Value::Table(t) => {
            let mut pairs: Vec<(String, serde_json::Value, Option<i64>)> = Vec::new();
            for pair in t.clone().pairs::<mlua::Value, mlua::Value>() {
                let Ok((k, v)) = pair else { continue };
                let (key, index) = match &k {
                    mlua::Value::Integer(i) => (i.to_string(), Some(*i)),
                    mlua::Value::Number(n) if n.fract() == 0.0 => {
                        ((*n as i64).to_string(), Some(*n as i64))
                    }
                    mlua::Value::String(s) => (s.to_string_lossy().to_string(), None),
                    other => (format!("{other:?}"), None),
                };
                pairs.push((key, to_json(&v), index));
            }
            // A recorded answer: `n` values, nils among them.
            let length = pairs.iter().find_map(|(k, v, _)| match v {
                serde_json::Value::Number(n) if k == "n" => n.as_i64(),
                _ => None,
            });
            if let Some(len) = length
                && pairs
                    .iter()
                    .all(|(k, _, i)| k == "n" || i.is_some_and(|i| i >= 1 && i <= len))
            {
                let mut list = vec![serde_json::Value::Null; len.max(0) as usize];
                for (_, v, i) in pairs {
                    if let Some(i) = i {
                        list[i as usize - 1] = v;
                    }
                }
                return serde_json::Value::Array(list);
            }
            let n = pairs.len() as i64;
            let array = n > 0
                && pairs
                    .iter()
                    .all(|(_, _, i)| i.is_some_and(|i| i >= 1 && i <= n));
            if array {
                pairs.sort_by_key(|(_, _, i)| *i);
                serde_json::Value::Array(pairs.into_iter().map(|(_, v, _)| v).collect())
            } else {
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                serde_json::Value::Object(pairs.into_iter().map(|(k, v, _)| (k, v)).collect())
            }
        }
        _ => serde_json::Value::Null,
    }
}

pub fn run(saved: &Path, out: &Path) -> Result<()> {
    let src = std::fs::read(saved).with_context(|| saved.display().to_string())?;
    let lua = mlua::Lua::new();
    lua.load(&src[..])
        .exec()
        .map_err(|e| anyhow!("{}: {e}", saved.display()))?;
    let state: mlua::Table = lua
        .globals()
        .get("StdTestState")
        .map_err(|_| anyhow!("no StdTestState in {}", saved.display()))?;
    let snapshot: mlua::Value = state
        .get("snapshot")
        .map_err(|_| anyhow!("no snapshot: run /stdtest snapshot, then /reload"))?;
    let (text, _) = profile_text(&state, &snapshot)?;
    std::fs::write(out, text).with_context(|| out.display().to_string())?;
    println!("wrote {}", out.display());
    Ok(())
}

/// A snapshot (`/stdtest snapshot`'s) as a test profile's JSON, scrubbed
/// as `wowlua_driver::snapshots` scrubs one, and the names of StdTest's
/// probes' character too; with the names it scrubbed (for other text
/// recorded with it).
pub fn profile_text(state: &mlua::Table, snapshot: &mlua::Value) -> Result<(String, Vec<String>)> {
    let profile = to_json(snapshot);
    if !profile.is_object() {
        bail!("no snapshot: run /stdtest snapshot, then /reload");
    }
    let errors = profile.get("errors").cloned();
    let (text, secrets) =
        wowlua_driver::snapshots::profile_text(&profile).map_err(|e| anyhow!(e))?;
    // The account's other character, the probes' player (`/stdtest
    // engine`): account-wide parts (macros) may name it.
    let battery_names: Option<String> = state
        .get::<mlua::Table>("engineProbe")
        .and_then(|p| p.get::<mlua::Table>("units"))
        .and_then(|u| u.get::<String>("UnitName(player)"))
        .ok();
    let text = match battery_names {
        Some(v) => {
            let re = regex::Regex::new(r"string ([^,]+)").expect("a valid pattern");
            let names: Vec<String> = re
                .captures_iter(&v)
                .map(|c| c[1].trim().to_string())
                .collect();
            let mut text = text;
            for (name, as_) in names.iter().zip(["Player", "Lastname"]) {
                if name.len() > 2 {
                    text = text.replace(name.as_str(), as_);
                }
            }
            text
        }
        None => text,
    };
    if let Some(serde_json::Value::Array(e)) = errors
        && !e.is_empty()
    {
        println!("parts the client refused: {e:?}");
    }
    Ok((text, secrets))
}
