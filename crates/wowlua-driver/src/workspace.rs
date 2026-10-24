//! Workspaces (spec §9.15, §10.8): one root manifest groups member
//! packages that share one lockfile, one `dist/` and their defaults. An
//! addon listing another workspace addon in `addon-dependencies` is checked
//! with that addon's sources, types only, so its `@global` API has its
//! types (`crate::foreign`).

use serde_json::Value;
use std::path::PathBuf;

/// A member addon of the workspace, whose sources the addons depending on
/// it are checked with.
#[derive(Clone, Debug)]
pub struct WorkspaceAddon {
    /// `addon.name` (the AddOn folder name).
    pub name: String,
    /// Its package name.
    pub package: String,
    /// Its source folder.
    pub src: PathBuf,
    pub manifest: wowlua_manifest::model::Manifest,
}

/// Root keys whose values members inherit (objects merge deeply; arrays and
/// scalars of the member replace).
pub const SHARED_KEYS: &[&str] = &[
    "profiles",
    "minify-levels",
    "check",
    "fmt",
    "config",
    "publish",
];

/// `over` merged onto `base`: objects deeply, everything else replaced.
pub fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// A member's manifest with the root's shared defaults beneath it.
pub fn with_defaults(member: &Value, root: &Value) -> Value {
    let mut merged = serde_json::Map::new();
    // A root that is a package itself publishes itself: its targets are
    // not the members' (decision T121).
    let root_is_package = root.get("package").is_some();
    for key in SHARED_KEYS {
        if root_is_package && *key == "publish" {
            continue;
        }
        if let Some(v) = root.get(*key) {
            let mut v = v.clone();
            // A root bundling its members keeps its targets for the bundle.
            if *key == "publish"
                && let Some(p) = v.as_object_mut()
                && p.get("bundle-addons")
                    .is_some_and(|b| b != &Value::Bool(false))
            {
                p.shift_remove("targets");
                p.shift_remove("bundle-addons");
            }
            merged.insert((*key).to_string(), v);
        }
    }
    let mut merged = Value::Object(merged);
    merge(&mut merged, member);
    // Keys keep the member's order, shared ones after.
    if let (Value::Object(m), Value::Object(own)) = (&merged, member) {
        let mut ordered = serde_json::Map::new();
        for k in own.keys() {
            ordered.insert(k.clone(), m[k].clone());
        }
        for (k, v) in m {
            if !ordered.contains_key(k) {
                ordered.insert(k.clone(), v.clone());
            }
        }
        return Value::Object(ordered);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_defaults_under_members() {
        let root = serde_json::json!({
            "workspace": { "members": ["a"] },
            "check": { "mode": "strict", "lua": true },
            "profiles": { "release": { "minify": "full" } },
            "publish": { "targets": { "cf": { "type": "curseforge", "project-id": 1 } } }
        });
        let member = serde_json::json!({
            "package": { "name": "a", "version": "1.0.0" },
            "check": { "lua": false },
            "publish": { "targets": { "cf": { "enabled": false } } }
        });
        let m = with_defaults(&member, &root);
        assert_eq!(
            m["check"],
            serde_json::json!({ "mode": "strict", "lua": false })
        );
        assert_eq!(m["profiles"]["release"]["minify"], "full");
        assert_eq!(m["publish"]["targets"]["cf"]["type"], "curseforge");
        assert_eq!(m["publish"]["targets"]["cf"]["enabled"], false);
        assert!(m.get("workspace").is_none());
        assert_eq!(m.as_object().unwrap().keys().next().unwrap(), "package");
    }
}
