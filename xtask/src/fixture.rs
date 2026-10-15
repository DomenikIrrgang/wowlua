//! `cargo xtask fixture <StdTest.lua> <name>`: turns the conformance results
//! `/stdtest probe` saved (`StdTestState.conformance` in the saved
//! variables) into `crates/wowlua-vm/tests/client/<name>.lua`, the fixture
//! the VM must reproduce (spec emulator §17.8).

use anyhow::{Context, Result, anyhow, bail};
use std::fmt::Write;
use std::path::Path;

/// A Lua string literal of `bytes`, ASCII only (others as `\ddd`).
fn quote(bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    for &b in bytes {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            32..=126 => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03}");
            }
        }
    }
    out.push('"');
    out
}

/// `[key] = value` lines, sorted, for a table of strings.
fn entries(t: &mlua::Table) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut out = Vec::new();
    for pair in t.clone().pairs::<mlua::LuaString, mlua::Value>() {
        let (k, v) = pair.map_err(|e| anyhow!("{e}"))?;
        let v = match v {
            mlua::Value::String(s) => s.as_bytes().to_vec(),
            other => bail!("a string result expected, got {}", other.type_name()),
        };
        out.push((k.as_bytes().to_vec(), v));
    }
    out.sort();
    Ok(out)
}

pub fn run(root: &Path, saved: &Path, name: &str) -> Result<()> {
    let src = std::fs::read(saved).with_context(|| saved.display().to_string())?;
    let lua = mlua::Lua::new();
    lua.load(&src[..])
        .exec()
        .map_err(|e| anyhow!("{}: {e}", saved.display()))?;
    let state: mlua::Table = lua
        .globals()
        .get("StdTestState")
        .map_err(|_| anyhow!("no StdTestState in {}", saved.display()))?;
    let conformance: mlua::Table = state
        .get("conformance")
        .map_err(|_| anyhow!("no conformance results: run /stdtest probe, then /reload"))?;
    let client: String = conformance.get("client").unwrap_or_default();
    let interface: i64 = conformance.get("interface").unwrap_or_default();
    let results: mlua::Table = conformance.get("results").map_err(|e| anyhow!("{e}"))?;
    let kinds: mlua::Table = conformance.get("kinds").map_err(|e| anyhow!("{e}"))?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "-- The client's conformance results: `cargo xtask fixture`."
    );
    let _ = writeln!(out, "return {{");
    let _ = writeln!(out, "    client = {},", quote(client.as_bytes()));
    let _ = writeln!(out, "    interface = {interface},");
    for (field, t) in [("results", &results), ("kinds", &kinds)] {
        let _ = writeln!(out, "    {field} = {{");
        for (k, v) in entries(t)? {
            let _ = writeln!(out, "        [{}] = {},", quote(&k), quote(&v));
        }
        let _ = writeln!(out, "    }},");
    }
    let _ = writeln!(out, "}}");
    let path = root.join(format!("crates/wowlua-vm/tests/client/{name}.lua"));
    std::fs::create_dir_all(path.parent().expect("a directory"))?;
    std::fs::write(&path, out)?;
    println!(
        "{}: {} results, {} functions ({client}, interface {interface})",
        path.display(),
        results.raw_len().max(entries(&results)?.len()),
        entries(&kinds)?.len()
    );
    Ok(())
}
