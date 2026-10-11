//! Build profiles (spec §9.9): built-in defaults, `inherits` chains, deep
//! merging, and the top-level `config` with per-profile overrides.

use crate::model::{
    CacheGlobals, Instrument, JsonObject, LineMode, LogLevel, Manifest, Reflection, RuntimeChecks,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// The three built-in profiles every custom profile ultimately builds on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinProfile {
    Dev,
    Release,
    Test,
}

impl BuiltinProfile {
    pub const ALL: [BuiltinProfile; 3] = [
        BuiltinProfile::Dev,
        BuiltinProfile::Release,
        BuiltinProfile::Test,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BuiltinProfile::Dev => "dev",
            BuiltinProfile::Release => "release",
            BuiltinProfile::Test => "test",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Defaults from Debugging §7 and Optimization §15.2.
    fn defaults(self) -> Value {
        match self {
            BuiltinProfile::Dev => json!({
                "debug": true, "assertions": true, "log-level": "debug",
                "instrument": "decorated", "runtime-checks": "exports",
                "profile-errors": false, "reflection": "decorated",
                "dev-console": true, "source-maps": true,
                "bundle": false, "minify": "none", "minify-libraries": false,
                "optimize": { "inline": false, "cache-globals": "off", "dead-code": false },
            }),
            BuiltinProfile::Release => json!({
                "debug": false, "assertions": false, "log-level": "warn",
                "instrument": "none", "runtime-checks": "none",
                "profile-errors": false, "reflection": "decorated",
                "dev-console": false, "source-maps": true,
                "bundle": true, "minify": "strip", "minify-libraries": false,
                "optimize": { "inline": true, "cache-globals": "safe", "dead-code": true },
            }),
            BuiltinProfile::Test => json!({
                "debug": true, "assertions": true, "log-level": "trace",
                "instrument": "none", "runtime-checks": "all",
                "profile-errors": false, "reflection": "all",
                "dev-console": false, "source-maps": true,
                "bundle": false, "minify": "none", "minify-libraries": false,
                "optimize": { "inline": false, "cache-globals": "off", "dead-code": false },
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct Optimize {
    pub inline: bool,
    pub cache_globals: CacheGlobals,
    pub dead_code: bool,
}

/// A fully resolved profile.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Profile {
    #[serde(skip_deserializing, default = "default_name")]
    pub name: String,
    #[serde(skip_deserializing, default = "default_base")]
    pub base: BuiltinProfile,
    pub debug: bool,
    pub assertions: bool,
    pub log_level: LogLevel,
    pub instrument: Instrument,
    pub runtime_checks: RuntimeChecks,
    pub profile_errors: bool,
    pub reflection: Reflection,
    pub dev_console: bool,
    pub source_maps: bool,
    pub bundle: bool,
    pub minify: String,
    pub minify_libraries: bool,
    pub optimize: Optimize,
    /// `None` means "derived from `bundle`" (§14.5).
    #[serde(default)]
    pub line_mode: Option<LineMode>,
    #[serde(default)]
    pub config: JsonObject,
}

fn default_name() -> String {
    String::new()
}

fn default_base() -> BuiltinProfile {
    BuiltinProfile::Dev
}

impl Profile {
    /// The effective line mode: explicit, or `preserve` for unbundled and
    /// `statement` for bundled output.
    pub fn effective_line_mode(&self) -> LineMode {
        self.line_mode.unwrap_or(if self.bundle {
            LineMode::Statement
        } else {
            LineMode::Preserve
        })
    }
}

/// Why a profile could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// No profile with that name exists (E4006).
    Unknown(String),
    /// A custom profile without `inherits` (E4315).
    MissingInherits(String),
    /// An `inherits` cycle (E4007), listing the chain.
    Cycle(Vec<String>),
}

/// Merges `overlay` into `base`: objects deeply, everything else replaces.
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

/// The `inherits` chain of a profile, base first, ending with the profile
/// itself. The first element is always a built-in profile.
pub fn chain(manifest: &Manifest, name: &str) -> Result<Vec<String>, ProfileError> {
    let mut chain = vec![name.to_string()];
    let mut current = name.to_string();
    loop {
        let settings = manifest.profiles.get(&current);
        let parent = match (
            settings.and_then(|s| s.inherits.clone()),
            BuiltinProfile::from_name(&current),
        ) {
            (Some(parent), _) => parent,
            (None, Some(_)) => break,
            (None, None) if settings.is_some() => {
                return Err(ProfileError::MissingInherits(current));
            }
            (None, None) => return Err(ProfileError::Unknown(current)),
        };
        if chain.contains(&parent) {
            chain.push(parent);
            chain.reverse();
            return Err(ProfileError::Cycle(chain));
        }
        if !manifest.profiles.contains_key(&parent) && BuiltinProfile::from_name(&parent).is_none()
        {
            return Err(ProfileError::Unknown(parent));
        }
        chain.push(parent.clone());
        current = parent;
    }
    chain.reverse();
    Ok(chain)
}

/// Resolves a profile by name: built-in defaults, then every profile of the
/// `inherits` chain (base first), merged deeply. `config` starts from the
/// manifest's top-level `config`.
pub fn resolve(manifest: &Manifest, name: &str) -> Result<Profile, ProfileError> {
    let chain = chain(manifest, name)?;
    let base = BuiltinProfile::from_name(&chain[0]).expect("chains start at a built-in profile");
    let mut value = base.defaults();
    value["config"] = Value::Object(manifest.config.clone());
    for step in &chain {
        if let Some(settings) = manifest.profiles.get(step) {
            let mut overlay = serde_json::to_value(settings).expect("profiles serialize");
            if let Value::Object(map) = &mut overlay {
                map.remove("inherits");
            }
            deep_merge(&mut value, &overlay);
        }
    }
    let mut profile: Profile =
        serde_json::from_value(value).expect("merged built-in and validated settings deserialize");
    profile.name = name.to_string();
    profile.base = base;
    Ok(profile)
}

/// Checks a profile `config` object against the top-level `config` (E4012):
/// every key must exist there with a value of the same JSON type. Returns the
/// dotted paths of offending keys with a reason.
pub fn config_mismatches(top: &JsonObject, profile: &JsonObject) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    compare(top, profile, &mut Vec::new(), &mut out);
    out
}

fn compare(
    top: &JsonObject,
    profile: &JsonObject,
    path: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, String)>,
) {
    for (key, value) in profile {
        path.push(key.clone());
        match top.get(key) {
            None => out.push((
                path.clone(),
                "is not defined in the top-level `config`".into(),
            )),
            Some(Value::Object(top_obj)) => match value {
                Value::Object(obj) => compare(top_obj, obj, path, out),
                other => out.push((
                    path.clone(),
                    format!(
                        "must be an object, found {}",
                        crate::model::json_type_name(other)
                    ),
                )),
            },
            Some(expected) if !same_type(expected, value) => out.push((
                path.clone(),
                format!(
                    "has type {} but the top-level `config` defines {}",
                    crate::model::json_type_name(value),
                    crate::model::json_type_name(expected)
                ),
            )),
            Some(_) => {}
        }
        path.pop();
    }
}

fn same_type(a: &Value, b: &Value) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: Value) -> Manifest {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn builtin_defaults() {
        let m = Manifest::default();
        let dev = resolve(&m, "dev").unwrap();
        assert!(dev.debug && !dev.bundle);
        assert_eq!(dev.effective_line_mode(), LineMode::Preserve);
        let release = resolve(&m, "release").unwrap();
        assert!(!release.debug && release.bundle);
        assert_eq!(release.minify, "strip");
        assert_eq!(release.effective_line_mode(), LineMode::Statement);
        assert_eq!(
            resolve(&m, "test").unwrap().runtime_checks,
            RuntimeChecks::All
        );
    }

    #[test]
    fn inherits_and_config_merge() {
        let m = manifest(json!({
            "config": { "channel": "SYSTEM", "max": 50, "nested": { "a": 1, "b": 2 } },
            "profiles": {
                "release": { "minify": "compact" },
                "beta": { "inherits": "release", "log-level": "info",
                          "config": { "max": 100, "nested": { "b": 3 } } }
            }
        }));
        let beta = resolve(&m, "beta").unwrap();
        assert_eq!(beta.base, BuiltinProfile::Release);
        assert_eq!(beta.minify, "compact");
        assert_eq!(beta.log_level, LogLevel::Info);
        assert_eq!(beta.config["max"], 100);
        assert_eq!(beta.config["channel"], "SYSTEM");
        assert_eq!(beta.config["nested"], json!({ "a": 1, "b": 3 }));
    }

    #[test]
    fn profile_errors() {
        let m = manifest(json!({ "profiles": {
            "lonely": { "debug": true },
            "a": { "inherits": "b" }, "b": { "inherits": "a" },
            "c": { "inherits": "nope" }
        }}));
        assert_eq!(
            resolve(&m, "lonely").unwrap_err(),
            ProfileError::MissingInherits("lonely".into())
        );
        assert!(matches!(
            resolve(&m, "a").unwrap_err(),
            ProfileError::Cycle(_)
        ));
        assert_eq!(
            resolve(&m, "c").unwrap_err(),
            ProfileError::Unknown("nope".into())
        );
        assert_eq!(
            resolve(&m, "zzz").unwrap_err(),
            ProfileError::Unknown("zzz".into())
        );
    }

    #[test]
    fn config_type_checks() {
        let top = json!({ "a": 1, "o": { "x": "s" } });
        let profile = json!({ "a": "str", "missing": true, "o": { "x": 2 } });
        let issues = config_mismatches(top.as_object().unwrap(), profile.as_object().unwrap());
        assert_eq!(issues.len(), 3);
    }
}
