//! `wowlua.lock` (spec §10.6): pins the flavor-table version and the
//! interface numbers of every targeted flavor, so every machine generates
//! identical TOCs. Dependency entries arrive with the package manager (M6).

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
    pub packages: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FlavorPins {
    pub version: String,
    pub interfaces: BTreeMap<String, Vec<u32>>,
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
        let Ok(text) = std::fs::read_to_string(&path) else {
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
        std::fs::write(root.join(LOCKFILE), self.to_json())
    }
}

/// Computes the lockfile a build needs. `pinned` is the current lockfile;
/// `flavors` are the targeted flavors that take their interface numbers from
/// the lockfile (flavors with explicit `interface` in the manifest are not
/// pinned). Existing pins are kept; missing ones come from the shipped
/// flavor table; pins of flavors no longer targeted are dropped.
pub fn required(pinned: Option<&Lockfile>, flavors: &[String]) -> Lockfile {
    let table = wowlua_flavors::table();
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
        },
        packages: pinned.map(|l| l.packages.clone()).unwrap_or_default(),
    }
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
            },
            packages: vec![],
        };
        let new = required(Some(&old), &["standard".into(), "vanilla".into()]);
        assert_eq!(new.flavor_table.version, "2026.01.01");
        assert_eq!(new.flavor_table.interfaces["standard"], vec![110200]);
        assert_eq!(new.flavor_table.interfaces["vanilla"], vec![11509]);
        assert!(!new.flavor_table.interfaces.contains_key("cata"));
    }

    #[test]
    fn round_trips_and_rejects_newer_versions() {
        let dir = tempfile::tempdir().unwrap();
        let lock = required(None, &["standard".into()]);
        lock.write(dir.path()).unwrap();
        assert_eq!(Lockfile::read(dir.path()).unwrap(), Some(lock));

        std::fs::write(dir.path().join(LOCKFILE), r#"{ "version": 99 }"#).unwrap();
        assert!(matches!(
            Lockfile::read(dir.path()),
            Err(LockError::TooNew(99))
        ));
    }
}
