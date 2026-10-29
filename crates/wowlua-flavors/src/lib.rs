//! The flavor table (spec §9.4.1).
//!
//! A *flavor* is a WoW ruleset, identified by Blizzard's game type
//! (`camelot`, `standard`, `vanilla`, `tbc`). A *client* is one installable
//! game client that runs a flavor (Retail, Retail PTR, WoW Forever, …). The
//! table records each flavor's display name and how far wowlua supports it,
//! interface numbers, TOC suffixes, install folders and which TOC features
//! each client supports. It is data shipped with the compiler; the version
//! in use is pinned in `wowlua.lock`.
//!
//! It is the one list of flavors every tool reads (the CLI, Studio, the
//! registry): [`FlavorTable::flavors`] is in the order pickers show them,
//! the supported flavors first.

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
        // Wrath, Cataclysm and Mists of Pandaria are no flavors (decision T507).
        for suffix in ["_Mists", "_Cata", "_Wrath", "-WOTLKC"] {
            assert_eq!(t.suffix_meaning(suffix), None, "{suffix}");
        }
        assert!(t.by_interface(50504).is_none());
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
    }

    /// The flavors, their names and support (decision T507): camelot is
    /// supported and listed first; the others are coming soon.
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
            ]
        );
        assert!(t.flavor("camelot").unwrap().is_supported());
        for gone in ["mists", "cata", "wrath", "titan"] {
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
                ("tbc", 70)
            ]
        );
    }
}
