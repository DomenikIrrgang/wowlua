//! The locale pipeline (spec stdlib "locale"): JSON translations in
//! `build.locales-dir`, checked against the base locale, compiled into
//! self-contained Lua files that fill the addon's locale table before any
//! other file runs, and the type of `wowlua:locale`'s `L`.

use indexmap::IndexMap;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::{LOCALES, LocaleLoading};

/// A piece of a locale string.
#[derive(Clone, Debug, PartialEq)]
enum Part {
    Text(String),
    Param(String),
}

/// One locale file.
#[derive(Clone, Debug)]
pub struct Locale {
    pub code: String,
    /// Key → the string's parts, in the file's order.
    entries: IndexMap<String, Vec<Part>>,
}

/// The project's locales, the base locale first.
#[derive(Clone, Debug)]
pub struct Locales {
    pub base: String,
    pub list: Vec<Locale>,
}

/// A generated file: its path in the AddOn folder, its Lua code, and the
/// client locales it is for (`None`: every client).
pub struct LocaleFile {
    pub path: String,
    pub code: String,
    pub locales: Option<Vec<String>>,
}

/// Splits `{name}` placeholders out of a string (`{{` and `}}` are
/// literal braces).
fn parts(text: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            buf.push('{');
            i += 2;
            continue;
        }
        if c == '}' && chars.get(i + 1) == Some(&'}') {
            buf.push('}');
            i += 2;
            continue;
        }
        if c == '{' {
            let rest: String = chars[i + 1..].iter().collect();
            if let Some(end) = rest.find('}') {
                let name = &rest[..end];
                let valid = name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if valid {
                    if !buf.is_empty() {
                        out.push(Part::Text(std::mem::take(&mut buf)));
                    }
                    out.push(Part::Param(name.to_string()));
                    i += 1 + name.chars().count() + 1;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    if !buf.is_empty() {
        out.push(Part::Text(buf));
    }
    out
}

/// The placeholder names of a string, sorted and deduplicated.
fn params(parts: &[Part]) -> BTreeSet<&str> {
    parts
        .iter()
        .filter_map(|p| match p {
            Part::Param(n) => Some(n.as_str()),
            Part::Text(_) => None,
        })
        .collect()
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Reads and checks the locale files of `dir` (the package root's
/// `build.locales-dir`): E8105 (file names), E8106 (base missing), E8115
/// (not an object of strings), E8060 (keys), E8061 (unknown keys), E8063
/// (placeholders), W8062 (missing translations). `rel_dir` names the folder
/// in diagnostics.
pub fn load(
    dir: &Path,
    rel_dir: &str,
    base: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Locales> {
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    if let Ok(entries) = crate::host::read_dir(dir) {
        for path in entries {
            if path.extension().is_some_and(|x| x == "json")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                files.push((stem.to_string(), path));
            }
        }
    }
    files.sort();
    let mut list: Vec<Locale> = Vec::new();
    let mut ok = true;
    for (code, path) in &files {
        let rel = format!("{rel_dir}/{code}.json");
        if !LOCALES.contains(&code.as_str()) {
            diagnostics.push(
                Diagnostic::error(
                    "E8105",
                    format!("`{code}.json`: `{code}` is no locale code"),
                )
                .with_path(&rel)
                .with_help(format!(
                    "locale files are named by locale: {}",
                    LOCALES.join(", ")
                )),
            );
            ok = false;
            continue;
        }
        let text = crate::host::read_to_string(path).unwrap_or_default();
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                diagnostics.push(
                    Diagnostic::error("E8115", format!("`{rel}` is not valid JSON: {e}"))
                        .with_path(&rel),
                );
                ok = false;
                continue;
            }
        };
        let Some(object) = value.as_object() else {
            diagnostics.push(
                Diagnostic::error("E8115", format!("`{rel}` must be an object of strings"))
                    .with_path(&rel),
            );
            ok = false;
            continue;
        };
        let mut entries = IndexMap::new();
        for (key, v) in object {
            if !valid_key(key) {
                diagnostics.push(
                    Diagnostic::error(
                        "E8060",
                        format!("`{key}` in `{rel}` is no locale key (`[A-Z0-9_]+`)"),
                    )
                    .with_path(&rel),
                );
                ok = false;
                continue;
            }
            let Some(s) = v.as_str() else {
                diagnostics.push(
                    Diagnostic::error("E8115", format!("`{key}` in `{rel}` must be a string"))
                        .with_path(&rel),
                );
                ok = false;
                continue;
            };
            entries.insert(key.clone(), parts(s));
        }
        list.push(Locale {
            code: code.clone(),
            entries,
        });
    }
    let Some(base_index) = list.iter().position(|l| l.code == base) else {
        if !files.iter().any(|(code, _)| code == base) {
            diagnostics.push(
                Diagnostic::error(
                    "E8106",
                    format!("the base locale `{rel_dir}/{base}.json` is missing"),
                )
                .with_path(rel_dir)
                .with_help("it defines the keys; `build.locale-base` names another base"),
            );
        }
        return None;
    };
    let base_locale = list.remove(base_index);
    // Non-base locales: known keys, the base's placeholders, missing keys.
    for l in &list {
        let rel = format!("{rel_dir}/{}.json", l.code);
        for (key, value) in &l.entries {
            match base_locale.entries.get(key) {
                None => {
                    diagnostics.push(
                        Diagnostic::error(
                            "E8061",
                            format!("`{key}` in `{rel}` is not a key of the base locale `{base}`"),
                        )
                        .with_path(&rel),
                    );
                    ok = false;
                }
                Some(b) if params(b) != params(value) => {
                    let names = |p: BTreeSet<&str>| {
                        p.iter()
                            .map(|n| format!("{{{n}}}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    diagnostics.push(
                        Diagnostic::error(
                            "E8063",
                            format!(
                                "`{key}` in `{rel}` uses the placeholders [{}], the base locale [{}]",
                                names(params(value)),
                                names(params(b))
                            ),
                        )
                        .with_path(&rel),
                    );
                    ok = false;
                }
                Some(_) => {}
            }
        }
        let missing: Vec<&String> = base_locale
            .entries
            .keys()
            .filter(|k| !l.entries.contains_key(*k))
            .collect();
        if !missing.is_empty() {
            let shown: Vec<String> = missing.iter().take(5).map(|k| format!("`{k}`")).collect();
            let more = if missing.len() > 5 {
                format!(" and {} more", missing.len() - 5)
            } else {
                String::new()
            };
            diagnostics.push(
                Diagnostic::warning(
                    "W8062",
                    format!(
                        "`{}` lacks {} of {} translations ({}{more}); the base strings are used",
                        l.code,
                        missing.len(),
                        base_locale.entries.len(),
                        shown.join(", ")
                    ),
                )
                .with_path(&rel),
            );
        }
    }
    if !ok {
        return None;
    }
    // The base first; `enUS` before `enGB`, which overrides it.
    list.sort_by_key(|l| (l.code == "enGB", l.code.clone()));
    list.insert(0, base_locale);
    Some(Locales {
        base: base.to_string(),
        list,
    })
}

/// A Lua string literal.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\{}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `L.KEY = …` for one entry.
fn assignment(key: &str, parts: &[Part]) -> String {
    let target = if key.starts_with(|c: char| c.is_ascii_digit()) {
        format!("L[{}]", quote(key))
    } else {
        format!("L.{key}")
    };
    if params(parts).is_empty() {
        let text: String = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => t.as_str(),
                Part::Param(_) => "",
            })
            .collect();
        return format!("{target} = {}", quote(&text));
    }
    let pieces: Vec<String> = parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => quote(t),
            Part::Param(n) => format!("tostring(p.{n})"),
        })
        .collect();
    format!("{target} = function(p) return {} end", pieces.join(" .. "))
}

/// The guard of a non-base locale: the client locales it applies to.
fn clients(code: &str) -> Vec<String> {
    if code == "enUS" {
        vec!["enUS".into(), "enGB".into()]
    } else {
        vec![code.to_string()]
    }
}

const PROLOGUE: &str = "local __wl_ns = select(2, ...); local __wl = __wl_ns.__wowlua or {}; __wl_ns.__wowlua = __wl\nlocal L = __wl.locale or {}; __wl.locale = L\n";

/// The Lua files of the locales: one per locale (`client`), each guarded
/// by `GetLocale()` but the base, or one with every locale (`all`).
pub fn emit(locales: &Locales, loading: LocaleLoading) -> Vec<LocaleFile> {
    let header = "-- Generated by wowlua from the locale files; do not edit.\n";
    match loading {
        LocaleLoading::Client => locales
            .list
            .iter()
            .map(|l| {
                let mut code = String::from(header);
                let guard = (l.code != locales.base).then(|| clients(&l.code));
                if let Some(codes) = &guard {
                    let test: Vec<String> = codes
                        .iter()
                        .map(|c| format!("__wl_locale ~= {}", quote(c)))
                        .collect();
                    let _ = writeln!(
                        code,
                        "local __wl_locale = GetLocale()\nif {} then return end",
                        test.join(" and ")
                    );
                }
                code.push_str(PROLOGUE);
                for (key, parts) in &l.entries {
                    code.push_str(&assignment(key, parts));
                    code.push('\n');
                }
                LocaleFile {
                    path: format!("wowlua/locales/{}.lua", l.code),
                    code,
                    locales: guard,
                }
            })
            .collect(),
        LocaleLoading::All => {
            let mut code = String::from(header);
            code.push_str(PROLOGUE);
            code.push_str("local __wl_locale = GetLocale()\n");
            for l in &locales.list {
                let body: String = l
                    .entries
                    .iter()
                    .map(|(k, p)| format!("{}\n", assignment(k, p)))
                    .collect();
                if l.code == locales.base {
                    code.push_str(&body);
                } else {
                    let test: Vec<String> = clients(&l.code)
                        .iter()
                        .map(|c| format!("__wl_locale == {}", quote(c)))
                        .collect();
                    let _ = write!(code, "if {} then\n{body}end\n", test.join(" or "));
                }
            }
            vec![LocaleFile {
                path: "wowlua/locales/locales.lua".into(),
                code,
                locales: None,
            }]
        }
    }
}

/// The declaration of `wowlua:locale`'s `L`: a string per key, a function
/// of a table of the placeholders' values for keys with placeholders.
pub fn declaration(locales: Option<&Locales>) -> String {
    let mut out = String::from(
        "-- Generated from the locale files: the type of `L` (spec stdlib \"locale\").\n",
    );
    let Some(locales) = locales else {
        out.push_str("@intrinsic(\"locale.L\")\ndeclare global L: any\n");
        return out;
    };
    out.push_str("interface WowluaLocale\n");
    // Keys starting with a digit are no names: fields `["1ST"]` of a
    // record beside the interface, read as `L["1ST"]`.
    let mut digit_keys: Vec<String> = Vec::new();
    for (key, parts) in &locales.list[0].entries {
        let names = params(parts);
        let ty = if names.is_empty() {
            "string".to_string()
        } else {
            let fields: Vec<String> = names.iter().map(|n| format!("{n}: any")).collect();
            format!("(params: {{ {} }}) -> string", fields.join(", "))
        };
        if key.starts_with(|c: char| c.is_ascii_digit()) {
            digit_keys.push(format!("[\"{key}\"]: {ty}"));
        } else {
            let _ = writeln!(out, "    {key}: {ty}");
        }
    }
    out.push_str("end\n@intrinsic(\"locale.L\")\n");
    if digit_keys.is_empty() {
        out.push_str("declare global L: WowluaLocale\n");
    } else {
        let _ = writeln!(
            out,
            "declare global L: WowluaLocale & {{ {} }}",
            digit_keys.join(", ")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_and_braces() {
        assert_eq!(
            parts("Hi {name}, {{literal}} {count}!"),
            vec![
                Part::Text("Hi ".into()),
                Part::Param("name".into()),
                Part::Text(", {literal} ".into()),
                Part::Param("count".into()),
                Part::Text("!".into()),
            ]
        );
        assert_eq!(parts("{ not one }"), vec![Part::Text("{ not one }".into())]);
        assert_eq!(
            assignment("WELCOME", &parts("Welcome, {name}!")),
            "L.WELCOME = function(p) return \"Welcome, \" .. tostring(p.name) .. \"!\" end"
        );
        assert_eq!(
            assignment("1ST", &parts("a\"b\n")),
            "L[\"1ST\"] = \"a\\\"b\\n\""
        );
    }
}
