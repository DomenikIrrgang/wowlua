//! `cargo xtask ingest <SavedVariables/StdTest.lua>`: a client's StdTest
//! recordings into the repository's fixtures and data, scrubbed of the
//! player (name, last name and GUID; any player's GUID; the social systems'
//! texts), so a new client is checked by running the tests and reading the
//! diff:
//!
//! - `/stdtest engine` (and order, protected, blocked):
//!   `crates/wowlua-engine/tests/client/forever-camelot-engine.lua`;
//! - `/stdtest engine input`: `…/forever-camelot-input.lua`;
//! - `/stdtest engine answers`: `…/forever-camelot-answers.lua`;
//! - `/stdtest snapshot`: the character's world as a test profile,
//!   `…/forever-camelot-character.json`;
//! - the CVar flags: `crates/wowlua-engine/data/camelot-cvar-flags.txt`.
//!
//! Each part is written only when the file has it.

use anyhow::{Context, Result, anyhow};
use std::fmt::Write;
use std::path::Path;

/// The player as the file names them.
struct Identity {
    name: String,
    last_name: Option<String>,
    guid: String,
}

/// Functions whose answers are the player's social life: their strings are
/// blanked (the structure stays).
const SOCIAL: &[&str] = &[
    "C_Club.",
    "C_ClubFinder.",
    "C_GuildInfo.",
    "GetGuild",
    "BNGet",
    "C_BattleNet.",
    "C_FriendList.",
    "C_DamageMeter.",
    "C_RecentAllies.",
    "C_SocialQueue.",
    "GetFriendInfo",
    "GetWhoInfo",
];

impl Identity {
    fn scrub(&self, text: &str) -> String {
        // The session's addresses (`function: 0x…`).
        let address = regex::Regex::new(r"(function|table|userdata|thread): 0x[0-9a-f]+")
            .expect("a valid pattern");
        let text = address.replace_all(text, "$1");
        let mut out = text.replace(&self.guid, "Player-0000-00000000");
        // Any other player's GUID.
        let guid = regex::Regex::new(r"Player-\d+-[0-9A-F]{8}").expect("a valid pattern");
        out = guid.replace_all(&out, "Player-0000-00000000").into_owned();
        out = out.replace(&self.name, "Player");
        if let Some(last) = &self.last_name {
            out = out.replace(last, "Lastname");
        }
        out
    }
}

/// A social function's answer with its strings blanked.
fn blank_strings(text: &str) -> String {
    let s = regex::Regex::new(r"string [^,}]*").expect("a valid pattern");
    s.replace_all(text, "string <scrubbed>").into_owned()
}

/// A Lua string literal (ASCII; others as `\ddd`).
fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for b in text.bytes() {
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

/// `StdTestState`'s tables flattened as the fixtures are (`a.b.c` keys,
/// `tostring` values), by a Lua function given the state.
fn flatten(lua: &mlua::Lua, state: &mlua::Table, script: &str) -> Result<Vec<(String, String)>> {
    let f: mlua::Function = lua.load(script).eval().map_err(|e| anyhow!("{e}"))?;
    let t: mlua::Table = f.call(state.clone()).map_err(|e| anyhow!("{e}"))?;
    let mut out = Vec::new();
    for pair in t.pairs::<String, String>() {
        out.push(pair.map_err(|e| anyhow!("{e}"))?);
    }
    out.sort();
    Ok(out)
}

const FLATTEN: &str = r#"return function(s)
    local out = {}
    local function dump(v, prefix)
        if type(v) ~= "table" then out[prefix] = tostring(v) return end
        for k, inner in pairs(v) do dump(inner, prefix == "" and tostring(k) or (prefix .. "." .. tostring(k))) end
    end
    if s.engineProbe then dump(s.engineProbe, "probe") end
    if s.engineOrder then dump(s.engineOrder, "order") end
    if s.engineProtected then dump(s.engineProtected, "protected") end
    if s.engineBlocked then dump(s.engineBlocked, "blocked") end
    return out
end"#;

fn write_table(path: &Path, header: &str, entries: &[(String, String)]) -> Result<()> {
    let mut out = String::new();
    for line in header.lines() {
        let _ = writeln!(out, "-- {line}");
    }
    out.push_str("return {\n");
    for (k, v) in entries {
        let _ = writeln!(out, "    [{}] = {},", quote(k), quote(v));
    }
    out.push_str("}\n");
    std::fs::write(path, out).with_context(|| path.display().to_string())?;
    println!("wrote {} ({} entries)", path.display(), entries.len());
    Ok(())
}

pub fn run(root: &Path, saved: &Path) -> Result<()> {
    let src = std::fs::read(saved).with_context(|| saved.display().to_string())?;
    let lua = mlua::Lua::new();
    lua.load(&src[..])
        .exec()
        .map_err(|e| anyhow!("{}: {e}", saved.display()))?;
    let state: mlua::Table = lua
        .globals()
        .get("StdTestState")
        .map_err(|_| anyhow!("no StdTestState in {}", saved.display()))?;
    let client = root.join("crates/wowlua-engine/tests/client");
    let battery = flatten(&lua, &state, FLATTEN)?;
    let get = |key: &str| {
        battery
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    // The player, from the battery's unit calls.
    let strings = |v: &str| -> Vec<String> {
        let re = regex::Regex::new(r"string ([^,]+)").expect("a valid pattern");
        re.captures_iter(v)
            .map(|c| c[1].trim().to_string())
            .collect()
    };
    let names = get("probe.units.UnitName(player)")
        .map(|v| strings(&v))
        .ok_or_else(|| anyhow!("no /stdtest engine results to name the player by"))?;
    let guid = get("probe.units.UnitGUID(player)")
        .and_then(|v| strings(&v).into_iter().next())
        .ok_or_else(|| anyhow!("no player GUID in the /stdtest engine results"))?;
    let me = Identity {
        name: names.first().cloned().unwrap_or_default(),
        last_name: names.get(1).cloned(),
        guid,
    };
    let version = get("probe.client").unwrap_or_else(|| "an unknown client".into());

    let scrubbed: Vec<(String, String)> = battery
        .iter()
        .map(|(k, v)| (me.scrub(k), me.scrub(v)))
        .collect();
    write_table(
        &client.join("forever-camelot-engine.lua"),
        &format!(
            "WoW Forever {version}: /stdtest engine, order, protected and blocked\n\
             (tests/ingame/src/engineprobe.wowlua), flattened; the player's name, realm\n\
             and GUID replaced (cargo xtask ingest)."
        ),
        &scrubbed,
    )?;

    // The CVar flags, as the engine's data.
    let mut flags = String::from(&format!(
        "# The CVars C_CVar.GetCVarInfo flags on WoW Forever {version}\n\
         # (/stdtest engine, probe.cvarFlags): secure (addons may not set them),\n\
         # readOnly, locked (from the user); one per line, the flag then the name.\n"
    ));
    let mut any_flags = false;
    for kind in ["secure", "readOnly", "locked"] {
        if let Some(list) = get(&format!("probe.cvarFlags.{kind}")) {
            any_flags = true;
            for name in list.split_whitespace() {
                let _ = writeln!(flags, "{kind}\t{name}");
            }
        }
    }
    if any_flags {
        let path = root.join("crates/wowlua-engine/data/camelot-cvar-flags.txt");
        std::fs::write(&path, flags)?;
        println!("wrote {}", path.display());
    }

    // The input recording, in order.
    if let Ok(input) = state.get::<mlua::Table>("engineInput") {
        let mut out = format!(
            "-- WoW Forever {version}: /stdtest engine input\n\
             -- (tests/ingame/src/engineprobe.wowlua): hover the button, click it,\n\
             -- Shift-click it, click the box, type ab, press Enter.\nreturn {{\n"
        );
        for line in input.sequence_values::<String>() {
            let line = line.map_err(|e| anyhow!("{e}"))?;
            let _ = writeln!(out, "    {},", quote(&me.scrub(&line)));
        }
        out.push_str("}\n");
        let path = client.join("forever-camelot-input.lua");
        std::fs::write(&path, out)?;
        println!("wrote {}", path.display());
    }

    // The character's world, as a test profile (`/stdtest snapshot`).
    if state.get::<mlua::Table>("snapshot").is_ok() {
        let tmp = tempfile::NamedTempFile::new()?;
        crate::profile::run(saved, tmp.path())?;
        let text = std::fs::read_to_string(tmp.path())?;
        let path = client.join("forever-camelot-character.json");
        std::fs::write(&path, me.scrub(&text))?;
        println!("wrote {}", path.display());
    }

    // The answers to the calls the engine could not answer.
    if let Ok(answers) = state.get::<mlua::Table>("engineAnswers") {
        let mut entries = Vec::new();
        for pair in answers.pairs::<String, String>() {
            let (call, answer) = pair.map_err(|e| anyhow!("{e}"))?;
            let answer = if SOCIAL.iter().any(|p| call.starts_with(p)) {
                blank_strings(&answer)
            } else {
                answer
            };
            entries.push((me.scrub(&call), me.scrub(&answer)));
        }
        entries.sort();
        write_table(
            &client.join("forever-camelot-answers.lua"),
            &format!(
                "WoW Forever {version}: /stdtest engine answers, the client's answers to the\n\
                 calls Blizzard's UI made in the engine's sweep that the engine could not\n\
                 answer (tests/ingame/src/answers.wowlua); the player scrubbed, the social\n\
                 systems' texts blanked (cargo xtask ingest)."
            ),
            &entries,
        )?;
    }
    Ok(())
}
