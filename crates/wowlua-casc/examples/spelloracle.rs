//! `cargo run -p wowlua-casc --example spelloracle`: spell descriptions
//! filled in from the client's tables (`spells`), against the client's own
//! answers recorded in `crates/wowlua-engine/tests/client` (the trainers'
//! and talents' recordings, at the character's level): how many match,
//! how many differ (each printed) and how many are left unknown.
use wowlua_casc::spells::SpellTables;
use wowlua_casc::spelltext::Caster;

fn main() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(
        Box::new(wowlua_casc::FsStorage { root }),
        "wow_classic_beta",
    )
    .unwrap();
    let tables = SpellTables::open(&casc).expect("the spell tables");
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../wowlua-engine/tests/client");
    let level_arg: Option<u32> = std::env::args().nth(1).and_then(|a| a.parse().ok());
    let (mut same, mut differ, mut unknown, mut empty, mut by_power) = (0, 0, 0, 0, 0);
    let mut seen = std::collections::HashSet::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("forever-camelot-trainer")
            || name.starts_with("forever-camelot-traits"))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let recorded_level = text
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("[\"UnitLevel(\\\"player\\\")\"] = \"1: number ")
            })
            .and_then(|r| r.trim_end_matches("\",").parse().ok())
            .or(level_arg);
        let answers: Vec<(u32, Option<String>)> = text
            .lines()
            .filter_map(|line| {
                let rest = line
                    .trim()
                    .strip_prefix("[\"C_Spell.GetSpellDescription(")?;
                let (id, answer) = rest.split_once(")\"] = \"")?;
                let answer = answer
                    .trim_end_matches("\",")
                    .replace("\\\"", "\"")
                    .replace("\\r", "\r")
                    .replace("\\n", "\n");
                Some((
                    id.parse().ok()?,
                    answer.strip_prefix("1: string ").map(str::to_string),
                ))
            })
            .collect();
        // without a recorded level: the one its answers fit best
        let level = recorded_level.unwrap_or_else(|| {
            (1..=60)
                .max_by_key(|&l| {
                    let c = Caster {
                        level: l,
                        ..Default::default()
                    };
                    answers
                        .iter()
                        .filter(|(id, a)| {
                            a.is_some() && tables.describe(*id, &c).as_deref() == a.as_deref()
                        })
                        .count()
                })
                .unwrap()
        });
        let fit = |c: &Caster| {
            answers
                .iter()
                .filter(|(id, a)| a.is_some() && tables.describe(*id, c).as_deref() == a.as_deref())
                .count()
        };
        let damage_done = [1.0f64, 1.01, 1.02, 1.03, 1.05, 1.1]
            .into_iter()
            .max_by_key(|&f| {
                fit(&Caster {
                    level,
                    damage_done: f,
                    ..Default::default()
                })
            })
            .unwrap();
        println!(
            "{name}: level {level}{}, damage done {damage_done}",
            if recorded_level.is_some() {
                ""
            } else {
                " (fitted)"
            }
        );
        let caster = Caster {
            level,
            damage_done,
            ..Default::default()
        };
        for (id, answer) in answers {
            let Some(recorded) = answer else {
                empty += 1;
                continue;
            };
            let recorded = recorded.as_str();
            if !seen.insert((id, recorded.to_string(), level)) {
                continue;
            }
            match tables.describe(id, &caster) {
                Some(d) if d == recorded => same += 1,
                // what the character's gear adds (unrecorded): set apart
                Some(_)
                    if tables.describe(
                        id,
                        &Caster {
                            spell_power: 1000.0,
                            healing_power: 1000.0,
                            attack_power: 1000.0,
                            ..caster
                        },
                    ) != tables.describe(id, &caster) =>
                {
                    by_power += 1;
                }
                Some(d) => {
                    differ += 1;
                    println!(
                        "DIFF {id} (level {level}, {name})\n  ours:   {d}\n  client: {recorded}"
                    );
                }
                None => unknown += 1,
            }
        }
    }
    println!(
        "{same} the same, {differ} different, {by_power} different where the character's power counts, {unknown} unknown, {empty} without a description"
    );
}
