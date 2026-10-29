//! Game version mapping (Publishing §11.3): each flavor's interface numbers
//! become the site's game versions, chosen like the BigWigs packager does:
//! the exact version, else the highest one below it, else the highest one
//! of that game type; game types a site does not know are left out.
//!
//! Interface numbers are grouped by the packager's game types of wowlua's
//! flavors (`retail`, `classic`, `bcc`, `forever`).

use std::collections::BTreeMap;

/// A game type's names on the sites.
pub struct GameType {
    pub packager: &'static str,
    pub curseforge: u64,
    pub wago: &'static str,
    /// `None`: WoWInterface has no such game.
    pub wowinterface: Option<&'static str>,
    /// The flavor name in GitHub's `release.json`.
    pub release_json: &'static str,
}

pub const GAME_TYPES: &[GameType] = &[
    GameType {
        packager: "retail",
        curseforge: 517,
        wago: "retail",
        wowinterface: Some("Retail"),
        release_json: "mainline",
    },
    GameType {
        packager: "classic",
        curseforge: 67408,
        wago: "classic",
        wowinterface: Some("Classic"),
        release_json: "classic",
    },
    GameType {
        packager: "bcc",
        curseforge: 73246,
        wago: "bc",
        wowinterface: Some("TBC-Classic"),
        release_json: "bcc",
    },
    GameType {
        packager: "forever",
        curseforge: 88568,
        wago: "forever",
        wowinterface: None,
        release_json: "forever",
    },
];

pub fn game_type(name: &str) -> Option<&'static GameType> {
    GAME_TYPES.iter().find(|g| g.packager == name)
}

/// The game versions of a package: game type → `x.y.z` versions and
/// interface numbers, in interface order.
pub fn versions(
    interfaces: &BTreeMap<String, Vec<u32>>,
) -> BTreeMap<&'static str, Vec<(String, u32)>> {
    let mut out: BTreeMap<&'static str, Vec<(String, u32)>> = BTreeMap::new();
    for i in interfaces.values().flatten() {
        let t = crate::package::packager_type(*i);
        let v = wowlua_flavors::interface_to_version(*i);
        let list = out.entry(t).or_default();
        if !list.iter().any(|(x, _)| *x == v) {
            list.push((v, *i));
        }
    }
    out
}

/// A choice and the warning it needs (W4328 / W4220).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice<T> {
    Exact(T),
    Fallback {
        wanted: String,
        chosen: T,
        shown: String,
    },
    Unknown(String),
}

/// CurseForge: `GET /api/game/wow/versions` entries `{id, gameVersionTypeID,
/// name}`; the packager compares names as strings and takes the highest ID.
pub fn curseforge(list: &serde_json::Value, game: &GameType, version: &str) -> Choice<u64> {
    let entries: Vec<(u64, u64, String)> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some((
                e["id"].as_u64()?,
                e["gameVersionTypeID"].as_u64()?,
                e["name"].as_str()?.to_string(),
            ))
        })
        .collect();
    let of_type = || entries.iter().filter(|(_, t, _)| *t == game.curseforge);
    if let Some((id, _, _)) = of_type().find(|(_, _, n)| n == version) {
        return Choice::Exact(*id);
    }
    let pick = of_type()
        .filter(|(_, _, n)| n.as_str() < version)
        .max_by_key(|(id, _, _)| *id)
        .or_else(|| of_type().max_by_key(|(id, _, _)| *id));
    match pick {
        Some((id, _, name)) => Choice::Fallback {
            wanted: version.into(),
            chosen: *id,
            shown: name.clone(),
        },
        None => Choice::Unknown(version.into()),
    }
}

/// Wago: `GET /api/data/game` → `patches.<type>` lists of versions.
pub fn wago(game_data: &serde_json::Value, game: &GameType, version: &str) -> Choice<String> {
    let list: Vec<&str> = game_data["patches"][game.wago]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    if list.contains(&version) {
        return Choice::Exact(version.into());
    }
    let pick = list
        .iter()
        .filter(|v| **v < version)
        .max()
        .or_else(|| list.iter().max());
    match pick {
        Some(v) => Choice::Fallback {
            wanted: version.into(),
            chosen: v.to_string(),
            shown: v.to_string(),
        },
        None => Choice::Unknown(version.into()),
    }
}

/// WoWInterface: `GET /addons/compatible.json` → `{id, game}` entries.
pub fn wowinterface(list: &serde_json::Value, game: &GameType, version: &str) -> Choice<String> {
    let Some(name) = game.wowinterface else {
        return Choice::Unknown(version.into());
    };
    let entries: Vec<(String, String)> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some((
                e["id"].as_str()?.to_string(),
                e["game"].as_str()?.to_string(),
            ))
        })
        .collect();
    if entries.iter().any(|(id, _)| id == version) {
        return Choice::Exact(version.into());
    }
    let max_of = |game: &str, below: Option<&str>| {
        entries
            .iter()
            .filter(|(id, g)| g == game && below.is_none_or(|b| id.as_str() < b))
            .map(|(id, _)| id.clone())
            .max()
    };
    let pick = max_of(name, Some(version)).or_else(|| max_of(name, None));
    match pick {
        Some(v) => Choice::Fallback {
            wanted: version.into(),
            chosen: v.clone(),
            shown: v,
        },
        None => Choice::Unknown(version.into()),
    }
}

/// Versions sorted like `sort -Vr` (newest first).
pub fn sort_desc(list: &mut [String]) {
    let key = |v: &String| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    list.sort_by_key(|v| std::cmp::Reverse(key(v)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_versions_like_the_packager() {
        let cf = serde_json::json!([
            { "id": 1, "gameVersionTypeID": 517, "name": "12.0.0" },
            { "id": 2, "gameVersionTypeID": 517, "name": "12.0.5" },
            { "id": 3, "gameVersionTypeID": 67408, "name": "1.15.8" }
        ]);
        let retail = game_type("retail").unwrap();
        assert_eq!(curseforge(&cf, retail, "12.0.5"), Choice::Exact(2));
        assert!(matches!(
            curseforge(&cf, retail, "12.1.0"),
            Choice::Fallback { chosen: 2, .. }
        ));
        let bcc = game_type("bcc").unwrap();
        assert!(matches!(curseforge(&cf, bcc, "2.5.6"), Choice::Unknown(_)));
        let wago_data = serde_json::json!({ "patches": { "bc": ["2.5.5"], "retail": ["12.1.0"] } });
        assert!(
            matches!(wago(&wago_data, bcc, "2.5.6"), Choice::Fallback { ref chosen, .. } if chosen == "2.5.5")
        );
        let wowi = serde_json::json!([{ "id": "2.5.4", "game": "TBC-Classic" }]);
        assert!(
            matches!(wowinterface(&wowi, bcc, "2.5.6"), Choice::Fallback { ref chosen, .. } if chosen == "2.5.4")
        );
        for gone in ["wrath", "titan", "cata", "mists"] {
            assert!(game_type(gone).is_none(), "{gone}");
        }
        let mut v = vec![
            "1.15.9".to_string(),
            "12.1.0".to_string(),
            "2.5.6".to_string(),
        ];
        sort_desc(&mut v);
        assert_eq!(v, ["12.1.0", "2.5.6", "1.15.9"]);
    }
}
