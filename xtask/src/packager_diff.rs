//! `cargo xtask packager-diff`: the M6 exit check. Compares a BigWigs
//! packager output folder with a wowlua build of the same addon (adopted
//! with `wowlua init --from-pkgmeta`):
//!
//! - for every flavor, the same sequence of loaded files (TOC lines plus XML
//!   includes), with wowlua's `src/X` mapped back to `X`;
//! - every loaded file byte-identical (in-game paths may name `src/`, and
//!   XML `file=` attributes may be spelled differently when they resolve to
//!   the same files);
//! - every asset present and identical;
//! - the same effective TOC directives and interface numbers (apart from
//!   `Version` and `X-Wowlua-*`).
//!
//! Files only the packager ships that the game never loads (docs, tests and
//! other files of library checkouts) are listed but allowed.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

struct Tree {
    name: String,
    /// Lowercase relative path → on-disk relative path.
    files: BTreeMap<String, String>,
    root: std::path::PathBuf,
}

impl Tree {
    fn read(root: &Path) -> Result<Tree> {
        let name = root
            .file_name()
            .context("an AddOn folder")?
            .to_string_lossy()
            .into_owned();
        let mut files = BTreeMap::new();
        for e in walkdir::WalkDir::new(root).into_iter().flatten() {
            if e.file_type().is_file() {
                let rel = e
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(rel.to_ascii_lowercase(), rel);
            }
        }
        Ok(Tree {
            name,
            files,
            root: root.to_path_buf(),
        })
    }

    fn bytes(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.root.join(rel)).unwrap_or_default()
    }

    fn resolve(&self, rel: &str) -> Option<String> {
        self.files
            .get(&normalize(rel)?.to_ascii_lowercase())
            .cloned()
    }

    /// The TOC the flavor's client reads.
    fn toc_for(&self, flavor: &str) -> Option<String> {
        let table = wowlua_flavors::table();
        let suffixes = table
            .clients_of(flavor)
            .next()
            .map(|c| c.suffixes.clone())
            .unwrap_or_default();
        for s in suffixes.iter().map(String::as_str).chain([""]) {
            if let Some(f) = self.resolve(&format!("{}{s}.toc", self.name)) {
                return Some(f);
            }
        }
        None
    }

    /// Loaded files in order: TOC lines, each XML file followed by what it
    /// includes. Absolute in-game paths into this AddOn resolve too.
    fn loaded(
        &self,
        toc: &str,
        flavor: &str,
        locale: &str,
        problems: &mut Vec<String>,
    ) -> Vec<String> {
        let text = String::from_utf8_lossy(&self.bytes(toc)).into_owned();
        let parsed = wowlua_toc::parse(&text);
        let mut out = Vec::new();
        for f in &parsed.files {
            if !applies(flavor, &f.conditions) || !locale_applies(locale, &f.conditions) {
                continue;
            }
            // TOC variables (`[Game]`, `[Family]`, `[TextLocale]`).
            let path = wowlua_driver::build::substitute(&f.path, flavor, locale).replace('\\', "/");
            match self.resolve(&path) {
                Some(rel) => self.visit(&rel, flavor, &mut out, problems, 0),
                None => problems.push(format!("{}: {toc} lists missing `{}`", self.name, f.path)),
            }
        }
        out
    }

    fn visit(
        &self,
        rel: &str,
        flavor: &str,
        out: &mut Vec<String>,
        problems: &mut Vec<String>,
        depth: usize,
    ) {
        out.push(rel.to_string());
        if depth > 32 || !rel.to_ascii_lowercase().ends_with(".xml") {
            return;
        }
        let text = String::from_utf8_lossy(&self.bytes(rel)).into_owned();
        let Ok(refs) = wowlua_xml::references(&text) else {
            problems.push(format!("{}: `{rel}` is not well-formed XML", self.name));
            return;
        };
        let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for r in refs {
            if !applies(flavor, &r.conditions()) {
                continue;
            }
            let target = self.target(dir, &r.normalized());
            match target {
                Some(t) => self.visit(&t, flavor, out, problems, depth + 1),
                None => problems.push(format!(
                    "{}: `{rel}` references missing `{}`",
                    self.name,
                    r.path()
                )),
            }
        }
    }

    fn target(&self, dir: &str, reference: &str) -> Option<String> {
        let prefix = format!("interface/addons/{}/", self.name.to_ascii_lowercase());
        let lower = reference.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix(&prefix) {
            return self.resolve(&reference[reference.len() - rest.len()..]);
        }
        if dir.is_empty() {
            self.resolve(reference)
        } else {
            self.resolve(&format!("{dir}/{reference}"))
        }
    }

    /// Effective directives for `flavor`: key (lowercase) → value.
    fn directives(&self, toc: &str, flavor: &str) -> (BTreeMap<String, String>, BTreeSet<u32>) {
        let text = String::from_utf8_lossy(&self.bytes(toc)).into_owned();
        let parsed = wowlua_toc::parse(&text);
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        let mut interfaces = BTreeSet::new();
        for d in &parsed.directives {
            if !applies(flavor, &d.conditions) {
                continue;
            }
            // Aliases the client treats alike.
            let key = match d.key.to_ascii_lowercase() {
                k if k == "requireddeps"
                    || (k.starts_with("dep") && !k.starts_with("dependson")) =>
                {
                    "dependencies".to_string()
                }
                k if k.starts_with("optionaldep") => "optionaldeps".to_string(),
                k => k,
            };
            // Explicit default values.
            let default = matches!(
                (key.as_str(), d.value.trim().to_ascii_lowercase().as_str()),
                ("defaultstate", "enabled")
                    | (
                        "loadondemand"
                            | "loadsavedvariablesfirst"
                            | "allowaddontableaccess"
                            | "onlybetaandptr",
                        "0"
                    )
            );
            if default {
                continue;
            }
            if key == "interface" {
                interfaces.extend(
                    d.value
                        .split(',')
                        .filter_map(|v| v.trim().parse::<u32>().ok()),
                );
                continue;
            }
            if key == "version" || key.starts_with("x-wowlua") || key.starts_with("interface-") {
                continue;
            }
            let value = d
                .value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            map.entry(key)
                .and_modify(|v| {
                    v.push_str(", ");
                    v.push_str(&value);
                })
                .or_insert(value);
        }
        (map, interfaces)
    }
}

fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

fn locale_applies(locale: &str, conditions: &[wowlua_toc::InlineDirective]) -> bool {
    conditions
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case("AllowLoadTextLocale"))
        .all(|c| c.values.iter().any(|v| v.eq_ignore_ascii_case(locale)))
}

fn applies(flavor: &str, conditions: &[wowlua_toc::InlineDirective]) -> bool {
    !wowlua_driver::scaffold::apply_conditions(&[flavor.to_string()], conditions).is_empty()
}

/// XML text without its `file="…"` values (compared separately, as
/// resolved targets).
fn without_file_attributes(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("file=\"") {
        out.push_str(&rest[..i + 6]);
        rest = &rest[i + 6..];
        match rest.find('"') {
            Some(j) => rest = &rest[j..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

pub fn run(packager: &Path, wowlua: &Path, manifest: &Path) -> Result<()> {
    let pk = Tree::read(packager)?;
    let wl = Tree::read(wowlua)?;
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(manifest)?)?;
    let src = m["build"]["src"].as_str().unwrap_or("src").to_string();
    let flavors: Vec<String> = m["flavors"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_else(|| vec!["standard".into()]);
    let map = |rel: &str| -> String {
        rel.strip_prefix(&format!("{src}/"))
            .unwrap_or(rel)
            .to_string()
    };
    let mut problems: Vec<String> = Vec::new();
    let mut loaded_pairs: BTreeMap<String, String> = BTreeMap::new();
    let mut loaded_pk: BTreeSet<String> = BTreeSet::new();
    for flavor in &flavors {
        let (Some(pk_toc), Some(wl_toc)) = (pk.toc_for(flavor), wl.toc_for(flavor)) else {
            problems.push(format!(
                "{flavor}: no TOC for this flavor in one of the outputs"
            ));
            continue;
        };
        for locale in wowlua_manifest::model::LOCALES {
            let a = pk.loaded(&pk_toc, flavor, locale, &mut problems);
            let b = wl.loaded(&wl_toc, flavor, locale, &mut problems);
            let b_mapped: Vec<String> = b.iter().map(|p| map(p)).collect();
            let lower = |v: &[String]| v.iter().map(|p| p.to_ascii_lowercase()).collect::<Vec<_>>();
            if lower(&a) != lower(&b_mapped) {
                let first = lower(&a)
                    .iter()
                    .zip(lower(&b_mapped))
                    .position(|(x, y)| *x != y)
                    .unwrap_or(a.len().min(b_mapped.len()));
                problems.push(format!(
                "{flavor} ({locale}): load order differs at #{first}: packager {:?}, wowlua {:?} (packager loads {}, wowlua {})",
                a.get(first),
                b_mapped.get(first),
                a.len(),
                b.len()
            ));
            } else if *locale == "enUS" {
                println!(
                    "{flavor}: {} files load in the same order (every client locale)",
                    a.len()
                );
            }
            for (pa, wb) in a.iter().zip(&b) {
                loaded_pairs.insert(pa.clone(), wb.clone());
            }
            loaded_pk.extend(a.iter().cloned());
        }
        // Directives.
        let (da, ia) = pk.directives(&pk_toc, flavor);
        let (db, ib) = wl.directives(&wl_toc, flavor);
        // A client reads only the numbers of its own flavor.
        let own = |set: &BTreeSet<u32>| -> BTreeSet<u32> {
            set.iter()
                .copied()
                .filter(|n| {
                    wowlua_flavors::table()
                        .by_interface(*n)
                        .is_some_and(|f| f.id == *flavor)
                })
                .collect()
        };
        let (ia, ib) = (own(&ia), own(&ib));
        if ia != ib {
            problems.push(format!(
                "{flavor}: Interface differs: packager {ia:?}, wowlua {ib:?}"
            ));
        }
        for key in da.keys().chain(db.keys()).collect::<BTreeSet<_>>() {
            if da.get(key) != db.get(key) {
                problems.push(format!(
                    "{flavor}: `## {key}` differs: packager {:?}, wowlua {:?}",
                    da.get(key),
                    db.get(key)
                ));
            }
        }
    }
    // Contents of loaded files.
    let in_game = format!(
        "Interface\\AddOns\\{}\\{}\\",
        wl.name,
        src.replace('/', "\\")
    );
    let in_game_plain = format!("Interface\\AddOns\\{}\\", wl.name);
    let mut identical = 0;
    for (pa, wb) in &loaded_pairs {
        let a = pk.bytes(pa);
        let b = wl.bytes(wb);
        if a == b {
            identical += 1;
            continue;
        }
        let ta = String::from_utf8_lossy(&a).into_owned();
        let tb = String::from_utf8_lossy(&b)
            .replace(&in_game, &in_game_plain)
            .replace(
                &in_game.replace('\\', "\\\\"),
                &in_game_plain.replace('\\', "\\\\"),
            );
        if ta == tb {
            identical += 1;
            continue;
        }
        if pa.to_ascii_lowercase().ends_with(".xml")
            && without_file_attributes(&ta) == without_file_attributes(&tb)
        {
            identical += 1;
            continue;
        }
        let line = ta
            .lines()
            .zip(tb.lines())
            .position(|(x, y)| x != y)
            .map(|i| i + 1)
            .unwrap_or_else(|| ta.lines().count().min(tb.lines().count()) + 1);
        problems.push(format!(
            "`{pa}` differs from wowlua's `{wb}` (first at line {line}; {} vs {} lines, {} vs {} bytes)",
            ta.lines().count(),
            tb.lines().count(),
            a.len(),
            b.len()
        ));
    }
    println!(
        "{identical} of {} loaded files are identical",
        loaded_pairs.len()
    );
    // Other files of the packager output.
    let mut allowed: Vec<String> = Vec::new();
    for rel in pk.files.values() {
        let lower = rel.to_ascii_lowercase();
        if loaded_pk.contains(rel) || lower.ends_with(".toc") {
            continue;
        }
        let code = lower.ends_with(".lua") || lower.ends_with(".xml");
        // Media the game can use; anything else (docs, tooling config) is
        // never loaded.
        let media = [
            ".tga", ".blp", ".png", ".jpg", ".jpeg", ".ogg", ".mp3", ".wav", ".ttf", ".otf", ".m2",
            ".flac",
        ]
        .iter()
        .any(|e| lower.ends_with(e));
        let doc = !media;
        let in_lib = m["dependencies"]
            .as_object()
            .map(|deps| {
                let libs = m["build"]["libs-dir"].as_str().unwrap_or("libs");
                deps.values().any(|d| {
                    let install = d["install-path"].as_str().unwrap_or("");
                    lower.starts_with(&format!("{}/{}/", libs, install).to_ascii_lowercase())
                })
            })
            .unwrap_or(false);
        if code || doc || in_lib {
            if !wl.files.contains_key(&lower) {
                allowed.push(rel.clone());
            }
            continue;
        }
        // An asset: present and identical.
        match wl.files.get(&lower) {
            Some(w) if pk.bytes(rel) == wl.bytes(w) => {}
            Some(_) => problems.push(format!("asset `{rel}` differs")),
            None => problems.push(format!("asset `{rel}` is missing from the wowlua build")),
        }
    }
    if !allowed.is_empty() {
        println!(
            "{} files only the packager ships, never loaded (allowed):",
            allowed.len()
        );
        for a in &allowed {
            println!("  {a}");
        }
    }
    problems.sort();
    problems.dedup();
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("FAIL {p}");
        }
        bail!("{} difference(s)", problems.len());
    }
    println!("OK: the builds load identical code");
    Ok(())
}

/// Compares every AddOn folder of a packager release folder with the
/// matching build of the wowlua project (workspace members included).
pub fn run_all(release: &Path, project: &Path) -> Result<()> {
    // AddOn name → manifest of the project and its members.
    let mut manifests: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    for e in walkdir::WalkDir::new(project)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(e.file_type().is_dir() && (n.starts_with('.') || n == "dist" || n == "src"))
        })
        .flatten()
    {
        if e.file_name() != "wowlua.json" {
            continue;
        }
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.path())?)?;
        let Some(package) = m["package"]["name"].as_str() else {
            continue;
        };
        let name = m["addon"]["name"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| wowlua_manifest::pascal_case(package));
        manifests.insert(name, e.path().to_path_buf());
    }
    let mut failed = Vec::new();
    let mut folders: Vec<_> = std::fs::read_dir(release)?
        .flatten()
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    folders.sort();
    for name in &folders {
        println!("== {name}");
        let Some(manifest) = manifests.get(name) else {
            println!("FAIL no wowlua package builds `{name}`");
            failed.push(name.clone());
            continue;
        };
        let built = project.join("dist/release").join(name);
        if let Err(e) = run(&release.join(name), &built, manifest) {
            println!("FAIL {name}: {e}");
            failed.push(name.clone());
        }
    }
    if !failed.is_empty() {
        bail!(
            "{} of {} addons differ: {}",
            failed.len(),
            folders.len(),
            failed.join(", ")
        );
    }
    println!("OK: all {} addons load identical code", folders.len());
    Ok(())
}
