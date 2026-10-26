//! The registry's protocol over HTTP (spec §10.10): a real server on a
//! temporary data directory, signed in to through a stand-in for GitHub.

use std::collections::BTreeMap;
use wowlua_registry::testing::TestRegistry;

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build(),
    )
}

struct Answer {
    status: u16,
    etag: Option<String>,
    body: String,
}

impl Answer {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_default()
    }

    fn code(&self) -> String {
        self.json()["errors"][0]["code"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }
}

fn answer(r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Answer {
    let mut r = r.expect("the registry answers");
    Answer {
        status: r.status().as_u16(),
        etag: r
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: r.body_mut().read_to_string().unwrap_or_default(),
    }
}

fn manifest(name: &str, version: &str, kind: &str, deps: serde_json::Value) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "package": { "name": name, "version": version, "kind": kind },
        "dependencies": deps,
    }))
    .unwrap()
}

fn archive(manifest: &str, extra: usize) -> Vec<u8> {
    let mut files = BTreeMap::new();
    files.insert("wowlua.json".to_string(), manifest.as_bytes().to_vec());
    files.insert("src/init.wowlua".to_string(), b"return {}\n".to_vec());
    if extra > 0 {
        // Random-looking bytes, so gzip cannot shrink them.
        let mut x: u32 = 12345;
        let noise: Vec<u8> = (0..extra)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 16) as u8
            })
            .collect();
        files.insert("media/noise.bin".to_string(), noise);
    }
    wowlua_pkg::registry::write_archive(&files, 0).unwrap()
}

fn publish(
    r: &TestRegistry,
    token: Option<&str>,
    manifest: &str,
    archive: &[u8],
    name: &str,
    version: &str,
) -> Answer {
    let boundary = "----wowluatest";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{manifest}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"a.tar.gz\"\r\nContent-Type: application/gzip\r\n\r\n").as_bytes());
    body.extend_from_slice(archive);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let mut req = agent()
        .put(&format!("{}/v1/packages/{name}/{version}", r.url()))
        .header(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    answer(req.send(&body[..]))
}

fn lib(r: &TestRegistry, token: &str, name: &str, version: &str) -> Answer {
    let m = manifest(name, version, "library", serde_json::json!({}));
    publish(r, Some(token), &m, &archive(&m, 0), name, version)
}

fn with_token(method: &str, url: &str, token: &str, body: Option<&str>) -> Answer {
    let a = agent();
    let auth = format!("Bearer {token}");
    answer(match (method, body) {
        ("DELETE", Some(b)) => a
            .delete(url)
            .header("Authorization", &auth)
            .force_send_body()
            .send(b),
        ("DELETE", None) => a.delete(url).header("Authorization", &auth).call(),
        ("PUT", b) => a
            .put(url)
            .header("Authorization", &auth)
            .send(b.unwrap_or("")),
        _ => a.get(url).header("Authorization", &auth).call(),
    })
}

#[test]
fn config_index_and_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let r = TestRegistry::start(dir.path(), &[], &["alice"], 1 << 20);
    let config = answer(agent().get(&format!("{}/config.json", r.url())).call()).json();
    assert_eq!(config["api"], r.url());
    assert_eq!(config["dl"], format!("{}/dl", r.url()));
    assert_eq!(
        answer(agent().get(&format!("{}/health", r.url())).call()).status,
        200
    );
    assert_eq!(
        answer(agent().get(&format!("{}/index/lib-a", r.url())).call()).status,
        404
    );

    let alice = r.token("alice");
    let m = manifest(
        "lib-a",
        "1.0.0",
        "library",
        serde_json::json!({ "lib-b": "^2.1" }),
    );
    let bytes = archive(&m, 0);
    let made = publish(&r, Some(&alice), &m, &bytes, "lib-a", "1.0.0");
    assert_eq!(made.status, 201, "{}", made.body);
    let files = wowlua_pkg::registry::read_archive(&bytes).unwrap();
    assert_eq!(made.json()["cksum"], wowlua_pkg::registry::checksum(&files));

    let index = answer(agent().get(&format!("{}/index/lib-a", r.url())).call());
    assert_eq!(index.status, 200);
    let lines = wowlua_pkg::registry::parse_index(&index.body);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].deps[0].name, "lib-b");
    assert_eq!(lines[0].deps[0].req, "^2.1");
    assert!(!lines[0].yanked);
    // ETag / If-None-Match.
    let etag = index.etag.clone().expect("an ETag");
    let again = answer(
        agent()
            .get(&format!("{}/index/lib-a", r.url()))
            .header("If-None-Match", &etag)
            .call(),
    );
    assert_eq!(again.status, 304);
    lib(&r, &alice, "lib-a", "1.1.0");
    let changed = answer(
        agent()
            .get(&format!("{}/index/lib-a", r.url()))
            .header("If-None-Match", &etag)
            .call(),
    );
    assert_eq!(changed.status, 200);
    assert_ne!(changed.etag, Some(etag));

    let mut dl = agent()
        .get(&format!("{}/dl/lib-a/1.0.0.tar.gz", r.url()))
        .call()
        .unwrap();
    assert_eq!(dl.status(), 200);
    assert_eq!(dl.body_mut().read_to_vec().unwrap(), bytes);
    assert_eq!(
        answer(
            agent()
                .get(&format!("{}/dl/lib-a/9.0.0.tar.gz", r.url()))
                .call()
        )
        .status,
        404
    );
}

#[test]
fn publishing_needs_a_token_an_owner_and_a_new_version() {
    let dir = tempfile::tempdir().unwrap();
    let r = TestRegistry::start(dir.path(), &[], &["alice", "bob"], 1 << 20);
    let m = manifest("lib-a", "1.0.0", "library", serde_json::json!({}));
    let none = publish(&r, None, &m, &archive(&m, 0), "lib-a", "1.0.0");
    assert_eq!((none.status, none.code().as_str()), (401, "unauthorized"));
    let bad = publish(&r, Some("wlr_nope"), &m, &archive(&m, 0), "lib-a", "1.0.0");
    assert_eq!(bad.status, 401);

    let alice = r.token("alice");
    let bob = r.token("bob");
    assert_eq!(lib(&r, &alice, "lib-a", "1.0.0").status, 201);
    let again = lib(&r, &alice, "lib-a", "1.0.0");
    assert_eq!(
        (again.status, again.code().as_str()),
        (409, "version-exists")
    );
    let other = lib(&r, &bob, "lib-a", "1.1.0");
    assert_eq!((other.status, other.code().as_str()), (403, "not-owner"));
    // The error body has the spec's form.
    assert!(
        other.json()["errors"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("bob")
    );
}

#[test]
fn names_manifests_and_archives_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let r = TestRegistry::start(dir.path(), &[], &["alice"], 64 * 1024);
    let alice = r.token("alice");
    for bad in ["A-Lib", "x", "lib_a"] {
        let m = manifest(bad, "1.0.0", "library", serde_json::json!({}));
        let a = publish(&r, Some(&alice), &m, &archive(&m, 0), bad, "1.0.0");
        assert_eq!(a.status, 400, "{bad}: {}", a.body);
    }
    // The URL and the manifest agree.
    let m = manifest("lib-a", "1.0.0", "library", serde_json::json!({}));
    let a = publish(&r, Some(&alice), &m, &archive(&m, 0), "lib-a", "1.0.1");
    assert_eq!((a.status, a.code().as_str()), (400, "manifest-mismatch"));
    // Addons go to the addon sites (E4230).
    let m = manifest("my-addon", "1.0.0", "addon", serde_json::json!({}));
    let a = publish(&r, Some(&alice), &m, &archive(&m, 0), "my-addon", "1.0.0");
    assert_eq!((a.status, a.code().as_str()), (400, "addon"));
    assert!(a.body.contains("E4230"));
    // Registry packages depend on registry packages.
    let m = manifest(
        "lib-g",
        "1.0.0",
        "library",
        serde_json::json!({ "x": { "git": "https://example.com/x" } }),
    );
    let a = publish(&r, Some(&alice), &m, &archive(&m, 0), "lib-g", "1.0.0");
    assert_eq!((a.status, a.code().as_str()), (400, "dependency-source"));
    // An invalid manifest by wowlua's rules.
    let m = r#"{ "package": { "name": "lib-u", "version": "1.0.0", "kind": "library" }, "unknown": 1 }"#;
    let a = publish(&r, Some(&alice), m, &archive(m, 0), "lib-u", "1.0.0");
    assert_eq!((a.status, a.code().as_str()), (400, "invalid-manifest"));
    // Not an archive.
    let m = manifest("lib-z", "1.0.0", "library", serde_json::json!({}));
    let a = publish(&r, Some(&alice), &m, b"not gzip", "lib-z", "1.0.0");
    assert_eq!((a.status, a.code().as_str()), (400, "invalid-archive"));
    // Larger than the limit: 413 with the JSON body.
    let a = publish(
        &r,
        Some(&alice),
        &m,
        &archive(&m, 100 * 1024),
        "lib-z",
        "1.0.0",
    );
    assert_eq!(
        (a.status, a.code().as_str()),
        (413, "too-large"),
        "{}",
        a.body
    );
    assert!(answer(agent().get(&format!("{}/index/lib-z", r.url())).call()).status == 404);
}

#[test]
fn reserved_names_are_for_their_authors() {
    let dir = tempfile::tempdir().unwrap();
    let r = TestRegistry::start(
        dir.path(),
        &["Admin"],
        &["alice", "kaelten", "Admin"],
        1 << 20,
    );
    let alice = r.token("alice");
    let kaelten = r.token("kaelten");
    let admin = r.token("admin");
    let a = lib(&r, &alice, "libstub", "1.0.0");
    assert_eq!(
        (a.status, a.code().as_str()),
        (403, "reserved"),
        "{}",
        a.body
    );
    // Only admins assign them.
    let url = format!("{}/v1/reserved/libstub", r.url());
    let no = with_token("PUT", &url, &alice, Some(r#"{"users": ["kaelten"]}"#));
    assert_eq!(no.status, 403);
    let yes = with_token("PUT", &url, &admin, Some(r#"{"users": ["kaelten"]}"#));
    assert_eq!(yes.status, 200, "{}", yes.body);
    let list = answer(agent().get(&format!("{}/v1/reserved", r.url())).call()).json();
    let entry = list["reserved"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "libstub")
        .unwrap()
        .clone();
    assert_eq!(entry["authors"][0], "kaelten");
    assert_eq!(lib(&r, &alice, "libstub", "1.0.0").status, 403);
    assert_eq!(lib(&r, &kaelten, "libstub", "1.0.0").status, 201);
    // An admin may extend the list with a name of its own.
    let extra = with_token(
        "PUT",
        &format!("{}/v1/reserved/lib-mine", r.url()),
        &admin,
        Some(r#"{"users": []}"#),
    );
    assert_eq!(extra.status, 200);
    assert_eq!(lib(&r, &alice, "lib-mine", "1.0.0").status, 403);
    assert_eq!(lib(&r, &admin, "lib-mine", "1.0.0").status, 201);
    // Released, it is first come, first served again.
    let rel = with_token(
        "DELETE",
        &format!("{}/v1/reserved/libdeflate", r.url()),
        &admin,
        None,
    );
    assert_eq!(rel.status, 200);
    assert_eq!(lib(&r, &alice, "libdeflate", "1.0.0").status, 201);
}

#[test]
fn yanks_and_owners() {
    let dir = tempfile::tempdir().unwrap();
    let r = TestRegistry::start(dir.path(), &[], &["alice", "Bob"], 1 << 20);
    let alice = r.token("alice");
    let bob = r.token("bob");
    lib(&r, &alice, "lib-a", "1.0.0");
    let url = format!("{}/v1/packages/lib-a/1.0.0", r.url());
    assert_eq!(with_token("DELETE", &url, &bob, None).status, 403);
    assert_eq!(with_token("DELETE", &url, &alice, None).status, 200);
    let index = answer(agent().get(&format!("{}/index/lib-a", r.url())).call());
    assert!(wowlua_pkg::registry::parse_index(&index.body)[0].yanked);
    // Still downloadable for existing lockfiles.
    assert_eq!(
        answer(
            agent()
                .get(&format!("{}/dl/lib-a/1.0.0.tar.gz", r.url()))
                .call()
        )
        .status,
        200
    );
    assert_eq!(
        with_token("PUT", &format!("{url}/unyank"), &alice, None).status,
        200
    );
    let index = answer(agent().get(&format!("{}/index/lib-a", r.url())).call());
    assert!(!wowlua_pkg::registry::parse_index(&index.body)[0].yanked);
    assert_eq!(
        with_token(
            "DELETE",
            &format!("{}/v1/packages/lib-a/2.0.0", r.url()),
            &alice,
            None
        )
        .status,
        404
    );
    assert_eq!(
        with_token(
            "DELETE",
            &format!("{}/v1/packages/nope/1.0.0", r.url()),
            &alice,
            None
        )
        .status,
        404
    );

    let owners = format!("{}/v1/packages/lib-a/owners", r.url());
    let list = answer(agent().get(&owners).call()).json();
    assert_eq!(list["users"][0]["login"], "alice");
    assert_eq!(
        with_token("PUT", &owners, &bob, Some(r#"{"users": ["Bob"]}"#)).status,
        403
    );
    let unknown = with_token(
        "PUT",
        &owners,
        &alice,
        Some(r#"{"users": ["nobody-here"]}"#),
    );
    assert_eq!(
        (unknown.status, unknown.code().as_str()),
        (400, "unknown-user")
    );
    // GitHub's spelling of the login is kept.
    let added = with_token("PUT", &owners, &alice, Some(r#"{"users": ["bob"]}"#));
    assert_eq!(added.status, 200, "{}", added.body);
    assert_eq!(added.json()["users"][1], "Bob");
    assert_eq!(lib(&r, &bob, "lib-a", "1.1.0").status, 201);
    assert_eq!(
        with_token("DELETE", &owners, &bob, Some(r#"{"users": ["alice"]}"#)).status,
        200
    );
    let last = with_token("DELETE", &owners, &bob, Some(r#"{"users": ["bob"]}"#));
    assert_eq!((last.status, last.code().as_str()), (400, "last-owner"));
    assert_eq!(lib(&r, &alice, "lib-a", "1.2.0").status, 403);
}

#[test]
fn the_account_page_needs_a_github_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    let r = TestRegistry::start(dir.path(), &[], &["alice"], 1 << 20);
    let no_redirects = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .build(),
    );
    let page = no_redirects
        .get(&format!("{}/account", r.url()))
        .call()
        .unwrap();
    assert_eq!(page.headers().get("location").unwrap(), "/login");
    // A callback without the state cookie is refused.
    let forged = no_redirects
        .get(&format!(
            "{}/login/callback?code=code-alice&state=x",
            r.url()
        ))
        .call()
        .unwrap();
    assert_eq!(forged.status(), 400);
    // Tokens are stored hashed.
    let token = r.token("alice");
    let stored = std::fs::read_to_string(dir.path().join("tokens.json")).unwrap();
    assert!(!stored.contains(&token));
    assert!(stored.contains(&wowlua_registry::auth::hash_token(&token)));
}

fn described(
    r: &TestRegistry,
    token: &str,
    name: &str,
    version: &str,
    description: &str,
    keywords: &[&str],
) -> Answer {
    let m = serde_json::to_string_pretty(&serde_json::json!({
        "package": { "name": name, "version": version, "kind": "library", "description": description, "keywords": keywords },
    }))
    .unwrap();
    publish(r, Some(token), &m, &archive(&m, 0), name, version)
}

#[test]
fn search_ranks_and_pages() {
    let dir = tempfile::tempdir().unwrap();
    let r = TestRegistry::start(dir.path(), &[], &["alice"], 1 << 20);
    let alice = r.token("alice");
    assert_eq!(
        described(
            &r,
            &alice,
            "tooltip",
            "1.0.0",
            "Tooltips for frames",
            &["ui"]
        )
        .status,
        201
    );
    assert_eq!(
        described(&r, &alice, "tooltip-extra", "0.1.0", "More", &["tooltip"]).status,
        201
    );
    assert_eq!(
        described(
            &r,
            &alice,
            "lib-frames",
            "2.0.0",
            "Frame helpers with a tooltip",
            &["ui", "frames"]
        )
        .status,
        201
    );
    assert_eq!(
        described(
            &r,
            &alice,
            "lib-frames",
            "2.1.0",
            "Frame helpers, newer",
            &["ui", "frames"]
        )
        .status,
        201
    );
    assert_eq!(
        described(&r, &alice, "lib-gone", "1.0.0", "tooltip", &[]).status,
        201
    );
    let gone = format!("{}/v1/packages/lib-gone/1.0.0", r.url());
    assert_eq!(with_token("DELETE", &gone, &alice, None).status, 200);
    let search = |q: &str| answer(agent().get(&format!("{}/v1/search?{q}", r.url())).call()).json();
    let found = search("q=tooltip");
    let names: Vec<&str> = found["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    // Name before keyword before description; only yanked: not found.
    assert_eq!(names, ["tooltip", "tooltip-extra"], "{found}");
    assert_eq!(found["total"], 2);
    assert_eq!(found["packages"][0]["vers"], "1.0.0");
    assert_eq!(found["packages"][0]["description"], "Tooltips for frames");
    assert_eq!(found["packages"][0]["keywords"][0], "ui");
    assert_eq!(found["packages"][0]["kind"], "library");
    // Every word must match; the newest version's description counts.
    let found = search("q=frames+newer");
    assert_eq!(found["total"], 1);
    assert_eq!(found["packages"][0]["name"], "lib-frames");
    assert_eq!(found["packages"][0]["vers"], "2.1.0");
    // Paged.
    let first = search("q=&per_page=2");
    assert_eq!(first["total"], 3);
    assert_eq!(first["packages"].as_array().unwrap().len(), 2);
    let second = search("q=&per_page=2&page=2");
    assert_eq!(second["packages"].as_array().unwrap().len(), 1);
    assert_eq!(second["page"], 2);
    let bad = answer(
        agent()
            .get(&format!("{}/v1/search?q=x&page=0", r.url()))
            .call(),
    );
    assert_eq!((bad.status, bad.code().as_str()), (400, "invalid-request"));
}

fn rate(text: &str) -> Option<wowlua_registry::limits::Rate> {
    wowlua_registry::limits::Rate::parse(text).unwrap()
}

#[test]
fn publishes_and_sign_ins_are_rate_limited() {
    let dir = tempfile::tempdir().unwrap();
    let limits = wowlua_registry::limits::Limits {
        publish: rate("2/hour"),
        login: rate("3/minute"),
        ..Default::default()
    };
    let r = TestRegistry::start_with(dir.path(), &[], &["alice"], 1 << 20, limits);
    let alice = r.token("alice");
    assert_eq!(lib(&r, &alice, "lib-a", "1.0.0").status, 201);
    assert_eq!(lib(&r, &alice, "lib-a", "1.1.0").status, 201);
    let mut third = agent()
        .put(&format!("{}/v1/packages/lib-a/1.2.0", r.url()))
        .header("Authorization", &format!("Bearer {alice}"))
        .send("")
        .unwrap();
    assert_eq!(third.status(), 429);
    let retry: u64 = third
        .headers()
        .get("retry-after")
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry > 1000 && retry <= 1800, "{retry}");
    let body: serde_json::Value =
        serde_json::from_str(&third.body_mut().read_to_string().unwrap()).unwrap();
    assert_eq!(body["errors"][0]["code"], "rate-limited");
    // Reads are not publishes; the sign-in used 2 of its 3.
    assert_eq!(
        answer(agent().get(&format!("{}/index/lib-a", r.url())).call()).status,
        200
    );
    let no_redirects = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .build(),
    );
    let login = || {
        no_redirects
            .get(&format!("{}/login/github", r.url()))
            .call()
            .unwrap()
            .status()
            .as_u16()
    };
    assert_eq!(login(), 303);
    assert_eq!(login(), 429);
    assert_eq!(
        answer(agent().get(&format!("{}/health", r.url())).call()).status,
        200
    );
}

#[test]
fn forwarded_for_counts_only_from_a_trusted_proxy() {
    let reads = |trusted: &str| {
        let dir = tempfile::tempdir().unwrap();
        let limits = wowlua_registry::limits::Limits {
            read: rate("2/minute"),
            trusted_proxies: trusted
                .split(',')
                .filter(|t| !t.is_empty())
                .map(|t| wowlua_registry::limits::Net::parse(t).unwrap())
                .collect(),
            ..Default::default()
        };
        let r = TestRegistry::start_with(dir.path(), &[], &[], 1 << 20, limits);
        let get = |from: &str| {
            agent()
                .get(&format!("{}/config.json", r.url()))
                .header("X-Forwarded-For", from)
                .call()
                .unwrap()
                .status()
                .as_u16()
        };
        [
            get("203.0.113.5"),
            get("203.0.113.5"),
            get("203.0.113.5"),
            get("203.0.113.6"),
            // A forged entry left of the proxy's own counts as that client.
            get("198.51.100.9, 203.0.113.6"),
        ]
    };
    // Behind the trusted proxy (here: this computer), each client has its own bucket.
    assert_eq!(reads("127.0.0.1,::1"), [200, 200, 429, 200, 200]);
    // Otherwise the header is ignored: one bucket for the connection's address.
    assert_eq!(reads(""), [200, 200, 429, 429, 429]);
}
