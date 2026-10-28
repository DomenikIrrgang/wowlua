//! The global config, `$WOWLUA_HOME/config.json` (CLI §12.7): read as a
//! whole and written back with mode 0600, since it may hold tokens.

// Only a computer has this (spec M14, decision T394).
#![allow(clippy::disallowed_methods)]

use serde_json::Value;
use std::path::PathBuf;

pub fn path() -> Option<PathBuf> {
    crate::install::wowlua_home().map(|h| h.join("config.json"))
}

/// The config, or an empty object when there is none (or it is not JSON).
pub fn read() -> Value {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()))
}

pub fn write(config: &Value) -> Result<(), String> {
    let path = path().ok_or("cannot find the wowlua home directory (set `WOWLUA_HOME`)")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())? + "\n";
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    // An existing file keeps its mode: tighten it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

/// The object at `key` of `config`, made when missing or not an object.
pub fn object<'a>(config: &'a mut Value, key: &str) -> &'a mut serde_json::Map<String, Value> {
    if !config.is_object() {
        *config = Value::Object(Default::default());
    }
    let value = config
        .as_object_mut()
        .expect("an object")
        .entry(key)
        .or_insert_with(|| Value::Object(Default::default()));
    if !value.is_object() {
        *value = Value::Object(Default::default());
    }
    value.as_object_mut().expect("an object")
}
