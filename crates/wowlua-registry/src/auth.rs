//! Accounts: API tokens (stored as their SHA-256), the sign-in page's
//! sessions (in memory: a restart signs everyone out), and GitHub's OAuth
//! web flow, whose logins are the registry's users.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `n` random bytes as hex.
pub fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("the operating system's random numbers");
    hex(&bytes)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of a token, as stored.
pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// A new API token's secret: `wlr_` and 32 random bytes.
pub fn new_token() -> String {
    format!("wlr_{}", random_hex(32))
}

/// Seconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A signed-in browser.
#[derive(Clone, Debug)]
pub struct Session {
    pub login: String,
    /// Sent with every form, so other sites cannot post them.
    pub csrf: String,
    started: Instant,
}

const SESSION_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Default)]
pub struct Sessions {
    map: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    /// A new session for `login`; returns its id (the cookie's value).
    pub fn start(&self, login: &str) -> String {
        let id = random_hex(32);
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, s| s.started.elapsed() < SESSION_LIFETIME);
        map.insert(
            id.clone(),
            Session {
                login: login.to_string(),
                csrf: random_hex(16),
                started: Instant::now(),
            },
        );
        id
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        let map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.get(id)
            .filter(|s| s.started.elapsed() < SESSION_LIFETIME)
            .cloned()
    }

    pub fn end(&self, id: &str) {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(id);
    }
}

/// GitHub's OAuth app and API (`github_url` and `api_url` point elsewhere
/// in tests).
#[derive(Clone, Debug)]
pub struct GitHub {
    pub client_id: String,
    pub client_secret: String,
    /// `https://github.com`.
    pub url: String,
    /// `https://api.github.com`.
    pub api_url: String,
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent("wowlua-registry")
        .build();
    ureq::Agent::new_with_config(config)
}

/// Percent-encodes a query value.
pub fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl GitHub {
    /// Where the browser signs in.
    pub fn authorize_url(&self, redirect: &str, state: &str) -> String {
        format!(
            "{}/login/oauth/authorize?client_id={}&redirect_uri={}&state={}&allow_signup=true",
            self.url,
            encode(&self.client_id),
            encode(redirect),
            encode(state)
        )
    }

    /// The login of the user who signed in with `code`.
    pub fn login(&self, code: &str, redirect: &str) -> Result<String, String> {
        let agent = agent();
        let body = serde_json::json!({
            "client_id": self.client_id,
            "client_secret": self.client_secret,
            "code": code,
            "redirect_uri": redirect,
        });
        let mut response = agent
            .post(&format!("{}/login/oauth/access_token", self.url))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| format!("cannot reach GitHub: {e}"))?;
        let value = read_json(response.body_mut())?;
        let Some(access) = value["access_token"].as_str() else {
            return Err(format!(
                "GitHub refused the sign-in: {}",
                value["error_description"]
                    .as_str()
                    .or(value["error"].as_str())
                    .unwrap_or("no access token")
            ));
        };
        let mut user = agent
            .get(&format!("{}/user", self.api_url))
            .header("Authorization", &format!("Bearer {access}"))
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| format!("cannot reach GitHub: {e}"))?;
        if !user.status().is_success() {
            return Err(format!("GitHub's /user answered {}", user.status()));
        }
        let value = read_json(user.body_mut())?;
        value["login"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "GitHub's /user has no login".to_string())
    }

    /// The canonical spelling of a GitHub login, `None` when there is no
    /// such user.
    pub fn user(&self, login: &str) -> Result<Option<String>, String> {
        if !valid_login(login) {
            return Ok(None);
        }
        let mut response = agent()
            .get(&format!("{}/users/{login}", self.api_url))
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| format!("cannot reach GitHub: {e}"))?;
        match response.status().as_u16() {
            200 => {
                let value = read_json(response.body_mut())?;
                Ok(value["login"].as_str().map(str::to_string))
            }
            404 => Ok(None),
            s => Err(format!("GitHub answered {s}")),
        }
    }
}

fn read_json(body: &mut ureq::Body) -> Result<serde_json::Value, String> {
    let text = body
        .read_to_string()
        .map_err(|e| format!("GitHub's answer: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("GitHub's answer: {e}"))
}

/// GitHub's login rule: 1–39 letters, digits and single inner `-`.
pub fn valid_login(login: &str) -> bool {
    (1..=39).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_hashed() {
        let a = new_token();
        assert!(a.starts_with("wlr_") && a.len() == 68);
        assert_ne!(a, new_token());
        assert_eq!(hash_token(&a).len(), 64);
        assert_ne!(hash_token(&a), a);
    }

    #[test]
    fn logins_follow_github_s_rule() {
        assert!(valid_login("DomenikIrrgang"));
        assert!(valid_login("a-b"));
        assert!(!valid_login("-a"));
        assert!(!valid_login("a--b"));
        assert!(!valid_login("a/b"));
        assert!(!valid_login(""));
    }

    #[test]
    fn query_values_are_encoded() {
        assert_eq!(encode("http://x/a b"), "http%3A%2F%2Fx%2Fa%20b");
    }
}
