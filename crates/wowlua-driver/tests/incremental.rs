//! `wowlua dev` rebuilds with a `BuildCache`: after every edit, a cached
//! build gives what a fresh build of the same tree gives (files, maps,
//! TOCs, diagnostics in order, success), and recompiles only what changed.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wowlua_diagnostics::{ColorChoice, MessageFormat, Renderer};
use wowlua_driver::{BuildCache, BuildRequest, CacheStats, Session, build, build_with_cache};

fn request() -> BuildRequest {
    BuildRequest {
        profile: "dev".into(),
        flavors: None,
        out_dir: None,
        write: true,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: wowlua_driver::parallel::Jobs::default(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    }
}

/// Everything a build shows: written tree, TOCs, diagnostics, success.
#[derive(Debug, PartialEq)]
struct Snapshot {
    ok: bool,
    diagnostics: Vec<String>,
    tocs: String,
    version: Option<(String, String)>,
    dist: BTreeMap<String, Vec<u8>>,
}

fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn run(root: &Path, cache: Option<&mut BuildCache>) -> Snapshot {
    let _ = std::fs::remove_dir_all(root.join("dist"));
    run_over(root, cache)
}

/// [`run`] over the output of the last build, as `wowlua dev` does.
fn run_over(root: &Path, cache: Option<&mut BuildCache>) -> Snapshot {
    let mut session = Session::new();
    let options = wowlua_driver::LoadOptions {
        cwd: root.to_path_buf(),
        offline: true,
        ..Default::default()
    };
    let project = wowlua_driver::load_project(&mut session, &options).expect("loads");
    let built = match cache {
        Some(cache) => build_with_cache(&mut session, &project, &request(), cache),
        None => build(&mut session, &project, &request()),
    };
    project.apply_lints(&mut session);
    let diagnostics = session.take_diagnostics();
    let ok = built.is_some() && !diagnostics.should_fail(false);
    let renderer = Renderer::new(MessageFormat::Human, ColorChoice::Never);
    let diagnostics = diagnostics
        .into_vec()
        .iter()
        .map(|d| renderer.render_to_string(&session.db, d))
        .collect();
    Snapshot {
        ok,
        diagnostics,
        tocs: built
            .as_ref()
            .map(|b| format!("{:?}", b.tocs))
            .unwrap_or_default(),
        version: built
            .as_ref()
            .map(|b| (b.version.clone(), b.build_id.clone())),
        dist: tree(&root.join("dist")),
    }
}

/// A project in a temporary folder, rebuilt after each edit.
struct Dev {
    _dir: tempfile::TempDir,
    root: PathBuf,
    cache: BuildCache,
}

impl Dev {
    fn new(files: &[(&str, &str)]) -> Dev {
        let dir = tempfile::tempdir().unwrap();
        let dev = Dev {
            root: dir.path().to_path_buf(),
            _dir: dir,
            cache: BuildCache::default(),
        };
        for (path, text) in files {
            dev.write(path, text);
        }
        dev
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path(rel)).unwrap();
    }

    /// Builds with the cache and fresh, compares, and returns the stats.
    #[track_caller]
    fn rebuild(&mut self) -> CacheStats {
        let cached = run(&self.root, Some(&mut self.cache));
        let stats = self.cache.stats;
        let fresh = run(&self.root, None);
        assert_eq!(cached, fresh, "a cached rebuild differs from a fresh build");
        stats
    }
}

const MANIFEST: &str = r#"{ "package": { "name": "dev-cache", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {} } }
"#;

/// The same file in both flavors: plain Lua compiles the same in each.
#[test]
fn an_edit_recompiles_only_the_edited_file() {
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/a.lua", "local _, ns = ...\nns.a = 1\n"),
        ("src/b.lua", "local _, ns = ...\nprint(ns.a)\n"),
        ("src/c.lua", "local _, ns = ...\nns.c = { 1, 2, 3 }\n"),
    ]);
    let cold = dev.rebuild();
    assert_eq!((cold.hits, cold.misses), (3, 3), "the second flavor reuses");
    assert_eq!(cold.summarized, 0);
    let again = dev.rebuild();
    assert_eq!((again.hits, again.misses), (6, 0), "{again:?}");
    assert_eq!(again.summarized, 3, "no file is parsed again: {again:?}");
    dev.write("src/a.lua", "local _, ns = ...\nns.a = 2 -- edited\n");
    let edit = dev.rebuild();
    assert_eq!((edit.hits, edit.misses), (5, 1), "{edit:?}");
    assert_eq!(edit.lua_checks, 2, "the unchanged files' Lua 5.1 checks");
    assert_eq!(
        edit.summarized, 2,
        "only the edited file is parsed: {edit:?}"
    );
}

/// A Lua 5.1 limit error (E5001) in a source stays reported on rebuilds.
#[test]
fn lua_checks_of_sources_are_kept() {
    let locals: String = (0..210).map(|i| format!("local v{i} = {i}\n")).collect();
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/a.lua", &locals),
        ("src/b.lua", "print('b')\n"),
    ]);
    dev.rebuild();
    dev.write("src/b.lua", "print('b', 2)\n");
    let edit = dev.rebuild();
    assert_eq!(edit.lua_checks, 1, "{edit:?}");
}

#[test]
fn importers_recompile_only_when_what_they_see_changes() {
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        (
            "src/util.wowlua",
            "export const LIMIT = 3\nexport function double(n: number): number return n * 2 end\n",
        ),
        (
            "src/main.wowlua",
            "import LIMIT, double from \"./util\"\nprint(double(LIMIT))\n",
        ),
    ]);
    dev.rebuild();
    dev.rebuild();
    // A function body: the importer's context is the same.
    dev.write(
        "src/util.wowlua",
        "export const LIMIT = 3\nexport function double(n: number): number return n + n end\n",
    );
    let body = dev.rebuild();
    // A constant the importer inlines: its context differs.
    dev.write(
        "src/util.wowlua",
        "export const LIMIT = 4\nexport function double(n: number): number return n + n end\n",
    );
    let constant = dev.rebuild();
    assert!(
        constant.misses > body.misses,
        "body {body:?}, constant {constant:?}"
    );
}

/// `build.id` changes with every edit: a file that reads `wowlua:meta` is
/// compiled every time.
#[test]
fn files_that_read_meta_are_compiled_every_time() {
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        (
            "src/meta.wowlua",
            "import build, flavor from \"wowlua:meta\"\nprint(build.id, flavor)\n",
        ),
        ("src/plain.lua", "print(1)\n"),
    ]);
    dev.rebuild();
    dev.write("src/plain.lua", "print(2)\n");
    let edit = dev.rebuild();
    assert_eq!(
        edit.misses, 3,
        "meta.wowlua twice, plain.lua once: {edit:?}"
    );
}

#[test]
fn added_removed_and_renamed_files() {
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/a.lua", "print('a')\n"),
        ("src/b.lua", "print('b')\n"),
    ]);
    dev.rebuild();
    dev.write("src/0.lua", "print('first')\n");
    dev.rebuild();
    dev.write("src/z.lua", "print('last')\n");
    dev.rebuild();
    dev.remove("src/0.lua");
    dev.rebuild();
    std::fs::rename(dev.path("src/a.lua"), dev.path("src/renamed.lua")).unwrap();
    dev.rebuild();
    dev.remove("src/z.lua");
    dev.rebuild();
}

#[test]
fn a_standard_library_import_adds_files() {
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/main.wowlua", "print(1)\n"),
        ("src/plain.lua", "print(2)\n"),
    ]);
    dev.rebuild();
    dev.write(
        "src/main.wowlua",
        "import events from \"wowlua:events\"\nprint(events)\n",
    );
    dev.rebuild();
    dev.write("src/main.wowlua", "print(1)\n");
    dev.rebuild();
}

/// A failed build keeps the cache: once fixed, the other files are reused.
#[test]
fn errors_keep_the_cache() {
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/a.lua", "print('a')\n"),
        ("src/b.lua", "print('b')\n"),
    ]);
    dev.rebuild();
    dev.write("src/a.lua", "print('a'\n");
    dev.rebuild();
    dev.write("src/a.lua", "print('a', 2)\n");
    let fixed = dev.rebuild();
    assert_eq!((fixed.hits, fixed.misses), (3, 1), "{fixed:?}");
}

/// Lowering warns (W3030) in one file, finishing fails (E5001) in another:
/// replayed diagnostics keep their order.
#[test]
fn replayed_diagnostics_keep_their_order() {
    let locals: String = (0..210).map(|i| format!("local v{i} = {i}\n")).collect();
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/a.lua", &locals),
        (
            "src/util.wowlua",
            "@deprecated(\"use twice\")\nexport function double(n: number): number return n * 2 end\nexport function twice(n: number): number return n * 2 end\n",
        ),
        (
            "src/main.wowlua",
            "import double from \"./util\"\nprint(double(1))\n",
        ),
        ("src/z.lua", "print('z')\n"),
    ]);
    let first = run(&dev.root, None);
    assert!(
        first.diagnostics.iter().any(|d| d.contains("E5001"))
            && first.diagnostics.iter().any(|d| d.contains("W3030")),
        "{:#?}",
        first.diagnostics
    );
    dev.rebuild();
    dev.write("src/z.lua", "print('z', 2)\n");
    dev.rebuild();
    dev.write(
        "src/main.wowlua",
        "import double from \"./util\"\nprint(double(2))\n",
    );
    dev.rebuild();
}

/// `-0` and `0` differ per flavor only in the sign: the contexts differ.
#[test]
fn flavor_constants_that_differ_only_in_sign() {
    let mut dev = Dev::new(&[
        (
            "wowlua.json",
            r#"{ "package": { "name": "dev-cache", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {}, "camelot": {} } }
"#,
        ),
        (
            "src/zero.wowlua",
            "import flavor from \"wowlua:meta\"\nexport const ZERO = flavor == \"vanilla\" and -0 or 0\n",
        ),
        (
            "src/main.wowlua",
            "import ZERO from \"./zero\"\nprint(1 / ZERO)\n",
        ),
        ("src/plain.lua", "print(1)\n"),
    ]);
    dev.rebuild();
    dev.write("src/plain.lua", "print(2)\n");
    dev.rebuild();
}

/// A setting in the manifest empties the cache.
#[test]
fn manifest_changes_empty_the_cache() {
    let mut dev = Dev::new(&[("wowlua.json", MANIFEST), ("src/a.lua", "print('a')\n")]);
    dev.rebuild();
    assert!(!dev.rebuild().cleared);
    dev.write(
        "wowlua.json",
        r#"{ "package": { "name": "dev-cache", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {} },
  "profiles": { "dev": { "log-level": "debug" } } }
"#,
    );
    let changed = dev.rebuild();
    assert!(changed.cleared && changed.hits == 1, "{changed:?}");
}

/// A file starts setting a global another reads: the load order moves.
#[test]
fn load_order_follows_edits() {
    let mut dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/a.lua", "print(Shared.x)\n"),
        ("src/b.lua", "print('b')\n"),
    ]);
    dev.rebuild();
    dev.write("src/b.lua", "Shared = { x = 1 }\n");
    dev.rebuild();
    dev.write("src/b.lua", "print('b')\n");
    dev.rebuild();
}

/// With `check.lua`, the type check's warnings are the same.
#[test]
fn check_lua_warnings_are_the_same() {
    let mut dev = Dev::new(&[
        (
            "wowlua.json",
            r#"{ "package": { "name": "dev-cache", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {} }, "check": { "lua": true } }
"#,
        ),
        ("src/a.lua", "local t = { x = 1 }\nprint(t.y)\n"),
        ("src/b.lua", "local n = 1\nprint(n .. {})\n"),
    ]);
    dev.rebuild();
    dev.write("src/a.lua", "local t = { x = 1 }\nprint(t.z)\n");
    dev.rebuild();
}

/// With `check.lua`, a later flavor takes the type check of a plain Lua
/// file from an earlier one when what the file reads of the declarations
/// is the same there: not for a file that reads an API that differs, nor
/// for one with a union of declared types (its order can differ).
#[test]
fn check_lua_flavors_share_what_checks_the_same() {
    let mut dev = Dev::new(&[
        (
            "wowlua.json",
            r#"{ "package": { "name": "dev-cache", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {} }, "check": { "lua": true } }
"#,
        ),
        (
            "src/a.lua",
            "local t = { x = 1 }\nprint(t.y, math.floor(1.5))\n",
        ),
        ("src/b.lua", "local n = 1\nprint(n .. {})\n"),
        ("src/c.lua", "print(C_Seasons.HasActiveSeason())\n"),
        (
            "src/d.lua",
            "local f = math.random() > 0.5 and UIParent or GameTooltip\nprint(string.rep(f, 2))\n",
        ),
        ("src/e.lua", "local v = { a = 1 }\nprint(typeName(v))\n"),
    ]);
    let cold = dev.rebuild();
    assert_eq!(cold.type_checks, 3, "a, b and e: {cold:?}");
    dev.write("src/b.lua", "local n = 2\nprint(n .. {})\n");
    let edit = dev.rebuild();
    assert_eq!(edit.type_checks, 3, "{edit:?}");
    assert_eq!(edit.summarized, 0, "type-checked files keep their parses");
}

/// Over the last build's output, a rebuild writes only what changed (#7
/// stage 4): an unchanged file is linked from the old folder, an edited one
/// written; folder and maps are what a fresh build writes.
#[test]
fn rebuilds_write_only_what_changed() {
    let dev = Dev::new(&[
        ("wowlua.json", MANIFEST),
        ("src/a.lua", "local _, ns = ...\nns.a = 1\n"),
        ("src/b.lua", "local _, ns = ...\nprint(ns.a)\n"),
    ]);
    let mut cache = BuildCache::default();
    #[cfg(unix)]
    let file = |rel: &str| {
        let dist = dev.root.join("dist").join("dev");
        let addon = std::fs::read_dir(&dist)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir() && !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .expect("the AddOn folder");
        addon.join(rel)
    };
    run_over(&dev.root, Some(&mut cache));
    #[cfg(unix)]
    let inode = |rel: &str| {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(file(rel)).unwrap().ino()
    };
    #[cfg(unix)]
    let before = (inode("src/a.lua"), inode("src/b.lua"));
    dev.write("src/b.lua", "local _, ns = ...\nprint(ns.a, 2)\n");
    let incremental = run_over(&dev.root, Some(&mut cache));
    #[cfg(unix)]
    {
        assert_eq!(inode("src/a.lua"), before.0, "unchanged: linked");
        assert_ne!(inode("src/b.lua"), before.1, "edited: written");
    }
    let fresh = run(&dev.root, None);
    assert_eq!(incremental.dist, fresh.dist);
    assert_eq!(incremental.diagnostics, fresh.diagnostics);
}

/// Replays edits on a real project (copied first): every Nth source gets
/// a comment, one file a new global, then a file is added and removed.
/// Run with `WOWLUA_DEV_REPLAY=<project folder> cargo test --release -p
/// wowlua-driver --test incremental replay -- --ignored --nocapture`.
#[test]
#[ignore]
fn replay() {
    let Ok(source) = std::env::var("WOWLUA_DEV_REPLAY") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let copy = |from: &Path, to: &Path| {
        let status = std::process::Command::new("cp")
            .arg("-R")
            .arg(from)
            .arg(to)
            .status()
            .unwrap();
        assert!(status.success());
    };
    let root = dir.path().join("p");
    copy(Path::new(&source), &root);
    let mut lua: Vec<String> = tree(&root)
        .into_keys()
        .filter(|p| {
            (p.ends_with(".lua") || (p.ends_with(".wowlua") && !p.ends_with(".d.wowlua")))
                && !p.starts_with("dist/")
        })
        .collect();
    lua.sort();
    let mut dev = Dev {
        _dir: dir,
        root,
        cache: BuildCache::default(),
    };
    let rel = |p: &str| p.to_string();
    let stats = dev.rebuild();
    eprintln!("cold: {stats:?}");
    for (round, file) in lua.iter().step_by(lua.len().div_ceil(5).max(1)).enumerate() {
        let path = dev.path(&rel(file));
        let mut text = std::fs::read(&path).unwrap();
        text.extend_from_slice(format!("\n-- edit {round}\n").as_bytes());
        std::fs::write(&path, text).unwrap();
        let stats = dev.rebuild();
        eprintln!("comment in {file}: {stats:?}");
    }
    let first = &lua[0];
    let path = dev.path(&rel(first));
    let mut text = std::fs::read(&path).unwrap();
    text.extend_from_slice(b"\nWowluaDevReplay = 1\n");
    std::fs::write(&path, text).unwrap();
    eprintln!("global in {first}: {:?}", dev.rebuild());
    let added = Path::new(first).with_file_name("wowlua_dev_replay.lua");
    let added = rel(&added.to_string_lossy());
    dev.write(&added, "print(WowluaDevReplay)\n");
    eprintln!("added a file: {:?}", dev.rebuild());
    dev.remove(&added);
    eprintln!("removed it: {:?}", dev.rebuild());
}
