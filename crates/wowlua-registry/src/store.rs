//! The registry's data directory: plain files, written through a temporary
//! file and a rename, so a reader never sees half a file; writes take one
//! lock, so concurrent requests never lose each other's changes. The
//! running server holds `.lock`, so a second server cannot open the same
//! directory.
//!
//! ```text
//! <data>/index/<name>                    the sparse index file (one JSON line per version)
//! <data>/archives/<name>/<version>.tar.gz
//! <data>/owners/<name>.json              {"users": [GitHub logins]}
//! <data>/tokens.json                     {"tokens": [{id, hash, login, name, created}]}
//! <data>/reserved.json                   the admins' additions to the reserved names
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use wowlua_pkg::registry::IndexEntry;

/// An API token, stored by the SHA-256 of its secret.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    /// Names it on the account page (the hash's first 12 characters).
    pub id: String,
    pub hash: String,
    pub login: String,
    pub name: String,
    /// Seconds since 1970.
    pub created: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Tokens {
    #[serde(default)]
    tokens: Vec<Token>,
}

/// A reserved package name: only its authors (GitHub logins) and the admins
/// may publish it first.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reserved {
    #[serde(default)]
    pub authors: Vec<String>,
    /// What the name stands for (`Ace3`, `LibStub`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// An admin gave the name free (first come, first served again).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub released: bool,
}

/// The reserved names that ship with the server (`data/reserved.json`).
pub fn builtin_reserved() -> BTreeMap<String, Reserved> {
    serde_json::from_str(include_str!("../data/reserved.json"))
        .expect("data/reserved.json is a map of reserved names")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Owners {
    #[serde(default)]
    users: Vec<String>,
}

pub struct Store {
    root: PathBuf,
    write: Mutex<()>,
    /// Held while the server runs.
    _lock: File,
}

/// Why a publish was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum PublishError {
    /// The version exists (versions are immutable).
    Exists,
    /// The package exists and the user does not own it.
    NotOwner,
    Io(String),
}

impl Store {
    /// Opens (and creates) the data directory.
    pub fn open(root: &Path) -> io::Result<Store> {
        for dir in ["index", "archives", "owners", "tmp"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(".lock"))?;
        if lock.try_lock().is_err() {
            return Err(io::Error::other(format!(
                "another registry server uses `{}`",
                root.display()
            )));
        }
        // Leftovers of writes a crash interrupted.
        if let Ok(entries) = std::fs::read_dir(root.join("tmp")) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        Ok(Store {
            root: root.to_path_buf(),
            write: Mutex::new(()),
            _lock: lock,
        })
    }

    fn lock(&self) -> MutexGuard<'_, ()> {
        self.write.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Writes `path` through a temporary file in `tmp/`.
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.root.join("tmp").join(format!(
            "{}-{}",
            std::process::id(),
            crate::auth::random_hex(8)
        ));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    fn index_path(&self, name: &str) -> PathBuf {
        self.root.join("index").join(name)
    }

    pub fn archive_path(&self, name: &str, version: &str) -> PathBuf {
        self.root
            .join("archives")
            .join(name)
            .join(format!("{version}.tar.gz"))
    }

    fn owners_path(&self, name: &str) -> PathBuf {
        self.root.join("owners").join(format!("{name}.json"))
    }

    /// The index file of a package, as served.
    pub fn index_text(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.index_path(name)).ok()
    }

    pub fn entries(&self, name: &str) -> Vec<IndexEntry> {
        self.index_text(name)
            .map(|t| wowlua_pkg::registry::parse_index(&t))
            .unwrap_or_default()
    }

    /// Every package name, sorted.
    pub fn packages(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.root.join("index"))
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| wowlua_pkg::registry::valid_name(n))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    pub fn owners(&self, name: &str) -> Vec<String> {
        std::fs::read(self.owners_path(name))
            .ok()
            .and_then(|b| serde_json::from_slice::<Owners>(&b).ok())
            .map(|o| o.users)
            .unwrap_or_default()
    }

    /// The packages a user owns.
    pub fn owned_by(&self, login: &str) -> Vec<String> {
        self.packages()
            .into_iter()
            .filter(|p| self.owners(p).iter().any(|o| same_login(o, login)))
            .collect()
    }

    /// Publishes a version: the archive, its index line, and for a new
    /// package its first owner. `admin`: the user may publish any package.
    pub fn publish(
        &self,
        entry: &IndexEntry,
        archive: &[u8],
        login: &str,
        admin: bool,
    ) -> Result<(), PublishError> {
        let _guard = self.lock();
        let io = |e: io::Error| PublishError::Io(e.to_string());
        let mut entries = self.entries(&entry.name);
        let new = entries.is_empty();
        if !new
            && !admin
            && !self
                .owners(&entry.name)
                .iter()
                .any(|o| same_login(o, login))
        {
            return Err(PublishError::NotOwner);
        }
        let version = entry.version();
        if entries.iter().any(|e| e.version() == version) {
            return Err(PublishError::Exists);
        }
        self.write_file(&self.archive_path(&entry.name, &entry.vers), archive)
            .map_err(io)?;
        if new {
            let owners = Owners {
                users: vec![login.to_string()],
            };
            self.write_file(
                &self.owners_path(&entry.name),
                &serde_json::to_vec_pretty(&owners).unwrap_or_default(),
            )
            .map_err(io)?;
        }
        entries.push(entry.clone());
        self.write_file(
            &self.index_path(&entry.name),
            wowlua_pkg::registry::write_index(&entries).as_bytes(),
        )
        .map_err(io)
    }

    /// Yanks or unyanks a version; false when it does not exist.
    pub fn set_yanked(&self, name: &str, version: &str, yanked: bool) -> io::Result<bool> {
        let _guard = self.lock();
        let mut entries = self.entries(name);
        let wanted = semver::Version::parse(version).ok();
        let Some(e) = entries
            .iter_mut()
            .find(|e| e.version().is_some() && e.version() == wanted)
        else {
            return Ok(false);
        };
        e.yanked = yanked;
        self.write_file(
            &self.index_path(name),
            wowlua_pkg::registry::write_index(&entries).as_bytes(),
        )?;
        Ok(true)
    }

    /// Changes a package's owners with `change` (under the write lock).
    pub fn change_owners<T>(
        &self,
        name: &str,
        change: impl FnOnce(&mut Vec<String>) -> Result<T, String>,
    ) -> io::Result<Result<T, String>> {
        let _guard = self.lock();
        let mut users = self.owners(name);
        let result = change(&mut users);
        if result.is_ok() {
            let owners = Owners { users };
            self.write_file(
                &self.owners_path(name),
                &serde_json::to_vec_pretty(&owners).unwrap_or_default(),
            )?;
        }
        Ok(result)
    }

    // ---- tokens -------------------------------------------------------------

    fn tokens_file(&self) -> Tokens {
        std::fs::read(self.root.join("tokens.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_tokens(&self, tokens: &Tokens) -> io::Result<()> {
        self.write_file(
            &self.root.join("tokens.json"),
            &serde_json::to_vec_pretty(tokens).unwrap_or_default(),
        )
    }

    /// The user a token's SHA-256 belongs to.
    pub fn token_login(&self, hash: &str) -> Option<String> {
        self.tokens_file()
            .tokens
            .into_iter()
            .find(|t| t.hash == hash)
            .map(|t| t.login)
    }

    pub fn tokens_of(&self, login: &str) -> Vec<Token> {
        self.tokens_file()
            .tokens
            .into_iter()
            .filter(|t| same_login(&t.login, login))
            .collect()
    }

    pub fn add_token(&self, token: Token) -> io::Result<()> {
        let _guard = self.lock();
        let mut tokens = self.tokens_file();
        tokens.tokens.push(token);
        self.save_tokens(&tokens)
    }

    /// Revokes one of the user's tokens; false when it has none with `id`.
    pub fn remove_token(&self, login: &str, id: &str) -> io::Result<bool> {
        let _guard = self.lock();
        let mut tokens = self.tokens_file();
        let before = tokens.tokens.len();
        tokens
            .tokens
            .retain(|t| !(t.id == id && same_login(&t.login, login)));
        if tokens.tokens.len() == before {
            return Ok(false);
        }
        self.save_tokens(&tokens)?;
        Ok(true)
    }

    // ---- reserved names -----------------------------------------------------

    fn reserved_file(&self) -> BTreeMap<String, Reserved> {
        std::fs::read(self.root.join("reserved.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// The reserved names: the built-in ones, with the admins' entries of
    /// `reserved.json` over them.
    pub fn reserved(&self) -> BTreeMap<String, Reserved> {
        let mut all = builtin_reserved();
        all.extend(self.reserved_file());
        all.retain(|_, r| !r.released);
        all
    }

    /// Reserves (or assigns) a name, or releases it with `None`.
    pub fn set_reserved(&self, name: &str, entry: Option<Reserved>) -> io::Result<()> {
        let _guard = self.lock();
        let mut file = self.reserved_file();
        let entry = entry.unwrap_or(Reserved {
            released: true,
            ..Default::default()
        });
        file.insert(name.to_string(), entry);
        self.write_file(
            &self.root.join("reserved.json"),
            &serde_json::to_vec_pretty(&file).unwrap_or_default(),
        )
    }
}

/// GitHub logins are case-insensitive.
pub fn same_login(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, vers: &str) -> IndexEntry {
        IndexEntry {
            name: name.into(),
            vers: vers.into(),
            deps: Vec::new(),
            cksum: "sha256-0".into(),
            features: Default::default(),
            yanked: false,
            wowlua: None,
        }
    }

    #[test]
    fn versions_are_immutable_and_packages_owned() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .publish(&entry("lib-a", "1.0.0"), b"x", "Alice", false)
            .unwrap();
        assert_eq!(
            store.publish(&entry("lib-a", "1.0.0"), b"y", "alice", false),
            Err(PublishError::Exists)
        );
        assert_eq!(
            store.publish(&entry("lib-a", "1.1.0"), b"y", "bob", false),
            Err(PublishError::NotOwner)
        );
        store
            .publish(&entry("lib-a", "1.1.0"), b"y", "ALICE", false)
            .unwrap();
        assert_eq!(store.entries("lib-a").len(), 2);
        assert_eq!(store.owners("lib-a"), vec!["Alice"]);
        assert_eq!(
            std::fs::read(store.archive_path("lib-a", "1.0.0")).unwrap(),
            b"x"
        );
        assert!(store.set_yanked("lib-a", "1.0.0", true).unwrap());
        assert!(!store.set_yanked("lib-a", "9.0.0", true).unwrap());
        assert!(store.entries("lib-a")[0].yanked);
        assert_eq!(store.owned_by("alice"), vec!["lib-a"]);
    }

    #[test]
    fn one_server_per_directory() {
        let dir = tempfile::tempdir().unwrap();
        let _first = Store::open(dir.path()).unwrap();
        assert!(Store::open(dir.path()).is_err());
    }

    #[test]
    fn admins_extend_and_release_reserved_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(store.reserved().contains_key("libstub"));
        store
            .set_reserved(
                "libstub",
                Some(Reserved {
                    authors: vec!["kaelten".into()],
                    ..Default::default()
                }),
            )
            .unwrap();
        assert_eq!(store.reserved()["libstub"].authors, vec!["kaelten"]);
        store.set_reserved("libstub", None).unwrap();
        assert!(!store.reserved().contains_key("libstub"));
    }

    #[test]
    fn tokens_are_per_user() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .add_token(Token {
                id: "abc".into(),
                hash: "h".into(),
                login: "alice".into(),
                name: "ci".into(),
                created: 0,
            })
            .unwrap();
        assert_eq!(store.token_login("h").as_deref(), Some("alice"));
        assert!(!store.remove_token("bob", "abc").unwrap());
        assert!(store.remove_token("Alice", "abc").unwrap());
        assert_eq!(store.token_login("h"), None);
    }
}
