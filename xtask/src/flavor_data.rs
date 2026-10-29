//! `cargo xtask flavor-data [<WoW installation>]`: what each flavor's game
//! says of its races and classes, read from the game's own tables in the
//! installed clients (decisions T512, T513), into the files the flavor
//! table embeds:
//!
//! - `crates/wowlua-flavors/data/races.txt`: each race's records, the
//!   classes each can be and its racial spells;
//! - `crates/wowlua-flavors/data/base-mana.txt`: each class's base mana by
//!   level.
//!
//! For each flavor, the first of its clients' folders the installation has
//! a build of is read (`_forever_` for camelot, `_classic_era_` for
//! vanilla, …); a flavor with none keeps its sections of the files as
//! they are.
//!
//! - The races: `ChrRaces`' playable records (a `PlayableRaceBit`) of the
//!   flavor table's races (`ClientFileString`), each with its faction (the
//!   `Alliance` column: 0 the Alliance, 1 the Horde, 2 neither), ID and
//!   name. A race the player picks a faction for has a record per faction
//!   (the Pandaren, the Skyborne).
//! - The classes a record can be: `CharBaseInfo`'s race and class pairs.
//! - The racial spells: `SkillLineAbility`'s records of the racial skill
//!   lines (`SkillLine`s named `… Racial` or `Racial - …`, as every client
//!   names them), each for the races whose bit its race mask has and the
//!   classes its `ClassMask` has; a mask of 0 is the line's own, the races
//!   and classes `SkillRaceClassInfo` gives the line. Named by `SpellName`.
//!   The server's changes (the client's hotfix cache) are applied where
//!   the layout gives the fields' sizes.
//! - The base mana: the game table `GameTables/BaseMp.txt` (file 1391664),
//!   a column per class by name, a row per level, up to the flavor's max
//!   level.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use wowlua_casc::tables::Table;
use wowlua_casc::{Casc, FsStorage};

pub const RACES: &str = "crates/wowlua-flavors/data/races.txt";
pub const BASE_MANA: &str = "crates/wowlua-flavors/data/base-mana.txt";

/// The game table of each class's base mana by level.
const BASE_MP_FILE: u32 = 1391664;

/// The class files by class ID (the same in every flavor; `ClassMask`'s
/// bit is the ID less one).
const CLASSES: &[(u32, &str)] = &[
    (1, "WARRIOR"),
    (2, "PALADIN"),
    (3, "HUNTER"),
    (4, "ROGUE"),
    (5, "PRIEST"),
    (6, "DEATHKNIGHT"),
    (7, "SHAMAN"),
    (8, "MAGE"),
    (9, "WARLOCK"),
    (10, "MONK"),
    (11, "DRUID"),
    (12, "DEMONHUNTER"),
    (13, "EVOKER"),
];

fn faction_name(alliance: u32) -> &'static str {
    match alliance {
        0 => "Alliance",
        1 => "Horde",
        _ => "Neutral",
    }
}

/// A playable race record.
struct RaceRow {
    file: String,
    faction: &'static str,
    id: u32,
    name: String,
    bit: u32,
}

/// The flavor's class files in its order, of those in `files`; `-` for
/// every class of the flavor.
fn class_list(flavor: &wowlua_flavors::Flavor, files: &BTreeSet<&str>) -> String {
    let listed: Vec<&str> = flavor
        .classes
        .iter()
        .map(|c| c.file.as_str())
        .filter(|c| files.contains(c))
        .collect();
    if listed.len() == flavor.classes.len() {
        "-".to_string()
    } else {
        listed.join(",")
    }
}

/// One flavor's section of `races.txt`, from one client's build.
fn races_section(casc: &Casc, flavor: &wowlua_flavors::Flavor) -> Result<String> {
    let open = |name: &str| {
        Table::open(casc, name).with_context(|| {
            format!(
                "{} {}: the table {name} (its layout unknown to wowlua-casc's schema?)",
                casc.product, casc.version
            )
        })
    };
    let races = open("ChrRaces")?;
    let pairs = open("CharBaseInfo")?;
    let lines = open("SkillLine")?;
    let abilities = open("SkillLineAbility")?;
    let skills = open("SkillRaceClassInfo")?;
    let names = open("SpellName")?;
    let mut rows: Vec<RaceRow> = Vec::new();
    for r in races.rows() {
        let bit = r.signed("PlayableRaceBit");
        let file = r.string("ClientFileString").unwrap_or_default();
        if bit < 0 || !flavor.has_race(&file) {
            continue;
        }
        rows.push(RaceRow {
            file,
            faction: faction_name(r.int("Alliance")),
            id: r.id(),
            name: r.string("Name_lang").unwrap_or_default(),
            bit: bit as u32,
        });
    }
    rows.sort_by_key(|r| r.id);
    for race in flavor.race_files() {
        if !rows.iter().any(|r| r.file == race) {
            bail!("{}: the client has no playable race {race}", flavor.id);
        }
    }
    // The classes each race record can be.
    let mut classes_of: BTreeMap<u32, BTreeSet<&str>> = BTreeMap::new();
    for p in pairs.rows() {
        let class = p.int("ClassID");
        if let Some((_, file)) = CLASSES.iter().find(|(id, _)| *id == class)
            && flavor.class(file).is_some()
        {
            classes_of.entry(p.int("RaceID")).or_default().insert(file);
        }
    }
    let racial: BTreeMap<u32, String> = lines
        .rows()
        .filter_map(|r| {
            let name = r.string("DisplayName_lang")?;
            name.contains("Racial").then(|| (r.id(), name))
        })
        .collect();
    // Which races and classes have each racial line (a line's ability
    // whose masks are 0 is for those).
    let mut line_masks: BTreeMap<u32, (u64, u32)> = BTreeMap::new();
    for r in skills.rows() {
        let line = r.int("SkillID");
        if !racial.contains_key(&line) {
            continue;
        }
        let m = line_masks.entry(line).or_default();
        m.0 |= mask_of(&r);
        m.1 |= r.int("ClassMask");
    }
    // (race file, spell) → the factions and classes it is for, its name.
    type Found<'a> = (BTreeSet<&'a str>, BTreeSet<&'a str>, String);
    let mut spells: BTreeMap<(String, u32), Found> = BTreeMap::new();
    for a in abilities.rows() {
        let line = a.int("SkillLine");
        if !racial.contains_key(&line) {
            continue;
        }
        let (line_races, line_classes) = line_masks.get(&line).copied().unwrap_or_default();
        let mask = match mask_of(&a) {
            0 => line_races,
            mask => mask,
        };
        let class_mask = match a.int("ClassMask") {
            0 => line_classes,
            mask => mask,
        };
        let classes: Vec<&str> = CLASSES
            .iter()
            .filter(|(id, file)| {
                (class_mask == 0 || class_mask & (1 << (id - 1)) != 0)
                    && flavor.class(file).is_some()
            })
            .map(|(_, file)| *file)
            .collect();
        if classes.is_empty() {
            continue;
        }
        let spell = a.int("Spell");
        let name = names
            .row(spell)
            .and_then(|r| r.string("Name_lang"))
            .unwrap_or_default();
        for r in rows.iter().filter(|r| mask & (1 << r.bit) != 0) {
            let entry = spells
                .entry((r.file.clone(), spell))
                .or_insert_with(|| (BTreeSet::new(), BTreeSet::new(), name.clone()));
            entry.0.insert(r.faction);
            entry.1.extend(classes.iter().copied());
        }
    }
    let mut out = format!("[{}]\t{} {}\n", flavor.id, casc.product, casc.version);
    for r in &rows {
        let classes = classes_of.get(&r.id).cloned().unwrap_or_default();
        if classes.is_empty() {
            bail!(
                "{}: the race {} ({}) can be no class",
                flavor.id,
                r.file,
                r.id
            );
        }
        let _ = writeln!(
            out,
            "race\t{}\t{}\t{}\t{}\t{}",
            r.file,
            r.faction,
            r.id,
            r.name,
            class_list(flavor, &classes)
        );
    }
    // By race in the table's order, then the spells of every faction,
    // then those of one, by spell ID.
    let mut lines: Vec<(usize, String, String, String, u32, String)> = Vec::new();
    for ((file, spell), (factions, classes, name)) in &spells {
        let all: BTreeSet<&str> = rows
            .iter()
            .filter(|r| &r.file == file)
            .map(|r| r.faction)
            .collect();
        let factions = if *factions == all {
            "-".to_string()
        } else {
            factions.iter().copied().collect::<Vec<_>>().join(",")
        };
        let order = flavor
            .race_files()
            .position(|f| f == file)
            .unwrap_or(usize::MAX);
        lines.push((
            order,
            factions,
            file.clone(),
            class_list(flavor, classes),
            *spell,
            name.clone(),
        ));
    }
    lines.sort_by(|a, b| {
        (a.0, a.1 != "-", &a.1, a.4, &a.3).cmp(&(b.0, b.1 != "-", &b.1, b.4, &b.3))
    });
    for (_, factions, file, classes, spell, name) in &lines {
        let _ = writeln!(out, "spell\t{file}\t{factions}\t{classes}\t{spell}\t{name}");
    }
    Ok(out)
}

/// One flavor's section of `base-mana.txt`: a column per class of the
/// flavor, a row per level to its max level.
fn base_mana_section(casc: &Casc, flavor: &wowlua_flavors::Flavor) -> Result<String> {
    let bytes = casc
        .file_by_id(BASE_MP_FILE)
        .with_context(|| format!("{} {}: GameTables/BaseMp.txt", casc.product, casc.version))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut rows = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = rows
        .next()
        .context("BaseMp.txt has no header")?
        .split('\t')
        .map(|c| c.trim().to_uppercase().replace(' ', ""))
        .collect();
    let columns: Vec<(usize, &str)> = flavor
        .classes
        .iter()
        .map(|c| {
            header
                .iter()
                .position(|h| *h == c.file)
                .map(|i| (i, c.file.as_str()))
                .with_context(|| format!("BaseMp.txt has no column for {}", c.file))
        })
        .collect::<Result<_>>()?;
    let mut out = format!("[{}]\t{} {}\nlevel", flavor.id, casc.product, casc.version);
    for (_, file) in &columns {
        let _ = write!(out, "\t{file}");
    }
    out.push('\n');
    for row in rows {
        let cells: Vec<&str> = row.split('\t').map(str::trim).collect();
        let Some(level) = cells.first().and_then(|l| l.parse::<u32>().ok()) else {
            continue;
        };
        if level == 0 || level > flavor.max_level {
            continue;
        }
        let _ = write!(out, "{level}");
        for (i, _) in &columns {
            let value: f64 = cells.get(*i).and_then(|v| v.parse().ok()).unwrap_or(0.0);
            let _ = write!(out, "\t{value}");
        }
        out.push('\n');
    }
    Ok(out)
}

/// A record's race mask (`RaceMasks`, else `RaceMask`: two 32-bit words,
/// a bit per `PlayableRaceBit`).
fn mask_of(r: &wowlua_casc::tables::Row<'_>) -> u64 {
    let words = r.ints("RaceMasks");
    let words = if words.is_empty() {
        r.ints("RaceMask")
    } else {
        words
    };
    u64::from(words.first().copied().unwrap_or(0))
        | (u64::from(words.get(1).copied().unwrap_or(0)) << 32)
}

/// The sections of a file as it is, by flavor.
fn sections(text: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('[') {
            let id = rest.split(']').next().unwrap_or_default().to_string();
            current = Some(id.clone());
            out.insert(id, String::new());
        }
        if let Some(id) = &current {
            let s = out.entry(id.clone()).or_default();
            s.push_str(line);
            s.push('\n');
        }
    }
    out
}

const RACES_HEADER: &str = "\
# Each flavor's races, the classes they can be and their racial spells,
# from the game's own tables (`cargo xtask flavor-data <WoW installation>`,
# decisions T512, T513): a section per flavor, `[flavor]` and the client
# build it was read from, then
#   race   <file> <faction> <race ID> <name> <classes>   (ChrRaces' playable
#          records; the classes CharBaseInfo pairs with each)
#   spell  <race file> <factions> <classes> <spell ID> <name>
# a spell of the race's racial skill lines (SkillLineAbility's race and
# class masks); `-` every faction of the race / every class of the flavor.
";

const BASE_MANA_HEADER: &str = "\
# Each flavor's base mana by class and level, from the game table
# GameTables/BaseMp.txt of its client (`cargo xtask flavor-data <WoW
# installation>`, decision T512): a section per flavor, `[flavor]` and the
# client build it was read from, a column per class, a row per level; 0 a
# class without base mana.
";

pub fn run(root: &Path, install: Option<&str>) -> Result<()> {
    let install = install
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("WOWLUA_WOW_PATH").map(Into::into))
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let mut races = sections(&std::fs::read_to_string(root.join(RACES)).unwrap_or_default());
    let mut mana = sections(&std::fs::read_to_string(root.join(BASE_MANA)).unwrap_or_default());
    let table = wowlua_flavors::table();
    let storage = FsStorage {
        root: install.clone(),
    };
    for flavor in &table.flavors {
        let mut read = None;
        'clients: for client in table.clients_of(&flavor.id) {
            for folder in &client.folders {
                let Some(product) = Casc::product_of_folder(&storage, folder) else {
                    continue;
                };
                let storage = FsStorage {
                    root: install.clone(),
                };
                match Casc::open(Box::new(storage), &product) {
                    Ok(casc) => {
                        read = Some((
                            races_section(&casc, flavor)?,
                            base_mana_section(&casc, flavor)?,
                        ));
                        break 'clients;
                    }
                    Err(e) => eprintln!("{}: {folder} ({product}): {}", flavor.id, e.0),
                }
            }
        }
        match read {
            Some((r, m)) => {
                println!("{}: {}", flavor.id, r.lines().next().unwrap_or_default());
                races.insert(flavor.id.clone(), r);
                mana.insert(flavor.id.clone(), m);
            }
            None => println!("{}: no client installed, its sections kept", flavor.id),
        }
    }
    for (path, header, kept) in [
        (RACES, RACES_HEADER, &races),
        (BASE_MANA, BASE_MANA_HEADER, &mana),
    ] {
        let mut out = String::from(header);
        for flavor in &table.flavors {
            if let Some(text) = kept.get(&flavor.id) {
                out.push_str(text);
            }
        }
        let path = root.join(path);
        std::fs::write(&path, out)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
