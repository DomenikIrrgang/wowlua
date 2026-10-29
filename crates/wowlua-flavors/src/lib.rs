//! The flavor table (spec §9.4.1).
//!
//! A *flavor* is a WoW ruleset, identified by Blizzard's game type
//! (`camelot`, `standard`, `vanilla`, `tbc`, `mists`). A *client* is one installable
//! game client that runs a flavor (Retail, Retail PTR, WoW Forever, …). The
//! table records each flavor's display name and how far wowlua supports it,
//! interface numbers, TOC suffixes, install folders and which TOC features
//! each client supports. It is data shipped with the compiler; the version
//! in use is pinned in `wowlua.lock`.
//!
//! It is the one list of flavors every tool reads (the CLI, Studio, the
//! registry): [`FlavorTable::flavors`] is in the order pickers show them,
//! the supported flavors first.
//!
//! It also records what each flavor's game has (decision T512): its
//! classes ([`Flavor::classes`], each with the powers it has, the first
//! the one it shows), its races ([`Flavor::races`]) and its powers'
//! defaults ([`Flavor::power_defaults`]), and the power types' tokens and
//! `Enum.PowerType` values ([`FlavorTable::power_types`]).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

static TABLE: LazyLock<FlavorTable> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/flavors.json"))
        .expect("the embedded flavor table is valid")
});

/// The flavor table shipped with this compiler.
pub fn table() -> &'static FlavorTable {
    &TABLE
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FlavorTable {
    pub version: String,
    /// Every flavor wowlua knows, in the order a picker lists them: the
    /// supported ones first, then those coming soon.
    pub flavors: Vec<Flavor>,
    pub clients: Vec<Client>,
    legacy_toc_suffixes: BTreeMap<String, String>,
    /// APIs that call a passed function later (deferred) or right away
    /// (immediate), for load-order analysis (spec §5.4.3).
    #[serde(default)]
    pub load_order_markers: LoadOrderMarkers,
    /// The power types by token (`MANA`, `RAGE`, …: `UnitPowerType`'s
    /// second result, the token of `UNIT_POWER_UPDATE`) and
    /// `Enum.PowerType` value, the same in every flavor.
    #[serde(default)]
    pub power_types: Vec<PowerType>,
}

/// A power type: its token and its `Enum.PowerType` value.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PowerType {
    pub token: String,
    pub id: u32,
}

/// A class of a flavor: its file name (`UnitClass`'s second result:
/// `DRUID`) and the powers it has by token, the first the one it shows
/// unless a form or specialization shows another (`UnitPowerType`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Class {
    pub file: String,
    pub powers: Vec<String>,
    /// The power each specialization shows, by token, in specialization
    /// order (`GetSpecialization`'s 1, 2, …), where they differ (a shadow
    /// priest's insanity); empty when every one shows the first power.
    #[serde(default)]
    pub specs: Vec<String>,
}

impl Class {
    /// The power the class shows by default (`MANA` for a druid).
    pub fn shown_power(&self) -> Option<&str> {
        self.powers.first().map(String::as_str)
    }

    /// The power a specialization (`GetSpecialization`'s index, from 1)
    /// shows: its own where the table names one, else the class's first.
    pub fn shown_power_for_spec(&self, spec: usize) -> Option<&str> {
        spec.checked_sub(1)
            .and_then(|i| self.specs.get(i))
            .map(String::as_str)
            .or_else(|| self.shown_power())
    }
}

/// A power's amount: how much there is (`UnitPower`) of how much at most
/// (`UnitPowerMax`).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct PowerAmount {
    pub current: f64,
    pub max: f64,
}

/// Load-order markers (decision M2: curated in the flavor table until the
/// declaration files mark parameters `@deferred`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LoadOrderMarkers {
    /// Global functions by path (`C_Timer.After`).
    pub deferred_functions: Vec<String>,
    /// Method names on any object (`SetScript`).
    pub deferred_methods: Vec<String>,
    pub immediate_functions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    Mainline,
    Classic,
}

impl Family {
    /// The family name as used in TOC load directives (`mainline`, `classic`).
    pub fn name(self) -> &'static str {
        match self {
            Family::Mainline => "mainline",
            Family::Classic => "classic",
        }
    }

    /// The family TOC suffix (`_Mainline`, `_Classic`).
    pub fn suffix(self) -> &'static str {
        match self {
            Family::Mainline => "_Mainline",
            Family::Classic => "_Classic",
        }
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AliasKind {
    /// Names a ruleset; permanent.
    Ruleset,
    /// Names a client; may move to another ruleset when the table is updated.
    Client,
}

/// How far wowlua supports a flavor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Support {
    /// The toolchain runs it fully: the emulated client, the debugger,
    /// Blizzard's UI.
    Supported,
    /// Named and checked (manifests, types, TOCs), but not run yet.
    ComingSoon,
}

#[derive(Debug, Deserialize)]
pub struct Alias {
    pub name: String,
    pub kind: AliasKind,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Flavor {
    pub id: String,
    /// The name people know the game by (`Retail`, `WoW Forever`).
    pub name: String,
    pub support: Support,
    /// The highest level a character reaches in this flavor
    /// (`GetMaxPlayerLevel`): the bound of a level picker and of a world
    /// profile's character.
    pub max_level: u32,
    pub family: Family,
    pub aliases: Vec<Alias>,
    pub project_id: u32,
    /// Current interface numbers of the flavor's live client(s).
    pub interface: Vec<u32>,
    /// Inclusive range of interface numbers that belong to this flavor.
    pub interface_range: [u32; 2],
    pub game_suffixes: Vec<String>,
    /// The classes a character can be, in the client's order
    /// (`CLASS_SORT_ORDER`), each with its powers.
    #[serde(default)]
    pub classes: Vec<Class>,
    /// The races a character can be, by file name (`UnitRace`'s second
    /// result: `Scourge`, `NightElf`), in race ID order.
    #[serde(default)]
    pub races: Vec<String>,
    /// What a unit has of each power, by token, when nothing says (the
    /// emulated client's units; a slider's start).
    #[serde(default)]
    pub power_defaults: BTreeMap<String, PowerAmount>,
}

impl PartialEq for Flavor {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Flavor {}

impl Flavor {
    pub fn is_supported(&self) -> bool {
        self.support == Support::Supported
    }

    pub fn contains_interface(&self, interface: u32) -> bool {
        (self.interface_range[0]..=self.interface_range[1]).contains(&interface)
    }

    /// The class with this file name (any case: `druid`, `DRUID`).
    pub fn class(&self, file: &str) -> Option<&Class> {
        self.classes
            .iter()
            .find(|c| c.file.eq_ignore_ascii_case(file))
    }

    /// Whether a character can be of this race (its file name, `Scourge`).
    pub fn has_race(&self, race: &str) -> bool {
        self.races.iter().any(|r| r == race)
    }

    /// What a unit has of a power (its token) when nothing says.
    pub fn power_default(&self, token: &str) -> Option<PowerAmount> {
        self.power_defaults.get(token).copied()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ClientFeatures {
    pub comma_interfaces: bool,
    pub inline_file_directives: bool,
    pub inline_directive_lines: bool,
    pub allow_load_text_locale: bool,
    /// `[Game]`, `[Family]` and `[TextLocale]` in TOC file lines.
    #[serde(default)]
    pub file_variables: bool,
    /// The `default` key attribute on addon keybindings (W8354 where not).
    #[serde(default)]
    pub binding_defaults: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Client {
    pub name: String,
    pub flavor: String,
    pub version: String,
    pub interface: u32,
    /// Folders inside the WoW installation (`_retail_`, `_classic_era_`, …).
    pub folders: Vec<String>,
    /// TOC suffixes this client looks for, in lookup order (before the plain name).
    pub suffixes: Vec<String>,
    pub features: ClientFeatures,
    /// Game-type names recognized in `[AllowLoadGameType …]`, besides the
    /// family names, which every client with inline directives recognizes.
    pub game_types: Vec<String>,
}

impl Client {
    /// Whether `name` (a game type or a family name) is recognized in load
    /// directives by this client.
    pub fn recognizes_game_type(&self, name: &str) -> bool {
        self.features.inline_file_directives
            && (name == Family::Mainline.name()
                || name == Family::Classic.name()
                || self.game_types.iter().any(|g| g == name))
    }

    pub fn recognizes_suffix(&self, suffix: &str) -> bool {
        self.suffixes.iter().any(|s| s == suffix)
    }
}

/// What a TOC file-name suffix refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuffixMeaning<'a> {
    Flavor(&'a Flavor),
    Family(Family),
}

impl FlavorTable {
    pub fn flavor(&self, id: &str) -> Option<&Flavor> {
        self.flavors.iter().find(|f| f.id == id)
    }

    /// Resolves a flavor ID or alias (case-insensitive).
    pub fn resolve(&self, name: &str) -> Option<&Flavor> {
        let name = name.to_ascii_lowercase();
        self.flavors
            .iter()
            .find(|f| f.id == name || f.aliases.iter().any(|a| a.name == name))
    }

    /// The alias entry for `name`, if `name` is an alias rather than an ID.
    pub fn alias(&self, name: &str) -> Option<(&Flavor, &Alias)> {
        let name = name.to_ascii_lowercase();
        self.flavors
            .iter()
            .find_map(|f| f.aliases.iter().find(|a| a.name == name).map(|a| (f, a)))
    }

    /// The flavor an interface number belongs to.
    pub fn by_interface(&self, interface: u32) -> Option<&Flavor> {
        self.flavors
            .iter()
            .find(|f| f.contains_interface(interface))
    }

    /// A power type's `Enum.PowerType` value by its token (`RAGE`: 1).
    pub fn power_type(&self, token: &str) -> Option<u32> {
        self.power_types
            .iter()
            .find(|p| p.token == token)
            .map(|p| p.id)
    }

    /// A power type's token by its `Enum.PowerType` value (1: `RAGE`).
    pub fn power_token(&self, id: u32) -> Option<&str> {
        self.power_types
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.token.as_str())
    }

    pub fn clients_of<'a>(&'a self, flavor: &'a str) -> impl Iterator<Item = &'a Client> + 'a {
        self.clients.iter().filter(move |c| c.flavor == flavor)
    }

    /// Interprets a TOC file-name suffix such as `_TBC`, `_Classic`, `-BCC`.
    pub fn suffix_meaning(&self, suffix: &str) -> Option<SuffixMeaning<'_>> {
        let lower = suffix.to_ascii_lowercase();
        if lower == Family::Mainline.suffix().to_ascii_lowercase() {
            return Some(SuffixMeaning::Family(Family::Mainline));
        }
        if lower == Family::Classic.suffix().to_ascii_lowercase() {
            return Some(SuffixMeaning::Family(Family::Classic));
        }
        if let Some(flavor) = self.flavors.iter().find(|f| {
            f.game_suffixes
                .iter()
                .any(|s| s.eq_ignore_ascii_case(suffix))
        }) {
            return Some(SuffixMeaning::Flavor(flavor));
        }
        if let Some(meaning) = self
            .legacy_toc_suffixes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(suffix))
            .and_then(|(_, id)| self.flavor(id))
        {
            return Some(SuffixMeaning::Flavor(meaning));
        }
        // The client (and the packager) take `-` in place of `_`.
        let rest = suffix.strip_prefix('-')?;
        self.suffix_meaning(&format!("_{rest}"))
    }
}

/// Formats an interface number as a game version: `120100` → `12.1.0`.
pub fn interface_to_version(interface: u32) -> String {
    format!(
        "{}.{}.{}",
        interface / 10000,
        (interface / 100) % 100,
        interface % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        let t = table();
        assert!(!t.version.is_empty());
        for client in &t.clients {
            assert!(t.flavor(&client.flavor).is_some(), "{}", client.name);
            let flavor = t.flavor(&client.flavor).unwrap();
            assert!(
                flavor.contains_interface(client.interface),
                "{}",
                client.name
            );
        }
        for flavor in &t.flavors {
            assert!(t.clients_of(&flavor.id).next().is_some(), "{}", flavor.id);
            for interface in &flavor.interface {
                assert_eq!(t.by_interface(*interface).unwrap().id, flavor.id);
            }
        }
    }

    #[test]
    fn resolves_ids_and_aliases() {
        let t = table();
        assert_eq!(t.resolve("Retail").unwrap().id, "standard");
        assert_eq!(t.resolve("classic-era").unwrap().id, "vanilla");
        assert_eq!(t.resolve("anniversary").unwrap().id, "tbc");
        assert!(t.resolve("bogus").is_none());
        assert_eq!(t.alias("forever").unwrap().1.kind, AliasKind::Client);
    }

    #[test]
    fn maps_interfaces_and_suffixes() {
        let t = table();
        assert_eq!(t.by_interface(110200).unwrap().id, "standard");
        assert_eq!(t.by_interface(11507).unwrap().id, "vanilla");
        assert_eq!(t.by_interface(16001).unwrap().id, "camelot");
        assert_eq!(
            t.suffix_meaning("_Classic"),
            Some(SuffixMeaning::Family(Family::Classic))
        );
        assert!(
            matches!(t.suffix_meaning("-BCC"), Some(SuffixMeaning::Flavor(f)) if f.id == "tbc")
        );
        assert!(
            matches!(t.suffix_meaning("-Forever"), Some(SuffixMeaning::Flavor(f)) if f.id == "camelot")
        );
        assert!(
            matches!(t.suffix_meaning("_Mists"), Some(SuffixMeaning::Flavor(f)) if f.id == "mists")
        );
        assert_eq!(t.by_interface(50504).unwrap().id, "mists");
        // Wrath and Cataclysm are no flavors (decisions T507, T509).
        for suffix in ["_Cata", "_Wrath", "-WOTLKC"] {
            assert_eq!(t.suffix_meaning(suffix), None, "{suffix}");
        }
        assert!(t.by_interface(40402).is_none() && t.by_interface(38002).is_none());
        assert_eq!(interface_to_version(120100), "12.1.0");
        assert_eq!(interface_to_version(11509), "1.15.9");
    }

    #[test]
    fn classic_clients_do_not_recognize_standard() {
        let t = table();
        let era = t.clients.iter().find(|c| c.name == "classic-era").unwrap();
        assert!(era.recognizes_game_type("mainline"));
        assert!(era.recognizes_game_type("vanilla"));
        assert!(!era.recognizes_game_type("standard"));
        let mists = t
            .clients
            .iter()
            .find(|c| c.name == "mists-classic")
            .unwrap();
        assert!(mists.recognizes_game_type("classic"));
        assert!(mists.recognizes_game_type("mists"));
        assert!(mists.features.file_variables);
    }

    /// The flavors, their names and support (decisions T507, T509): camelot
    /// is supported and listed first; the others are coming soon.
    #[test]
    fn names_and_support_in_picker_order() {
        let t = table();
        let listed: Vec<(&str, &str, Support)> = t
            .flavors
            .iter()
            .map(|f| (f.id.as_str(), f.name.as_str(), f.support))
            .collect();
        assert_eq!(
            listed,
            [
                ("camelot", "WoW Forever", Support::Supported),
                ("standard", "Retail", Support::ComingSoon),
                ("vanilla", "Classic Era", Support::ComingSoon),
                ("tbc", "TBC Anniversary", Support::ComingSoon),
                ("mists", "MoP Classic", Support::ComingSoon),
            ]
        );
        assert!(t.flavor("camelot").unwrap().is_supported());
        for gone in ["cata", "wrath", "titan"] {
            assert!(t.resolve(gone).is_none(), "{gone}");
        }
        // Clients name only flavors of the table.
        for client in &t.clients {
            for game_type in &client.game_types {
                assert!(
                    t.flavor(game_type).is_some(),
                    "{}: {game_type}",
                    client.name
                );
            }
        }
    }

    /// The classes each flavor has, in `CLASS_SORT_ORDER` (Blizzard's UI of
    /// each), with their powers; every power has a token and a default
    /// (decision T512).
    #[test]
    fn classes_powers_and_races() {
        let t = table();
        let files = |id: &str| -> Vec<&str> {
            t.flavor(id)
                .unwrap()
                .classes
                .iter()
                .map(|c| c.file.as_str())
                .collect()
        };
        let classic = [
            "WARRIOR", "PALADIN", "PRIEST", "SHAMAN", "DRUID", "ROGUE", "MAGE", "WARLOCK", "HUNTER",
        ];
        for id in ["camelot", "vanilla", "tbc"] {
            assert_eq!(files(id), classic, "{id}");
        }
        assert_eq!(
            files("mists"),
            [
                "WARRIOR",
                "DEATHKNIGHT",
                "PALADIN",
                "MONK",
                "PRIEST",
                "SHAMAN",
                "DRUID",
                "ROGUE",
                "MAGE",
                "WARLOCK",
                "HUNTER"
            ]
        );
        assert_eq!(files("standard").len(), 13);
        let powers = |id: &str, class: &str| -> Vec<&str> {
            let class = t.flavor(id).unwrap().class(class).unwrap();
            class.powers.iter().map(String::as_str).collect()
        };
        // Recorded on camelot: a warrior's rage, a paladin's, a druid's and
        // a shaman's mana shown; a paladin has no other power.
        assert_eq!(powers("camelot", "WARRIOR"), ["RAGE"]);
        assert_eq!(powers("camelot", "paladin"), ["MANA"]);
        assert_eq!(
            powers("camelot", "DRUID"),
            ["MANA", "RAGE", "ENERGY", "COMBO_POINTS"]
        );
        assert_eq!(
            t.flavor("camelot")
                .unwrap()
                .class("DRUID")
                .unwrap()
                .shown_power(),
            Some("MANA")
        );
        // Hunters use mana until Cataclysm, focus from then on.
        assert_eq!(powers("vanilla", "HUNTER"), ["MANA"]);
        assert_eq!(powers("tbc", "HUNTER"), ["MANA"]);
        assert_eq!(powers("mists", "HUNTER"), ["FOCUS"]);
        assert_eq!(powers("standard", "HUNTER"), ["FOCUS"]);
        assert_eq!(powers("standard", "DEMONHUNTER"), ["FURY"]);
        assert_eq!(powers("mists", "PALADIN"), ["MANA", "HOLY_POWER"]);
        assert_eq!(powers("standard", "DEATHKNIGHT"), ["RUNIC_POWER", "RUNES"]);
        // A specialization's shown power: a shadow priest's insanity, a
        // mistweaver's mana; without a list, the class's first.
        let priest = t.flavor("standard").unwrap().class("PRIEST").unwrap();
        assert_eq!(priest.shown_power_for_spec(3), Some("INSANITY"));
        assert_eq!(priest.shown_power_for_spec(1), Some("MANA"));
        let monk = t.flavor("mists").unwrap().class("MONK").unwrap();
        assert_eq!(monk.shown_power_for_spec(2), Some("MANA"));
        let rogue = t.flavor("camelot").unwrap().class("ROGUE").unwrap();
        assert_eq!(rogue.shown_power_for_spec(2), Some("ENERGY"));
        for flavor in &t.flavors {
            for class in &flavor.classes {
                for shown in &class.specs {
                    assert!(class.powers.contains(shown), "{} {}", flavor.id, class.file);
                }
            }
        }
        assert!(t.flavor("vanilla").unwrap().class("MONK").is_none());
        for flavor in &t.flavors {
            assert!(!flavor.races.is_empty(), "{}", flavor.id);
            for class in &flavor.classes {
                assert!(!class.powers.is_empty(), "{} {}", flavor.id, class.file);
                for power in &class.powers {
                    assert!(t.power_type(power).is_some(), "{power}");
                    let d = flavor.power_default(power);
                    assert!(
                        d.is_some_and(|d| d.current <= d.max),
                        "{} {power}",
                        flavor.id
                    );
                }
            }
            for token in flavor.power_defaults.keys() {
                assert!(t.power_type(token).is_some(), "{token}");
            }
        }
        // The races by file name.
        let camelot = t.flavor("camelot").unwrap();
        assert!(camelot.has_race("Scourge") && camelot.has_race("Skyborne"));
        assert!(!camelot.has_race("BloodElf") && !camelot.has_race("Undead"));
        assert!(t.flavor("tbc").unwrap().has_race("Draenei"));
        assert!(t.flavor("mists").unwrap().has_race("Pandaren"));
        assert_eq!(t.flavor("vanilla").unwrap().races.len(), 8);
        // Enum.PowerType's values, both ways.
        assert_eq!(t.power_type("MANA"), Some(0));
        assert_eq!(t.power_type("ESSENCE"), Some(19));
        assert_eq!(t.power_token(13), Some("INSANITY"));
        assert_eq!(t.power_token(28), Some("SHADOW_ORBS"));
        assert_eq!(t.power_token(30), None);
        assert_eq!(t.power_types.len(), 30);
    }

    /// Each flavor's max level (camelot's recorded: `GetMaxPlayerLevel()`
    /// answers 60; the others are the games' caps).
    #[test]
    fn max_levels() {
        let t = table();
        let levels: Vec<(&str, u32)> = t
            .flavors
            .iter()
            .map(|f| (f.id.as_str(), f.max_level))
            .collect();
        assert_eq!(
            levels,
            [
                ("camelot", 60),
                ("standard", 90),
                ("vanilla", 60),
                ("tbc", 70),
                ("mists", 90)
            ]
        );
    }
}
