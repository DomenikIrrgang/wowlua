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

#[test]
fn dependency_commands() {
    let e = env();
    let lib = e.root.join("LibFoo");
    let url = git_repo(&lib, &[("LibFoo.lua", "-- 1")], "1.0");
    commit(&lib, &[("LibFoo.lua", "-- 1.1")], Some("1.1"));
    let branch_lib = e.root.join("LibBranch");
    let branch_url = git_repo(&branch_lib, &[("LibBranch.lua", "-- a")], "a");
    addon(&e.project, serde_json::json!({}), serde_json::json!({}));
    let manifest = || fs::read_to_string(e.project.join("wowlua.json")).unwrap();

    wowlua(&e.project, &e.home)
        .args([
            "add",
            "libfoo",
            "--git",
            &url,
            "--tag",
            "1.0",
            "--kind",
            "lua-library",
            "--files",
            "LibFoo.lua",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("Adding libfoo to dependencies"))
        .stderr(predicate::str::contains("Locking libfoo 0.0.0+"));
    let m: serde_json::Value = serde_json::from_str(&manifest()).unwrap();
    assert_eq!(
        m["dependencies"]["libfoo"],
        serde_json::json!({ "git": url, "tag": "1.0", "kind": "lua-library", "files": ["LibFoo.lua"] })
    );
    // The rest of the file keeps its formatting.
    assert!(manifest().starts_with("{\n  \"package\": {\n    \"name\": \"my-addon\""));

    wowlua(&e.project, &e.home)
        .args([
            "add",
            "libbranch",
            "--git",
            &branch_url,
            "--branch",
            "main",
            "--kind",
            "lua-library",
            "--files",
            "LibBranch.lua",
            "--dev",
        ])
        .assert()
        .success();
    wowlua(&e.project, &e.home)
        .args(["add", "--addon", "Details", "--optional"])
        .assert()
        .success();
    let m: serde_json::Value = serde_json::from_str(&manifest()).unwrap();
    assert_eq!(
        m["addon-dependencies"]["optional"],
        serde_json::json!(["Details"])
    );

    wowlua(&e.project, &e.home)
        .arg("tree")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "my-addon 1.0.0\n├── libfoo 0.0.0+",
        ))
        .stdout(predicate::str::contains("(tag 1.0))\n└── libbranch 0.0.0+"))
        .stdout(predicate::str::contains("[dev]"));

    // A new tag, and a new commit on the branch.
    commit(&branch_lib, &[("LibBranch.lua", "-- b")], None);
    wowlua(&e.project, &e.home)
        .arg("outdated")
        .assert()
        .success()
        .stdout(predicate::str::contains("libfoo"))
        .stdout(predicate::str::contains("change `tag` to \"1.1\""))
        .stdout(predicate::str::contains("libbranch"));

    let before = lock(&e.project);
    wowlua(&e.project, &e.home)
        .args(["update", "libbranch"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Updating libbranch"));
    let after = lock(&e.project);
    let source = |l: &serde_json::Value, name: &str| {
        l["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap()["source"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_ne!(source(&before, "libbranch"), source(&after, "libbranch"));
    assert_eq!(source(&before, "libfoo"), source(&after, "libfoo"));

    // `--precise` moves a branch pin back; tags are pinned by the manifest.
    let first = source(&before, "libbranch");
    let first_commit = first.rsplit('#').next().unwrap().to_string();
    wowlua(&e.project, &e.home)
        .args(["update", "libbranch", "--precise", &first_commit])
        .assert()
        .success();
    assert_eq!(source(&lock(&e.project), "libbranch"), first);
    wowlua(&e.project, &e.home)
        .args(["update", "libfoo", "--precise", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("pinned by its declaration"));

    // A failed add leaves the manifest unchanged.
    let unchanged = manifest();
    wowlua(&e.project, &e.home)
        .args([
            "add",
            "missing",
            "--git",
            &file_url(&e.root.join("nope")),
            "--kind",
            "lua-library",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("wowlua.json was left unchanged"));
    assert_eq!(manifest(), unchanged);

    wowlua(&e.project, &e.home)
        .args(["remove", "libbranch", "--dev"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Removing libbranch"));
    assert!(!manifest().contains("libbranch"));
    wowlua(&e.project, &e.home)
        .args(["remove", "libbranch"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "`libbranch` is not in `dependencies`",
        ));

    wowlua(&e.project, &e.home)
        .args(["cache", "clean"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Removed 0 cache entries"));
    wowlua(&e.project, &e.home)
        .args(["cache", "clean", "--all"])
        .assert()
        .success();
    wowlua(&e.project, &e.home)
        .arg("fetch")
        .assert()
        .success()
        .stderr(predicate::str::contains("Fetched 1 package"));
}

#[test]
fn wowlua_library_is_compiled_into_the_addon() {
    let e = env();
    let ui = e.root.join("ui");
    write(
        &ui,
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "wowlua-ui", "version": "0.4.2", "kind": "library", "license": "MIT" },
  "features": { "default": ["shadows"], "shadows": [], "fancy": [] },
  "config": { "size": 10 },
  "check": { "mode": "strict" } }"#,
            ),
            (
                "src/init.wowlua",
                "import ButtonClass from \"./widgets/button\"\nexport const Button = ButtonClass\nexport const VERSION = \"0.4.2\"\n",
            ),
            (
                "src/widgets/button.wowlua",
                "import config, features from \"wowlua:meta\"\nimport clamp from \"/util\"\nexport class ButtonClass\n    size: number = clamp(config.size, 1, 100)\n    shadows: boolean = features.shadows\n    fancy: boolean = features.fancy\n    constructor()\n    end\nend\n",
            ),
            (
                "src/util.wowlua",
                "export function clamp(v: number, lo: number, hi: number): number\n    if v < lo then return lo end\n    if v > hi then return hi end\n    return v\nend\n",
            ),
            ("src/unused.wowlua", "export const unused = 1\n"),
            // A warning in library code is not the addon's problem.
            (
                "src/noisy.wowlua",
                "local unusedLocal = 1\nexport const noisy = 2\n",
            ),
        ],
    );
    write(
        &e.project,
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "my-addon", "version": "1.0.0" },
  "dependencies": { "wowlua-ui": { "path": "../ui", "features": ["fancy"], "config": { "size": 500 } } } }"#,
            ),
            (
                "src/main.wowlua",
                "import Button, VERSION from \"wowlua-ui\"\nimport clamp from \"wowlua-ui/util\"\nimport noisy from \"wowlua-ui/noisy\"\nlocal b = Button.new()\nprint(VERSION, b.size, b.shadows, b.fancy, clamp(5, 1, 3), noisy)\n",
            ),
        ],
    );
    wowlua(&e.project, &e.home)
        .args(["build", "--release"])
        .assert()
        .success()
        .stderr(predicate::str::contains("W1003").not());
    let out = e.project.join("dist/release/MyAddon");
    let toc = fs::read_to_string(out.join("MyAddon.toc")).unwrap();
    assert!(
        toc.contains("libs\\wowlua-ui\\util.lua\nlibs\\wowlua-ui\\widgets\\button.lua\n"),
        "{toc}"
    );
    assert!(!out.join("libs/wowlua-ui/unused.lua").exists());
    let button = fs::read_to_string(out.join("libs/wowlua-ui/widgets/button.lua")).unwrap();
    assert!(
        button.contains("__wl.module(\"wowlua-ui/widgets/button\")"),
        "{button}"
    );
    // The library's config, overridden by the entry; its own features.
    assert!(button.contains("clamp(500, 1, 100)"), "{button}");
    assert!(
        button.contains("self.shadows = true self.fancy = true"),
        "{button}"
    );
    let main = fs::read_to_string(out.join("src/main.lua")).unwrap();
    assert!(
        main.contains("__wl.require(\"wowlua-ui/init\").Button"),
        "{main}"
    );
    let licenses = fs::read_to_string(out.join("LICENSES.txt")).unwrap();
    assert!(
        licenses.contains("== wowlua-ui (libs/wowlua-ui) ==\n\nLicense: MIT"),
        "{licenses}"
    );

    // The library checks on its own, but is not built on its own.
    wowlua(&ui, &e.home).arg("check").assert().success();
    wowlua(&ui, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "run `wowlua check` to check the library",
        ));

    // Unknown packages and escaping paths.
    write(
        &e.project,
        &[("src/main.wowlua", "import x from \"nope/x\"\nprint(x)\n")],
    );
    wowlua(&e.project, &e.home)
        .arg("check")
        .assert()
        .failure()
        .stderr(predicate::str::contains("package `nope` not found"));
}

#[test]
fn types_packages_type_the_checker() {
    let e = env();
    write(
        &e.root.join("libfoo-types"),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "libfoo-types", "version": "1.0.0", "kind": "types" } }"#,
            ),
            (
                "LibFoo.d.wowlua",
                "declare function LibFoo_Greet(name: string): string\n",
            ),
        ],
    );
    write(
        &e.project,
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "my-addon", "version": "1.0.0" },
  "check": { "mode": "strict" },
  "dev-dependencies": { "libfoo-types": { "path": "../libfoo-types" } } }"#,
            ),
            (
                "src/main.wowlua",
                "local s: string = LibFoo_Greet(\"x\")\nprint(s)\n",
            ),
        ],
    );
    wowlua(&e.project, &e.home).arg("check").assert().success();
    write(
        &e.project,
        &[(
            "src/main.wowlua",
            "local n: number = 42\nprint(LibFoo_Greet(n))\n",
        )],
    );
    wowlua(&e.project, &e.home)
        .arg("check")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E2"));
}

fn workspace(root: &Path) {
    write(
        root,
        &[
            (
                "wowlua.json",
                r#"{ "workspace": { "members": ["addons/*", "libs/*"], "default-members": ["addons/core"] },
  "check": { "mode": "strict" } }"#,
            ),
            (
                "addons/core/wowlua.json",
                r#"{ "package": { "name": "suite-core", "version": "1.0.0" },
  "addon": { "name": "SuiteCore", "saved-variables": ["SuiteDB"] },
  "dependencies": { "shared": "*" } }"#,
            ),
            (
                "addons/core/src/main.wowlua",
                "import greet from \"shared/greet\"\n@global\nexport function SuiteCore_Score(unit: string): number\n    return #greet(unit)\nend\n",
            ),
            (
                "addons/options/wowlua.json",
                r#"{ "package": { "name": "suite-options", "version": "1.0.0" },
  "addon": { "name": "SuiteOptions", "load-on-demand": true },
  "addon-dependencies": { "required": ["SuiteCore"] } }"#,
            ),
            (
                "addons/options/src/main.wowlua",
                "local s: number = SuiteCore_Score(\"player\")\nprint(s)\n",
            ),
            (
                "libs/shared/wowlua.json",
                r#"{ "package": { "name": "shared", "version": "0.1.0", "kind": "library", "license": "MIT" } }"#,
            ),
            (
                "libs/shared/src/greet.wowlua",
                "export function greet(name: string): string\n    return \"hi \" .. name\nend\n",
            ),
        ],
    );
}

#[test]
fn workspaces() {
    let e = env();
    let root = &e.project;
    workspace(root);
    // At the root: the default members.
    wowlua(root, &e.home)
        .arg("build")
        .assert()
        .success()
        .stderr(predicate::str::contains("Finished SuiteCore"))
        .stderr(predicate::str::contains("SuiteOptions").not());
    assert!(
        root.join("dist/dev/SuiteCore/libs/shared/greet.lua")
            .exists()
    );
    assert!(root.join("wowlua.lock").exists());
    assert!(!root.join("addons/core/wowlua.lock").exists());
    // `-p`, and the member around the current directory; one `dist/`.
    wowlua(root, &e.home)
        .args(["-p", "suite-options", "build"])
        .assert()
        .success();
    wowlua(&root.join("addons/options"), &e.home)
        .args(["build", "--release"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dist/release/SuiteOptions"));
    assert!(
        root.join("dist/release/SuiteOptions/SuiteOptions.toc")
            .exists()
    );
    // The library member checks on its own.
    wowlua(root, &e.home)
        .args(["check", "-p", "shared"])
        .assert()
        .success();
    // The generated declarations of SuiteCore type-check SuiteOptions.
    write(
        root,
        &[(
            "addons/options/src/main.wowlua",
            "local s: string = SuiteCore_Score(\"player\")\nprint(s)\n",
        )],
    );
    wowlua(root, &e.home)
        .args(["check", "-p", "suite-options"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E2"));
    write(
        root,
        &[(
            "addons/options/src/main.wowlua",
            "local s: number = SuiteCore_Score(\"player\")\nprint(s)\n",
        )],
    );
    // Commands on one package need a selection.
    wowlua(root, &e.home)
        .args(["add", "--addon", "Details"])
        .assert()
        .success();
    let all =
        r#"{ "workspace": { "members": ["addons/*", "libs/*"] }, "check": { "mode": "strict" } }"#;
    write(root, &[("wowlua.json", all)]);
    wowlua(root, &e.home)
        .args(["add", "--addon", "Details"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("choose one with `-p <package>`"));
    wowlua(root, &e.home)
        .arg("check")
        .assert()
        .success()
        .stderr(predicate::str::contains("Checked SuiteCore"))
        .stderr(predicate::str::contains("Checked SuiteOptions"))
        .stderr(predicate::str::contains("Checked Shared"));

    // Workspace rules.
    write(
        root,
        &[(
            "addons/core/wowlua.json",
            r#"{ "package": { "name": "suite-core", "version": "1.0.0" },
  "addon": { "name": "SuiteCore", "saved-variables": ["SuiteDB"] },
  "addon-dependencies": { "required": ["SuiteOptions"] },
  "dependencies": { "shared": "*" } }"#,
        )],
    );
    wowlua(root, &e.home)
        .arg("check")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4324]"));
    write(
        root,
        &[(
            "addons/options/wowlua.json",
            r#"{ "package": { "name": "suite-options", "version": "1.0.0" },
  "addon": { "name": "SuiteCore", "saved-variables": ["SuiteDB"] },
  "build": { "out": "x" } }"#,
        )],
    );
    wowlua(root, &e.home)
        .arg("check")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4316]"));
    write(
        root,
        &[(
            "addons/options/wowlua.json",
            r#"{ "package": { "name": "suite-options", "version": "1.0.0" },
  "addon": { "name": "SuiteCore", "saved-variables": ["SuiteDB"] } }"#,
        )],
    );
    wowlua(root, &e.home)
        .arg("check")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4325]"))
        .stderr(predicate::str::contains("error[E4120]"));
}

#[test]
fn init_from_pkgmeta_converts_externals() {
    let e = env();
    let stub = git_repo(
        &e.root.join("LibStub"),
        &[
            (
                "LibStub.lua",
                "-- LibStub, Public Domain\nLibStub = {}\n--@do-not-package@\nLibStub.test = true\n--@end-do-not-package@\n",
            ),
            ("README.md", "x"),
        ],
        "1.0",
    );
    let cbh = git_repo(
        &e.root.join("CallbackHandler"),
        &[
            (
                "CallbackHandler-1.0/CallbackHandler-1.0.xml",
                "<Ui><Script file=\"CallbackHandler-1.0.lua\"/></Ui>",
            ),
            (
                "CallbackHandler-1.0/CallbackHandler-1.0.lua",
                "-- License: BSD\nCBH = {}\n",
            ),
        ],
        "r1",
    );
    let classic = git_repo(
        &e.root.join("LibClassic"),
        &[("LibClassic.lua", "-- MIT License\n")],
        "v1",
    );
    let addon = &e.project;
    write(
        addon,
        &[
            (
                ".pkgmeta",
                &format!(
                    "package-as: MyAddon\nexternals:\n  Libs/LibStub:\n    url: {stub}\n    tag: 1.0\n  Libs/CallbackHandler-1.0:\n    url: {cbh}\n    path: CallbackHandler-1.0\n  Libs/LibClassic: {classic}\n  Libs/Hg: https://hg.wowace.com/wow/x/mainline\nignore:\n  - Docs\nmanual-changelog: CHANGELOG.md\nenable-nolib-creation: yes\nmove-folders:\n  MyAddon/Options: MyAddon_Options\n  MyAddon/Media: MyAddon_Media\n"
                ),
            ),
            (
                "MyAddon.toc",
                "## Interface: 120100, 11509\n## Title: My Addon\n## Version: @project-version@\n## X-Curse-Project-ID: 99\n\n#@no-lib-strip@\nLibs\\LibStub\\LibStub.lua\n#@end-no-lib-strip@\nembeds.xml\nLibs\\LibClassic\\LibClassic.lua [AllowLoadGameType vanilla]\nCore.lua\n",
            ),
            (
                "embeds.xml",
                "<Ui>\n  <Include file=\"Libs\\CallbackHandler-1.0\\CallbackHandler-1.0.xml\"/>\n</Ui>\n",
            ),
            (
                "Core.lua",
                "local _, ns = ...\n--@debug@\nns.debug = true\n--@end-debug@\nns.version = \"@project-version@\"\n",
            ),
            ("Docs/notes.txt", "docs"),
            ("Media/logo.tga", "TGA"),
            (
                "Options/MyAddon_Options.toc",
                "## Interface: 120100\n## Title: My Addon Options\n## LoadOnDemand: 1\n## Dependencies: MyAddon\nOptions.lua\n",
            ),
            ("Options/Options.lua", "-- options\n"),
        ],
    );
    wowlua(addon, &e.home)
        .args(["init", "--from-pkgmeta"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Converted 3 externals from .pkgmeta",
        ))
        .stderr(predicate::str::contains(
            "Adopted Options as a workspace member",
        ))
        .stderr(predicate::str::contains("warning[W4333]"))
        .stderr(predicate::str::contains("warning[W4332]"));
    let m: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(addon.join("wowlua.json")).unwrap()).unwrap();
    assert_eq!(m["build"]["libs-dir"], "Libs");
    assert_eq!(
        m["dependencies"]["libstub"],
        serde_json::json!({ "git": stub, "tag": "1.0", "submodules": true, "kind": "lua-library", "install-path": "LibStub", "files": ["LibStub.lua"] })
    );
    assert_eq!(
        m["dependencies"]["callbackhandler-1-0"]["subdir"],
        "CallbackHandler-1.0"
    );
    assert_eq!(
        m["dependencies"]["callbackhandler-1-0"]["files"],
        serde_json::json!(["CallbackHandler-1.0.xml"])
    );
    assert_eq!(
        m["dependencies"]["libclassic"]["flavors"],
        serde_json::json!(["vanilla"])
    );
    assert_eq!(
        m["build"]["load-order"][0], "Libs/LibStub/LibStub.lua",
        "{m}"
    );
    assert_eq!(m["build"]["exclude"], serde_json::json!(["Docs/**"]));
    assert_eq!(m["publish"]["changelog"], "CHANGELOG.md");
    // No GitHub `origin`: the nolib target is left out.
    assert!(m["publish"]["targets"].get("github").is_none());
    assert_eq!(m["compat"]["packager-keywords"], true);
    assert_eq!(m["workspace"]["members"], serde_json::json!(["Options"]));
    assert!(addon.join("Options/wowlua.json").exists());
    let lock = lock(addon);
    assert_eq!(
        lock["flavor-table"]["interfaces"]["standard"],
        serde_json::json!([120100])
    );

    wowlua(addon, &e.home)
        .args(["-p", "my-addon", "build", "--release"])
        .assert()
        .success();
    let out = addon.join("dist/release/MyAddon");
    let toc = fs::read_to_string(out.join("MyAddon.toc")).unwrap();
    assert!(
        toc.contains("Libs\\LibStub\\LibStub.lua\nsrc\\embeds.xml\nLibs\\LibClassic\\LibClassic.lua [AllowLoadGameType "),
        "{toc}"
    );
    let embeds = fs::read_to_string(out.join("src/embeds.xml")).unwrap();
    assert!(
        embeds.contains(
            "Interface\\AddOns\\MyAddon\\Libs\\CallbackHandler-1.0\\CallbackHandler-1.0.xml"
        ),
        "{embeds}"
    );
    assert!(
        out.join("Libs/CallbackHandler-1.0/CallbackHandler-1.0.lua")
            .exists()
    );
    assert!(!out.join("Libs/LibStub/README.md").exists());
    // Library files get the packager's processing too (decision T120).
    let stub_out = fs::read_to_string(out.join("Libs/LibStub/LibStub.lua")).unwrap();
    assert_eq!(stub_out, "-- LibStub, Public Domain\nLibStub = {}\n");
    let core = fs::read_to_string(out.join("src/Core.lua")).unwrap();
    assert!(
        core.contains("--[==[@debug@\nns.debug = true\n--@end-debug@]==]"),
        "{core}"
    );
    assert!(!core.contains("@project-version@"), "{core}");
    // The dev profile keeps debug blocks active.
    wowlua(addon, &e.home)
        .args(["-p", "my-addon", "build"])
        .assert()
        .success();
    let dev = fs::read_to_string(addon.join("dist/dev/MyAddon/src/Core.lua")).unwrap();
    assert!(
        dev.contains("--@debug@\nns.debug = true\n--@end-debug@"),
        "{dev}"
    );
    let stub_dev =
        fs::read_to_string(addon.join("dist/dev/MyAddon/Libs/LibStub/LibStub.lua")).unwrap();
    assert!(stub_dev.contains("LibStub.test = true"), "{stub_dev}");
    // The moved folder builds as its own addon.
    wowlua(addon, &e.home)
        .args(["-p", "my-addon-options", "build"])
        .assert()
        .success();
    assert!(
        addon
            .join("dist/dev/MyAddon_Options/MyAddon_Options.toc")
            .exists()
    );
}

#[test]
fn compat_pkgmeta_keeps_pkgmeta_as_the_source() {
    let e = env();
    let stub = git_repo(
        &e.root.join("LibStub"),
        &[("LibStub.lua", "LibStub = {}\n")],
        "1.0",
    );
    write(
        &e.project,
        &[
            (
                ".pkgmeta",
                &format!("externals:\n  libs/LibStub:\n    url: {stub}\n    tag: 1.0\n"),
            ),
            (
                "wowlua.json",
                r#"{ "package": { "name": "my-addon", "version": "1.0.0" },
  "compat": { "pkgmeta": true },
  "dependencies": { "libstub": { "files": ["LibStub.lua"] } } }"#,
            ),
            ("src/main.lua", "print(1)\n"),
        ],
    );
    wowlua(&e.project, &e.home)
        .args(["build", "--release"])
        .assert()
        .success();
    let toc = fs::read_to_string(e.project.join("dist/release/MyAddon/MyAddon.toc")).unwrap();
    assert!(
        toc.contains("libs\\LibStub\\LibStub.lua\nsrc\\main.lua"),
        "{toc}"
    );
    write(
        &e.project,
        &[(
            "wowlua.json",
            r#"{ "package": { "name": "my-addon", "version": "1.0.0" },
  "compat": { "pkgmeta": true },
  "dependencies": { "libstub": { "git": "https://example.com/x", "files": ["LibStub.lua"] } } }"#,
        )],
    );
    wowlua(&e.project, &e.home)
        .arg("build")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4005]"));
}
