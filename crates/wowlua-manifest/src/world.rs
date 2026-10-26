//! World profiles (spec Tooling §16.1.4, decisions T213, T401): the JSON
//! files under `tests/profiles/` a test or a run starts in. Their model
//! here gives the schema (`schema/world-profile-0.1.json`, completion in
//! editors) and the checks (E4400, E4401): an unknown part at the top is an
//! error (the world ignores it); within a part the known fields are typed
//! and others pass to the engine's controls as they are.

use crate::load::{json_syntax_error, schema_error, span_of};
use crate::locate::JsonPath;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use wowlua_diagnostics::{Diagnostic, Diagnostics};
use wowlua_span::{FileId, SourceDb, Span};

/// The schema's version (its file name and `$id`).
pub const SCHEMA_VERSION: &str = "0.1";

/// The schema's URL, for a profile's `$schema`.
pub fn schema_url() -> String {
    format!(
        "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/world-profile-{SCHEMA_VERSION}.json"
    )
}

/// Where a package's world profiles are, relative to its root.
pub const PROFILES_DIR: &str = "tests/profiles";

/// Other fields, passed to the engine's controls as they are.
type Rest = BTreeMap<String, Value>;

/// The situation a test or a run starts in, applied through `wowlua:mock`'s
/// controls before the addon loads.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[schemars(
    title = "World profile",
    description = "The world a wowlua test or run starts in (tests/profiles/<name>.json)"
)]
pub struct WorldProfile {
    /// This schema's URL, for editors.
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    /// The client a recorded profile comes from (`/stdtest snapshot`).
    pub client: Option<String>,
    /// The leader boards a snapshot recorded (kept, not applied).
    pub leader_boards: Option<Value>,
    /// The player's character (`mock.setPlayer`).
    pub character: Option<UnitData>,
    /// Item data by item ID (`mock.defineItem`).
    pub items: Option<List<Item>>,
    /// Spell data by spell ID (`mock.defineSpell`).
    pub spells: Option<List<Spell>>,
    /// The character's money, in copper.
    pub money: Option<f64>,
    /// The character's stats by key (`mock.player:setStats`).
    pub stats: Option<Rest>,
    /// What is worn, by slot ID or the client's slot name
    /// (`MainHandSlot`): an item ID or an item.
    pub equipment: Option<List<ItemRef>>,
    /// The bags by bag ID (0 the backpack).
    pub bags: Option<List<Bag>>,
    /// The action bars' actions by slot (`mock.setAction`).
    pub actions: Option<List<Action>>,
    /// The account's and the character's macros (`mock.setMacros`).
    pub macros: Option<Macros>,
    /// The mount journal by mount ID (`mock.setMounts`).
    pub mounts: Option<List<Mount>>,
    /// The toy box by item ID (`mock.setToys`).
    pub toys: Option<List<Toy>>,
    /// The wardrobe (`mock.setAppearances`).
    pub appearances: Option<Appearances>,
    /// The chat channels the character is in (`mock.setChannels`).
    pub channels: Option<Channels>,
    /// Where the character is (`mock.moveTo`).
    pub position: Option<Position>,
    /// The character's auras (`mock.player:applyAura`).
    pub auras: Option<List<Aura>>,
    /// The units around: creatures and other players (`mock.spawnUnit`).
    pub units: Option<List<WorldUnit>>,
    /// CVars by name.
    pub cvars: Option<BTreeMap<String, Scalar>>,
    /// Key bindings: key to action (`SetBinding`).
    pub bindings: Option<BTreeMap<String, String>>,
    /// SavedVariables by global name, as the addon finds them at load.
    pub saved_variables: Option<BTreeMap<String, Value>>,
}

/// An object of items by key, or a list of them (recorded from Lua, a
/// table keyed 1, 2, 3 is a list and an empty one `{}`; the world reads
/// either with `pairs`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum List<T> {
    Items(Vec<T>),
    Keyed(BTreeMap<String, T>),
}

/// A string, number or boolean.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Scalar {
    String(String),
    Number(f64),
    Bool(bool),
}

/// A unit's data (the character's, or a unit's around).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnitData {
    pub name: Option<String>,
    /// A WoW Forever character's last name.
    pub last_name: Option<String>,
    pub realm: Option<String>,
    pub guid: Option<String>,
    pub level: Option<f64>,
    /// The class by file name (`WARRIOR`).
    pub class: Option<String>,
    pub class_id: Option<f64>,
    /// The race by file name (`Human`).
    pub race: Option<String>,
    /// 1 unknown, 2 male, 3 female.
    pub sex: Option<f64>,
    pub health: Option<f64>,
    pub health_max: Option<f64>,
    pub max_health: Option<f64>,
    pub power: Option<f64>,
    pub power_max: Option<f64>,
    pub max_power: Option<f64>,
    pub power_type: Option<f64>,
    /// `hostile`, `neutral` or `friendly`.
    pub reaction: Option<String>,
    /// A player character (not a creature).
    pub player: Option<bool>,
    pub dead: Option<bool>,
    /// Yards from the player.
    pub distance: Option<f64>,
    pub faction: Option<String>,
    pub npc_id: Option<f64>,
    pub classification: Option<String>,
    pub creature_type: Option<String>,
    pub creature_family: Option<String>,
    pub zone: Option<String>,
    pub role: Option<String>,
    pub pvp: Option<bool>,
    #[serde(flatten)]
    pub other: Rest,
}

/// A unit around the player, and how it relates to them.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorldUnit {
    #[serde(flatten)]
    pub data: UnitData,
    /// The player's target.
    pub target: Option<bool>,
    /// The player's focus.
    pub focus: Option<bool>,
    /// In the player's group: `party` or `raid`.
    pub group: Option<GroupKind>,
    /// Its nameplate shows.
    pub nameplate: Option<bool>,
    /// Another player accepts the player's invitations (true by default).
    pub accepts_invites: Option<bool>,
    pub auras: Option<List<Aura>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    Party,
    Raid,
}

/// Item data (`GetItemInfo`'s fields by name).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub name: Option<String>,
    pub link: Option<String>,
    pub quality: Option<f64>,
    pub item_level: Option<f64>,
    pub min_level: Option<f64>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub sub_type: Option<String>,
    pub stack_count: Option<f64>,
    pub equip_loc: Option<String>,
    pub icon: Option<Scalar>,
    pub sell_price: Option<f64>,
    pub class_id: Option<f64>,
    pub subclass_id: Option<f64>,
    pub bind_type: Option<f64>,
    pub expansion_id: Option<f64>,
    /// In the client's cache (true unless false).
    pub cached: Option<bool>,
    /// Seconds until its data loads.
    pub delay: Option<f64>,
    /// The server does not know it.
    pub missing: Option<bool>,
    #[serde(flatten)]
    pub other: Rest,
}

/// Spell data.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Spell {
    pub name: Option<String>,
    #[serde(rename = "iconID")]
    pub icon_id: Option<f64>,
    pub description: Option<String>,
    /// The player knows it.
    pub known: Option<bool>,
    pub pet_known: Option<bool>,
    pub cooldown: Option<Value>,
    pub charges: Option<Value>,
    pub cast_time: Option<f64>,
    pub min_range: Option<f64>,
    pub max_range: Option<f64>,
    pub cached: Option<bool>,
    #[serde(flatten)]
    pub other: Rest,
}

/// An item ID, or an item stack.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ItemRef {
    Id(f64),
    Stack(ItemStack),
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemStack {
    pub id: f64,
    pub count: Option<f64>,
    pub link: Option<String>,
    pub bound: Option<bool>,
    pub locked: Option<bool>,
    /// `{ remaining, duration }` in seconds.
    pub cooldown: Option<Value>,
    #[serde(flatten)]
    pub other: Rest,
}

/// A bag: its size and what is in it by slot.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Bag {
    pub slots: Option<f64>,
    pub items: Option<List<ItemRef>>,
    #[serde(flatten)]
    pub other: Rest,
}

/// An action bar slot's action.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// `spell`, `item`, `macro`, …
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub id: Option<Scalar>,
    pub sub_type: Option<String>,
    pub texture: Option<Scalar>,
    /// A number, or `""` as the client reports none.
    pub count: Option<Scalar>,
    pub text: Option<String>,
    /// A boolean, or `{ usable, noPower }`.
    pub usable: Option<Value>,
    #[serde(flatten)]
    pub other: Rest,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Macros {
    pub account: Option<List<Macro>>,
    pub character: Option<List<Macro>>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Macro {
    pub name: Option<String>,
    pub icon: Option<Scalar>,
    pub body: Option<String>,
    #[serde(flatten)]
    pub other: Rest,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Mount {
    #[serde(rename = "spellID")]
    pub spell_id: Option<f64>,
    pub name: Option<String>,
    pub icon: Option<Scalar>,
    pub collected: Option<bool>,
    pub usable: Option<bool>,
    pub favorite: Option<bool>,
    #[serde(flatten)]
    pub other: Rest,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Toy {
    pub collected: Option<bool>,
    pub usable: Option<bool>,
    pub favorite: Option<bool>,
    pub fanfare: Option<bool>,
    #[serde(flatten)]
    pub other: Rest,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Appearances {
    /// Collected source IDs.
    pub collected: Option<List<f64>>,
    /// Favourite appearance IDs.
    pub favorites: Option<List<f64>>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Channels {
    pub joined: Option<List<Rest>>,
    pub group_channels: Option<Value>,
    pub regional_service: Option<Value>,
}

/// Where the character is.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    #[serde(rename = "mapID")]
    pub map_id: Option<f64>,
    pub zone: Option<String>,
    pub sub_zone: Option<String>,
    /// On the map, 0 to 1.
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// `UnitPosition`'s world coordinates.
    pub world: Option<Value>,
    /// Radians counter-clockwise from north.
    pub facing: Option<f64>,
    #[serde(flatten)]
    pub other: Rest,
}

/// An aura (`applyAura`'s data).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Aura {
    pub spell_id: Option<f64>,
    pub name: Option<String>,
    pub icon: Option<Scalar>,
    /// Seconds.
    pub duration: Option<f64>,
    pub applications: Option<f64>,
    pub harmful: Option<bool>,
    pub source: Option<String>,
    pub dispel_name: Option<String>,
    pub stealable: Option<bool>,
    #[serde(flatten)]
    pub other: Rest,
}

/// The world profile schema (`schema/world-profile-0.1.json`).
pub fn schema_json() -> String {
    let mut schema = serde_json::to_value(schemars::schema_for!(WorldProfile))
        .expect("the world profile schema serializes");
    schema["$id"] = schema_url().into();
    let mut text = serde_json::to_string_pretty(&schema).expect("schema serializes");
    text.push('\n');
    text
}

/// The parts whose keys are IDs (whole numbers).
const ID_KEYS: &[(&str, &str)] = &[
    ("items", "item IDs"),
    ("spells", "spell IDs"),
    ("bags", "bag IDs"),
    ("actions", "action slots"),
    ("mounts", "mount IDs"),
    ("toys", "item IDs"),
];

/// Checks a world profile's text (`path` names it): E4400 when it is no
/// JSON object, E4401 when a part does not match the schema.
pub fn check(db: &mut SourceDb, path: &Path, text: String) -> (FileId, Diagnostics) {
    let file = db.add(path, text);
    let text = db.get(file).text();
    let mut diagnostics = Diagnostics::new();
    let raw: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(json_syntax_error(
                db,
                file,
                &error,
                "E4400",
                "a world profile",
            ));
            return (file, diagnostics);
        }
    };
    if !raw.is_object() {
        diagnostics.push(
            Diagnostic::error("E4400", "a world profile is a JSON object")
                .with_label(Span::new(file, 0..text.len().min(1)), "expected `{`"),
        );
        return (file, diagnostics);
    }
    let mut deserializer = serde_json::Deserializer::from_str(text);
    if let Err(error) = serde_path_to_error::deserialize::<_, WorldProfile>(&mut deserializer) {
        diagnostics.push(schema_error(text, file, &error, "E4401"));
        return (file, diagnostics);
    }
    for (part, what) in ID_KEYS {
        let Some(map) = raw.get(*part).and_then(Value::as_object) else {
            continue;
        };
        for key in map.keys() {
            if key.parse::<i64>().is_err() {
                let at = JsonPath::root().key(part).key(key);
                diagnostics.push(
                    Diagnostic::error("E4401", format!("`{part}` is keyed by {what}, not `{key}`"))
                        .with_label(span_of(text, file, &at, false), "not a whole number"),
                );
            }
        }
    }
    (file, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(text: &str) -> Vec<(String, String)> {
        let mut db = SourceDb::new();
        let (_, found) = check(
            &mut db,
            Path::new("tests/profiles/p.json"),
            text.to_string(),
        );
        found
            .into_vec()
            .into_iter()
            .map(|d| (d.code.unwrap_or_default().to_string(), d.message))
            .collect()
    }

    #[test]
    fn the_spec_s_example_is_a_profile() {
        let example = r#"{
  "character": { "name": "Thrall", "realm": "Realm", "class": "SHAMAN", "race": "Orc", "level": 60 },
  "money": 123456,
  "items": { "19019": { "name": "Thunderfury, Blessed Blade of the Windseeker", "quality": 5 } },
  "spells": { "403": { "name": "Lightning Bolt", "known": true } },
  "equipment": { "MainHandSlot": 19019 },
  "bags": { "0": { "slots": 16, "items": { "1": { "id": 159, "count": 5 } } } },
  "position": { "mapID": 1453, "zone": "Stormwind City", "subZone": "Trade District", "x": 0.5, "y": 0.6 },
  "auras": [ { "spellId": 546, "name": "Water Walking", "duration": 600 } ],
  "units": [ { "name": "Hogger", "level": 11, "reaction": "hostile", "distance": 12, "target": true },
             { "name": "Jaina", "lastName": "Proudmoore", "player": true, "class": "MAGE", "race": "Human", "level": 60 } ],
  "cvars": { "nameplateShowEnemies": "1" },
  "bindings": { "SHIFT-1": "ACTIONBUTTON1" },
  "mounts": { "125": { "name": "Riding Turtle", "spellID": 30174, "icon": 132199, "favorite": true } },
  "toys": { "54452": { "favorite": true } },
  "appearances": { "collected": [21598], "favorites": [11257] },
  "savedVariables": { "MyAddonDB": { "logins": 41 } }
}"#;
        assert_eq!(codes(example), Vec::<(String, String)>::new());
    }

    #[test]
    fn mistakes_are_located() {
        let found = codes(r#"{ "monies": 5 }"#);
        assert_eq!(found[0].0, "E4401");
        assert!(found[0].1.contains("unknown key `monies`"), "{found:?}");
        assert_eq!(codes(r#"{ "money": "lots" }"#)[0].0, "E4401");
        assert_eq!(
            codes(r#"{ "units": [ { "group": "guild" } ] }"#)[0].0,
            "E4401"
        );
        assert_eq!(codes(r#"{ "items": { "sword": {} } }"#)[0].0, "E4401");
        assert_eq!(codes("{ \"money\": 5, }")[0].0, "E4400");
        assert_eq!(codes("[1]")[0].0, "E4400");
    }

    /// Every profile of the repository: the recorded fresh character, the
    /// examples' and the client's snapshots.
    #[test]
    fn the_repository_s_profiles_are_profiles() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = vec![root.join("crates/wowlua-engine/data/camelot-default-character.json")];
        for example in ["typed-demo", "widget-demo"] {
            let dir = root.join("examples").join(example).join(PROFILES_DIR);
            #[allow(clippy::disallowed_methods)]
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                files.push(e.path());
            }
        }
        // the client's snapshots (`/stdtest snapshot`), as `xtask profile`
        // writes them
        fn snapshots(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            #[allow(clippy::disallowed_methods)]
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    snapshots(&p, out);
                } else if p.extension().is_some_and(|x| x == "json")
                    && std::fs::read_to_string(&p).is_ok_and(|t| t.contains("\"character\""))
                {
                    out.push(p);
                }
            }
        }
        snapshots(&root.join("crates/wowlua-engine/tests/client"), &mut files);
        assert!(files.len() > 50, "{}", files.len());
        for file in files {
            #[allow(clippy::disallowed_methods)]
            let text = std::fs::read_to_string(&file).unwrap();
            let mut db = SourceDb::new();
            let (_, found) = check(&mut db, &file, text);
            let found: Vec<String> = found.into_vec().into_iter().map(|d| d.message).collect();
            assert!(found.is_empty(), "{}: {found:?}", file.display());
        }
    }

    #[test]
    fn a_part_keeps_fields_the_schema_does_not_name() {
        assert_eq!(
            codes(r#"{ "character": { "name": "A", "honorLevel": 3 } }"#),
            Vec::<(String, String)>::new()
        );
    }
}
