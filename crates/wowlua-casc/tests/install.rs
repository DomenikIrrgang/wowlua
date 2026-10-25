//! The reader on the WoW installation of this machine (`WOWLUA_WOW_PATH`,
//! else the default place); skipped where there is none.

use wowlua_casc::{Casc, FsStorage, Storage};

fn install() -> Option<std::path::PathBuf> {
    let p = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    p.join(".build.info").exists().then_some(p)
}

#[test]
fn reads_every_products_textures_and_fonts() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    let camelot = Casc::product_of_folder(&storage, "_classic_beta_");
    eprintln!("_classic_beta_ is {camelot:?}");
    let products = Casc::products(&storage);
    assert!(!products.is_empty());
    for (product, version) in products {
        let started = std::time::Instant::now();
        let casc = match Casc::open(Box::new(FsStorage { root: root.clone() }), &product) {
            Ok(c) => c,
            Err(e) => panic!("{product} {version}: {e}"),
        };
        let opened = started.elapsed();
        let white = casc
            .file_by_path("Interface\\Buttons\\WHITE8X8")
            .unwrap_or_else(|| panic!("{product}: WHITE8X8"));
        let img = wowlua_casc::decode_image(&white).expect("a BLP");
        assert_eq!((img.width, img.height), (8, 8));
        // Near white: a DXT1 file stores 565 colors (era's green is 251).
        assert!(img.pixels.iter().all(|&c| c >= 248), "{product}: white");
        let font = casc
            .file_by_path("Fonts\\FRIZQT__.TTF")
            .unwrap_or_else(|| panic!("{product}: FRIZQT__.TTF"));
        assert_eq!(&font[..4], &[0, 1, 0, 0], "{product}: a TrueType font");
        let button = casc
            .file_by_path("Interface/Buttons/UI-Panel-Button-Up.blp")
            .and_then(|b| wowlua_casc::decode_image(&b));
        eprintln!(
            "{product} {version}: {} files, opened in {opened:?}, button {:?}",
            casc.file_count(),
            button.map(|b| (b.width, b.height))
        );
    }
    let _ = storage.list("Data");
}

#[test]
fn reads_the_builds_own_atlases() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, version) in Casc::products(&storage) {
        let casc = Casc::open(Box::new(FsStorage { root: root.clone() }), &product).expect("opens");
        let started = std::time::Instant::now();
        let atlases = wowlua_casc::atlas::read(&casc);
        eprintln!(
            "{product} {version}: {} atlases in {:?}",
            atlases.len(),
            started.elapsed()
        );
        // Some builds encrypt the tables (classic's): none then.
        if product == "wow_classic_beta" {
            assert!(atlases.len() > 1000, "{product}: {}", atlases.len());
            // An element's camelot variant at 1x: the level circle.
            let c = atlases["ui-hud-unitframe-smallcircle"];
            assert_eq!(c.file, 8036204, "{c:?}");
            assert_eq!((c.width, c.height), (39, 39));
            assert_eq!(
                c.coords,
                [979.0 / 1024.0, 1018.0 / 1024.0, 87.0 / 512.0, 126.0 / 512.0]
            );
            // A member's own name.
            assert_eq!(atlases["ui-hud-unitframe-smallcircle-c60-2x"].file, 8036209);
        }
    }
}

/// Every product's creature displays resolve to models that read: the
/// tables (encrypted ones with the client's keys), the model and its
/// skin, posed in its first sequence.
#[test]
fn reads_every_products_creature_models() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, version) in Casc::products(&storage) {
        let Ok(casc) = Casc::open(Box::new(FsStorage { root: root.clone() }), &product) else {
            continue;
        };
        let mut lib = wowlua_casc::display::Library::default();
        let rows = lib
            .table(&casc, "CreatureDisplayInfo")
            .map_or(0, wowlua_casc::tables::Table::len);
        assert!(rows > 1000, "{product}: {rows} displays");
        // The first displays that name a model file.
        let mut read = 0;
        for id in 1..3000 {
            let Some(d) = lib.creature_display(&casc, id) else {
                continue;
            };
            let part = &d.parts[0];
            let Some(bytes) = casc.file_by_id(part.file) else {
                continue;
            };
            let m = wowlua_casc::m2::Model::parse(&bytes)
                .unwrap_or_else(|| panic!("{product}: display {id}'s model {}", part.file));
            let skin = casc
                .file_by_id(m.skin_files[0])
                .and_then(|b| wowlua_casc::m2::Skin::parse(&b))
                .unwrap_or_else(|| panic!("{product}: model {}'s skin", part.file));
            assert!(!skin.batches.is_empty() && !m.vertices.is_empty());
            let pose = m.pose(m.sequence(0), 0);
            assert_eq!(pose.len(), m.bones.len());
            read += 1;
            if read == 20 {
                break;
            }
        }
        eprintln!(
            "{product} {version}: {rows} displays, {} keys, {read} models read",
            casc.key_count()
        );
        assert_eq!(read, 20, "{product}");
    }
}

/// Every product's characters resolve: a body with its customizations'
/// skin composited and textures, what it wears as parts; and a humanoid
/// creature with its baked skin.
#[test]
fn resolves_every_products_characters() {
    use wowlua_casc::character::{Look, Worn};
    use wowlua_casc::display::Texture;
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, _) in Casc::products(&storage) {
        let Ok(casc) = Casc::open(Box::new(FsStorage { root: root.clone() }), &product) else {
            continue;
        };
        let mut lib = wowlua_casc::display::Library::default();
        // A human warrior in a recruit's shirt with a sword (the starting
        // items of every classic client).
        let look = Look {
            race: 1,
            sex: 0,
            class: 1,
            choices: Vec::new(),
            worn: vec![(4, Worn::Item(38)), (16, Worn::Item(25))],
            bake: None,
        };
        let d = lib
            .character(&casc, &look)
            .unwrap_or_else(|| panic!("{product}: the human"));
        let body = &d.parts[0];
        let Some(Texture::Composite(skin)) = body.textures.get(&1) else {
            panic!("{product}: no composited skin: {:?}", body.textures.keys());
        };
        assert!(skin.layers.len() > 2, "{product}: {skin:?}");
        assert!(
            d.parts.iter().any(|p| p.attachment == Some(1)),
            "{product}: the sword"
        );
        assert!(casc.file_by_id(body.file).is_some());
        // A humanoid creature: its baked skin.
        let humanoid = lib
            .table(&casc, "CreatureDisplayInfo")
            .and_then(|t| {
                t.rows()
                    .find(|r| r.int("ExtendedDisplayInfoID") != 0)
                    .map(|r| r.id())
            })
            .expect("a humanoid");
        let d = lib
            .creature_display(&casc, humanoid)
            .unwrap_or_else(|| panic!("{product}: display {humanoid}"));
        assert!(
            matches!(d.parts[0].textures.get(&1), Some(Texture::File(_))),
            "{product}: display {humanoid}'s bake"
        );
    }
}

/// Every product's maps have WDTs (by file ID or path), which list each
/// terrain tile's minimap texture: Eastern Kingdoms has Northshire's
/// (column 32, row 48), and the texture reads.
#[test]
fn reads_the_minimap_tiles() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, version) in Casc::products(&storage) {
        let casc = Casc::open(Box::new(FsStorage { root: root.clone() }), &product).expect("opens");
        let mut lib = wowlua_casc::display::Library::default();
        let tile = lib.minimap_tile(&casc, 0, 32, 48);
        eprintln!("{product} {version}: Northshire's minimap tile {tile:?}");
        {
            let f = tile.expect("the tile");
            let img = wowlua_casc::decode_image(&casc.file_by_id(f).expect("the file"))
                .expect("an image");
            // 512 pixels on camelot, 256 on the older builds.
            assert!(img.width == img.height && img.width >= 256, "{product}");
            assert_eq!(lib.minimap_tile(&casc, 0, 0, 0), None);
            assert_eq!(lib.minimap_tile(&casc, 0, 64, 48), None);
        }
    }
}

/// Every product's `ItemSparse` (a sparse table: records of their own
/// sizes, strings inline) reads: the Hearthstone's name and quality, and
/// the `Item` table's class for it.
#[test]
fn reads_the_sparse_item_table() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, version) in Casc::products(&storage) {
        let casc = Casc::open(Box::new(FsStorage { root: root.clone() }), &product).expect("opens");
        let sparse = wowlua_casc::tables::Table::open(&casc, "ItemSparse").expect("ItemSparse");
        let r = sparse.row(6948).expect("the Hearthstone");
        eprintln!("{product} {version}: {} items", sparse.len());
        assert_eq!(
            r.string("Display_lang").as_deref(),
            Some("Hearthstone"),
            "{product}"
        );
        assert_eq!(r.int("OverallQualityID"), 1, "{product}");
        let item = wowlua_casc::tables::Table::open(&casc, "Item").expect("Item");
        assert_eq!(
            item.row(6948).expect("its row").int("ClassID"),
            15,
            "{product}"
        );
        if product == "wow_classic_beta" {
            let cub = sparse.row(277714).expect("the cub");
            assert_eq!(
                cub.string("Display_lang").as_deref(),
                Some("Spectral Bear Cub")
            );
        }
    }
}

/// Ribbon emitters read: camelot's fire totem (display 4589) has three,
/// laying edges down 23 times a second for 0.53 seconds, each 0.22 yards
/// above and below its bone's point, additive.
#[test]
fn reads_ribbon_emitters() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let Ok(casc) = Casc::open(Box::new(FsStorage { root }), "wow_classic_beta") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let mut lib = wowlua_casc::display::Library::default();
    let d = lib.creature_display(&casc, 4589).expect("the totem");
    let m = wowlua_casc::m2::Model::parse(&casc.file_by_id(d.parts[0].file).expect("its file"))
        .expect("its model");
    assert_eq!(m.ribbons.len(), 3);
    let r = &m.ribbons[0];
    assert_eq!((r.edges_per_second, r.bone), (23.0, 27));
    assert!((r.edge_lifetime - 0.53).abs() < 1e-4);
    let look = m.ribbon_look(0, Some(0), 0).expect("its look");
    assert!((look.above - 0.2222).abs() < 1e-3 && (look.below - 0.2222).abs() < 1e-3);
    let blend = r.materials.first().map(|i| m.materials[*i as usize].blend);
    assert_eq!(blend, Some(4));
}

/// WoW Forever's wardrobe cameras: an Undead male's head is UI camera 542
/// (as the client answered `GetAppearanceCameraID` for him), a Skyborne
/// female's 1957; its weapons' table reads.
#[test]
fn reads_the_wardrobe_camera_tables() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    if !Casc::products(&storage)
        .iter()
        .any(|(p, _)| p == "wow_classic_beta")
    {
        return;
    }
    let casc = Casc::open(Box::new(FsStorage { root }), "wow_classic_beta").expect("opens");
    let t = wowlua_casc::tables::Table::open(&casc, "UiCamFbackTransmogChrRace").expect("races");
    let cam = |race: u32, gender: u32| {
        t.rows()
            .find(|r| {
                r.int("ChrRaceID") == race
                    && r.int("Gender") == gender
                    && r.int("InventoryType") == 1
                    && r.int("Variation") == 0
            })
            .map(|r| r.int("UiCameraID"))
    };
    assert_eq!((cam(5, 0), cam(96, 1)), (Some(542), Some(1957)));
    let w = wowlua_casc::tables::Table::open(&casc, "UiCamFbackTransmogWeapon").expect("weapons");
    assert!(
        w.rows()
            .any(|r| r.int("ItemClass") == 2 && r.int("UiCameraID") > 0)
    );
}

/// WoW Forever's areas: 2437 is Ragefire Chasm (AtlasLoot names its
/// dungeons by `C_Map.GetAreaInfo`), 1637 Orgrimmar (as the client
/// answered).
#[test]
fn reads_the_area_table() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    if !Casc::products(&storage)
        .iter()
        .any(|(p, _)| p == "wow_classic_beta")
    {
        return;
    }
    let casc = Casc::open(Box::new(FsStorage { root }), "wow_classic_beta").expect("opens");
    let t = wowlua_casc::tables::Table::open(&casc, "AreaTable").expect("areas");
    let name = |id: u32| t.row(id).and_then(|r| r.string("AreaName_lang"));
    assert_eq!(name(2437).as_deref(), Some("Ragefire Chasm"));
    assert_eq!(name(1637).as_deref(), Some("Orgrimmar"));
}

/// camelot's spells as the client's tables and hotfixes have them
/// (decision T391): the descriptions filled in for the character a
/// recording was taken with (`wowlua-engine/tests/client`, its level, no
/// gear) are the client's, except where the character's power counts
/// (the recordings lack it); a description that cannot be filled exactly
/// is left unknown, never wrong. The facts of a spell: its name, rank,
/// cast time, range and icon.
#[test]
fn spells_read_as_the_client_shows_them() {
    use wowlua_casc::spells::SpellTables;
    use wowlua_casc::spelltext::Caster;
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    let Some(product) = Casc::product_of_folder(&storage, "_classic_beta_") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let casc = Casc::open(Box::new(FsStorage { root }), &product).unwrap();
    let tables = SpellTables::open(&casc).expect("the spell tables");
    let fireball = tables.info(133).expect("Fireball");
    assert_eq!(fireball.name, "Fireball");
    assert_eq!(fireball.subtext, "Rank 1");
    assert!(
        fireball.cast_ms > 0 && fireball.max_range > 0.0,
        "{fireball:?}"
    );
    assert!(
        fireball.icon.is_some() && !fireball.costs.is_empty(),
        "{fireball:?}"
    );
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../wowlua-engine/tests/client");
    let (mut same, mut by_power, mut unknown) = (0, 0, 0);
    let mut differ = Vec::new();
    for name in [
        "forever-camelot-trainer-druid-3033.lua",
        "forever-camelot-trainer-druid-3060.lua",
        "forever-camelot-trainer-druid-3064.lua",
        "forever-camelot-trainer-warrior-911.lua",
        "forever-camelot-trainer-warrior-913.lua",
    ] {
        let text = std::fs::read_to_string(dir.join(name)).unwrap();
        let level: u32 = text
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("[\"UnitLevel(\\\"player\\\")\"] = \"1: number ")
            })
            .and_then(|r| r.trim_end_matches("\",").parse().ok())
            .expect("the recording's level");
        // the level-21 druid's damage is 2 % more (something of the
        // character's the recording does not show)
        let damage_done = if level == 21 { 1.02 } else { 1.0 };
        let caster = Caster {
            level,
            damage_done,
            ..Default::default()
        };
        let more = Caster {
            spell_power: 1000.0,
            healing_power: 1000.0,
            attack_power: 1000.0,
            ..caster
        };
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("[\"C_Spell.GetSpellDescription(") else {
                continue;
            };
            let Some((id, answer)) = rest.split_once(")\"] = \"1: string ") else {
                continue;
            };
            let id: u32 = id.parse().unwrap();
            let recorded = answer
                .trim_end_matches("\",")
                .replace("\\\"", "\"")
                .replace("\\r", "\r")
                .replace("\\n", "\n");
            match tables.describe(id, &caster) {
                Some(d) if d == recorded => same += 1,
                Some(_) if tables.describe(id, &more) != tables.describe(id, &caster) => {
                    by_power += 1
                }
                Some(d) => differ.push(format!("{id} at {level}: {d}\n  client: {recorded}")),
                None => unknown += 1,
            }
        }
    }
    eprintln!("{same} the same, {by_power} where power counts, {unknown} unknown");
    assert!(differ.is_empty(), "{}", differ.join("\n"));
    assert!(same >= 200, "{same}");
}

/// camelot's spells' power costs as the client's `GetSpellPowerCost`
/// answers them in the trainers' recordings: each cost's power type and
/// amount (rage and runic power in whole points, the tables' tenths;
/// a finisher's the most it takes); costs in percent of the base power,
/// and costs a recorded character's talents lowered, depend on the
/// character.
#[test]
fn spell_costs_read_as_the_client_answers_them() {
    use wowlua_casc::spells::SpellTables;
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    let Some(product) = Casc::product_of_folder(&storage, "_classic_beta_") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let casc = Casc::open(Box::new(FsStorage { root }), &product).unwrap();
    let tables = SpellTables::open(&casc).expect("the spell tables");
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../wowlua-engine/tests/client");
    let (mut same, mut by_character, mut differ) = (0, 0, Vec::new());
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("forever-camelot-trainer") {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).unwrap();
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("[\"C_Spell.GetSpellPowerCost(") else {
                continue;
            };
            let Some((id, answer)) = rest.split_once(")\"] = \"") else {
                continue;
            };
            let id: u32 = id.parse().unwrap();
            // (type, cost) of each recorded cost
            let recorded: Vec<(i32, u32)> = answer
                .split("},")
                .filter_map(|c| {
                    let num = |key: &str| -> Option<i64> {
                        let at = c.find(&format!("{key}=number "))? + key.len() + 8;
                        c[at..]
                            .split(|ch: char| !ch.is_ascii_digit() && ch != '-')
                            .next()?
                            .parse()
                            .ok()
                    };
                    Some((num("type")? as i32, num("cost")? as u32))
                })
                .filter(|_| {
                    !answer.contains("costPercent=number 1")
                        && !answer.contains("costPercent=number 2")
                })
                .collect();
            if recorded.is_empty() {
                continue;
            }
            let info = tables.info(id);
            // (a share of the base power: the character's)
            if info
                .as_ref()
                .is_some_and(|s| s.costs.iter().any(|c| c.percent > 0.0))
            {
                continue;
            }
            let ours: Vec<(i32, u32)> = info
                .map(|s| s.costs.iter().map(|c| (c.power_type, c.shown())).collect())
                .unwrap_or_default();
            // (less of the same power: the character's talents, which the
            // tables do not show: Ferocity, Improved Thunder Clap)
            let reduced = ours.len() == recorded.len()
                && ours
                    .iter()
                    .zip(&recorded)
                    .all(|(o, r)| o.0 == r.0 && r.1 <= o.1);
            if ours == recorded {
                same += 1;
            } else if reduced {
                by_character += 1;
            } else {
                differ.push(format!("{id}: ours {ours:?}, client {recorded:?}"));
            }
        }
    }
    eprintln!(
        "{same} the same, {by_character} less by the character's talents, {} different",
        differ.len()
    );
    assert!(differ.is_empty(), "{}", differ.join("\n"));
    assert!(
        same >= 1000 && by_character < same / 20,
        "{same}, {by_character}"
    );
}
