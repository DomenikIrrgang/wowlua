//! API tokens of `wowlua login` (Package manager §10.11): the OS credential
//! store (macOS Keychain, Windows Credential Manager, Secret Service), or,
//! where none is available, `tokens` in `$WOWLUA_HOME/config.json` (mode
//! 0600, W4326). `WOWLUA_NO_KEYRING=1` skips the credential store.

use serde_json::Value;
use std::path::PathBuf;

/// The credential store's service name.
const SERVICE: &str = "wowlua";

/// Where a token was saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    CredentialStore,
    Config,
}

fn keyring_enabled() -> bool {
    std::env::var("WOWLUA_NO_KEYRING").map_or(true, |v| v.is_empty() || v == "0")
}

/// Whether [`get`] asks the OS credential store (not `WOWLUA_NO_KEYRING`).
pub fn uses_store() -> bool {
    keyring_enabled()
}

/// Whether [`get`] may wait for the user to allow reading a token: the OS
/// credential store is used and may show a dialog (on macOS only with a
/// terminal).
pub fn may_ask() -> bool {
    use std::io::IsTerminal;
    uses_store()
        && (!cfg!(target_os = "macos")
            || may_prompt(
                std::io::stdin().is_terminal(),
                std::io::stderr().is_terminal(),
            ))
}

fn entry(kind: &str) -> Option<keyring::Entry> {
    keyring_enabled()
        .then(|| keyring::Entry::new(SERVICE, kind).ok())
        .flatten()
}

fn config_path() -> Option<PathBuf> {
    crate::install::wowlua_home().map(|h| h.join("config.json"))
}

fn read_config() -> Value {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| Value::Object(Default::default()))
}

fn write_config(config: &Value) -> Result<(), String> {
    let path = config_path().ok_or("cannot find the wowlua home directory (set `WOWLUA_HOME`)")?;
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

/// Why there is no saved token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unread {
    /// None was saved.
    Missing,
    /// The OS credential store would have asked the user to allow reading
    /// it (a macOS Keychain dialog), but there is no terminal: it counts as
    /// missing.
    NeedsApproval,
}

/// What the OS credential store answered.
#[derive(Debug, PartialEq, Eq)]
enum StoreAnswer {
    Found(String),
    /// No entry, no store, or the store failed.
    Nothing,
    /// The store needs the user's approval, and prompts were turned off.
    NeedsApproval,
}

/// Whether the credential store may ask the user: only with a terminal on
/// both ends, so a background job never waits for a dialog nobody answers.
fn may_prompt(stdin_terminal: bool, stderr_terminal: bool) -> bool {
    stdin_terminal && stderr_terminal
}

/// A token from the store's answer, else from `config.json`.
fn decide(store: StoreAnswer, config: Option<String>) -> Result<String, Unread> {
    let (found, why) = match store {
        StoreAnswer::Found(token) => (Some(token), Unread::Missing),
        StoreAnswer::Nothing => (None, Unread::Missing),
        StoreAnswer::NeedsApproval => (None, Unread::NeedsApproval),
    };
    found
        .filter(|t| !t.is_empty())
        .or(config.filter(|t| !t.is_empty()))
        .ok_or(why)
}

/// The saved token of a site type (`curseforge`, `wago`, …). Without a
/// terminal, the OS credential store does not ask the user (macOS).
pub fn get(kind: &str) -> Result<String, Unread> {
    use std::io::IsTerminal;
    let prompts = may_prompt(
        std::io::stdin().is_terminal(),
        std::io::stderr().is_terminal(),
    );
    let store = match entry(kind) {
        Some(entry) => read_store(&entry, prompts),
        None => StoreAnswer::Nothing,
    };
    decide(
        store,
        read_config()["tokens"][kind].as_str().map(str::to_string),
    )
}

/// Reads an entry of the OS credential store; without `prompts`, with the
/// macOS Keychain's user interaction turned off for the call.
fn read_store(entry: &keyring::Entry, prompts: bool) -> StoreAnswer {
    #[cfg(target_os = "macos")]
    let _no_prompts = if prompts {
        None
    } else {
        security_framework::os::macos::keychain::SecKeychain::disable_user_interaction().ok()
    };
    #[cfg(not(target_os = "macos"))]
    let _ = prompts;
    match entry.get_password() {
        Ok(token) => StoreAnswer::Found(token),
        Err(error) if needs_approval(&error) => StoreAnswer::NeedsApproval,
        Err(_) => StoreAnswer::Nothing,
    }
}

/// Whether the store refused because it would have had to ask the user
/// (macOS `errSecInteractionNotAllowed`).
fn needs_approval(error: &keyring::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        const INTERACTION_NOT_ALLOWED: i32 = -25308;
        match error {
            keyring::Error::PlatformFailure(e) | keyring::Error::NoStorageAccess(e) => e
                .downcast_ref::<security_framework::base::Error>()
                .is_some_and(|e| e.code() == INTERACTION_NOT_ALLOWED),
            _ => false,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = error;
        false
    }
}

/// Saves a token: in the credential store, else in `config.json`.
pub fn set(kind: &str, token: &str) -> Result<Saved, String> {
    if let Some(e) = entry(kind)
        && e.set_password(token).is_ok()
    {
        // A token saved before in `config.json` would be stale.
        let _ = remove_from_config(kind);
        return Ok(Saved::CredentialStore);
    }
    let mut config = read_config();
    if !config.is_object() {
        config = Value::Object(Default::default());
    }
    let tokens = config
        .as_object_mut()
        .expect("an object")
        .entry("tokens")
        .or_insert_with(|| Value::Object(Default::default()));
    if !tokens.is_object() {
        *tokens = Value::Object(Default::default());
    }
    tokens[kind] = Value::String(token.to_string());
    write_config(&config)?;
    Ok(Saved::Config)
}

fn remove_from_config(kind: &str) -> Result<bool, String> {
    let mut config = read_config();
    let removed = config
        .get_mut("tokens")
        .and_then(Value::as_object_mut)
        .is_some_and(|t| t.shift_remove(kind).is_some());
    if removed {
        write_config(&config)?;
    }
    Ok(removed)
}

/// Removes a saved token; `false` when there was none.
pub fn remove(kind: &str) -> Result<bool, String> {
    let from_store = entry(kind).is_some_and(|e| e.delete_credential().is_ok());
    let from_config = remove_from_config(kind)?;
    Ok(from_store || from_config)
}

/// The result of checking a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verified {
    Accepted,
    /// The site has no check, or the check could not run (why).
    Unchecked(String),
    Rejected(String),
}

/// Checks a token with a read-only request where the site offers one
/// (CurseForge's game versions, GitHub's `/user`).
pub fn verify(kind: &str, token: &str) -> Verified {
    let http = crate::publish::http::Http::default();
    let (url, headers, rejected): (&str, Vec<(&str, String)>, &[u16]) = match kind {
        "curseforge" => (
            "https://wow.curseforge.com/api/game/wow/versions",
            vec![("x-api-token", token.to_string())],
            &[401, 403],
        ),
        "github" => (
            "https://api.github.com/user",
            vec![
                ("authorization", format!("Bearer {token}")),
                ("accept", "application/vnd.github+json".to_string()),
            ],
            &[401],
        ),
        _ => return Verified::Unchecked("the site offers no token check".into()),
    };
    match http.request("GET", url, &headers, None) {
        Ok(r) if r.ok() => Verified::Accepted,
        Ok(r) if rejected.contains(&r.status) => {
            Verified::Rejected(format!("the site rejected the token (HTTP {})", r.status))
        }
        Ok(r) => Verified::Unchecked(format!("the check answered HTTP {}", r.status)),
        Err(e) => Verified::Unchecked(format!("cannot reach the site: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The credential store may ask only with a terminal on both ends.
    #[test]
    fn prompts_need_a_terminal() {
        assert!(may_prompt(true, true));
        assert!(!may_prompt(false, true));
        assert!(!may_prompt(true, false));
        assert!(!may_prompt(false, false));
    }

    /// A token the store would have asked for counts as missing, with the
    /// reason; `config.json` still answers.
    #[test]
    fn a_token_needing_approval_is_missing() {
        let found = || StoreAnswer::Found("from-store".into());
        assert_eq!(decide(found(), None), Ok("from-store".into()));
        assert_eq!(
            decide(found(), Some("from-config".into())),
            Ok("from-store".into())
        );
        assert_eq!(decide(StoreAnswer::Nothing, None), Err(Unread::Missing));
        assert_eq!(
            decide(StoreAnswer::Nothing, Some("from-config".into())),
            Ok("from-config".into())
        );
        assert_eq!(
            decide(StoreAnswer::NeedsApproval, None),
            Err(Unread::NeedsApproval)
        );
        assert_eq!(
            decide(StoreAnswer::NeedsApproval, Some("from-config".into())),
            Ok("from-config".into())
        );
        assert_eq!(
            decide(StoreAnswer::Found(String::new()), Some(String::new())),
            Err(Unread::Missing)
        );
    }

    /// The Keychain's `errSecInteractionNotAllowed` is a token needing
    /// approval; other failures are not.
    #[test]
    fn interaction_not_allowed_needs_approval() {
        assert!(!needs_approval(&keyring::Error::NoEntry));
        #[cfg(target_os = "macos")]
        {
            use security_framework::base::Error;
            let failure = |code| keyring::Error::PlatformFailure(Box::new(Error::from_code(code)));
            assert!(needs_approval(&failure(-25308)));
            assert!(!needs_approval(&failure(-25293)));
            assert!(!needs_approval(&keyring::Error::PlatformFailure(
                "other".into()
            )));
        }
    }
}
