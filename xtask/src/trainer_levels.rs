//! `cargo xtask trainer-levels <SuuUI>/src/Data/Trainer`: each recorded
//! class trainer service's level from SuuUI's trainer data
//! (`<CLASS>.lua`: its probes at the class trainers, a spell the probing
//! character knew at the level `C_Spell.GetSpellLevelLearned` gives),
//! matched to the recorded services (`data/camelot-trainers.txt`) by name
//! and description (a rank's own numbers), into
//! `data/camelot-trainer-levels.txt`: the class, the spell, the level. A
//! match with two levels is left out. The engine reads a level from it
//! where the recorded one is 0 (a service the probing character knew); a
//! test holds it equal to every recorded level.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

/// A data file's field back from its symbols (␤ ␉ ␍).
fn unescape(text: &str) -> String {
    text.replace('\u{2424}', "\n")
        .replace('\u{2409}', "\t")
        .replace('\u{240D}', "\r")
}

/// A Lua string literal's text (`%q`'s escapes).
fn lua_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

pub fn run(root: &Path, suuui: &Path) -> Result<()> {
    let data_path = root.join("crates/wowlua-engine/data/camelot-trainers.txt");
    let data = std::fs::read_to_string(&data_path).context("the recorded trainers")?;
    let mut answers: BTreeMap<String, HashMap<String, String>> = BTreeMap::new();
    for line in data.lines().filter(|l| !l.starts_with('#')) {
        let mut parts = line.splitn(3, '\t');
        let (Some(key), Some(call), Some(answer)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        answers
            .entry(key.to_string())
            .or_default()
            .insert(unescape(call), unescape(answer));
    }
    let entry = regex::Regex::new(
        r#"\{ name = "((?:[^"\\]|\\.)*)", level = (\d+), cost = \d+, school = "(?:[^"\\]|\\.)*", icon = \d+, text = "((?:[^"\\]|\\.)*)" \}"#,
    )?;
    let number = regex::Regex::new(r"number (\d+)")?;
    let spell = regex::Regex::new(r"[{,]id=number (\d+)")?;
    let info_name = regex::Regex::new(r"^6: string ([^,]*), ")?;
    // Each class's levels by name and description.
    let mut theirs: HashMap<String, HashMap<(String, String), Vec<u32>>> = HashMap::new();
    let mut levels: BTreeMap<(String, i64), Option<u32>> = BTreeMap::new();
    for (key, calls) in &answers {
        let class = key.split('-').next().unwrap_or(key).to_string();
        if !theirs.contains_key(&class) {
            let file = suuui.join(format!("{class}.lua"));
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let mut by: HashMap<(String, String), Vec<u32>> = HashMap::new();
            for c in entry.captures_iter(&text) {
                by.entry((lua_text(&c[1]), lua_text(&c[3])))
                    .or_default()
                    .push(c[2].parse()?);
            }
            theirs.insert(class.clone(), by);
        }
        let by = &theirs[&class];
        let n: usize = calls
            .get("GetNumTrainerServices()")
            .and_then(|a| number.captures(a))
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(0);
        for i in 1..=n {
            let get = |call: String| calls.get(&call).map(String::as_str).unwrap_or("");
            let Some(name) = info_name.captures(get(format!("GetTrainerServiceInfo({i})"))) else {
                continue;
            };
            let Some(id) = spell.captures(get(format!("C_TooltipInfo.GetTrainerService({i})")))
            else {
                continue;
            };
            let Some(description) =
                get(format!("GetTrainerServiceDescription({i})")).strip_prefix("1: string ")
            else {
                continue;
            };
            let Some(found) = by.get(&(name[1].to_string(), description.to_string())) else {
                continue;
            };
            let level = if found.iter().all(|l| *l == found[0]) {
                Some(found[0])
            } else {
                None
            };
            let id: i64 = id[1].parse()?;
            let slot = levels.entry((class.clone(), id)).or_insert(level);
            if *slot != level {
                *slot = None;
            }
        }
    }
    let mut out = String::from(
        "# Each recorded class trainer service's level from SuuUI's trainer data\n\
         # (src/Data/Trainer/<CLASS>.lua: its probes at the class trainers, a\n\
         # spell the probing character knew at C_Spell.GetSpellLevelLearned's\n\
         # level), matched to data/camelot-trainers.txt by name and description\n\
         # (cargo xtask trainer-levels): the class, the spell, the level.\n",
    );
    let mut written = 0;
    for ((class, id), level) in &levels {
        if let Some(level) = level {
            let _ = writeln!(out, "{class}\t{id}\t{level}");
            written += 1;
        }
    }
    let path = root.join("crates/wowlua-engine/data/camelot-trainer-levels.txt");
    std::fs::write(&path, out)?;
    println!("wrote {} ({written} levels)", path.display());
    Ok(())
}
