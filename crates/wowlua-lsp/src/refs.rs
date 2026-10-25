//! Find references and rename (spec §16.2, decision T389): from the name at
//! a place, what it refers to and every name of the checked workspace that
//! refers to the same, from a whole check's definitions (go to definition's
//! pairs, read backwards).

use crate::check::{Roots, absolute, materialize};
use crate::text;
use serde_json::{Value as Json, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use wowlua_driver::Session;
use wowlua_span::{FileId, Span};

/// What a references or rename check looks for: the name at byte `offset`
/// of `path`, and for a rename, its new name.
#[derive(Clone, Debug, PartialEq)]
pub struct Find {
    pub path: PathBuf,
    pub offset: u32,
    pub rename: Option<String>,
}

impl Find {
    pub fn to_json(&self) -> Json {
        json!({ "path": self.path, "offset": self.offset, "rename": self.rename })
    }

    pub fn from_json(json: &Json) -> Option<Find> {
        Some(Find {
            path: PathBuf::from(json["path"].as_str()?),
            offset: json["offset"].as_u64()? as u32,
            rename: json["rename"].as_str().map(str::to_string),
        })
    }
}

/// A name in a file: the file (canonical, or the copy of a file built into
/// wowlua), its bytes, and its lines and UTF-16 columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub path: PathBuf,
    pub start: u32,
    pub end: u32,
    pub range: [u32; 4],
}

impl Place {
    fn to_json(&self) -> Json {
        let [l0, c0, l1, c1] = self.range;
        json!([self.path, self.start, self.end, l0, c0, l1, c1])
    }

    fn from_json(json: &Json) -> Option<Place> {
        let n = |i: usize| json.get(i).and_then(Json::as_u64).map(|v| v as u32);
        Some(Place {
            path: PathBuf::from(json.get(0)?.as_str()?),
            start: n(1)?,
            end: n(2)?,
            range: [n(3)?, n(4)?, n(5)?, n(6)?],
        })
    }

    /// The protocol's range.
    pub fn lsp_range(&self) -> Json {
        let [l0, c0, l1, c1] = self.range;
        json!({
            "start": { "line": l0, "character": c0 },
            "end": { "line": l1, "character": c1 },
        })
    }
}

/// What a references or rename check found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// Where what the name refers to is declared.
    pub declarations: Vec<Place>,
    /// The names that refer to it (its declarations left out).
    pub references: Vec<Place>,
    /// The names a rename changes: the declarations and references spelled
    /// as the name (an import's alias: that file's names spelled as it).
    pub renamed: Vec<Place>,
    /// A rename check's answer: the places to give the new name, or why
    /// there are none.
    pub rename: Option<Result<Vec<Place>, String>>,
}

impl Found {
    pub fn to_json(&self) -> Json {
        let places = |list: &[Place]| list.iter().map(Place::to_json).collect::<Vec<_>>();
        json!({
            "declarations": places(&self.declarations),
            "references": places(&self.references),
            "renamed": places(&self.renamed),
            "rename": match &self.rename {
                None => Json::Null,
                Some(Ok(list)) => json!({ "edits": places(list) }),
                Some(Err(why)) => json!({ "refused": why }),
            },
        })
    }

    pub fn from_json(json: &Json) -> Option<Found> {
        let places = |v: &Json| -> Option<Vec<Place>> {
            v.as_array()?.iter().map(Place::from_json).collect()
        };
        let rename = match &json["rename"] {
            Json::Null => None,
            r => match r["refused"].as_str() {
                Some(why) => Some(Err(why.to_string())),
                None => Some(Ok(places(&r["edits"])?)),
            },
        };
        Some(Found {
            declarations: places(&json["declarations"])?,
            references: places(&json["references"])?,
            renamed: places(&json["renamed"])?,
            rename,
        })
    }
}

/// Lua's keywords: never a name.
pub const RESERVED: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Why `name` cannot be a new name, if it cannot.
pub fn invalid_name(name: &str) -> Option<String> {
    let identifier = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !identifier {
        return Some(format!(
            "`{name}` is not a name (letters, digits and `_`, not starting with a digit)"
        ));
    }
    if RESERVED.contains(&name) {
        return Some(format!("`{name}` is a keyword"));
    }
    None
}

/// The files the definitions name, each once (a dependency's file is in
/// the session once for each package that builds it).
struct Files<'s> {
    session: &'s Session,
    roots: &'s Roots,
    ids: HashMap<FileId, usize>,
    by_path: HashMap<PathBuf, usize>,
    list: Vec<File>,
}

struct File {
    /// Canonical on disk; else the session's name of a built-in file.
    path: PathBuf,
    disk: bool,
    id: FileId,
}

impl<'s> Files<'s> {
    fn key(&mut self, id: FileId) -> usize {
        if let Some(&k) = self.ids.get(&id) {
            return k;
        }
        let file = self.session.db.get(id);
        let (path, disk) = match absolute(self.roots, file.path()) {
            Some(p) => (p, true),
            None => (file.path().to_path_buf(), false),
        };
        let k = match self.by_path.get(&path) {
            Some(&k) => k,
            None => {
                self.list.push(File {
                    path: path.clone(),
                    disk,
                    id,
                });
                self.by_path.insert(path, self.list.len() - 1);
                self.list.len() - 1
            }
        };
        self.ids.insert(id, k);
        k
    }

    fn text(&self, k: usize) -> &'s str {
        self.session.db.get(self.list[k].id).text()
    }

    fn slice(&self, k: usize, start: u32, end: u32) -> &'s str {
        self.text(k).get(start as usize..end as usize).unwrap_or("")
    }
}

/// A name in one of [`Files`]: its file's key and bytes.
type Key = (usize, u32, u32);

/// What `find` asks of the names of a whole check: `defs` are its (name,
/// what it refers to) spans, `roots` where its files are.
pub(crate) fn find(session: &Session, roots: &Roots, defs: &[(Span, Span)], find: &Find) -> Found {
    let mut files = Files {
        session,
        roots,
        ids: HashMap::new(),
        by_path: HashMap::new(),
        list: Vec::new(),
    };
    let rows: Vec<(Key, Key)> = defs
        .iter()
        .map(|(at, to)| {
            (
                (files.key(at.file), at.start, at.end),
                (files.key(to.file), to.start, to.end),
            )
        })
        .collect();
    let Some(&cursor) = files.by_path.get(&text::canonical(&find.path)) else {
        return Found::default();
    };
    let off = find.offset;
    // the innermost name at the cursor (on it, else just after it)
    let around = |(k, s, e): Key| k == cursor && s <= off && off <= e;
    let rank = |(_, s, e): Key| (off == e, e - s);
    let used = rows
        .iter()
        .filter(|(at, _)| around(*at))
        .map(|(at, _)| *at)
        .min_by_key(|at| rank(*at));
    // a name that refers to something; else a declaration others refer to
    let (at, targets): (Key, BTreeSet<Key>) = match used {
        Some(at) => (
            at,
            rows.iter()
                .filter(|(a, _)| *a == at)
                .map(|(_, t)| *t)
                .collect(),
        ),
        None => {
            let Some(to) = rows
                .iter()
                .filter(|(_, to)| around(*to))
                .map(|(_, to)| *to)
                .min_by_key(|to| rank(*to))
            else {
                return Found::default();
            };
            (to, BTreeSet::from([to]))
        }
    };
    let name = files.slice(at.0, at.1, at.2);
    let declared = targets
        .iter()
        .map(|&(k, s, e)| files.slice(k, s, e))
        .find(|t| !t.is_empty())
        .unwrap_or("");
    // the name is an import's alias (`import a as b`, `import * as m`):
    // the names spelled as it in its file
    let alias = name != declared;
    let mut refs: Vec<Key> = rows
        .iter()
        .filter(|(_, to)| targets.contains(to))
        .map(|(at, _)| *at)
        .filter(|at| !targets.contains(at))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    // what is spelled otherwise: an import's alias and the names spelled as
    // it in its file; else a member's use that leads to its class (one
    // declared without a place of its own), not a use of the class
    let mut aliases: HashSet<(usize, &str)> = HashSet::new();
    for &(k, s, e) in &refs {
        let t = files.slice(k, s, e);
        if t != declared && after_as(files.text(k), s as usize) {
            aliases.insert((k, t));
        }
    }
    refs.retain(|&(k, s, e)| {
        let t = files.slice(k, s, e);
        t == declared || aliases.contains(&(k, t))
    });
    // spelled otherwise and no alias: a member that leads to its class
    if alias && !aliases.contains(&(cursor, name)) {
        return Found::default();
    }
    let renamed: Vec<Key> = if alias {
        refs.iter()
            .copied()
            .filter(|&(k, s, e)| k == cursor && files.slice(k, s, e) == name)
            .collect()
    } else {
        targets
            .iter()
            .copied()
            .chain(refs.iter().copied())
            .filter(|&(k, s, e)| s < e && files.slice(k, s, e) == name)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    let refused = find
        .rename
        .as_deref()
        .map(|new| refusal(&files, &rows, &targets, name, alias, new));
    // the places, with their lines: built-in files written where the
    // editor can open them
    let mut lines: HashMap<usize, (Option<PathBuf>, text::Lines<'_>)> = HashMap::new();
    let mut place = |(k, s, e): Key| -> Option<Place> {
        let (path, lines) = lines.entry(k).or_insert_with(|| {
            let f = &files.list[k];
            let path = if f.disk {
                Some(f.path.clone())
            } else {
                materialize(&f.path, files.text(k))
            };
            (path, text::Lines::new(files.text(k)))
        });
        let (l0, c0) = lines.line_column(s as usize);
        let (l1, c1) = lines.line_column(e as usize);
        Some(Place {
            path: path.clone()?,
            start: s,
            end: e,
            range: [l0, c0, l1, c1],
        })
    };
    let declarations = targets
        .iter()
        .filter(|(_, s, e)| s < e)
        .filter_map(|&t| place(t))
        .collect();
    let references = refs.iter().filter_map(|&r| place(r)).collect();
    let renamed: Vec<Place> = renamed.iter().filter_map(|&r| place(r)).collect();
    let rename = refused.map(|r| r.map(|()| renamed.clone()));
    Found {
        declarations,
        references,
        renamed,
        rename,
    }
}

/// Whether the name at byte `at` follows `as` (an import's alias).
fn after_as(text: &str, at: usize) -> bool {
    let before = text.get(..at).unwrap_or("").trim_end();
    before.ends_with(" as") || before.ends_with("\tas")
}

/// Why the name `name`, which refers to `targets`, cannot be renamed to
/// `new` from what the check found (a check of the renamed text follows).
fn refusal(
    files: &Files<'_>,
    rows: &[(Key, Key)],
    targets: &BTreeSet<Key>,
    name: &str,
    alias: bool,
    new: &str,
) -> Result<(), String> {
    if let Some(why) = invalid_name(new) {
        return Err(why);
    }
    if matches!(name, "self" | "super") {
        return Err(format!(
            "`{name}` is part of the language and can't be renamed"
        ));
    }
    if alias {
        return Ok(());
    }
    // the files the check found names in: the workspace's own
    let indexed: HashSet<usize> = rows.iter().map(|((k, ..), _)| *k).collect();
    for &(k, s, e) in targets {
        let f = &files.list[k];
        if !f.disk {
            return Err(format!(
                "`{name}` is declared by wowlua (the game's API, Lua's library or the standard library) and can't be renamed"
            ));
        }
        if s == e {
            return Err(format!("`{name}` names a module; rename its file instead"));
        }
        if !indexed.contains(&k) {
            return Err(format!(
                "`{name}` is declared in {}, which isn't checked as part of this workspace (a dependency?), so its uses can't all be found",
                f.path.display()
            ));
        }
    }
    // files no check reads names in: an XML file's attributes and scripts,
    // a Lua file that isn't type-checked
    for (id, file) in files.session.db.files() {
        let ext = file.path().extension().and_then(|e| e.to_str());
        if !matches!(ext, Some("lua" | "xml")) {
            continue;
        }
        let Some(path) = absolute(files.roots, file.path()) else {
            continue;
        };
        if files.ids.get(&id).is_some_and(|k| indexed.contains(k)) {
            continue;
        }
        let haystack = if ext == Some("xml") {
            xml_values(file.text())
        } else {
            file.text().to_string()
        };
        if has_word(&haystack, name) {
            return Err(format!(
                "`{name}` also appears in {}, which wowlua doesn't check, so it can't be renamed there",
                shown(files.roots, &path)
            ));
        }
    }
    Ok(())
}

/// `path` below the first root that has it, else whole.
fn shown(roots: &[PathBuf], path: &Path) -> String {
    roots
        .iter()
        .find_map(|r| path.strip_prefix(text::canonical(r)).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// An XML file's attribute values and text, where it names what code
/// declares (`mixin="…"`, `method="…"`, a script handler's body); not its
/// element and attribute names.
fn xml_values(xml: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut quote: Option<char> = None;
    for c in xml.chars() {
        match (in_tag, quote, c) {
            (true, Some(q), c) if c == q => {
                quote = None;
                out.push(' ');
            }
            (true, Some(_), c) => out.push(c),
            (true, None, '"' | '\'') => quote = Some(c),
            (true, None, '>') => in_tag = false,
            (true, None, _) => {}
            (false, _, '<') => {
                in_tag = true;
                out.push(' ');
            }
            (false, _, c) => out.push(c),
        }
    }
    out
}

/// Whether `word` stands in `text` as a whole name.
fn has_word(text: &str, word: &str) -> bool {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(is_name) && !after.is_some_and(is_name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_names_are_names_and_not_keywords() {
        assert_eq!(invalid_name("count"), None);
        assert_eq!(invalid_name("_x2"), None);
        assert!(invalid_name("2x").is_some());
        assert!(invalid_name("a-b").is_some());
        assert!(invalid_name("").is_some());
        assert!(invalid_name("end").unwrap().contains("keyword"));
    }

    #[test]
    fn xml_names_are_its_values_and_text() {
        let xml = r#"<Frame name="Foo" mixin="BarMixin"><Scripts><OnLoad>Baz(self)</OnLoad></Scripts></Frame>"#;
        let values = xml_values(xml);
        assert!(has_word(&values, "BarMixin"));
        assert!(has_word(&values, "Baz"));
        assert!(has_word(&values, "Foo"));
        // element and attribute names are not
        assert!(!has_word(&values, "mixin"));
        assert!(!has_word(&values, "OnLoad"));
        assert!(!has_word(&values, "Bar"));
    }

    #[test]
    fn an_alias_follows_as() {
        let text = "import a as b from \"./x\"\nprint(b)";
        assert!(after_as(text, text.find(" b").unwrap() + 1));
        assert!(!after_as(text, text.find("a as").unwrap()));
        assert!(!after_as(text, text.rfind('b').unwrap()));
    }
}
