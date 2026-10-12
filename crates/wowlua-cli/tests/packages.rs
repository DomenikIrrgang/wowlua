//! End-to-end tests of the package manager (milestone M6): sources,
//! lockfile, cache, resolution and embedding of `lua-library` dependencies.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

fn wowlua(dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", home)
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

fn run(dir: &Path, program: &str, args: &[&str]) {
    let status = std::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("cannot run {program}: {e}"));
    assert!(status.success(), "{program} {args:?} failed");
}

fn git(dir: &Path, args: &[&str]) {
    let mut all = vec!["-c", "user.email=t@example.com", "-c", "user.name=t"];
    all.extend_from_slice(args);
    run(dir, "git", &all);
}

/// A git repository with `files`, committed and tagged `tag`.
fn git_repo(dir: &Path, files: &[(&str, &str)], tag: &str) -> String {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    write(dir, files);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "c"]);
    git(dir, &["tag", tag]);
    file_url(dir)
}

fn commit(dir: &Path, files: &[(&str, &str)], tag: Option<&str>) {
    write(dir, files);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "c"]);
    if let Some(t) = tag {
        git(dir, &["tag", t]);
    }
}

fn file_url(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

fn has(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A minimal addon with `deps` as its `dependencies` object.
fn addon(root: &Path, deps: serde_json::Value, extra: serde_json::Value) {
    let mut manifest = serde_json::json!({
        "package": { "name": "my-addon", "version": "1.0.0" },
        "addon": { "title": "My Addon" },
        "dependencies": deps,
    });
    if let serde_json::Value::Object(map) = extra {
        for (k, v) in map {
            manifest[k] = v;
        }
    }
    write(
        root,
        &[
            (
                "wowlua.json",
                &serde_json::to_string_pretty(&manifest).unwrap(),
            ),
            ("src/main.lua", "local _, ns = ...\nns.ready = true\n"),
        ],
    );
}

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = plain(dir.path().canonicalize().unwrap());
    Env {
        home: root.join("home"),
        project: root.join("project"),
        root,
        _dir: dir,
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

fn lock(project: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(project.join("wowlua.lock")).unwrap()).unwrap()
}

#[test]
fn git_dependency_is_locked_embedded_and_verified() {
    let e = env();
    let url = git_repo(
        &e.root.join("LibStub"),
        &[
            (
                "LibStub.lua",
                "-- LibStub is placed in the Public Domain\nLibStub = {}\n",
            ),
            ("README.md", "docs"),
            ("tests/test.lua", "-- test"),
        ],
        "1.0",
    );
    addon(
        &e.project,
        serde_json::json!({ "libstub": { "git": url, "tag": "1.0", "kind": "lua-library", "files": ["LibStub.lua"] } }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home)
        .args(["build", "--release"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Cloning"));
    let out = e.project.join("dist/release/MyAddon");
    let toc = fs::read_to_string(out.join("MyAddon.toc")).unwrap();
    assert!(
        toc.contains("libs\\libstub\\LibStub.lua\nsrc\\main.lua\n"),
        "{toc}"
    );
    assert!(out.join("libs/libstub/LibStub.lua").exists());
    // Only what the game loads is copied.
    assert!(!out.join("libs/libstub/README.md").exists());
    assert!(!out.join("libs/libstub/tests").exists());
    let licenses = fs::read_to_string(out.join("LICENSES.txt")).unwrap();
    assert!(
        licenses.contains("== libstub (libs/libstub) =="),
        "{licenses}"
    );
    assert!(licenses.contains("Public Domain"));

    let lock = lock(&e.project);
    let package = &lock["packages"][0];
    assert_eq!(package["name"], "libstub");
    let source = package["source"].as_str().unwrap();
    assert!(
        source.starts_with(&format!("git+{url}?tag=1.0#")),
        "{source}"
    );
    assert!(package["version"].as_str().unwrap().starts_with("0.0.0+"));
    assert!(package["checksum"].as_str().unwrap().starts_with("sha256-"));
    assert_eq!(package["used-by"], serde_json::json!(["my-addon"]));

    // The cache serves `--frozen` builds.
    wowlua(&e.project, &e.home)
        .args(["build", "--frozen"])
        .assert()
        .success();

    // A moved tag does not move the lock; `update` does.
    commit(
        &e.root.join("LibStub"),
        &[("LibStub.lua", "LibStub = { v = 2 }\n")],
        None,
    );
    git(&e.root.join("LibStub"), &["tag", "-f", "1.0"]);
    wowlua(&e.project, &e.home).arg("build").assert().success();
    assert_eq!(super_lock_source(&e.project), source);

    // Tampering with the cached files is detected.
    let checkouts = e.home.join("cache/checkouts");
    let checkout = fs::read_dir(&checkouts)
        .unwrap()
        .flatten()
        .find(|d| d.path().is_dir() && d.file_name().to_string_lossy().starts_with("git-"))
        .unwrap()
        .path();
    fs::write(checkout.join("LibStub.lua"), "evil()").unwrap();
    wowlua(&e.project, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4103]"));
}

fn super_lock_source(project: &Path) -> String {
    lock(project)["packages"][0]["source"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn offline_and_locked_modes() {
    let e = env();
    let url = git_repo(&e.root.join("Lib"), &[("Lib.lua", "Lib = {}\n")], "v1");
    addon(
        &e.project,
        serde_json::json!({ "lib": { "git": url, "tag": "v1", "kind": "lua-library", "files": ["Lib.lua"] } }),
        serde_json::json!({}),
    );
    // Nothing cached yet.
    wowlua(&e.project, &e.home)
        .args(["build", "--offline"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4105]"));
    // No lockfile yet: `--locked` refuses to write one.
    wowlua(&e.project, &e.home)
        .args(["build", "--locked"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4104]"));
    wowlua(&e.project, &e.home).arg("build").assert().success();
    wowlua(&e.project, &e.home)
        .args(["build", "--locked"])
        .assert()
        .success();
}

#[test]
fn library_toc_and_xml_reachability() {
    let e = env();
    let url = git_repo(
        &e.root.join("LibMulti"),
        &[
            ("LibMulti.toc", "## Interface: 120100\nLibMulti.xml\n"),
            (
                "LibMulti_Vanilla.toc",
                "## Interface: 11509\nLibMulti.xml\nClassic.lua\n",
            ),
            (
                "LibMulti.xml",
                "<Ui><Script file=\"Core.lua\"/><Include file=\"sub\\Widgets.xml\"/></Ui>",
            ),
            ("Core.lua", "-- core"),
            ("Classic.lua", "-- classic"),
            ("sub/Widgets.xml", "<Ui><Script file=\"Widget.lua\"/></Ui>"),
            ("sub/Widget.lua", "-- widget"),
            ("Unused.lua", "-- never loaded"),
            ("LICENSE.txt", "MIT License"),
        ],
        "1",
    );
    addon(
        &e.project,
        serde_json::json!({ "libmulti": { "git": url, "kind": "lua-library", "install-path": "LibMulti-1.0" } }),
        serde_json::json!({ "flavors": { "standard": {}, "vanilla": {} }, "build": { "libs-dir": "Libs" } }),
    );
    wowlua(&e.project, &e.home)
        .args(["build", "--release"])
        .assert()
        .success();
    let out = e.project.join("dist/release/MyAddon");
    let toc = fs::read_to_string(out.join("MyAddon.toc")).unwrap();
    assert!(
        toc.contains(
            "Libs\\LibMulti-1.0\\LibMulti.xml\nLibs\\LibMulti-1.0\\Classic.lua [AllowLoadGameType "
        ),
        "{toc}"
    );
    for f in [
        "LibMulti.xml",
        "Core.lua",
        "Classic.lua",
        "sub/Widgets.xml",
        "sub/Widget.lua",
        "LICENSE.txt",
    ] {
        assert!(
            out.join("Libs/LibMulti-1.0").join(f).exists(),
            "{f} missing"
        );
    }
    assert!(!out.join("Libs/LibMulti-1.0/Unused.lua").exists());
    assert!(!out.join("Libs/LibMulti-1.0/LibMulti.toc").exists());
}

#[test]
fn embeds_xml_of_the_package_loads_library_files() {
    let e = env();
    let url = git_repo(
        &e.root.join("LibStub"),
        &[("LibStub.lua", "LibStub = {}\n")],
        "1",
    );
    addon(
        &e.project,
        serde_json::json!({ "libstub": { "git": url, "kind": "lua-library", "install-path": "LibStub", "files": ["LibStub.lua"] } }),
        serde_json::json!({ "build": { "libs-dir": "Libs" } }),
    );
    write(
        &e.project,
        &[(
            "src/embeds.xml",
            "<Ui>\n  <Script file=\"Libs\\LibStub\\LibStub.lua\"/>\n</Ui>\n",
        )],
    );
    wowlua(&e.project, &e.home)
        .args(["build", "--release"])
        .assert()
        .success();
    let out = e.project.join("dist/release/MyAddon");
    let toc = fs::read_to_string(out.join("MyAddon.toc")).unwrap();
    // Loaded through the package's XML, so not listed again.
    assert!(!toc.contains("Libs\\LibStub\\LibStub.lua"), "{toc}");
    let xml = fs::read_to_string(out.join("src/embeds.xml")).unwrap();
    assert!(
        xml.contains("file=\"Interface\\AddOns\\MyAddon\\Libs\\LibStub\\LibStub.lua\""),
        "{xml}"
    );
    assert!(out.join("Libs/LibStub/LibStub.lua").exists());
}

#[test]
fn svn_dependency() {
    if !has("svn") || !has("svnadmin") {
        eprintln!("svn not installed; skipping");
        return;
    }
    let e = env();
    let repo = e.root.join("svnrepo");
    run(&e.root, "svnadmin", &["create", repo.to_str().unwrap()]);
    write(
        &e.root.join("import/trunk/LibBar"),
        &[("LibBar.lua", "LibBar = {}\n")],
    );
    let url = file_url(&repo);
    run(
        &e.root,
        "svn",
        &[
            "import",
            "-q",
            "-m",
            "init",
            e.root.join("import").to_str().unwrap(),
            &url,
        ],
    );
    addon(
        &e.project,
        serde_json::json!({ "libbar": { "svn": format!("{url}/trunk/LibBar"), "kind": "lua-library", "files": ["LibBar.lua"] } }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home).arg("build").assert().success();
    let source = super_lock_source(&e.project);
    assert!(source.ends_with("/trunk/LibBar#1"), "{source}");
    assert!(
        e.project
            .join("dist/dev/MyAddon/libs/libbar/LibBar.lua")
            .exists()
    );
}

fn targz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        let bytes = name.as_bytes();
        header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, &tar).unwrap();
    gz.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn archive_dependency_checksums_and_safety() {
    let e = env();
    let bytes = targz(&[
        ("LibDeflate/LibDeflate.lua", b"LibDeflate = {}\n"),
        ("LibDeflate/LICENSE", b"zlib"),
    ]);
    fs::create_dir_all(&e.root).unwrap();
    let archive = e.root.join("LibDeflate.tar.gz");
    fs::write(&archive, &bytes).unwrap();
    let url = file_url(&archive);
    addon(
        &e.project,
        serde_json::json!({ "libdeflate": { "archive": url, "sha256": "00", "kind": "lua-library", "strip-prefix": "LibDeflate/", "files": ["LibDeflate.lua"] } }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4103]"));
    addon(
        &e.project,
        serde_json::json!({ "libdeflate": { "archive": url, "sha256": sha256(&bytes), "kind": "lua-library", "strip-prefix": "LibDeflate/", "files": ["LibDeflate.lua"] } }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home).arg("build").assert().success();
    assert!(
        e.project
            .join("dist/dev/MyAddon/libs/libdeflate/LibDeflate.lua")
            .exists()
    );
    assert!(
        e.project
            .join("dist/dev/MyAddon/libs/libdeflate/LICENSE")
            .exists()
    );

    // Entries that would escape the package are refused.
    let evil = targz(&[("../evil.lua", b"x")]);
    let evil_path = e.root.join("evil.tar.gz");
    fs::write(&evil_path, &evil).unwrap();
    addon(
        &e.project,
        serde_json::json!({ "evil": { "archive": file_url(&evil_path), "sha256": sha256(&evil), "kind": "lua-library" } }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4323]"));
}

#[test]
fn conflicts_and_identity() {
    let e = env();
    let lib = e.root.join("LibCommon");
    let url = git_repo(&lib, &[("LibCommon.lua", "-- v1")], "v1");
    commit(&lib, &[("LibCommon.lua", "-- v2")], Some("v2"));
    // Two local libraries that need different tags of LibCommon.
    for (name, tag) in [("liba", "v1"), ("libb", "v2")] {
        write(
            &e.root.join(name),
            &[(
                "wowlua.json",
                &serde_json::to_string_pretty(&serde_json::json!({
                    "package": { "name": name, "version": "1.0.0", "kind": "lua-library" },
                    "lua-library": { "files": [format!("{name}.lua")] },
                    "dependencies": { "libcommon": { "git": url, "tag": tag, "kind": "lua-library", "files": ["LibCommon.lua"] } }
                }))
                .unwrap(),
            ), (&format!("{name}.lua"), "-- lib")],
        );
    }
    addon(
        &e.project,
        serde_json::json!({ "liba": { "path": "../liba" }, "libb": { "path": "../libb" } }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4101]"))
        .stderr(predicate::str::contains("libcommon"));

    // Same name, another source.
    let other = git_repo(&e.root.join("Fork"), &[("LibCommon.lua", "-- fork")], "v1");
    addon(
        &e.project,
        serde_json::json!({
            "liba": { "path": "../liba" },
            "libcommon": { "git": other, "tag": "v1", "kind": "lua-library", "files": ["LibCommon.lua"] }
        }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4321]"));

    // Agreeing requirements embed the nested library before its user.
    addon(
        &e.project,
        serde_json::json!({ "liba": { "path": "../liba" } }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home)
        .args(["build", "--release"])
        .assert()
        .success();
    let toc = fs::read_to_string(e.project.join("dist/release/MyAddon/MyAddon.toc")).unwrap();
    assert!(
        toc.contains("libs\\libcommon\\LibCommon.lua\nlibs\\liba\\liba.lua\nsrc\\main.lua"),
        "{toc}"
    );
    let lock = lock(&e.project);
    assert_eq!(lock["packages"][0]["name"], "liba");
    assert_eq!(lock["packages"][0]["source"], "path+../liba");
    assert_eq!(
        lock["packages"][0]["dependencies"],
        serde_json::json!(["libcommon"])
    );
}

#[test]
fn registry_dependencies_are_not_available_yet() {
    let e = env();
    addon(
        &e.project,
        serde_json::json!({ "wowlua-ui": "^0.4" }),
        serde_json::json!({}),
    );
    wowlua(&e.project, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4102]"))
        .stderr(predicate::str::contains("registry"));
}
