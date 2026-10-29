//! Where the engine's answer to each function and widget method comes
//! from, for the coverage report (spec emulator §17.8, `cargo xtask
//! coverage-report`). Read from the tables [`crate::install`] builds the
//! engine from, in the order it lets them take each other's place, so the
//! report follows the code.

use std::collections::{BTreeSet, HashMap, HashSet};

use wowlua_vm::{Value, Vm};

use crate::{CAMELOT_ABSENT, OVER_RECORDED, api, extras, facts, protection, widgets};

/// Where an answer comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// A model or rule of an engine module, or the VM's runtime.
    Model,
    /// The client's recorded answers.
    Recorded,
    /// A fill-in module's answer, from the documentation, Blizzard's UI
    /// code and the function's name (`fill_ins`; unverified).
    FillIn,
    /// The documentation's generated stand-in alone: "not mocked" for an
    /// addon, the documented types' empty values for Blizzard's code.
    Generated,
}

/// Which list a function comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// The flavor's API documentation.
    Documented,
    /// The client's C functions the documentation lacks (its inventory,
    /// `data/<flavor>-functions.txt`).
    Undocumented,
    /// Functions the engine gives behaviour that neither the documentation
    /// nor the client's inventory lists.
    EngineOnly,
}

/// A function or widget method and where its answer comes from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// `C_Timer.After`, `GetTime`; a method's class and name
    /// (`Frame:Show`).
    pub name: String,
    pub group: Group,
    pub source: Source,
    /// The module that answers (`clock`, `vm`), empty for a generated one.
    pub module: &'static str,
}

/// Whether the VM's runtime defines `name` before the engine is installed
/// (`issecretvalue`, `strsplit`): the engine leaves it.
fn defined(vm: &mut Vm, name: &str) -> bool {
    let (table, short) = match name.split_once('.') {
        None => (vm.globals, name),
        Some((ns, short)) => match vm.get_global(ns) {
            Value::Table(t) => (t, short),
            _ => return false,
        },
    };
    let key = vm.heap.intern(short.as_bytes());
    !vm.heap.table(table).get_str(key).is_nil()
}

/// Every global and namespaced function of a flavor: the documented ones,
/// the client's undocumented ones and the engine's own, by name.
pub fn functions(flavor: &str) -> Result<Vec<Entry>, String> {
    let api = api::api(flavor).ok_or_else(|| format!("no API documentation for `{flavor}`"))?;
    let mut vm = Vm::wow();
    let mut fill: HashMap<&'static str, &'static str> = HashMap::new();
    for m in crate::fill_ins() {
        for (name, _) in m.list {
            fill.insert(name, m.module);
        }
    }
    let mut own: HashMap<&'static str, (&'static str, bool)> = HashMap::new();
    for m in crate::own_behaviours(flavor) {
        for (name, _) in m.list {
            if m.recorded && OVER_RECORDED.contains(&name) {
                continue;
            }
            own.insert(name, (m.module, m.recorded));
        }
    }
    let recorded: HashSet<&'static str> = facts::recorded_names(flavor).into_iter().collect();
    // The behaviours as `install` lets them take each other's place: the
    // engine's own over the recorded facts over the fill-ins, but for the
    // fill-ins a setter changes.
    let behaviour = |name: &str| -> Option<(Source, &'static str)> {
        if let Some(&(module, rec)) = own.get(name) {
            let source = if rec { Source::Recorded } else { Source::Model };
            return Some((source, module));
        }
        if let Some(&module) = fill.get(name) {
            if !OVER_RECORDED.contains(&name) && recorded.contains(name) {
                return Some((Source::Recorded, "facts"));
            }
            return Some((Source::FillIn, module));
        }
        recorded
            .contains(name)
            .then_some((Source::Recorded, "facts"))
    };
    let protected =
        |name: &str| protection::PROTECTED.contains(&name) || protection::HARDWARE.contains(&name);
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for f in &api.functions {
        if !seen.insert(f.name) {
            continue;
        }
        let (source, module) = if defined(&mut vm, f.name) {
            (Source::Model, "vm")
        } else if extras::silent_unchecked(&f.doc) || extras::silent_checked(&f.doc) {
            // Answered nothing by the documentation's flags, before any
            // behaviour (the full probe).
            (Source::Model, "extras")
        } else {
            behaviour(f.name).unwrap_or((Source::Generated, ""))
        };
        out.push(Entry {
            name: f.name.to_string(),
            group: Group::Documented,
            source,
            module,
        });
    }
    let inventory: BTreeSet<&'static str> = crate::inventory(flavor)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    for &name in &inventory {
        if !seen.insert(name) {
            continue;
        }
        let (source, module) = if defined(&mut vm, name) {
            (Source::Model, "vm")
        } else if name == "CreateFrame" {
            (Source::Model, "widgets")
        } else if let Some(found) = behaviour(name) {
            found
        } else if protected(name) {
            (Source::Model, "protection")
        } else {
            (Source::Generated, "")
        };
        out.push(Entry {
            name: name.to_string(),
            group: Group::Undocumented,
            source,
            module,
        });
    }
    let mut extra: BTreeSet<&'static str> = own.keys().copied().collect();
    extra.extend(fill.keys().copied());
    extra.extend(recorded.iter().copied());
    extra.extend(protection::PROTECTED.iter().copied());
    extra.extend(protection::HARDWARE.iter().filter(|n| !n.contains('.')));
    for name in extra {
        if seen.contains(name) || defined(&mut vm, name) {
            continue;
        }
        // Not defined in camelot's environment (`install`).
        if flavor == "camelot" && CAMELOT_ABSENT.contains(&name) {
            continue;
        }
        seen.insert(name);
        let (source, module) = behaviour(name).unwrap_or((Source::Model, "protection"));
        out.push(Entry {
            name: name.to_string(),
            group: Group::EngineOnly,
            source,
            module,
        });
    }
    out.sort();
    Ok(out)
}

/// Every documented widget method of a flavor (`Frame:Show`), by class.
/// Whether a fresh widget of a class deriving from `class` (the
/// documentation's `Region`, `TextureBase`) has `getter` recorded: the
/// widgets made answer it so.
fn recorded_below(e: &crate::Engine, class: &str, getter: &str) -> bool {
    crate::widgetdata::data(&e.flavor).is_some_and(|data| {
        data.defaults
            .keys()
            .any(|(c, g)| g == getter && widgets::is_a(e, c, class))
    })
}

pub fn methods(flavor: &str) -> Result<Vec<Entry>, String> {
    let mut vm = Vm::wow();
    crate::install(&mut vm, flavor)?;
    let e = crate::engine(&vm);
    let mut modules: HashMap<&'static str, (&'static str, bool)> = HashMap::new();
    for m in crate::method_behaviours() {
        let fill_in = matches!(m.module, "methods2" | "methods3");
        for (name, _) in m.list {
            modules.insert(name, (m.module, fill_in));
        }
    }
    let mut out = Vec::new();
    for f in &e.api.methods {
        let class = f.namespace.unwrap_or("");
        let returns = f
            .doc
            .get("Returns")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        // As `widgets::method` answers: a behaviour (by the full name,
        // then the name), the stored setter, a fresh widget's recorded
        // getter, the stored flag, nothing.
        let (source, module) = match modules.get(f.name).or_else(|| modules.get(f.short)) {
            Some(&(module, true)) => (Source::FillIn, module),
            Some(&(module, false)) => (Source::Model, module),
            None if crate::widgetops::EMPTY_METHODS.contains(&f.name) => {
                (Source::FillIn, "widgetops")
            }
            None if f.short.starts_with("Set") || f.short.starts_with("Enable") => {
                (Source::Model, "widgets")
            }
            None if widgets::recorded_default(e, class, f.short).is_some()
                || recorded_below(e, class, f.short) =>
            {
                (Source::Recorded, "widgetdata")
            }
            None if f.short.starts_with("Is") && returns == 1 => (Source::Model, "widgets"),
            None => (Source::Generated, ""),
        };
        out.push(Entry {
            name: f.name.to_string(),
            group: Group::Documented,
            source,
            module,
        });
    }
    out.sort();
    out.dedup_by(|a, b| a.name == b.name);
    Ok(out)
}

/// An event and whether the engine fires it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    /// `BAG_UPDATE`.
    pub name: String,
    /// `Documented`: the documentation describes it (its payload);
    /// `Undocumented`: the client knows it, the documentation does not
    /// describe it.
    pub group: Group,
    /// The engine fires it itself, as the world it models changes (a
    /// test's `mock` call, the login, a reply): else only a test's
    /// `mock.fireEvent` does.
    pub fired: bool,
    /// The modules that fire it (`inventory`, `login`), empty when none
    /// does.
    pub modules: Vec<&'static str>,
}

/// The events the engine fires and the modules that do: generated from
/// the engine's code and the login's recordings, kept current by the test
/// `coverage::tests::the_fired_events_follow_the_code`.
pub const FIRED_EVENTS: &str = include_str!("../data/fired-events.txt");

/// The events [`FIRED_EVENTS`] lists, with their modules.
pub fn fired_events() -> Vec<(&'static str, Vec<&'static str>)> {
    FIRED_EVENTS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let (name, modules) = l.split_once('\t')?;
            Some((name, modules.split(',').collect()))
        })
        .collect()
}

/// Every event of a flavor, documented or known to the client, by name:
/// whether the engine fires it or a test must (`mock.fireEvent`).
pub fn events(flavor: &str) -> Result<Vec<Event>, String> {
    let api = api::api(flavor).ok_or_else(|| format!("no API documentation for `{flavor}`"))?;
    let fired: HashMap<&'static str, Vec<&'static str>> = fired_events().into_iter().collect();
    let mut names: BTreeSet<&str> = api.events.iter().map(String::as_str).collect();
    names.extend(api.event_docs.keys().map(String::as_str));
    Ok(names
        .into_iter()
        .map(|name| {
            let modules = fired.get(name).cloned().unwrap_or_default();
            Event {
                name: name.to_string(),
                group: if api.event_docs.contains_key(name) {
                    Group::Documented
                } else {
                    Group::Undocumented
                },
                fired: !modules.is_empty(),
                modules,
            }
        })
        .collect())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // (the test reads the engine's code from the disk)
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// The modules that name events without firing them: the
    /// documentation's reader, this one.
    const SILENT_MODULES: &[&str] = &["api", "coverage"];

    /// Lists of events not fired where they are listed: the events batched
    /// to a frame's end (the modules that owe them fire them), the callback
    /// events tainted code may not register.
    const SILENT_LISTS: &[&str] = &["BATCHED", "RESTRICTED_CALLBACK_EVENTS"];

    /// Events whose names the code puts together: the module, the code, the
    /// events it makes.
    const CONSTRUCTED: &[(&str, &str, &[&str])] = &[(
        "chat",
        "format!(\"CHAT_MSG_{chat_type}\")",
        &[
            "CHAT_MSG_SAY",
            "CHAT_MSG_YELL",
            "CHAT_MSG_EMOTE",
            "CHAT_MSG_PARTY",
            "CHAT_MSG_INSTANCE_CHAT",
            "CHAT_MSG_GUILD",
            "CHAT_MSG_OFFICER",
        ],
    )];

    /// The recordings `login` replays, event by event: the login, the
    /// /reload, the loading screen of a world transfer.
    const REPLAYED: &[&str] = &[
        "camelot-login.txt",
        "camelot-reload.txt",
        "camelot-teleport.txt",
    ];

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("the engine's sources")
            .map(|e| e.expect("an entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Rust code with its comments blanked, and its string literals: where
    /// each starts and ends (its quotes) and its text.
    fn lex(src: &str) -> (String, Vec<(usize, usize, &str)>) {
        let b = src.as_bytes();
        let mut code = b.to_vec();
        let mut literals = Vec::new();
        let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        let mut i = 0;
        while i < b.len() {
            if b[i..].starts_with(b"//") {
                while i < b.len() && b[i] != b'\n' {
                    code[i] = b' ';
                    i += 1;
                }
            } else if b[i..].starts_with(b"/*") {
                let mut depth = 0;
                while i < b.len() {
                    if b[i..].starts_with(b"/*") {
                        depth += 1;
                        code[i..i + 2].fill(b' ');
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        depth -= 1;
                        code[i..i + 2].fill(b' ');
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if b[i] != b'\n' {
                            code[i] = b' ';
                        }
                        i += 1;
                    }
                }
            } else if b[i] == b'r'
                && (i == 0
                    || !ident(b[i - 1])
                    || (b[i - 1] == b'b' && (i == 1 || !ident(b[i - 2]))))
                && b[i + 1..]
                    .iter()
                    .position(|&c| c != b'#')
                    .is_some_and(|n| b[i + 1 + n] == b'"')
            {
                let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
                let open = i + 1 + hashes;
                let close: Vec<u8> = std::iter::once(b'"')
                    .chain(std::iter::repeat_n(b'#', hashes))
                    .collect();
                let len = b[open + 1..]
                    .windows(close.len())
                    .position(|w| w == close.as_slice())
                    .expect("a raw string's end");
                literals.push((open, open + 1 + len + 1, &src[open + 1..open + 1 + len]));
                i = open + 1 + len + close.len();
            } else if b[i] == b'"' {
                let mut j = i + 1;
                while b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                literals.push((i, j + 1, &src[i + 1..j]));
                i = j + 1;
            } else if b[i] == b'\'' {
                // A character (`'"'`, `'\''`) or a lifetime.
                if b.get(i + 1) == Some(&b'\\') {
                    i += 2 + b[i + 2..].iter().position(|&c| c == b'\'').unwrap_or(0) + 1;
                } else {
                    let len = src[i + 1..].chars().next().map_or(1, char::len_utf8);
                    i += if b.get(i + 1 + len) == Some(&b'\'') {
                        len + 2
                    } else {
                        1
                    };
                }
            } else {
                i += 1;
            }
        }
        (String::from_utf8(code).expect("UTF-8"), literals)
    }

    /// Every event any flavor knows.
    fn known_events() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for flavor in &wowlua_flavors::table().flavors {
            if let Some(api) = api::api(&flavor.id) {
                out.extend(api.events.iter().cloned());
                out.extend(api.event_docs.keys().cloned());
            }
        }
        out
    }

    /// The events the engine fires, by module, read from its code: every
    /// event's name the code writes as a string, but in a comparison or a
    /// pattern (`event == "X"`, `"X" =>`, `matches!`), in the silent
    /// modules and lists, and in the tests; the events the code puts
    /// together; the events of the recordings `login` replays.
    fn fired_in_code(root: &Path) -> BTreeMap<String, BTreeSet<String>> {
        let known = known_events();
        let src = root.join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut lists = BTreeSet::new();
        let mut sources = BTreeMap::new();
        for file in files {
            let rel = file.strip_prefix(&src).expect("below src");
            // Path components, so Windows' backslashes name modules as `/` does.
            let module = rel
                .with_extension("")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("::");
            let module = module.strip_suffix("::mod").unwrap_or(&module).to_string();
            let text = std::fs::read_to_string(&file).expect("a source");
            sources.insert(module.clone(), text.clone());
            if SILENT_MODULES.contains(&module.as_str()) {
                continue;
            }
            // The tests at the module's end.
            let text = match text.find("\n#[cfg(test)]") {
                Some(at) => &text[..at],
                None => &text[..],
            };
            let (code, literals) = lex(text);
            // The silent lists' spans, from their names to their ends.
            let mut silent = Vec::new();
            for name in SILENT_LISTS {
                for pattern in [format!("const {name}:"), format!("static {name}:")] {
                    if let Some(at) = code.find(&pattern) {
                        let end = code[at..].find("];").map_or(code.len(), |e| at + e);
                        silent.push(at..end);
                        lists.insert(*name);
                    }
                }
            }
            for (start, end, literal) in literals {
                if !known.contains(literal) || silent.iter().any(|s| s.contains(&start)) {
                    continue;
                }
                let line_start = code[..start].rfind('\n').map_or(0, |i| i + 1);
                let line_end = code[end..].find('\n').map_or(code.len(), |i| end + i);
                let before = code[line_start..start].trim_end();
                let after = code[end..line_end].trim_start();
                let pattern = before.ends_with("==")
                    || before.ends_with("!=")
                    || (before.ends_with('|') && !before.ends_with("||"))
                    || after.starts_with("=>")
                    || after.starts_with("if ")
                    || (after.starts_with('|') && !after.starts_with("||"))
                    || code[line_start..line_end].contains("matches!(");
                if !pattern {
                    out.entry(literal.to_string())
                        .or_default()
                        .insert(module.clone());
                }
            }
        }
        for name in SILENT_LISTS {
            assert!(lists.contains(name), "no list `{name}` in the engine");
        }
        for (module, code, events) in CONSTRUCTED {
            assert!(
                sources[*module].contains(code),
                "`{module}` has no `{code}`: CONSTRUCTED is stale"
            );
            for e in *events {
                assert!(known.contains(*e), "{e} is no event");
                out.entry(e.to_string())
                    .or_default()
                    .insert(module.to_string());
            }
        }
        for file in REPLAYED {
            assert!(
                sources["login"].contains(&format!("include_str!(\"../data/{file}\")")),
                "`login` replays no {file}: REPLAYED is stale"
            );
            let text = std::fs::read_to_string(root.join("data").join(file)).expect("a recording");
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                if let Some(event) = line.split('\t').nth(1)
                    && known.contains(event)
                {
                    out.entry(event.to_string())
                        .or_default()
                        .insert("login".to_string());
                }
            }
        }
        out
    }

    #[test]
    fn the_fired_events_follow_the_code() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut text = String::from(
            "# The events the engine fires itself (`coverage::events`), and the modules\n\
             # that fire them: every event the engine's code names but to compare it, and\n\
             # the events of the recordings the login replays. A test fires the others\n\
             # with `mock.fireEvent`. Generated from the code by the test\n\
             # `coverage::tests::the_fired_events_follow_the_code`\n\
             # (`WOWLUA_FIRED_EVENTS_WRITE=1 cargo test -p wowlua-engine fired_events`\n\
             # writes it). Do not edit.\n",
        );
        for (event, modules) in fired_in_code(root) {
            let modules: Vec<&str> = modules.iter().map(String::as_str).collect();
            text.push_str(&format!("{event}\t{}\n", modules.join(",")));
        }
        if std::env::var_os("WOWLUA_FIRED_EVENTS_WRITE").is_some() {
            std::fs::write(root.join("data/fired-events.txt"), &text).expect("writes");
            return;
        }
        // (a checkout may end its lines with CRLF)
        if text != FIRED_EVENTS.replace("\r\n", "\n") {
            let want: BTreeSet<&str> = text.lines().collect();
            let have: BTreeSet<&str> = FIRED_EVENTS.lines().collect();
            let diff: Vec<String> = want
                .difference(&have)
                .map(|l| format!("+ {l}"))
                .chain(have.difference(&want).map(|l| format!("- {l}")))
                .collect();
            panic!(
                "data/fired-events.txt is out of date: run `WOWLUA_FIRED_EVENTS_WRITE=1 cargo \
                 test -p wowlua-engine fired_events`\n{}",
                diff.join("\n")
            );
        }
    }

    #[test]
    fn events_tell_the_fired_from_the_mocked() {
        let events = events("camelot").expect("camelot");
        assert!(events.len() > 1500, "{}", events.len());
        assert!(events.windows(2).all(|w| w[0].name < w[1].name));
        let event = |name: &str| events.iter().find(|e| e.name == name).expect(name);
        // A bought item's bag tells of it; the login fires its own.
        let bag = event("BAG_UPDATE");
        assert!(bag.fired && bag.group == Group::Documented);
        assert!(bag.modules.contains(&"merchant"), "{:?}", bag.modules);
        assert_eq!(event("PLAYER_LOGIN").modules, ["login"]);
        // A callback event the documentation describes and the client's
        // `RegisterEvent` refuses (`api`) is listed, and fired.
        assert!(event("MINIMAP_PING").fired);
        // A chat channel's message, put together by `chat`.
        assert!(event("CHAT_MSG_SAY").modules.contains(&"chat"));
        // The combat log: only `mock.fireEvent`.
        let log = event("COMBAT_LOG_EVENT_UNFILTERED");
        assert!(!log.fired && log.modules.is_empty());
        // An event the client knows without documentation.
        assert!(events.iter().any(|e| e.group == Group::Undocumented));
        let fired = events.iter().filter(|e| e.fired).count();
        assert!(fired > 300 && fired < events.len() / 2, "{fired}");
    }
}
