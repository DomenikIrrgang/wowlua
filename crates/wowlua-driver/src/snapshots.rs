//! The world snapshots the dev window takes in the game (`/wl snapshot`,
//! spec Debugging §7.7.10, decision T498): found in `WowluaDevDB_<Addon>`
//! in the installation's SavedVariables, read without running Lua, and
//! made into world profiles (`tests/profiles/<name>.json`) with the
//! character's and other players' names scrubbed. wowlua studio imports
//! them; `cargo xtask profile` makes StdTest's the same way.

use crate::recordings::each_saved;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// A snapshot in one SavedVariables file.
pub struct Found {
    /// The client folder (`_retail_`).
    pub client: String,
    pub account: String,
    pub path: PathBuf,
    /// When it was taken (the game's `time()`), if recorded.
    pub at: Option<f64>,
    /// Its data, as a world profile's parts (not scrubbed yet).
    pub profile: Value,
    /// The parts the client refused (`part: error`).
    pub failed: Vec<String>,
}

/// Every snapshot of the dev data `db` (`recordings::db_name`) in the
/// installation (the `client` and `account` given only), newest first, and
/// a warning for each file that can't be read.
pub fn find(
    wow: &Path,
    db: &str,
    client: Option<&str>,
    account: Option<&str>,
) -> (Vec<Found>, Vec<String>) {
    let mut out = Vec::new();
    let warnings = each_saved(wow, db, client, account, |client, account, path, value| {
        let Some(snapshot) = value.get("snapshot") else {
            return;
        };
        let Some(profile) = snapshot.get("profile") else {
            return;
        };
        out.push(Found {
            client,
            account,
            path,
            at: snapshot.get("at").and_then(Value::as_f64),
            profile: from_saved(profile.clone()),
            failed: crate::recordings::list(snapshot.get("failed").unwrap_or(&Value::Null))
                .iter()
                .filter_map(|f| f.as_str().map(str::to_string))
                .collect(),
        });
    });
    out.sort_by(|a, b| b.at.unwrap_or(0.0).total_cmp(&a.at.unwrap_or(0.0)));
    (out, warnings)
}

/// SavedVariables' tables (`recordings::parse_saved`) as the snapshot's
/// Lua tables are profile data: a table of the keys 1..n an array, a
/// sparse one (action slots) an object of its keys, and a recorded answer
/// (its length in `n`) an array of `n` values, nils null.
pub fn from_saved(v: Value) -> Value {
    match v {
        Value::Array(items) if items.iter().any(Value::is_null) => Value::Object(
            items
                .into_iter()
                .enumerate()
                .filter(|(_, v)| !v.is_null())
                .map(|(i, v)| ((i + 1).to_string(), from_saved(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(from_saved).collect()),
        Value::Object(map) => {
            let map: Map<String, Value> =
                map.into_iter().map(|(k, v)| (k, from_saved(v))).collect();
            let length = map.get("n").and_then(Value::as_i64);
            if let Some(n) = length
                && n >= 0
                && map
                    .keys()
                    .all(|k| k == "n" || k.parse::<i64>().is_ok_and(|i| i >= 1 && i <= n))
            {
                let mut list = vec![Value::Null; n as usize];
                for (k, v) in map {
                    if let Ok(i) = k.parse::<usize>() {
                        list[i - 1] = v;
                    }
                }
                return Value::Array(list);
            }
            Value::Object(map)
        }
        other => other,
    }
}

/// A snapshot's data as a world profile's JSON: the character's name and
/// realm and other players' names scrubbed (`Player`, `Realm`,
/// `Player2`…), the chat channels as `mock.setChannels` takes them, CVars
/// that may name the account or the character left out, the refused parts'
/// errors dropped; with the names it scrubbed (for other text recorded
/// with it).
pub fn profile_text(snapshot: &Value) -> Result<(String, Vec<String>), String> {
    let mut profile = snapshot.clone();
    let obj = profile
        .as_object_mut()
        .ok_or_else(|| "the snapshot is no table".to_string())?;
    let mut secrets: Vec<String> = Vec::new();
    if let Some(c) = obj.get_mut("character").and_then(|c| c.as_object_mut()) {
        for (key, scrubbed) in [("name", "Player"), ("realm", "Realm")] {
            if let Some(v) = c.get(key).and_then(|v| v.as_str()) {
                secrets.push(v.to_string());
            }
            c.insert(key.into(), scrubbed.into());
        }
    }
    if let Some(units) = obj.get_mut("units").and_then(|u| u.as_array_mut()) {
        for (i, u) in units.iter_mut().enumerate() {
            if let Some(u) = u.as_object_mut()
                && u.get("player").and_then(|p| p.as_bool()) == Some(true)
            {
                if let Some(v) = u.get("name").and_then(|v| v.as_str()) {
                    secrets.push(v.to_string());
                }
                u.insert("name".into(), format!("Player{}", i + 2).into());
            }
        }
    }
    if let Some(raw) = obj.remove("channels") {
        obj.insert("channels".into(), channels(&raw));
    }
    if let Some(cvars) = obj.get_mut("cvars").and_then(|c| c.as_object_mut()) {
        cvars.retain(|k, v| {
            !identifying(k)
                && !v.as_str().is_some_and(|s| {
                    secrets
                        .iter()
                        .any(|x| !x.is_empty() && s.contains(x.as_str()))
                })
        });
    }
    obj.remove("errors");
    let text = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
    // Anything else naming the character (an item link would not).
    let text = secrets
        .iter()
        .filter(|s| s.len() > 2)
        .fold(text, |t, s| t.replace(s.as_str(), "Player"));
    Ok((text + "\n", secrets))
}

/// The chat channels as a profile's `channels` (`mock.setChannels`), from
/// the client's lists (`GetChannelList`: number, name, disabled; the
/// pane's rows: a channel's category; `GetChannelName`: its full name;
/// `GetChannelInfoFromIdentifier` and `IsChannelRegional`: its info).
fn channels(raw: &Value) -> Value {
    let list = raw["list"].as_array().cloned().unwrap_or_default();
    let display = raw["display"].as_array().cloned().unwrap_or_default();
    let names = raw["names"].as_array().cloned().unwrap_or_default();
    let mut joined = Vec::new();
    for triple in list.chunks(3) {
        let [number, name, disabled] = triple else {
            continue;
        };
        let number = number.as_i64().unwrap_or(0);
        let name = name.as_str().unwrap_or("");
        // Its row in the pane (not a header).
        let category = display.iter().find_map(|row| {
            let row = row.as_array()?;
            (row.first()?.as_str()? == name && row.get(1)?.as_bool() == Some(false))
                .then(|| row.get(6)?.as_str().map(str::to_string))?
        });
        let full_name = names
            .get((number - 1).max(0) as usize)
            .and_then(|n| n.get(1))
            .and_then(|n| n.as_str())
            .unwrap_or(name);
        let mut channel = serde_json::json!({
            "name": name,
            "fullName": full_name,
            "category": category.unwrap_or_else(|| "CHANNEL_CATEGORY_CUSTOM".into()),
            "disabled": disabled.as_bool().unwrap_or(false),
        });
        // Its info (`GetChannelInfoFromIdentifier`) and regional flag, by
        // number.
        let key = number.to_string();
        let info = raw["infos"].get(&key).or_else(|| {
            raw["infos"]
                .as_array()
                .and_then(|a| a.get((number - 1).max(0) as usize))
        });
        if let Some(info) = info.and_then(|i| i.get(0)).and_then(|i| i.as_object()) {
            for k in ["shortcut", "instanceID", "zoneChannelID", "channelType"] {
                if let Some(v) = info.get(k) {
                    channel[k] = v.clone();
                }
            }
        }
        let regional = raw["regional"].get(&key).or_else(|| {
            raw["regional"]
                .as_array()
                .and_then(|a| a.get((number - 1).max(0) as usize))
        });
        if let Some(r) = regional.and_then(|r| r.get(0)).and_then(|r| r.as_bool()) {
            channel["regional"] = r.into();
        }
        joined.push(channel);
    }
    let group = raw["groupChannels"]
        .get(0)
        .cloned()
        .unwrap_or(serde_json::json!(0));
    let mut out = serde_json::json!({ "joined": joined, "groupChannels": group });
    if let Some(b) = raw["regionalService"].get(0).and_then(|b| b.as_bool()) {
        out["regionalService"] = b.into();
    }
    out
}

/// CVars whose names suggest they hold the account, the realm or the
/// character.
fn identifying(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "account",
        "realm",
        "portal",
        "password",
        "lastcharacter",
        "character",
        "battlenet",
        "bnet",
    ]
    .iter()
    .any(|w| n.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the client writes for `/wl snapshot` (shortened): the profile's
    /// tables as Lua keeps them.
    const SAVED: &[u8] = br#"
WowluaDevDB_Probe = {
	["snapshot"] = {
		["at"] = 1793170000,
		["failed"] = {
			"toys: no toy box", -- [1]
		},
		["profile"] = {
			["character"] = {
				["name"] = "Suuna",
				["realm"] = "Golemagg",
				["class"] = "SHAMAN",
				["level"] = 60,
			},
			["money"] = 123456,
			["actions"] = {
				[1] = { ["type"] = "spell", ["id"] = 403 },
				[2] = { ["type"] = "spell", ["id"] = 324 },
				[5] = { ["type"] = "item", ["id"] = 159 },
			},
			["stats"] = {
				["stat1"] = { ["n"] = 4, 120, 100, nil, 20 },
			},
			["units"] = {
				{ ["name"] = "Hogger", ["level"] = 11, ["target"] = true }, -- [1]
				{ ["name"] = "Thrall", ["player"] = true }, -- [2]
			},
			["cvars"] = {
				["nameplateShowEnemies"] = "1",
				["lastCharacterIndex"] = "3",
				["chatName"] = "Suuna",
			},
			["auras"] = {
			},
		},
	},
}
"#;

    fn found() -> Found {
        let vars = crate::recordings::parse_saved(SAVED).unwrap();
        let snapshot = vars[0].1.get("snapshot").unwrap();
        Found {
            client: "_classic_era_".into(),
            account: "A".into(),
            path: PathBuf::new(),
            at: snapshot.get("at").and_then(Value::as_f64),
            profile: from_saved(snapshot["profile"].clone()),
            failed: Vec::new(),
        }
    }

    #[test]
    fn lua_tables_read_as_the_profile_s_parts() {
        let f = found();
        assert_eq!(f.at, Some(1793170000.0));
        // Sparse action slots: an object of their slots.
        let actions = f.profile["actions"].as_object().unwrap();
        assert_eq!(actions.keys().collect::<Vec<_>>(), ["1", "2", "5"]);
        // A recorded answer keeps its nils.
        assert_eq!(
            f.profile["stats"]["stat1"],
            serde_json::json!([120, 100, null, 20])
        );
        assert_eq!(f.profile["units"][1]["name"], "Thrall");
    }

    #[test]
    fn the_profile_is_scrubbed_and_checks_clean() {
        let (text, secrets) = profile_text(&found().profile).unwrap();
        assert_eq!(secrets, ["Suuna", "Golemagg", "Thrall"]);
        assert!(!text.contains("Suuna") && !text.contains("Golemagg") && !text.contains("Thrall"));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["character"]["name"], "Player");
        assert_eq!(v["units"][1]["name"], "Player3");
        // CVars that name the character or the account are left out.
        let cvars = v["cvars"].as_object().unwrap();
        assert_eq!(cvars.keys().collect::<Vec<_>>(), ["nameplateShowEnemies"]);
        let mut db = wowlua_span::SourceDb::new();
        let (_, found) =
            wowlua_manifest::world::check(&mut db, Path::new("tests/profiles/snapshot.json"), text);
        let messages: Vec<String> = found.into_vec().into_iter().map(|d| d.message).collect();
        assert!(messages.is_empty(), "{messages:?}");
    }
}
