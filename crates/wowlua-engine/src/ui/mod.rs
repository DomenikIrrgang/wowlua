//! Blizzard's UI code (spec emulator §17.7, decision T186): the flavor's
//! Blizzard addons loaded as secure code before the addon under test, in
//! the client's load order, their XML built into widgets.
//!
//! The startup set is the client's (`/stdtest engine`: the 201 addons WoW
//! Forever had loaded when StdTest loaded): every Blizzard addon allowed in
//! the game and for the flavor's game type, read from the client's
//! suffixed TOC first, not load-on-demand unless a startup addon depends on
//! it, beta- and PTR-only ones on such clients. Their order is inferred
//! (unverified): `LoadFirst` ones first, each after the addons it depends
//! on, otherwise by name.

pub mod build;
pub mod xml;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use wowlua_vm::{Value, Vm};

use crate::{engine, engine_mut};
use xml::Element;

/// The UI code's templates, intrinsic frame types and load errors.
#[derive(Default)]
pub struct Registry {
    /// Virtual elements by lower-case name.
    pub templates: HashMap<String, Rc<Element>>,
    /// The file each template came from (for errors).
    pub template_files: HashMap<String, String>,
    /// Intrinsic frame types by lower-case tag: the base class and element.
    pub intrinsics: HashMap<String, (String, Rc<Element>)>,
    /// Errors while loading: files that did not parse, Lua errors.
    pub errors: Vec<String>,
    /// The addons loaded, in order.
    pub loaded: Vec<String>,
    /// Every Blizzard addon of the UI code, by lower-case name: what
    /// `C_AddOns.LoadAddOn` can load later.
    available: HashMap<String, Addon>,
    /// The folder holding `Interface`, once the code is loaded.
    root: Option<PathBuf>,
}

/// What loading the UI code did.
pub struct Report {
    pub addons: Vec<String>,
    pub errors: Vec<String>,
}

/// The game types a flavor's Blizzard code loads for: its id and family.
struct Target {
    game_types: Vec<String>,
    family: String,
    game: String,
    /// The client's TOC suffixes, in lookup order (`_Camelot`, `_Forever`,
    /// `_Mainline`).
    suffixes: Vec<String>,
    /// A beta or PTR client (`## OnlyBetaAndPTR` addons load): WoW Forever
    /// runs from `_classic_beta_`.
    beta: bool,
}

impl Target {
    fn new(flavor: &str) -> Target {
        let table = wowlua_flavors::table();
        let family = table
            .flavor(flavor)
            .map(|f| f.family.name().to_string())
            .unwrap_or_else(|| "mainline".into());
        let cap = |s: &str| {
            let mut c = s.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        };
        let client = table.clients.iter().find(|c| c.flavor == flavor);
        Target {
            game_types: vec![flavor.to_string(), family.clone()],
            game: cap(flavor),
            family: cap(&family),
            suffixes: client.map(|c| c.suffixes.clone()).unwrap_or_default(),
            beta: client.is_some_and(|c| {
                c.folders
                    .iter()
                    .any(|f| f.contains("_beta_") || f.contains("ptr_"))
            }),
        }
    }

    /// Whether a line's or directive's inline conditions hold in the game
    /// (not the glue screens).
    fn allows(&self, conditions: &[wowlua_toc::InlineDirective]) -> bool {
        conditions.iter().all(|c| {
            let any = |list: &[String]| {
                list.iter()
                    .any(|v| self.game_types.iter().any(|g| g.eq_ignore_ascii_case(v)))
            };
            match c.name.to_ascii_lowercase().as_str() {
                "allowloadgametype" => any(&c.values),
                "excludeloadgametype" => !any(&c.values),
                "allowload" => c.values.iter().any(|v| {
                    let v = v.to_ascii_lowercase();
                    v == "game" || v == "both"
                }),
                "allowloadtextlocale" => c.values.iter().any(|v| v == "enUS"),
                _ => true,
            }
        })
    }

    /// `[Family]`, `[Game]` and `[TextLocale]` in a file line.
    fn path(&self, line: &str) -> String {
        line.replace("[Family]", &self.family)
            .replace("[Game]", &self.game)
            .replace("[TextLocale]", "enUS")
            .replace('\\', "/")
    }
}

/// One Blizzard addon: its directory, TOC and dependencies.
#[derive(Clone)]
struct Addon {
    name: String,
    dir: PathBuf,
    toc: wowlua_toc::ParsedToc,
    deps: Vec<String>,
    load_first: bool,
    /// Load-on-demand: loaded at startup only as a startup addon's
    /// dependency.
    on_demand: bool,
}

fn directive<'a>(toc: &'a wowlua_toc::ParsedToc, t: &Target, key: &str) -> Vec<&'a str> {
    toc.directives
        .iter()
        .filter(|d| d.key.eq_ignore_ascii_case(key) && t.allows(&d.conditions))
        .map(|d| d.value.as_str())
        .collect()
}

/// The addons the game loads at startup, in load order, and every addon
/// it could load (by lower-case name).
fn startup_addons(interface: &Path, t: &Target) -> (Vec<Addon>, HashMap<String, Addon>) {
    let mut all: HashMap<String, Addon> = HashMap::new();
    let Ok(dirs) = std::fs::read_dir(interface.join("AddOns")) else {
        return (Vec::new(), HashMap::new());
    };
    for entry in dirs.flatten() {
        let dir = entry.path();
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        // The client's suffixed TOC first (`Blizzard_WorldMap_Mainline.toc`).
        let text = t
            .suffixes
            .iter()
            .map(|s| format!("{name}{s}.toc"))
            .chain(std::iter::once(format!("{name}.toc")))
            .find_map(|f| std::fs::read_to_string(dir.join(f)).ok());
        let Some(text) = text else {
            continue;
        };
        let toc = wowlua_toc::parse(&text);
        let on = |v: &str| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true");
        let on_demand = directive(&toc, t, "LoadOnDemand").iter().any(|v| on(v));
        // `AllowLoad` names where the addon loads (default: both).
        let allow: Vec<String> = toc
            .directives
            .iter()
            .filter(|d| d.key.eq_ignore_ascii_case("AllowLoad") && t.allows(&d.conditions))
            .map(|d| d.value.trim().to_ascii_lowercase())
            .collect();
        if !allow.is_empty() && !allow.iter().any(|a| a == "game" || a == "both") {
            continue;
        }
        let gated = |key: &str| {
            toc.directives
                .iter()
                .any(|d| d.key.eq_ignore_ascii_case(key))
        };
        let allowed_type = toc
            .directives
            .iter()
            .filter(|d| d.key.eq_ignore_ascii_case("AllowLoadGameType"))
            .all(|d| {
                d.value.split(',').any(|v| {
                    t.game_types
                        .iter()
                        .any(|g| g.eq_ignore_ascii_case(v.trim()))
                })
            });
        let excluded_type = toc
            .directives
            .iter()
            .filter(|d| d.key.eq_ignore_ascii_case("ExcludeLoadGameType"))
            .any(|d| {
                d.value.split(',').any(|v| {
                    t.game_types
                        .iter()
                        .any(|g| g.eq_ignore_ascii_case(v.trim()))
                })
            });
        if !allowed_type || excluded_type || (gated("OnlyBetaAndPTR") && !t.beta) {
            continue;
        }
        let mut deps = Vec::new();
        for key in [
            "Dep",
            "Dependencies",
            "RequiredDep",
            "RequiredDeps",
            "OptionalDep",
            "OptionalDeps",
        ] {
            for v in directive(&toc, t, key) {
                deps.extend(
                    v.split(',')
                        .map(|d| d.trim().to_string())
                        .filter(|d| !d.is_empty()),
                );
            }
        }
        let load_first = directive(&toc, t, "LoadFirst").iter().any(|v| on(v));
        all.insert(
            name.to_ascii_lowercase(),
            Addon {
                name,
                dir,
                toc,
                deps,
                load_first,
                on_demand,
            },
        );
    }
    let mut names: Vec<String> = all.keys().cloned().collect();
    names.sort();
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    fn visit(
        n: &str,
        all: &HashMap<String, Addon>,
        seen: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) {
        if !seen.insert(n.to_string()) {
            return;
        }
        if let Some(a) = all.get(n) {
            for d in &a.deps {
                visit(&d.to_ascii_lowercase(), all, seen, order);
            }
            order.push(n.to_string());
        }
    }
    for first in [true, false] {
        for n in &names {
            if all[n].load_first == first && !all[n].on_demand {
                visit(n, &all, &mut seen, &mut order);
            }
        }
    }
    let startup = order.iter().filter_map(|n| all.get(n).cloned()).collect();
    (startup, all)
}

/// A TOC line's file and whether it is marked `[Bootstrap]`: loaded at
/// startup for a load-on-demand addon (its `…_LoadUI` functions).
fn bootstrap(path: &str) -> (String, bool) {
    let trimmed = path.trim_end();
    match trimmed
        .to_ascii_lowercase()
        .strip_suffix("[bootstrap]")
        .map(str::len)
    {
        Some(len) => (trimmed[..len].trim_end().to_string(), true),
        None => (trimmed.to_string(), false),
    }
}

/// Loads an addon's files (all but its bootstrap ones, or only those),
/// then fires `ADDON_LOADED` unless only bootstrapping.
fn load_addon_files(vm: &mut Vm, root: &Path, t: &Target, addon: &Addon, only_bootstrap: bool) {
    let table = Value::Table(vm.new_table());
    for file in &addon.toc.files {
        if !t.allows(&file.conditions) {
            continue;
        }
        let (rel, marked) = bootstrap(&file.path);
        let is_bootstrap = marked
            || file
                .conditions
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case("Bootstrap"));
        if is_bootstrap != only_bootstrap {
            continue;
        }
        let path = resolve(&addon.dir, &t.path(&rel));
        load_file(vm, root, &path, &addon.name, table);
    }
    if only_bootstrap {
        return;
    }
    engine_mut(vm).world.ui.loaded.push(addon.name.clone());
    let name = vm.str(&addon.name);
    crate::widgets::fire_event(vm, "ADDON_LOADED", &[name, Value::Bool(false)]);
}

/// `C_AddOns.LoadAddOn(name)` for Blizzard's addons: their dependencies,
/// then their files. `Some(true)` once loaded (or if it was), `Some(false)`
/// for an addon the code does not have, `None` without the UI code.
pub fn load_addon(vm: &mut Vm, name: &str) -> Option<bool> {
    let root = engine(vm).world.ui.root.clone()?;
    let key = name.to_ascii_lowercase();
    if engine(vm)
        .world
        .ui
        .loaded
        .iter()
        .any(|l| l.eq_ignore_ascii_case(name))
    {
        return Some(true);
    }
    let Some(addon) = engine(vm).world.ui.available.get(&key).cloned() else {
        return Some(false);
    };
    for dep in &addon.deps {
        if engine(vm)
            .world
            .ui
            .available
            .contains_key(&dep.to_ascii_lowercase())
        {
            load_addon(vm, dep);
        }
    }
    let t = Target::new(&engine(vm).flavor.clone());
    load_addon_files(vm, &root, &t, &addon, false);
    Some(true)
}

/// Loads one UI file (`.lua` or `.xml`) of an addon.
fn load_file(vm: &mut Vm, interface_root: &Path, path: &Path, addon: &str, addon_table: Value) {
    let shown = path
        .strip_prefix(interface_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("lua") => build::run_lua(vm, path, &shown, addon, addon_table),
        Some("xml") => {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    engine_mut(vm).world.ui.errors.push(format!("{shown}: {e}"));
                    return;
                }
            };
            let root = match xml::parse(&text) {
                Ok(r) => r,
                Err(e) => {
                    engine_mut(vm).world.ui.errors.push(format!("{shown}: {e}"));
                    return;
                }
            };
            let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
            let cx = build::Context { file: &shown };
            process_ui(vm, interface_root, &dir, &cx, &root, addon, addon_table);
        }
        _ => {}
    }
}

/// A file a `<Script>` or `<Include>` names, relative to the XML file's
/// directory (any case, as on Windows).
fn resolve(dir: &Path, file: &str) -> PathBuf {
    let rel = wowlua_toc::split_conditions(file).0.replace('\\', "/");
    let direct = dir.join(&rel);
    if direct.exists() {
        return direct;
    }
    // Case-insensitive lookup, one component at a time.
    let mut cur = dir.to_path_buf();
    for part in rel.split('/') {
        let next = std::fs::read_dir(&cur).ok().and_then(|entries| {
            entries.flatten().map(|e| e.path()).find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(part))
            })
        });
        match next {
            Some(p) => cur = p,
            None => return direct,
        }
    }
    cur
}

/// A `<Ui>` element's children, in order.
fn process_ui(
    vm: &mut Vm,
    interface_root: &Path,
    dir: &Path,
    cx: &build::Context,
    root: &Rc<Element>,
    addon: &str,
    addon_table: Value,
) {
    for el in &root.children {
        match el.tag.to_ascii_lowercase().as_str() {
            "script" => {
                if let Some(file) = el.attr("file") {
                    let t = Target::new(&engine(vm).flavor);
                    if t.allows(&wowlua_toc::split_conditions(file).1) {
                        load_file(vm, interface_root, &resolve(dir, file), addon, addon_table);
                    }
                } else if !el.text.trim().is_empty() {
                    let src = format!("{}{}", "\n".repeat(el.line.saturating_sub(1)), el.text);
                    match vm.load(src.as_bytes(), &format!("@{}", cx.file)) {
                        Ok(f) => {
                            if let Err(e) = vm.call(f, &[]) {
                                let msg = match e.value {
                                    Value::Str(s) => {
                                        String::from_utf8_lossy(vm.str_bytes(s)).into_owned()
                                    }
                                    _ => "?".into(),
                                };
                                engine_mut(vm).world.ui.errors.push(msg);
                            }
                        }
                        Err(e) => engine_mut(vm).world.ui.errors.push(e),
                    }
                }
            }
            "include" => {
                if let Some(file) = el.attr("file") {
                    let t = Target::new(&engine(vm).flavor);
                    if t.allows(&wowlua_toc::split_conditions(file).1) {
                        load_file(vm, interface_root, &resolve(dir, file), addon, addon_table);
                    }
                }
            }
            "font" => font(vm, el),
            "fontfamily" => {
                // The family's first member's font stands for it.
                if let Some(name) = el.attr("name")
                    && let Some(f) = el.child("Member").and_then(|m| m.child("Font"))
                {
                    font_named(vm, name, f);
                }
            }
            "scopedmodifier" => process_ui(vm, interface_root, dir, cx, el, addon, addon_table),
            _ => {
                if (el.flag("virtual") || el.flag("intrinsic"))
                    && let Some(name) = el.attr("name")
                {
                    engine_mut(vm)
                        .world
                        .ui
                        .template_files
                        .insert(name.to_ascii_lowercase(), cx.file.to_string());
                }
                build::top_level(vm, cx, el);
            }
        }
    }
}

/// `<Font name=… inherits=… font=… height=… outline=…>`: a font object.
fn font(vm: &mut Vm, el: &Rc<Element>) {
    if let Some(name) = el.attr("name") {
        font_named(vm, name, el);
    }
}

fn font_named(vm: &mut Vm, name: &str, el: &Element) {
    let f = crate::widgets::create(vm, "Font", Some(name.to_string()), None);
    let call = |vm: &mut Vm, method: &str, args: &[Value]| {
        let key = vm.str(method);
        if let Ok(m) = vm.index(f, key, None)
            && !m.is_nil()
        {
            let mut a = vec![f];
            a.extend_from_slice(args);
            let _ = vm.call(m, &a);
        }
    };
    if let Some(parent) = el.attr("inherits") {
        let p = vm.get_global(parent);
        if !p.is_nil() {
            call(vm, "SetFontObject", &[p]);
        }
    }
    if let Some(file) = el.attr("font") {
        let height = el
            .number("height")
            .or_else(|| {
                el.child("FontHeight")
                    .and_then(|h| h.child("AbsValue"))
                    .and_then(|v| v.number("val"))
            })
            .unwrap_or(12.0);
        let outline = match el.attr("outline").map(str::to_ascii_uppercase).as_deref() {
            Some("NORMAL") => "OUTLINE",
            Some("THICK") => "THICKOUTLINE",
            _ => "",
        };
        let file = vm.str(file);
        let flags = vm.str(outline);
        call(vm, "SetFont", &[file, Value::Num(height), flags]);
    }
}

/// The client's global strings: the file's `NAME = "text";` lines (a line
/// whose name is not an identifier is skipped).
fn load_global_strings(vm: &mut Vm, path: &Path) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            engine_mut(vm)
                .world
                .ui
                .errors
                .push(format!("GlobalStrings.lua: {e}"));
            return;
        }
    };
    let ident = |s: &str| {
        let mut c = s.chars();
        c.next()
            .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
            && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    };
    let src: String = text
        .lines()
        .filter(|l| l.split_once(" = ").is_some_and(|(name, _)| ident(name)))
        .flat_map(|l| [l, "\n"])
        .collect();
    match vm.load(src.as_bytes(), "@GlobalStrings.lua") {
        Ok(f) => {
            if let Err(e) = vm.call(f, &[]) {
                let msg = match e.value {
                    Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                    _ => "?".into(),
                };
                engine_mut(vm).world.ui.errors.push(msg);
            }
        }
        Err(e) => engine_mut(vm).world.ui.errors.push(e),
    }
}

/// Loads the flavor's Blizzard addons from `interface` (a folder holding
/// `AddOns/Blizzard_*`), as secure code, after the client's global strings
/// (`GlobalStrings.lua`: `NAME = "text"` lines) if given; `ADDON_LOADED`
/// fires for each addon.
pub fn load(vm: &mut Vm, interface: &Path, global_strings: Option<&Path>) -> Report {
    let flavor = engine(vm).flavor.clone();
    let t = Target::new(&flavor);
    let root = interface.parent().unwrap_or(interface).to_path_buf();
    // Errors of the code's handlers while it loads are the report's too.
    let saved_handler = vm.error_handler;
    let recorder = vm.native("errorhandler", |vm, c| {
        let msg = match c.arg(1) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => "UNKNOWN ERROR".into(),
        };
        engine_mut(vm).world.ui.errors.push(msg);
        Ok(vec![])
    });
    vm.error_handler = recorder;
    // The VM's stand-in for what RestrictedExecution.lua does to the string
    // metatable (`__metatable = string`) gives way to the real code.
    if let Some(meta) = vm.string_meta {
        let k = vm.heap.intern(b"__metatable");
        vm.heap.table_mut(meta).set_str(k, Value::Nil);
    }
    if let Some(path) = global_strings {
        load_global_strings(vm, path);
    }
    // What only Blizzard's code sees; its EnvironmentCleanup.lua removes it.
    for s in engine(vm).secure_tables.clone() {
        vm.heap.table_mut(s.root).set_str(s.name, s.table);
    }
    let (startup, all) = startup_addons(interface, &t);
    {
        let ui = &mut engine_mut(vm).world.ui;
        ui.available = all.clone();
        ui.root = Some(root.clone());
    }
    for addon in &startup {
        load_addon_files(vm, &root, &t, addon, false);
    }
    // The load-on-demand addons' bootstrap files (order unverified: after
    // the startup addons, by name).
    let mut rest: Vec<&Addon> = all
        .values()
        .filter(|a| !startup.iter().any(|s| s.name == a.name))
        .collect();
    rest.sort_by(|a, b| a.name.cmp(&b.name));
    for addon in rest {
        load_addon_files(vm, &root, &t, addon, true);
    }
    // Blizzard's code may have set its own handler meanwhile.
    if let (Value::Func(a), Value::Func(b)) = (vm.error_handler, recorder)
        && a == b
    {
        vm.error_handler = saved_handler;
    }
    let ui = &engine(vm).world.ui;
    Report {
        addons: ui.loaded.clone(),
        errors: ui.errors.clone(),
    }
}
