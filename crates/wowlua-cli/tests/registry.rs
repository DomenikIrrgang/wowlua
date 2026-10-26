//! The wowlua registry end to end (spec §10.10, Publishing §11.8): the
//! real `wowlua` against the real server (`wowlua-registry`), in this
//! process on a temporary data directory, signed in to through a stand-in
//! for GitHub.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use wowlua_registry::testing::TestRegistry;

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    registry: TestRegistry,
}

fn env_with(max_archive: usize) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let registry = TestRegistry::start(
        &root.join("data"),
        &["admin"],
        &["alice", "bob", "admin"],
        max_archive,
    );
    fs::write(
        home.join("config.json"),
        serde_json::json!({ "registries": { "default": registry.url() } }).to_string(),
    )
    .unwrap();
    Env {
        _dir: dir,
        root,
        home,
        registry,
    }
}

fn env() -> Env {
    env_with(1 << 20)
}

fn wowlua(dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", home)
        .env("WOWLUA_NO_KEYRING", "1")
        .env("WOWLUA_PUBLISH_BACKOFF_MS", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("no-gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("WOWLUA_REGISTRY_TOKEN")
        .env_remove("SOURCE_DATE_EPOCH")
        .env_remove("WOWLUA_WOW_PATH");
    cmd
}

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

/// A wowlua library `name` at `version` in `<root>/<name>-<version>`.
fn library(e: &Env, name: &str, version: &str, deps: &str, body: &str) -> PathBuf {
    let dir = e.root.join(format!("{name}-{version}"));
    write(
        &dir,
        &[
            (
                "wowlua.json",
                &format!(
                    r#"{{ "package": {{ "name": "{name}", "version": "{version}", "kind": "library", "license": "MIT" }},
  "dependencies": {{ {deps} }},
  "dev-dependencies": {{ "helpers": {{ "path": "../helpers" }} }} }}"#
                ),
            ),
            ("LICENSE", "MIT License\n"),
            ("src/init.wowlua", body),
            ("tests/init.test.wowlua", "-- a test\n"),
            ("src/thing.test.wowlua", "-- another test\n"),
            (".github/workflows/ci.yml", "on: push\n"),
        ],
    );
    write(
        &e.root.join("helpers"),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "helpers", "version": "0.1.0", "kind": "library", "license": "MIT" } }"#,
            ),
            ("src/init.wowlua", "export const H = 1\n"),
            ("LICENSE", "MIT\n"),
        ],
    );
    dir
}

fn publish(e: &Env, dir: &Path, token: &str) -> assert_cmd::assert::Assert {
    wowlua(dir, &e.home)
        .args(["publish", "--allow-dirty"])
        .env("WOWLUA_REGISTRY_TOKEN", token)
        .assert()
}

fn addon(e: &Env, deps: &str) -> PathBuf {
    let dir = e.root.join("my-addon");
    write(
        &dir,
        &[
            (
                "wowlua.json",
                &format!(
                    r#"{{ "package": {{ "name": "my-addon", "version": "1.0.0" }},
  "profiles": {{ "release": {{ "bundle": false }} }},
  "dependencies": {{ {deps} }} }}"#
                ),
            ),
            (
                "src/main.wowlua",
                "import VERSION from \"lib-a\"\nprint(VERSION)\n",
            ),
        ],
    );
    dir
}

fn lock(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(dir.join("wowlua.lock")).unwrap()).unwrap()
}

fn locked<'a>(lock: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    lock["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not locked: {lock}"))
}

fn http(method: &str, url: &str, token: &str, body: Option<&str>) -> u16 {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build(),
    );
    let auth = format!("Bearer {token}");
    let r = match method {
        "DELETE" => agent.delete(url).header("Authorization", &auth).call(),
        _ => agent
            .put(url)
            .header("Authorization", &auth)
            .send(body.unwrap_or("")),
    };
    r.unwrap().status().as_u16()
}

#[test]
fn publish_resolve_lock_and_download() {
    let e = env();
    let alice = e.registry.token("alice");
    let b = library(&e, "lib-b", "1.0.0", "", "export const B = \"b1\"\n");
    // `wowlua package --registry`: the archive of §10.10.3.
    wowlua(&b, &e.home)
        .args(["package", "--registry"])
        .assert()
        .success()
        .stderr(predicate::str::contains("lib-b-1.0.0.tar.gz (registry"));
    let archive = fs::read(b.join("dist/packages/lib-b-1.0.0.tar.gz")).unwrap();
    let files = wowlua_pkg::registry::read_archive(&archive).unwrap();
    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(names, ["LICENSE", "src/init.wowlua", "wowlua.json"]);
    let inner: serde_json::Value = serde_json::from_slice(&files["wowlua.json"]).unwrap();
    assert!(inner.get("dev-dependencies").is_none(), "{inner}");
    // A library publishes to its implicit `registry` target.
    publish(&e, &b, &alice)
        .success()
        .stderr(predicate::str::contains("Uploaded"))
        .stderr(predicate::str::contains("lib-b 1.0.0 (sha256-"));
    let a1 = library(
        &e,
        "lib-a",
        "1.0.0",
        r#""lib-b": "^1.0""#,
        "import B from \"lib-b\"\nexport const VERSION = \"a1-\" .. B\n",
    );
    wowlua(&a1, &e.home)
        .args(["publish", "--allow-dirty", "--target", "registry"])
        .env("WOWLUA_REGISTRY_TOKEN", &alice)
        .assert()
        .success();

    // An addon depends on it by version requirement.
    let app = addon(&e, r#""lib-a": "^1.0""#);
    wowlua(&app, &e.home)
        .args(["build", "--release"])
        .assert()
        .success();
    let l = lock(&app);
    let source = format!("registry+{}", e.registry.url());
    for (name, version) in [("lib-a", "1.0.0"), ("lib-b", "1.0.0")] {
        let p = locked(&l, name);
        assert_eq!(p["version"], version, "{l}");
        assert_eq!(p["source"], source.as_str(), "{l}");
        assert!(
            p["checksum"].as_str().unwrap().starts_with("sha256-"),
            "{l}"
        );
    }
    assert_eq!(locked(&l, "lib-a")["dependencies"][0], "lib-b");
    let out = app.join("dist/release/MyAddon");
    assert!(out.join("libs/lib-a/init.lua").exists());
    assert!(out.join("libs/lib-b/init.lua").exists());
    // The cache layout of §10.7.
    let host = e
        .registry
        .url()
        .trim_start_matches("http://")
        .replace(':', "_");
    let cached = e.home.join("cache/registry").join(&host);
    assert!(cached.join("lib-a/1.0.0/wowlua.json").is_file());
    assert!(cached.join("lib-a/.index").is_file());

    // A newer version: the lockfile keeps the old one until `update`.
    let a11 = library(
        &e,
        "lib-a",
        "1.1.0",
        r#""lib-b": "^1.0""#,
        "import B from \"lib-b\"\nexport const VERSION = \"a11-\" .. B\n",
    );
    publish(&e, &a11, &alice).success();
    wowlua(&app, &e.home)
        .args(["build", "--release"])
        .assert()
        .success();
    assert_eq!(locked(&lock(&app), "lib-a")["version"], "1.0.0");
    wowlua(&app, &e.home)
        .args(["outdated"])
        .assert()
        .success()
        .stdout(predicate::str::contains("lib-a").and(predicate::str::contains("1.1.0")));
    wowlua(&app, &e.home).args(["update"]).assert().success();
    assert_eq!(locked(&lock(&app), "lib-a")["version"], "1.1.0");
    // `--precise` pins a registry dependency to a version.
    wowlua(&app, &e.home)
        .args(["update", "lib-a", "--precise", "1.0.0"])
        .assert()
        .success();
    assert_eq!(locked(&lock(&app), "lib-a")["version"], "1.0.0");
    wowlua(&app, &e.home).args(["update"]).assert().success();
    assert_eq!(locked(&lock(&app), "lib-a")["version"], "1.1.0");
    wowlua(&app, &e.home)
        .args(["build", "--release", "--locked"])
        .assert()
        .success();

    // Yanked: the locked version keeps working with W4322 ...
    let yank = format!("{}/v1/packages/lib-a/1.1.0", e.registry.url());
    assert_eq!(http("DELETE", &yank, &alice, None), 200);
    wowlua(&app, &e.home)
        .args(["fetch"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning[W4322]"))
        .stderr(predicate::str::contains("`lib-a` 1.1.0"));
    assert_eq!(locked(&lock(&app), "lib-a")["version"], "1.1.0");
    // ... and a new resolution never chooses it.
    wowlua(&app, &e.home)
        .args(["update", "lib-a"])
        .assert()
        .success()
        .stderr(predicate::str::contains("W4322").not());
    assert_eq!(locked(&lock(&app), "lib-a")["version"], "1.0.0");
    let fresh = e.root.join("fresh-addon");
    fs::create_dir_all(&fresh).unwrap();
    write(
        &fresh,
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "fresh-addon", "version": "1.0.0" }, "dependencies": { "lib-a": "=1.1.0" } }"#,
            ),
            ("src/main.wowlua", "print(1)\n"),
        ],
    );
    wowlua(&fresh, &e.home)
        .args(["build"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4102]"))
        .stderr(predicate::str::contains("yanked"));

    // Offline with nothing cached is E4105; with the cache it builds.
    let other_home = e.root.join("other-home");
    fs::create_dir_all(&other_home).unwrap();
    fs::copy(e.home.join("config.json"), other_home.join("config.json")).unwrap();
    wowlua(&app, &other_home)
        .args(["build", "--offline"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("E4105"));
    wowlua(&app, &e.home)
        .args(["build", "--frozen"])
        .assert()
        .success();
}

#[test]
fn unknown_packages_and_requirements_are_e4102() {
    let e = env();
    let alice = e.registry.token("alice");
    let a = library(&e, "lib-a", "1.0.0", "", "export const VERSION = \"1\"\n");
    publish(&e, &a, &alice).success();
    // `wowlua add <name>` asks for the version it resolved, and compatible ones.
    let app = addon(&e, "");
    wowlua(&app, &e.home)
        .args(["add", "lib-a"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Locking lib-a 1.0.0"));
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(app.join("wowlua.json")).unwrap()).unwrap();
    assert_eq!(manifest["dependencies"]["lib-a"], "^1.0.0");
    let app = addon(&e, r#""lib-a": "^2.0""#);
    wowlua(&app, &e.home)
        .args(["build"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4102]"))
        .stderr(predicate::str::contains("matches `^2.0`"))
        .stderr(predicate::str::contains("1.0.0"));
    let app = addon(&e, r#""lib-nope": "^1.0""#);
    wowlua(&app, &e.home)
        .args(["build"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4102]"))
        .stderr(predicate::str::contains("not in the registry"));
    // A named registry of config.json, and a plain-HTTP one elsewhere.
    let app = addon(
        &e,
        r#""lib-a": { "version": "^1.0", "registry": "nowhere" }"#,
    );
    wowlua(&app, &e.home)
        .args(["build"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown registry `nowhere`"));
    let app = addon(
        &e,
        r#""lib-a": { "version": "^1.0", "registry": "http://registry.example.com" }"#,
    );
    wowlua(&app, &e.home)
        .args(["build"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("over HTTPS"));
}

#[test]
fn owners_versions_reserved_names_and_size() {
    let e = env_with(32 * 1024);
    let alice = e.registry.token("alice");
    let bob = e.registry.token("bob");
    let a = library(&e, "lib-a", "1.0.0", "", "export const VERSION = \"1\"\n");
    publish(&e, &a, &alice).success();
    // 409: versions are immutable.
    publish(&e, &a, &alice)
        .failure()
        .stderr(predicate::str::contains("error[E4202]"))
        .stderr(predicate::str::contains("HTTP 409"))
        .stderr(predicate::str::contains("version-exists"));
    // 403: only owners publish.
    let a2 = library(&e, "lib-a", "1.1.0", "", "export const VERSION = \"2\"\n");
    publish(&e, &a2, &bob)
        .failure()
        .stderr(predicate::str::contains("HTTP 403"))
        .stderr(predicate::str::contains("not-owner"));
    let owners = format!("{}/v1/packages/lib-a/owners", e.registry.url());
    assert_eq!(
        http("PUT", &owners, &alice, Some(r#"{"users": ["bob"]}"#)),
        200
    );
    publish(&e, &a2, &bob).success();
    // Reserved names wait for their authors.
    let stub = library(&e, "libstub", "1.0.0", "", "export const VERSION = \"1\"\n");
    publish(&e, &stub, &alice)
        .failure()
        .stderr(predicate::str::contains("HTTP 403"))
        .stderr(predicate::str::contains("reserved"));
    // 413: larger than the registry takes.
    let big = library(&e, "lib-big", "1.0.0", "", "export const VERSION = \"1\"\n");
    let mut x: u32 = 7;
    let noise: Vec<u8> = (0..200 * 1024)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (x >> 16) as u8
        })
        .collect();
    fs::write(big.join("media.bin"), noise).unwrap();
    publish(&e, &big, &alice)
        .failure()
        .stderr(predicate::str::contains("HTTP 413"))
        .stderr(predicate::str::contains("too-large"));
    // Without a token: E4201, and nothing is uploaded.
    let c = library(&e, "lib-c", "1.0.0", "", "export const VERSION = \"1\"\n");
    wowlua(&c, &e.home)
        .args(["publish", "--allow-dirty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4201]"))
        .stderr(predicate::str::contains("WOWLUA_REGISTRY_TOKEN"))
        .stderr(predicate::str::contains("/login"));
    // A token saved by `wowlua login registry` works too.
    wowlua(&c, &e.home)
        .args(["login", "registry", "--token", &alice])
        .assert()
        .success();
    wowlua(&c, &e.home)
        .args(["publish", "--allow-dirty"])
        .assert()
        .success();
}

#[test]
fn addons_and_git_dependencies_stay_off_the_registry() {
    let e = env();
    let alice = e.registry.token("alice");
    let app = addon(&e, "");
    wowlua(&app, &e.home)
        .args(["package", "--registry"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4230]"));
    wowlua(&app, &e.home)
        .args(["publish", "--allow-dirty", "--target", "registry"])
        .env("WOWLUA_REGISTRY_TOKEN", &alice)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4230]"));
    // An addon with a registry target in its manifest.
    write(
        &app,
        &[(
            "wowlua.json",
            r#"{ "package": { "name": "my-addon", "version": "1.0.0" },
  "publish": { "targets": { "lib": { "type": "registry" } } } }"#,
        )],
    );
    wowlua(&app, &e.home)
        .args(["publish", "--allow-dirty", "--dry-run"])
        .env("WOWLUA_REGISTRY_TOKEN", &alice)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4230]"));
    // A library whose dependency the registry cannot resolve (E4231).
    let helpers = e.root.join("helpers");
    let g = library(
        &e,
        "lib-g",
        "1.0.0",
        r#""helpers": { "path": "../helpers" }"#,
        "export const VERSION = \"1\"\n",
    );
    assert!(helpers.is_dir());
    wowlua(&g, &e.home)
        .args(["package", "--registry"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4231]"));
    // A dry run shows what the registry would get, without a request.
    let ok = library(&e, "lib-ok", "1.0.0", "", "export const VERSION = \"1\"\n");
    wowlua(&ok, &e.home)
        .args(["publish", "--allow-dirty", "--dry-run"])
        .env("WOWLUA_REGISTRY_TOKEN", "unused")
        .assert()
        .success()
        .stderr(predicate::str::contains("PUT "))
        .stderr(predicate::str::contains("/v1/packages/lib-ok/1.0.0"));
    let index = format!("{}/index/lib-ok", e.registry.url());
    let status = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build(),
    )
    .get(&index)
    .call()
    .unwrap()
    .status();
    assert_eq!(status, 404);
}
