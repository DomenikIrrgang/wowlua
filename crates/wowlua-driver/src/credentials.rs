//! API tokens of `wowlua login` (Package manager §10.11): the OS credential
//! store (macOS Keychain, Windows Credential Manager, Secret Service), or,
//! where none is available, `tokens` in `$WOWLUA_HOME/config.json` (mode
//! 0600, W4326). `WOWLUA_NO_KEYRING=1` skips the credential store, and
//! without a terminal reading, saving and removing a token skip it too,
//! unless an app someone is in front of allows its dialogs
//! ([`allow_dialogs`]).

// Only a computer has this (spec M14, decision T394).
#![allow(clippy::disallowed_methods)]

use crate::config;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

/// The credential store's service name.
const SERVICE: &str = "wowlua";

/// Where a token was saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    CredentialStore,
    Config,
    /// In `config.json` because there is no terminal: a later publish
    /// without one reads only there.
    ConfigNoTerminal,
}

fn keyring_enabled() -> bool {
    std::env::var("WOWLUA_NO_KEYRING").map_or(true, |v| v.is_empty() || v == "0")
}

/// How [`get`] treats the OS credential store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreUse {
    /// Read; it may ask the user to allow it (a dialog).
    Read,
    /// Turned off (`WOWLUA_NO_KEYRING`).
    Off,
    /// Not read, since there is no terminal: a background job never waits
    /// for a dialog nobody answers.
    NoTerminal,
}

/// Set by [`allow_dialogs`].
static DIALOGS_ALLOWED: AtomicBool = AtomicBool::new(false);

/// Lets this process read, save and remove tokens in the OS credential
/// store without a terminal: for an app someone is in front of to answer
/// the store's dialogs (wowlua Studio). `WOWLUA_NO_KEYRING` still wins.
pub fn allow_dialogs() {
    DIALOGS_ALLOWED.store(true, Ordering::Relaxed);
}

/// The store is read only with a terminal on both ends (standard input and
/// standard error), on every platform, or when dialogs are allowed.
fn store_use(
    enabled: bool,
    dialogs: bool,
    stdin_terminal: bool,
    stderr_terminal: bool,
) -> StoreUse {
    if !enabled {
        StoreUse::Off
    } else if dialogs || (stdin_terminal && stderr_terminal) {
        StoreUse::Read
    } else {
        StoreUse::NoTerminal
    }
}

fn current_store_use() -> StoreUse {
    use std::io::IsTerminal;
    store_use(
        keyring_enabled(),
        DIALOGS_ALLOWED.load(Ordering::Relaxed),
        std::io::stdin().is_terminal(),
        std::io::stderr().is_terminal(),
    )
}

/// Whether [`get`] reads the OS credential store, which may wait for the
/// user to allow it.
pub fn reads_store() -> bool {
    current_store_use() == StoreUse::Read
}

fn entry(kind: &str) -> Option<keyring::Entry> {
    keyring_enabled()
        .then(|| keyring::Entry::new(SERVICE, kind).ok())
        .flatten()
}

/// Why there is no saved token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unread {
    /// None was saved.
    Missing,
    /// There is no terminal, so the OS credential store was not read, and
    /// `config.json` has none.
    NoTerminal,
}

/// A token from the credential store (when it was read), else from
/// `config.json`.
fn decide(
    store: StoreUse,
    stored: Option<String>,
    config: Option<String>,
) -> Result<String, Unread> {
    let stored = if store == StoreUse::Read {
        stored
    } else {
        None
    };
    stored
        .filter(|t| !t.is_empty())
        .or(config.filter(|t| !t.is_empty()))
        .ok_or(if store == StoreUse::NoTerminal {
            Unread::NoTerminal
        } else {
            Unread::Missing
        })
}

/// The saved token of a site type (`curseforge`, `wago`, …). Without a
/// terminal, only `config.json` is read.
pub fn get(kind: &str) -> Result<String, Unread> {
    let store = current_store_use();
    let stored = match store {
        StoreUse::Read => entry(kind).and_then(|e| e.get_password().ok()),
        StoreUse::Off | StoreUse::NoTerminal => None,
    };
    decide(
        store,
        stored,
        config::read()["tokens"][kind].as_str().map(str::to_string),
    )
}

/// Saves a token: in the credential store (with a terminal), else in
/// `config.json`.
pub fn set(kind: &str, token: &str) -> Result<Saved, String> {
    let store = current_store_use();
    if store == StoreUse::Read
        && let Some(e) = entry(kind)
        && e.set_password(token).is_ok()
    {
        // A token saved before in `config.json` would be stale.
        let _ = remove_from_config(kind);
        return Ok(Saved::CredentialStore);
    }
    let mut config = config::read();
    config::object(&mut config, "tokens").insert(kind.into(), Value::String(token.to_string()));
    config::write(&config)?;
    Ok(if store == StoreUse::NoTerminal {
        Saved::ConfigNoTerminal
    } else {
        Saved::Config
    })
}

fn remove_from_config(kind: &str) -> Result<bool, String> {
    let mut config = config::read();
    let removed = config
        .get_mut("tokens")
        .and_then(Value::as_object_mut)
        .is_some_and(|t| t.shift_remove(kind).is_some());
    if removed {
        config::write(&config)?;
    }
    Ok(removed)
}

/// Removes a saved token; `false` when there was none. Without a terminal
/// only from `config.json`.
pub fn remove(kind: &str) -> Result<bool, String> {
    let from_store = current_store_use() == StoreUse::Read
        && entry(kind).is_some_and(|e| e.delete_credential().is_ok());
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

    /// The credential store is read only with a terminal on both ends, on
    /// every platform, or in an app that allows its dialogs, and never with
    /// `WOWLUA_NO_KEYRING`.
    #[test]
    fn the_store_needs_a_terminal() {
        assert_eq!(store_use(true, false, true, true), StoreUse::Read);
        assert_eq!(store_use(true, false, false, true), StoreUse::NoTerminal);
        assert_eq!(store_use(true, false, true, false), StoreUse::NoTerminal);
        assert_eq!(store_use(true, false, false, false), StoreUse::NoTerminal);
        assert_eq!(store_use(false, false, true, true), StoreUse::Off);
        assert_eq!(store_use(false, false, false, false), StoreUse::Off);
        // wowlua Studio: no terminal, someone to answer the dialog.
        assert_eq!(store_use(true, true, false, false), StoreUse::Read);
        assert_eq!(store_use(false, true, false, false), StoreUse::Off);
    }

    /// The store's token wins when it was read; otherwise `config.json`
    /// answers, and a token missing without a terminal says why.
    #[test]
    fn tokens_without_a_terminal_come_from_the_config() {
        let some = |t: &str| Some(t.to_string());
        let read = StoreUse::Read;
        assert_eq!(decide(read, some("store"), None), Ok("store".into()));
        assert_eq!(
            decide(read, some("store"), some("config")),
            Ok("store".into())
        );
        assert_eq!(decide(read, None, some("config")), Ok("config".into()));
        assert_eq!(decide(read, None, None), Err(Unread::Missing));
        assert_eq!(decide(read, some(""), some("")), Err(Unread::Missing));
        let none = StoreUse::NoTerminal;
        assert_eq!(
            decide(none, some("store"), some("config")),
            Ok("config".into())
        );
        assert_eq!(decide(none, some("store"), None), Err(Unread::NoTerminal));
        assert_eq!(decide(none, None, None), Err(Unread::NoTerminal));
        assert_eq!(decide(StoreUse::Off, None, None), Err(Unread::Missing));
        assert_eq!(
            decide(StoreUse::Off, None, some("config")),
            Ok("config".into())
        );
    }
}
