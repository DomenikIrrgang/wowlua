//! End-to-end tests of `wowlua publish` (milestone M7) against mock sites:
//! CurseForge, Wago Addons, WoWInterface and GitHub Releases on one local
//! HTTP server.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn wowlua(dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", home)
        .env("WOWLUA_PUBLISH_BACKOFF_MS", "1")
        // Never the developer's credential store.
        .env("WOWLUA_NO_KEYRING", "1")
        // Like a CI runner: no git identity.
        .env("GIT_CONFIG_GLOBAL", home.join("no-gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("WOWLUA_WOW_PATH");
    for var in [
        "CF_API_TOKEN",
        "CF_API_KEY",
        "WAGO_API_TOKEN",
        "WOWI_API_TOKEN",
        "GITHUB_TOKEN",
        "GH_API_TOKEN",
        "GITHUB_API_TOKEN",
        "GITHUB_OAUTH",
        "GITHUB_ACTIONS",
        "GITHUB_REF",
        "GITHUB_REF_NAME",
        "GITHUB_EVENT_NAME",
        "SOURCE_DATE_EPOCH",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

fn git(dir: &Path, args: &[&str]) {
    let mut all = vec!["-c", "user.email=t@example.com", "-c", "user.name=t"];
    all.extend_from_slice(args);
    let status = std::process::Command::new("git")
        .args(&all)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn commit_all(dir: &Path, tag: Option<&str>) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "Add the first feature"]);
    if let Some(t) = tag {
        git(dir, &["tag", t]);
    }
}

/// Without the Windows verbatim prefix, which git cannot handle.
fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().into_owned();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

/// One request the mock server received.
#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// A `multipart/form-data` field.
    fn field(&self, name: &str) -> Option<String> {
        let text = self.text();
        let marker = format!("name=\"{name}\"");
        let start = text.find(&marker)?;
        let rest = &text[start..];
        let body = &rest[rest.find("\r\n\r\n")? + 4..];
        let end = body.find("\r\n--")?;
        Some(body[..end].to_string())
    }

    fn json_field(&self, name: &str) -> serde_json::Value {
        serde_json::from_str(&self.field(name).unwrap()).unwrap()
    }
}

type Handler = dyn Fn(&Request, &str, usize) -> (u16, String) + Send + Sync;

/// A local HTTP server answering with `handler(request, base url, count of
/// earlier requests to the same path)`.
struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Mock {
    fn start(handler: Box<Handler>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let (base, seen) = (url.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let mut headers = Vec::new();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push((k.trim().to_string(), v.trim().to_string()));
                    }
                }
                let header = |name: &str| {
                    headers
                        .iter()
                        .find(|(k, _): &&(String, String)| k.eq_ignore_ascii_case(name))
                        .map(|(_, v)| v.clone())
                };
                let mut body = Vec::new();
                if let Some(n) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
                    body.resize(n, 0);
                    reader.read_exact(&mut body).unwrap();
                } else if header("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
                    loop {
                        let mut size = String::new();
                        reader.read_line(&mut size).unwrap();
                        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
                        let mut chunk = vec![0; n + 2];
                        reader.read_exact(&mut chunk).unwrap();
                        if n == 0 {
                            break;
                        }
                        body.extend_from_slice(&chunk[..n]);
                    }
                }
                let request = Request {
                    method,
                    path,
                    headers,
                    body,
                };
                let count = {
                    let mut all = seen.lock().unwrap();
                    let count = all.iter().filter(|r| r.path == request.path).count();
                    all.push(request.clone());
                    count
                };
                let (status, text) = handler(&request, &base, count);
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.flush();
            }
        });
        Mock { url, requests }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    fn find(&self, method: &str, path: &str) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == method && r.path.starts_with(path))
            .collect()
    }
}

/// The sites' answers; `fail_curseforge`: every CurseForge upload fails.
fn sites(fail_curseforge: bool) -> Box<Handler> {
    Box::new(move |r: &Request, base: &str, count: usize| {
        let path = r.path.split('?').next().unwrap_or_default();
        match (r.method.as_str(), path) {
            ("GET", "/api/game/wow/versions") => (
                200,
                r#"[{"id":9001,"gameVersionTypeID":517,"name":"12.0.5"},{"id":9002,"gameVersionTypeID":67408,"name":"1.15.8"},{"id":9003,"gameVersionTypeID":79434,"name":"5.5.4"}]"#.into(),
            ),
            ("POST", p) if p.ends_with("/upload-file") => {
                if fail_curseforge {
                    (400, r#"{"errorMessage":"bad"}"#.into())
                } else if count < 2 {
                    // Retried: two server errors first.
                    (502, "busy".into())
                } else {
                    (200, r#"{"id":555}"#.into())
                }
            }
            ("GET", "/api/data/game") => (
                200,
                r#"{"patches":{"retail":["12.1.0"],"classic":["1.15.8"],"mop":["5.5.4"]}}"#.into(),
            ),
            ("POST", p) if p.ends_with("/version") => (201, "{}".into()),
            ("GET", "/addons/compatible.json") => (
                200,
                r#"[{"id":"12.1.0","game":"Retail"},{"id":"1.15.8","game":"Classic"},{"id":"4.4.2","game":"Cata-Classic"}]"#.into(),
            ),
            ("POST", "/addons/update") => (202, "{}".into()),
            ("GET", p) if p.contains("/releases/tags/") => (404, "{}".into()),
            ("GET", "/repos/o/r") => (200, "{}".into()),
            ("POST", "/repos/o/r/releases") => (
                201,
                format!(
                    r#"{{"id":77,"upload_url":"{base}/uploads/77/assets{{?name,label}}","html_url":"https://github.com/o/r/releases/77"}}"#
                ),
            ),
            ("GET", "/repos/o/r/releases/77/assets") => (200, "[]".into()),
            ("POST", "/uploads/77/assets") => (201, "{}".into()),
            _ => (404, "{}".into()),
        }
    })
}

struct Env {
    _dir: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = plain(dir.path().canonicalize().unwrap());
    Env {
        home: root.join("home"),
        project: root.join("project"),
        _dir: dir,
    }
}

/// A native addon with a target per site, all on the mock server.
fn native_addon(e: &Env, mock: &Mock) {
    let manifest = serde_json::json!({
        "package": { "name": "my-addon", "version": "1.0.0" },
        "addon": { "title": "My Addon" },
        "flavors": { "standard": { "interface": [120100] }, "vanilla": { "interface": [11508] } },
        "addon-dependencies": {
            "optional": [{ "name": "Details", "curseforge-slug": "details" }]
        },
        "publish": { "targets": {
            "cf": { "type": "curseforge", "project-id": 1, "api-url": mock.url },
            "wago": { "type": "wago", "project-id": "abcdefgh", "api-url": mock.url },
            "wowi": { "type": "wowinterface", "project-id": 42, "api-url": mock.url },
            "gh": { "type": "github", "repository": "o/r", "api-url": mock.url }
        } }
    });
    write(
        &e.project,
        &[
            (
                "wowlua.json",
                &serde_json::to_string_pretty(&manifest).unwrap(),
            ),
            ("src/main.wowlua", "local count: number = 1\nprint(count)\n"),
            (
                "CHANGELOG.md",
                "# Changelog\n\n## 1.0.0\n- **First** release\n\n## 0.9.0\n- old\n",
            ),
        ],
    );
    wowlua(&e.project, &e.home).arg("fetch").assert().success();
    git(&e.project, &["init", "-q", "-b", "main"]);
    commit_all(&e.project, Some("v1.0.0"));
}

fn tokens(cmd: &mut Command) -> &mut Command {
    cmd.env("CF_API_TOKEN", "cf-secret")
        .env("WAGO_API_TOKEN", "wago-secret")
        .env("WOWI_API_TOKEN", "wowi-secret")
        .env("GITHUB_TOKEN", "gh-secret")
}

#[test]
fn publish_uploads_to_every_site() {
    let e = env();
    let mock = Mock::start(sites(false));
    native_addon(&e, &mock);
    let output = tokens(&mut wowlua(&e.project, &e.home))
        .arg("publish")
        .assert()
        .success()
        .stderr(predicate::str::contains("warning[W4328]"))
        .get_output()
        .clone();
    let text = String::from_utf8_lossy(&output.stderr).into_owned()
        + &String::from_utf8_lossy(&output.stdout);
    for secret in ["cf-secret", "wago-secret", "wowi-secret", "gh-secret"] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(
        text.contains("Uploaded MyAddon-1.0.0.zip → cf: file 555"),
        "{text}"
    );

    // CurseForge: retried after two server errors.
    let cf = mock.find("POST", "/api/projects/1/upload-file");
    assert_eq!(cf.len(), 3);
    assert_eq!(cf[2].header("x-api-token"), Some("cf-secret"));
    let meta = cf[2].json_field("metadata");
    assert_eq!(meta["displayName"], "1.0.0");
    assert_eq!(meta["gameVersions"], serde_json::json!([9002, 9001]));
    assert_eq!(meta["releaseType"], "release");
    assert_eq!(meta["changelogType"], "markdown");
    assert_eq!(meta["changelog"], "## 1.0.0\n- **First** release\n");
    assert_eq!(
        meta["relations"]["projects"],
        serde_json::json!([{ "slug": "details", "type": "optionalDependency" }])
    );
    assert!(cf[2].text().contains("filename=\"MyAddon-1.0.0.zip\""));

    let wago = mock.find("POST", "/api/projects/abcdefgh/version");
    assert_eq!(wago.len(), 1);
    assert_eq!(wago[0].header("authorization"), Some("Bearer wago-secret"));
    let meta = wago[0].json_field("metadata");
    assert_eq!(
        meta["supported_retail_patches"],
        serde_json::json!(["12.1.0"])
    );
    assert_eq!(
        meta["supported_classic_patches"],
        serde_json::json!(["1.15.8"])
    );
    assert_eq!(meta["stability"], "stable");

    let wowi = mock.find("POST", "/addons/update");
    assert_eq!(wowi.len(), 1);
    assert_eq!(wowi[0].header("x-api-token"), Some("wowi-secret"));
    assert_eq!(wowi[0].field("id").as_deref(), Some("42"));
    assert_eq!(wowi[0].field("version").as_deref(), Some("1.0.0"));
    assert_eq!(
        wowi[0].field("compatible").as_deref(),
        Some("12.1.0,1.15.8")
    );
    assert!(
        wowi[0]
            .field("changelog")
            .unwrap()
            .contains("[*][b]First[/b] release")
    );
    assert_eq!(wowi[0].field("archive"), None);

    let release = mock.find("POST", "/repos/o/r/releases");
    assert_eq!(release.len(), 1);
    assert_eq!(release[0].header("authorization"), Some("Bearer gh-secret"));
    let body: serde_json::Value = serde_json::from_slice(&release[0].body).unwrap();
    assert_eq!(body["tag_name"], "v1.0.0");
    assert_eq!(body["prerelease"], false);
    let assets: Vec<String> = mock
        .find("POST", "/uploads/77/assets")
        .iter()
        .map(|r| r.path.split("name=").nth(1).unwrap().to_string())
        .collect();
    assert!(
        assets.contains(&"MyAddon-1.0.0.zip".to_string()),
        "{assets:?}"
    );
    assert!(
        assets.contains(&"MyAddon-1.0.0-maps.zip".to_string()),
        "{assets:?}"
    );
    let json = mock
        .find("POST", "/uploads/77/assets?name=release.json")
        .pop()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&json.body).unwrap();
    assert_eq!(
        json["releases"][0],
        serde_json::json!({
            "name": "My Addon", "version": "1.0.0", "filename": "MyAddon-1.0.0.zip", "nolib": false,
            "metadata": [
                { "flavor": "classic", "interface": 11508 },
                { "flavor": "mainline", "interface": 120100 }
            ]
        })
    );
    // The source maps are archived by build id.
    assert!(e.home.join("maps").read_dir().unwrap().count() > 0);
}

#[test]
fn publish_checks_before_uploading() {
    let e = env();
    let mock = Mock::start(sites(false));
    native_addon(&e, &mock);
    // A dry run uploads nothing and shows what each site would get.
    tokens(&mut wowlua(&e.project, &e.home))
        .args(["publish", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Dry run cf (MyAddon-1.0.0.zip): would upload with",
        ))
        .stderr(predicate::str::contains("cf-secret").not());
    assert!(mock.requests().iter().all(|r| r.method == "GET"));
    // A missing token stops the publish before any upload.
    wowlua(&e.project, &e.home)
        .env("CF_API_TOKEN", "cf-secret")
        .arg("publish")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4201]"))
        .stderr(predicate::str::contains("WAGO_API_TOKEN"));
    assert!(mock.requests().iter().all(|r| r.method == "GET"));
    // A dirty tree needs `--allow-dirty`.
    write(
        &e.project,
        &[("src/main.wowlua", "local count: number = 2\nprint(count)\n")],
    );
    tokens(&mut wowlua(&e.project, &e.home))
        .arg("publish")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4336]"));
    // `--tag` must match the version.
    git(&e.project, &["checkout", "--", "."]);
    tokens(&mut wowlua(&e.project, &e.home))
        .args(["publish", "--tag", "v2.0.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4319]"));
}

#[test]
fn untagged_builds_skip_wowinterface_and_github() {
    let e = env();
    let mock = Mock::start(sites(false));
    native_addon(&e, &mock);
    git(&e.project, &["tag", "-d", "v1.0.0"]);
    tokens(&mut wowlua(&e.project, &e.home))
        .arg("publish")
        .assert()
        .success()
        .stderr(predicate::str::contains("warning[W4329]"))
        .stderr(predicate::str::contains("Skipped wowi"));
    assert!(mock.find("POST", "/addons/update").is_empty());
    assert!(mock.find("POST", "/repos/o/r/releases").is_empty());
    let cf = mock.find("POST", "/api/projects/1/upload-file");
    assert_eq!(
        cf.last().unwrap().json_field("metadata")["releaseType"],
        "alpha"
    );
    // `--tag` tags HEAD and publishes the tag, creating it on GitHub.
    tokens(&mut wowlua(&e.project, &e.home))
        .args(["publish", "--tag", "v1.0.0"])
        .assert()
        .success();
    let release = mock.find("POST", "/repos/o/r/releases");
    let body: serde_json::Value = serde_json::from_slice(&release[0].body).unwrap();
    assert_eq!(body["tag_name"], "v1.0.0");
    assert_eq!(body["target_commitish"].as_str().unwrap().len(), 40);
}

/// A converted packager project on the mock server.
fn packager_project(e: &Env, mock: &Mock) {
    let dir = &e.project;
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(
        dir,
        &["remote", "add", "origin", "https://github.com/o/r.git"],
    );
    write(
        dir,
        &[
            (
                ".pkgmeta",
                "package-as: MyAddon\nwowi-archive-previous: no\n",
            ),
            (
                "MyAddon.toc",
                "## Interface: 120100\n## Title: My Addon\n## X-Curse-Project-ID: 123\n## X-Wago-ID: abcdefgh\n## X-WoWI-ID: 42\ncore.lua\n",
            ),
            ("core.lua", "local _, ns = ...\nns.ready = true\n"),
        ],
    );
    commit_all(dir, None);
    wowlua(dir, &e.home)
        .args(["init", "--from-pkgmeta"])
        .assert()
        .success();
    let path = dir.join("wowlua.json");
    let mut m: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let targets = m["publish"]["targets"].as_object_mut().unwrap();
    assert_eq!(
        targets.keys().cloned().collect::<Vec<_>>(),
        ["curseforge", "wago", "wowinterface", "github"],
        "{targets:?}"
    );
    for target in targets.values_mut() {
        target["api-url"] = serde_json::json!(mock.url);
    }
    assert_eq!(targets["wowinterface"]["archive-previous"], false);
    fs::write(&path, serde_json::to_string_pretty(&m).unwrap()).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "Convert"]);
    git(dir, &["tag", "v1.0"]);
}

#[test]
fn converted_projects_publish_like_the_packager() {
    let e = env();
    let mock = Mock::start(sites(false));
    packager_project(&e, &mock);
    // The packager's `.env` file and token variables.
    write(
        &e.project,
        &[(".env", "export CF_API_TOKEN=\"from-dotenv\"\n")],
    );
    wowlua(&e.project, &e.home)
        .env("CF_API_TOKEN", "from-env")
        .env("WAGO_API_TOKEN", "wago-secret")
        .env("WOWI_API_TOKEN", "wowi-secret")
        .env("GITHUB_TOKEN", "not-read")
        .env("GITHUB_OAUTH", "oauth-secret")
        .arg("publish")
        .assert()
        .success();
    // The packager's order: CurseForge, WoWInterface, Wago, GitHub.
    let uploads: Vec<String> = mock
        .requests()
        .iter()
        .filter(|r| r.method == "POST")
        .map(|r| r.path.clone())
        .collect();
    let first = |p: &str| uploads.iter().position(|u| u.starts_with(p)).unwrap();
    assert!(first("/api/projects/123/upload-file") < first("/addons/update"));
    assert!(first("/addons/update") < first("/api/projects/abcdefgh/version"));
    assert!(first("/api/projects/abcdefgh/version") < first("/repos/o/r/releases"));

    let cf = mock
        .find("POST", "/api/projects/123/upload-file")
        .pop()
        .unwrap();
    assert_eq!(cf.header("x-api-token"), Some("from-dotenv"));
    let meta = cf.json_field("metadata");
    assert_eq!(meta["displayName"], "v1.0");
    assert_eq!(meta["changelogType"], "markdown");
    assert!(
        meta["changelog"]
            .as_str()
            .unwrap()
            .starts_with("# My Addon\r\n"),
        "{meta}"
    );
    assert!(cf.text().contains("filename=\"MyAddon-v1.0.zip\""));

    let wowi = mock.find("POST", "/addons/update").pop().unwrap();
    assert_eq!(wowi.field("version").as_deref(), Some("v1.0"));
    assert_eq!(wowi.field("archive").as_deref(), Some("No"));
    let changelog = wowi.field("changelog").unwrap();
    assert!(
        changelog.starts_with("[size=5]My Addon[/size]"),
        "{changelog}"
    );

    let release = mock.find("POST", "/repos/o/r/releases").pop().unwrap();
    assert_eq!(release.header("authorization"), Some("token oauth-secret"));
    let body: serde_json::Value = serde_json::from_slice(&release.body).unwrap();
    assert_eq!(body["name"], "v1.0");
    assert!(body.get("target_commitish").is_none());
    let json = mock
        .find("POST", "/uploads/77/assets?name=release.json")
        .pop()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&json.body).unwrap();
    assert_eq!(json["releases"][0]["name"], "My Addon");
    assert_eq!(json["releases"][0]["version"], "v1.0");
    // No source maps on the packager's releases.
    assert!(
        mock.find("POST", "/uploads/77/assets?name=MyAddon-v1.0-maps.zip")
            .is_empty()
    );
}

#[test]
fn a_failed_curseforge_upload_stops_a_packager_publish() {
    let e = env();
    let mock = Mock::start(sites(true));
    packager_project(&e, &mock);
    wowlua(&e.project, &e.home)
        .env("CF_API_TOKEN", "cf-secret")
        .env("WOWI_API_TOKEN", "wowi-secret")
        .arg("publish")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4202]"))
        .stderr(predicate::str::contains("aborting the other uploads"));
    // Tried ten times, like the packager's `retry`; nothing else uploaded.
    assert_eq!(mock.find("POST", "/api/projects/123/upload-file").len(), 10);
    assert!(mock.find("POST", "/addons/update").is_empty());
}

#[test]
fn login_saves_tokens_for_publish() {
    let e = env();
    let mock = Mock::start(sites(false));
    native_addon(&e, &mock);
    // Without a credential store: `config.json`, readable only by the user.
    wowlua(&e.project, &e.home)
        .args(["login", "wago", "--token", "wago-saved"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning[W4326]"))
        .stderr(predicate::str::contains(
            "Saved the Wago token in config.json",
        ));
    wowlua(&e.project, &e.home)
        .args(["login", "wowinterface"])
        .write_stdin("wowi-saved\n")
        .assert()
        .success();
    let config_path = e.home.join("config.json");
    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        config["tokens"],
        serde_json::json!({ "wago": "wago-saved", "wowinterface": "wowi-saved" })
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&config_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // The environment wins over saved tokens.
    wowlua(&e.project, &e.home)
        .env("CF_API_TOKEN", "cf-secret")
        .env("GITHUB_TOKEN", "gh-secret")
        .env("WAGO_API_TOKEN", "wago-env")
        .arg("publish")
        .assert()
        .success();
    let wago = mock.find("POST", "/api/projects/abcdefgh/version");
    assert_eq!(wago[0].header("authorization"), Some("Bearer wago-env"));
    let wowi = mock.find("POST", "/addons/update");
    assert_eq!(wowi[0].header("x-api-token"), Some("wowi-saved"));
    wowlua(&e.project, &e.home)
        .args(["logout", "wago"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Removed the Wago token"));
    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        config["tokens"],
        serde_json::json!({ "wowinterface": "wowi-saved" })
    );
    wowlua(&e.project, &e.home)
        .args(["login", "wago", "--token", " "])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no token given"));
}

#[test]
fn workspaces_bundle_addons_and_keep_member_targets_apart() {
    let e = env();
    let mock = Mock::start(sites(false));
    let member = |name: &str, extra: serde_json::Value| {
        let mut m = serde_json::json!({
            "package": { "name": name.to_lowercase(), "version": "1.0.0" },
            "addon": { "name": name, "title": name },
            "flavors": { "standard": { "interface": [120100] } },
        });
        if let serde_json::Value::Object(map) = extra {
            for (k, v) in map {
                m[k] = v;
            }
        }
        serde_json::to_string_pretty(&m).unwrap()
    };
    let options = member(
        "Options",
        serde_json::json!({ "publish": { "targets": {
            "cf": { "type": "curseforge", "project-id": 2, "api-url": mock.url }
        } } }),
    );
    let root = serde_json::json!({
        "workspace": { "members": ["addons/*"], "default-members": ["addons/core"] },
        "publish": { "bundle-addons": true, "targets": {
            "cf": { "type": "curseforge", "project-id": 1, "api-url": mock.url }
        } }
    });
    write(
        &e.project,
        &[
            ("wowlua.json", &serde_json::to_string_pretty(&root).unwrap()),
            (
                "addons/core/wowlua.json",
                &member("Core", serde_json::json!({})),
            ),
            ("addons/core/src/main.lua", "CoreLoaded = true\n"),
            ("addons/core/CHANGELOG.md", "## 1.0.0\n- core\n"),
            ("addons/options/wowlua.json", &options),
            ("addons/options/src/main.lua", "OptionsLoaded = true\n"),
            ("addons/options/CHANGELOG.md", "## 1.0.0\n- options\n"),
        ],
    );
    wowlua(&e.project, &e.home).arg("fetch").assert().success();
    git(&e.project, &["init", "-q", "-b", "main"]);
    commit_all(&e.project, Some("v1.0.0"));
    let out = tokens(&mut wowlua(&e.project, &e.home))
        .arg("publish")
        .assert()
        .success()
        .get_output()
        .clone();
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    let bundle = mock
        .find("POST", "/api/projects/1/upload-file")
        .pop()
        .unwrap();
    assert!(bundle.text().contains("filename=\"Core-1.0.0.zip\""));
    assert_eq!(
        bundle.json_field("metadata")["changelog"],
        "## 1.0.0\n- core\n"
    );
    // A root publish works on the default members: Options' own target
    // publishes with `-p`, and the bundle then stays put.
    assert!(
        mock.find("POST", "/api/projects/2/upload-file").is_empty(),
        "{log}"
    );
    tokens(&mut wowlua(&e.project, &e.home))
        .args(["publish", "-p", "options"])
        .assert()
        .success();
    let own = mock
        .find("POST", "/api/projects/2/upload-file")
        .pop()
        .unwrap();
    assert!(own.text().contains("filename=\"Options-1.0.0.zip\""));
    assert_eq!(mock.find("POST", "/api/projects/1/upload-file").len(), 3);
    let file = fs::File::open(e.project.join("dist/packages/Core-1.0.0.zip")).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let folders: std::collections::BTreeSet<String> = (0..zip.len())
        .map(|i| {
            zip.by_index(i)
                .unwrap()
                .name()
                .unwrap()
                .split('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(folders.into_iter().collect::<Vec<_>>(), ["Core", "Options"]);
}
