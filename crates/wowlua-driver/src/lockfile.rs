//! `wowlua.lock` (spec §10.6): pins the flavor-table version and the
//! interface numbers of every targeted flavor, so every machine generates
//! identical TOCs, and the resolved dependencies with their pins and
//! checksums.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const LOCKFILE: &str = "wowlua.lock";
pub const LOCKFILE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Lockfile {
    pub version: u32,
    pub flavor_table: FlavorPins,
    #[serde(default)]
    pub packages: Vec<wowlua_pkg::LockedPackage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FlavorPins {
    pub version: String,
    pub interfaces: BTreeMap<String, Vec<u32>>,
    /// The client aliases the manifest uses (`anniversary`,
    /// `forever`) and the ruleset each named when it was pinned (spec
    /// Manifest §9.4.1): `wowlua types sync` reports W4313 when the flavor
    /// table moves one.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, String>,
}

#[derive(Debug)]
pub enum LockError {
    /// Unreadable or not a valid lockfile.
    Invalid(String),
    /// Written by a newer compiler (E4320).
    TooNew(u32),
}

impl Lockfile {
    /// Reads `wowlua.lock` from the package root, if present.
    pub fn read(root: &Path) -> Result<Option<Lockfile>, LockError> {
        let path = root.join(LOCKFILE);
        let Ok(text) = crate::host::read_to_string(&path) else {
            return Ok(None);
        };
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| LockError::Invalid(e.to_string()))?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if version > u64::from(LOCKFILE_VERSION) {
            return Err(LockError::TooNew(
                u32::try_from(version).unwrap_or(u32::MAX),
            ));
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|e| LockError::Invalid(e.to_string()))
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("lockfiles serialize");
        text.push('\n');
        text
    }

    pub fn write(&self, root: &Path) -> std::io::Result<()> {
        crate::host::write(root.join(LOCKFILE), self.to_json())
    }
}

/// The client aliases (`anniversary`, `forever`) among the keys of
/// the manifest's `flavors`, lowercase.
pub fn client_aliases(manifest: &wowlua_manifest::LoadedManifest) -> Vec<String> {
    let table = wowlua_flavors::table();
    let mut aliases: Vec<String> = manifest
        .manifest
        .flavors
        .iter()
        .flatten()
        .filter_map(|(name, _)| {
            let (_, alias) = table.alias(name)?;
            (alias.kind == wowlua_flavors::AliasKind::Client).then(|| alias.name.clone())
        })
        .collect();
    aliases.sort();
    aliases.dedup();
    aliases
}

/// Computes the lockfile a build needs. `pinned` is the current lockfile;
/// `flavors` are the targeted flavors that take their interface numbers from
/// the lockfile (flavors with explicit `interface` in the manifest are not
/// pinned). Existing pins are kept; missing ones come from the shipped
/// flavor table; pins of flavors no longer targeted are dropped. `aliases`
/// are the client aliases the manifest uses: a new lockfile pins what they
/// name; an existing one keeps its alias pins (of the aliases still used)
/// and adds none, so only `wowlua types sync` changes them.
pub fn required(pinned: Option<&Lockfile>, flavors: &[String], aliases: &[String]) -> Lockfile {
    let table = wowlua_flavors::table();
    let aliases = match pinned {
        Some(lock) => lock
            .flavor_table
            .aliases
            .iter()
            .filter(|(alias, _)| aliases.contains(alias))
            .map(|(a, f)| (a.clone(), f.clone()))
            .collect(),
        None => alias_pins(table, aliases),
    };
    let version = pinned
        .map(|l| l.flavor_table.version.clone())
        .unwrap_or_else(|| table.version.clone());
    let interfaces = flavors
        .iter()
        .map(|id| {
            let pins = pinned
                .and_then(|l| l.flavor_table.interfaces.get(id).cloned())
                .unwrap_or_else(|| {
                    table
                        .flavor(id)
                        .map(|f| f.interface.clone())
                        .unwrap_or_default()
                });
            (id.clone(), pins)
        })
        .collect();
    Lockfile {
        version: LOCKFILE_VERSION,
        flavor_table: FlavorPins {
            version,
            interfaces,
            aliases,
        },
        packages: pinned.map(|l| l.packages.clone()).unwrap_or_default(),
    }
}

/// What each alias names in `table`.
fn alias_pins(table: &wowlua_flavors::FlavorTable, aliases: &[String]) -> BTreeMap<String, String> {
    aliases
        .iter()
        .filter_map(|a| table.resolve(a).map(|f| (a.clone(), f.id.clone())))
        .collect()
}

/// A client alias `wowlua types sync` moved to another ruleset (W4313).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasMove {
    pub alias: String,
    pub from: String,
    pub to: String,
}

/// `wowlua types sync` (spec Package manager §10.9, Manifest §9.4.1): the
/// lockfile pinned to `table`: its version, the interface numbers of the
/// pinned flavors (of `only`, when given), and what the client `aliases`
/// the manifest uses name now, with the aliases that moved to another
/// ruleset.
pub fn sync(
    pinned: &Lockfile,
    table: &wowlua_flavors::FlavorTable,
    only: Option<&[String]>,
    aliases: &[String],
) -> (Lockfile, Vec<AliasMove>) {
    let mut lock = pinned.clone();
    lock.flavor_table.version = table.version.clone();
    for (id, pins) in &mut lock.flavor_table.interfaces {
        if only.is_some_and(|only| !only.contains(id)) {
            continue;
        }
        if let Some(flavor) = table.flavor(id) {
            pins.clone_from(&flavor.interface);
        }
    }
    let now = alias_pins(table, aliases);
    let moved = now
        .iter()
        .filter_map(|(alias, to)| {
            let from = pinned.flavor_table.aliases.get(alias)?;
            (from != to).then(|| AliasMove {
                alias: alias.clone(),
                from: from.clone(),
                to: to.clone(),
            })
        })
        .collect();
    lock.flavor_table.aliases = now;
    (lock, moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_existing_pins_and_adds_missing_ones() {
        let old = Lockfile {
            version: 1,
            flavor_table: FlavorPins {
                version: "2026.01.01".into(),
                interfaces: BTreeMap::from([
                    ("standard".to_string(), vec![110200]),
                    ("cata".to_string(), vec![40402]),
                ]),
                aliases: BTreeMap::new(),
            },
            packages: vec![],
        };
        let new = required(Some(&old), &["standard".into(), "vanilla".into()], &[]);
        assert_eq!(new.flavor_table.version, "2026.01.01");
        assert_eq!(new.flavor_table.interfaces["standard"], vec![110200]);
        assert_eq!(new.flavor_table.interfaces["vanilla"], vec![11509]);
        assert!(!new.flavor_table.interfaces.contains_key("cata"));
    }

    #[test]
    fn round_trips_and_rejects_newer_versions() {
        let dir = tempfile::tempdir().unwrap();
        let lock = required(None, &["standard".into()], &["anniversary".into()]);
        assert_eq!(lock.flavor_table.aliases["anniversary"], "tbc");
        lock.write(dir.path()).unwrap();
        assert_eq!(Lockfile::read(dir.path()).unwrap(), Some(lock));

        crate::host::write(dir.path().join(LOCKFILE), r#"{ "version": 99 }"#).unwrap();
        assert!(matches!(
            Lockfile::read(dir.path()),
            Err(LockError::TooNew(99))
        ));
    }

    /// A flavor table of another date, where `anniversary` names `vanilla`.
    fn moved_table() -> wowlua_flavors::FlavorTable {
        let mut table: serde_json::Value =
            serde_json::from_str(include_str!("../../wowlua-flavors/data/flavors.json")).unwrap();
        table["version"] = "2027.01.01".into();
        for flavor in table["flavors"].as_array_mut().unwrap() {
            match flavor["id"].as_str().unwrap() {
                "tbc" => flavor["aliases"] = serde_json::json!([]),
                "vanilla" => {
                    flavor["aliases"] = serde_json::json!([
                        { "name": "classic-era", "kind": "ruleset" },
                        { "name": "anniversary", "kind": "client" }
                    ]);
                }
                "standard" => flavor["interface"] = serde_json::json!([120200]),
                _ => {}
            }
        }
        serde_json::from_value(table).unwrap()
    }

    #[test]
    fn sync_moves_pins_to_the_table_and_reports_moved_aliases() {
        let old = Lockfile {
            version: 1,
            flavor_table: FlavorPins {
                version: "2026.01.01".into(),
                interfaces: BTreeMap::from([
                    ("standard".to_string(), vec![110200]),
                    ("vanilla".to_string(), vec![11507]),
                ]),
                aliases: BTreeMap::from([("anniversary".to_string(), "tbc".to_string())]),
            },
            packages: vec![],
        };
        let table = moved_table();
        let (new, moved) = sync(&old, &table, None, &["anniversary".into()]);
        assert_eq!(new.flavor_table.version, "2027.01.01");
        assert_eq!(new.flavor_table.interfaces["standard"], vec![120200]);
        assert_eq!(new.flavor_table.interfaces["vanilla"], vec![11509]);
        assert_eq!(new.flavor_table.aliases["anniversary"], "vanilla");
        assert_eq!(
            moved,
            [AliasMove {
                alias: "anniversary".into(),
                from: "tbc".into(),
                to: "vanilla".into()
            }]
        );
        // `--flavor` moves only those pins; an alias that stays is no move.
        let (new, moved) = sync(
            &old,
            wowlua_flavors::table(),
            Some(&["vanilla".into()]),
            &["anniversary".into()],
        );
        assert_eq!(new.flavor_table.interfaces["standard"], vec![110200]);
        assert_eq!(new.flavor_table.interfaces["vanilla"], vec![11509]);
        assert!(moved.is_empty());
    }
}
