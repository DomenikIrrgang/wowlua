//! For tests (this crate's and the CLI's): a stand-in for GitHub's OAuth
//! and users API, a registry on a temporary data directory signed in to
//! through it, and a browser's sign-in that returns an API token.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

/// A fake GitHub on a local port: the code `code-<login>` signs in as
/// `<login>`, and `/users/<login>` knows the `users` given.
pub struct FakeGitHub {
    pub url: String,
}

impl FakeGitHub {
    pub fn start(client_secret: &str, users: &[&str]) -> FakeGitHub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
        let url = format!("http://{}", listener.local_addr().expect("an address"));
        let secret = client_secret.to_string();
        let users: Vec<String> = users.iter().map(|u| u.to_string()).collect();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(s) => s,
                    Err(_) => continue,
                });
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut length = 0;
                let mut auth = String::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                        break;
                    }
                    let lower = h.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                    if lower.starts_with("authorization:") {
                        auth = h["authorization:".len()..].trim().to_string();
                    }
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let (status, answer) = match (method.as_str(), path.as_str()) {
                    ("POST", "/login/oauth/access_token") => {
                        let code = body["code"].as_str().unwrap_or("");
                        match code.strip_prefix("code-") {
                            Some(login) if body["client_secret"] == secret.as_str() => (
                                200,
                                serde_json::json!({ "access_token": format!("gho-{login}") }),
                            ),
                            _ => (200, serde_json::json!({ "error": "bad_verification_code" })),
                        }
                    }
                    ("GET", "/user") => match auth.strip_prefix("Bearer gho-") {
                        Some(login) => (200, serde_json::json!({ "login": login })),
                        None => (401, serde_json::json!({ "message": "Bad credentials" })),
                    },
                    ("GET", p) if p.starts_with("/users/") => {
                        let wanted = &p["/users/".len()..];
                        match users.iter().find(|u| u.eq_ignore_ascii_case(wanted)) {
                            Some(u) => (200, serde_json::json!({ "login": u })),
                            None => (404, serde_json::json!({ "message": "Not Found" })),
                        }
                    }
                    _ => (404, serde_json::json!({ "message": "Not Found" })),
                };
                let text = answer.to_string();
                let mut stream = reader.into_inner();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        FakeGitHub { url }
    }
}

/// A registry on `data`, signed in to through a [`FakeGitHub`].
pub struct TestRegistry {
    pub server: crate::Running,
    pub github: FakeGitHub,
}

impl TestRegistry {
    /// `admins` and `users` are GitHub logins; `max_archive` in bytes.
    pub fn start(
        data: &std::path::Path,
        admins: &[&str],
        users: &[&str],
        max_archive: usize,
    ) -> TestRegistry {
        Self::start_with(
            data,
            admins,
            users,
            max_archive,
            crate::limits::Limits::default(),
        )
    }

    /// [`start`](Self::start) with other rate limits.
    pub fn start_with(
        data: &std::path::Path,
        admins: &[&str],
        users: &[&str],
        max_archive: usize,
        limits: crate::limits::Limits,
    ) -> TestRegistry {
        let github = FakeGitHub::start("secret", users);
        let config = crate::Config {
            public_url: String::new(),
            data_dir: data.to_path_buf(),
            github: crate::auth::GitHub {
                client_id: "client".into(),
                client_secret: "secret".into(),
                url: github.url.clone(),
                api_url: github.url.clone(),
            },
            admins: admins.iter().map(|a| a.to_string()).collect(),
            max_archive,
            limits,
        };
        let server = crate::spawn(config, "127.0.0.1:0").expect("the registry starts");
        TestRegistry { server, github }
    }

    pub fn url(&self) -> &str {
        &self.server.url
    }

    /// Signs in as `login` the way a browser does (the sign-in page,
    /// GitHub, the callback) and creates an API token on the account page.
    pub fn token(&self, login: &str) -> String {
        sign_in_token(self.url(), &self.github.url, login)
    }

    /// Signs in as `login`; returns the session's `Cookie` header value.
    pub fn session(&self, login: &str) -> String {
        sign_in_session(self.url(), &self.github.url, login)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .build(),
    )
}

fn cookies(response: &ureq::http::Response<ureq::Body>) -> Vec<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .filter(|kv| !kv.ends_with('='))
        .map(str::to_string)
        .collect()
}

fn query_value(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| {
            let mut out = Vec::new();
            let b = v.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%'
                    && i + 2 < b.len()
                    && let Ok(x) = u8::from_str_radix(&v[i + 1..i + 3], 16)
                {
                    out.push(x);
                    i += 3;
                    continue;
                }
                out.push(b[i]);
                i += 1;
            }
            String::from_utf8_lossy(&out).into_owned()
        })
    })
}

/// The sign-in flow against a registry at `registry` whose GitHub is at
/// `github`; returns the session's cookie (`wlr_session=…`).
pub fn sign_in_session(registry: &str, github: &str, login: &str) -> String {
    let agent = agent();
    let start = agent
        .get(&format!("{registry}/login/github"))
        .call()
        .expect("the sign-in starts");
    assert!(start.status().is_redirection(), "{}", start.status());
    let location = start
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("a redirect to GitHub")
        .to_string();
    assert!(
        location.starts_with(&format!("{github}/login/oauth/authorize?")),
        "{location}"
    );
    assert_eq!(
        query_value(&location, "client_id").as_deref(),
        Some("client")
    );
    let state = query_value(&location, "state").expect("a state");
    let redirect = query_value(&location, "redirect_uri").expect("a redirect URI");
    assert_eq!(redirect, format!("{registry}/login/callback"));
    let state_cookie = cookies(&start).join("; ");
    let back = agent
        .get(&format!("{redirect}?code=code-{login}&state={state}"))
        .header("Cookie", &state_cookie)
        .call()
        .expect("the callback answers");
    assert!(back.status().is_redirection(), "{}", back.status());
    cookies(&back)
        .into_iter()
        .find(|c| c.starts_with("wlr_session="))
        .expect("a session cookie")
}

/// The sign-in flow, then a token from the account page; returns the token.
pub fn sign_in_token(registry: &str, github: &str, login: &str) -> String {
    let agent = agent();
    let session = sign_in_session(registry, github, login);
    let mut account = agent
        .get(&format!("{registry}/account"))
        .header("Cookie", &session)
        .call()
        .expect("the account page");
    let page = account.body_mut().read_to_string().expect("a page");
    assert!(page.contains(login), "{page}");
    let csrf = page
        .split("name=\"csrf\" value=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .expect("a form")
        .to_string();
    let mut created = agent
        .post(&format!("{registry}/account/tokens"))
        .header("Cookie", &session)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(format!("csrf={csrf}&name=tests"))
        .expect("the token form");
    let page = created.body_mut().read_to_string().expect("a page");
    page.split("<code class=\"wl-copy-cmd__text\" data-new-token>")
        .nth(1)
        .and_then(|r| r.split('<').next())
        .expect("the new token on the page")
        .to_string()
}
