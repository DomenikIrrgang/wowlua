//! What the game's API functions do, for the declarations' doc comments
//! (the language server's hover and completion, spec §16.2): the summary,
//! parameters and results warcraft.wiki.gg's API pages describe (CC BY-SA
//! 4.0), and the descriptions Blizzard's own documentation gives. Keys:
//! `UnitHealth`, `C_Spell.GetSpellInfo`, `Button:GetText`, and an event's
//! `event:BAG_UPDATE` (its payload's descriptions as its parameters).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What one function's documentation says.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    /// What it does (Markdown).
    #[serde(rename = "s", default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// Parameters with a description: name, description.
    #[serde(rename = "p", default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<(String, String)>,
    /// Results with a description.
    #[serde(rename = "r", default, skip_serializing_if = "Vec::is_empty")]
    pub returns: Vec<(String, String)>,
    /// Its wiki page (`API_UnitHealth`).
    #[serde(rename = "w", default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// The summary is wowlua's own (`Written`): no documentation says it.
    #[serde(skip)]
    pub written: bool,
}

/// A description wowlua wrote where no documentation gives one, from the
/// function's name, signature and wowlua's emulation of it
/// (`xtask/data/written-docs.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Written {
    #[serde(rename = "s")]
    pub summary: String,
    #[serde(rename = "p", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    #[serde(rename = "r", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub returns: BTreeMap<String, String>,
}

/// Documentation by function.
pub type Docs = BTreeMap<String, Doc>;

/// What the generator documents the functions from: the wiki's pages and
/// wowlua's own descriptions.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub wiki: Docs,
    pub written: BTreeMap<String, Written>,
}

const WIKI: &str = "https://warcraft.wiki.gg/wiki/";

/// The key of the function a page title names: `API:UnitHealth` →
/// `UnitHealth`, `API:C Spell.GetSpellInfo` → `C_Spell.GetSpellInfo`,
/// `API:Button GetText` → `Button:GetText`.
pub fn key(title: &str) -> Option<String> {
    let rest = title.strip_prefix("API:")?;
    if let Some(name) = crate::wiki::function_name(title) {
        return Some(name);
    }
    let (class, method) = rest.rsplit_once(' ')?;
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (word(class) && word(method)).then(|| format!("{class}:{method}"))
}

/// A page's documentation: its summary (the text before the first
/// section, templates left out) and the descriptions of its arguments and
/// results.
pub fn from_wiki(title: &str, text: &str) -> Option<(String, Doc)> {
    let key = key(title)?;
    // (an old page parts its sections with `----` lines and `;''Returns''`
    // headings)
    let end = [text.find("\n=="), text.find("\n----"), text.find("\n;''")]
        .into_iter()
        .flatten()
        .min();
    let lead = match end {
        Some(i) => &text[..i],
        None => text,
    };
    // (a template may span lines: `{{apisig|…}}` of several calls)
    let lead = strip_templates(lead);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in lead.lines() {
        // (a definition's `:` indents a line)
        let t = line.trim().trim_start_matches(':').trim_start();
        let prose = markdown(t);
        // (an old page writes its call as text: the signature shows it)
        let prose = strip_call(&prose);
        if t.is_empty() || prose.is_empty() || t.starts_with("[[Category") || t.starts_with("__") {
            if !current.is_empty() && t.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(&prose);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    let doc = Doc {
        summary: paragraphs.join("\n\n"),
        params: described(section(text, "Arguments").or_else(|| section(text, "Parameters"))),
        returns: described(section(text, "Returns")),
        page: Some(title.replace([':', ' '], "_")),
        written: false,
    };
    Some((key, doc))
}

/// `s` without a call written at its end (`text = fontString:GetText()`,
/// `UnitHealth("unit")`).
fn strip_call(s: &str) -> String {
    let t = s.trim().trim_matches('`').trim();
    let Some(open) = t.find('(') else {
        return s.to_string();
    };
    if !t.ends_with(')') {
        return s.to_string();
    }
    // the results named before ` = `, then the function's path
    let head = &t[..open];
    let callee = head.rsplit_once(" = ").map_or(head, |(_, c)| c).trim();
    let path = !callee.is_empty()
        && callee
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':'));
    let results = head.rsplit_once(" = ").is_none_or(|(r, _)| {
        r.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ',' | ' ' | '[' | ']'))
    });
    if path && results {
        String::new()
    } else {
        s.to_string()
    }
}

/// The text of a `==Name==` section.
fn section<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        let heading = t.starts_with("==") && !t.starts_with("===") && t.ends_with("==");
        if let Some(s) = start {
            if heading {
                return Some(&text[s..offset]);
            }
        } else if heading && t.trim_matches('=').trim().eq_ignore_ascii_case(name) {
            start = Some(offset + line.len());
        }
        offset += line.len();
    }
    start.map(|s| &text[s..])
}

/// The values of a section that have a description: `:;name:{{apitype|T}}
/// - text` and table rows `| 1 || name || {{apitype|T}} || text`.
fn described(section: Option<&str>) -> Vec<(String, String)> {
    let Some(section) = section else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for line in section.lines() {
        let t = line.trim_end();
        let (name, rest) = if let Some(rest) = t.strip_prefix(":;") {
            let Some((name, tail)) = rest.split_once(':') else {
                continue;
            };
            (name, after_type(tail))
        } else if let Some(rest) = t.strip_prefix('|') {
            let cells: Vec<&str> = rest.split("||").map(str::trim).collect();
            if cells.len() < 4 || !cells[0].chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            (cells[1], cells[3..].join(" "))
        } else {
            continue;
        };
        // (a name may be a link or carry a template: `[[TitleId|titleId]]`)
        let name = markdown(&strip_templates(name));
        let name = name
            .trim()
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
            .trim();
        // (after the type a dash or a colon: `{{apitype|number}} : [[UiMapID]]`)
        let text = markdown(rest.trim().trim_start_matches(['-', '–', ':']).trim());
        if !name.is_empty() && !text.is_empty() && !out.iter().any(|(n, _)| n == name) {
            out.push((name.to_string(), strip_templates(&text)));
        }
    }
    out
}

/// The text after a value's `{{apitype|…}}`.
fn after_type(s: &str) -> String {
    match s.find("{{apitype|") {
        Some(i) => match s[i..].find("}}") {
            Some(j) => s[i + j + 2..].to_string(),
            None => String::new(),
        },
        None => s.to_string(),
    }
}

/// `s` without its templates that are not text (`{{wowapi}}`,
/// `{{apisig|…}}`): what is left is prose.
fn strip_templates(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        if b[i..].starts_with(b"{{") {
            // (the templates that read as text stay)
            if depth == 0
                && (s[i..].starts_with("{{api|")
                    || s[i..].starts_with("{{apitype|")
                    || s[i..].starts_with("{{=}}"))
            {
                let end = s[i..].find("}}").map_or(b.len(), |j| i + j + 2);
                out.push_str(&s[i..end]);
                i = end;
                continue;
            }
            depth += 1;
            i += 2;
        } else if b[i..].starts_with(b"}}") && depth > 0 {
            depth -= 1;
            i += 2;
        } else {
            if depth == 0 {
                out.push(b[i] as char);
            }
            i += 1;
        }
    }
    out.trim().to_string()
}

/// Wiki markup as Markdown: `{{api|X}}` and `<code>x</code>` as code,
/// links as their text, bold and italics, other templates and tags left
/// out.
pub fn markdown(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("{{=}}") {
            out.push('=');
            rest = r;
        } else if rest.starts_with("{{api|") || rest.starts_with("{{apitype|") {
            let end = rest.find("}}").map_or(rest.len(), |i| i + 2);
            // `{{api|t=e|NAME|…}}`: the first part that is no option
            let body = rest.get(2..end.saturating_sub(2)).unwrap_or("");
            let name = body
                .split('|')
                .skip(1)
                .find(|p| !p.contains('='))
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                out.push('`');
                out.push_str(name);
                out.push('`');
            }
            rest = &rest[end..];
        } else if rest.starts_with("{{") {
            // another template: left out, nested ones too
            let mut depth = 0;
            let mut end = rest.len();
            let b = rest.as_bytes();
            let mut i = 0;
            while i + 1 < b.len() {
                if b[i] == b'{' && b[i + 1] == b'{' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'}' && b[i + 1] == b'}' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            rest = &rest[end..];
        } else if let Some(r) = rest.strip_prefix("}}") {
            // (the end of a template that began elsewhere)
            rest = r;
        } else if let Some(r) = rest.strip_prefix("<code>") {
            let end = r.find("</code>").unwrap_or(r.len());
            out.push('`');
            out.push_str(&r[..end]);
            out.push('`');
            rest = r.get(end + "</code>".len()..).unwrap_or("");
        } else if rest.starts_with("<ref") {
            let end = match (rest.find("/>"), rest.find("</ref>")) {
                (Some(a), Some(b)) if a < b => a + 2,
                (_, Some(b)) => b + "</ref>".len(),
                (Some(a), None) => a + 2,
                (None, None) => rest.len(),
            };
            rest = &rest[end..];
        } else if rest.starts_with("<br") {
            out.push(' ');
            rest = &rest[rest.find('>').map_or(rest.len(), |i| i + 1)..];
        } else if rest.starts_with('<')
            && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/')
        {
            rest = &rest[rest.find('>').map_or(rest.len(), |i| i + 1)..];
        } else if let Some(r) = rest.strip_prefix("[[") {
            let end = r.find("]]").unwrap_or(r.len());
            let link = &r[..end];
            out.push_str(link.rsplit('|').next().unwrap_or(link));
            rest = r.get(end + 2..).unwrap_or("");
        } else if rest.starts_with("[http") {
            let end = rest.find(']').unwrap_or(rest.len());
            let link = &rest[1..end];
            out.push_str(link.split_once(' ').map_or(link, |(_, text)| text));
            rest = rest.get(end + 1..).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix("'''") {
            out.push_str("**");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("''") {
            out.push('*');
            rest = r;
        } else if let Some(r) = rest.strip_prefix("&nbsp;") {
            out.push(' ');
            rest = r;
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The descriptions in Blizzard's documentation (`Documentation = {…}` of
/// a function and its arguments and results, of an event and its payload),
/// by key; an event's is `event:BAG_UPDATE`.
pub fn from_blizzard(api: &serde_json::Value) -> Docs {
    let mut out = Docs::new();
    let lines = |v: &serde_json::Value| -> String {
        v["Documentation"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str())
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    };
    let values = |v: &serde_json::Value| -> Vec<(String, String)> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let d = lines(a);
                Some((a["Name"].as_str()?.to_string(), d)).filter(|(_, d)| !d.is_empty())
            })
            .collect()
    };
    for system in api["systems"].as_array().into_iter().flatten() {
        let prefix = match (system["Namespace"].as_str(), system["Class"].as_str()) {
            (Some(ns), _) => format!("{ns}."),
            (None, Some(class)) => format!("{class}:"),
            (None, None) => String::new(),
        };
        for f in system["Functions"].as_array().into_iter().flatten() {
            let Some(name) = f["Name"].as_str() else {
                continue;
            };
            let doc = Doc {
                summary: lines(f),
                params: values(&f["Arguments"]),
                returns: values(&f["Returns"]),
                page: None,
                written: false,
            };
            if doc != Doc::default() {
                out.insert(format!("{prefix}{name}"), doc);
            }
        }
        for e in system["Events"].as_array().into_iter().flatten() {
            let Some(name) = e["LiteralName"].as_str() else {
                continue;
            };
            let doc = Doc {
                summary: lines(e),
                params: values(&e["Payload"]),
                returns: Vec::new(),
                page: None,
                written: false,
            };
            if doc != Doc::default() {
                out.insert(format!("event:{name}"), doc);
            }
        }
    }
    out
}

/// `wiki`'s documentation, completed by `blizzard`'s where it says
/// nothing.
pub fn merge(wiki: &Docs, blizzard: &Docs) -> Docs {
    let mut out = wiki.clone();
    for (key, b) in blizzard {
        let d = out.entry(key.clone()).or_default();
        if d.summary.is_empty() {
            d.summary = b.summary.clone();
        }
        for (list, from) in [(&mut d.params, &b.params), (&mut d.returns, &b.returns)] {
            for (n, text) in from {
                if !list.iter().any(|(m, _)| m == n) {
                    list.push((n.clone(), text.clone()));
                }
            }
        }
    }
    out
}

/// `docs` completed by wowlua's own descriptions where no documentation
/// says what a function does (marked as such).
pub fn add_written(docs: &mut Docs, written: &BTreeMap<String, Written>) {
    for (key, w) in written {
        let d = docs.entry(key.clone()).or_default();
        if !d.summary.is_empty() || w.summary.trim().is_empty() {
            continue;
        }
        d.summary = w.summary.trim().to_string();
        d.written = true;
        for (list, from) in [(&mut d.params, &w.params), (&mut d.returns, &w.returns)] {
            for (n, text) in from {
                if !list.iter().any(|(m, _)| m == n) && !text.trim().is_empty() {
                    list.push((n.clone(), text.trim().to_string()));
                }
            }
        }
    }
}

/// A doc comment's lines (without `---`): the summary, the parameters and
/// results described, the page.
pub fn lines(doc: &Doc) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let gap = |out: &mut Vec<String>| {
        if !out.is_empty() {
            out.push(String::new());
        }
    };
    if !doc.summary.is_empty() {
        out.extend(doc.summary.lines().map(str::to_string));
    }
    for (title, list) in [("Parameters", &doc.params), ("Returns", &doc.returns)] {
        if list.is_empty() {
            continue;
        }
        gap(&mut out);
        out.push(format!("**{title}**"));
        for (name, text) in list {
            out.push(format!("- `{name}`: {text}"));
        }
    }
    if doc.written {
        gap(&mut out);
        out.push("*Description by wowlua: the API's documentation has none.*".into());
    }
    if let Some(page) = &doc.page {
        gap(&mut out);
        out.push(format!("[warcraft.wiki.gg]({WIKI}{page})"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_gives_its_summary_parameters_and_results() {
        let text = "{{wowapi|t=a|system=Unit}}\nReturns the current health of the unit.\n{{apisig|health {{=}} UnitHealth(unit [, usePredicted])}}\n\n==Arguments==\n:;unit:{{apitype|UnitToken}}\n:;usePredicted:{{apitype|boolean?|default=true}} - Predicts the values from the <code>combat log</code>.<ref>x</ref>\n\n==Returns==\n:;health:{{apitype|number}} - Returns <code>0</code> if the unit is dead, see {{api|UnitIsDead}}.\n";
        let (key, doc) = from_wiki("API:UnitHealth", text).unwrap();
        assert_eq!(key, "UnitHealth");
        assert_eq!(doc.summary, "Returns the current health of the unit.");
        assert_eq!(
            doc.params,
            vec![(
                "usePredicted".to_string(),
                "Predicts the values from the `combat log`.".to_string()
            )]
        );
        assert_eq!(
            doc.returns,
            vec![(
                "health".to_string(),
                "Returns `0` if the unit is dead, see `UnitIsDead`.".to_string()
            )]
        );
        assert_eq!(
            lines(&doc).last().map(String::as_str),
            Some("[warcraft.wiki.gg](https://warcraft.wiki.gg/wiki/API_UnitHealth)")
        );
    }

    #[test]
    fn an_old_page_gives_its_lead_and_no_markup() {
        let text = "{{wowapi}}\nReturns true if on a Windows client.\n{{apisig|IsWindowsClient()}}\n\n----\n;''Returns''\n:;true - If running on a Windows\n\n----\n;''Description''\n\n: Will check the user client.";
        let (_, doc) = from_wiki("API:IsWindowsClient", text).unwrap();
        assert_eq!(doc.summary, "Returns true if on a Windows client.");
        let text = "{{wowapi}}\nUpdates the digsites.\n\n==Arguments==\n:;uiMapID:{{apitype|number}} : [[UiMapID]]\n";
        let (_, doc) = from_wiki("API:ArchaeologyMapUpdateAll", text).unwrap();
        assert_eq!(
            doc.params,
            vec![("uiMapID".to_string(), "UiMapID".to_string())]
        );
    }

    #[test]
    fn keys_name_functions_namespaces_and_methods() {
        assert_eq!(
            key("API:C Spell.GetSpellInfo").as_deref(),
            Some("C_Spell.GetSpellInfo")
        );
        assert_eq!(key("API:Button GetText").as_deref(), Some("Button:GetText"));
        assert_eq!(key("API:GetTime").as_deref(), Some("GetTime"));
    }

    #[test]
    fn a_stub_page_gives_its_link_only() {
        let (_, doc) = from_wiki(
            "API:ToggleGravity",
            "{{wowapi|t=a|system=DebugToggle}}\n{{apisig|ToggleGravity()}}",
        )
        .unwrap();
        assert_eq!(doc.summary, "");
        assert_eq!(
            lines(&doc),
            vec!["[warcraft.wiki.gg](https://warcraft.wiki.gg/wiki/API_ToggleGravity)".to_string()]
        );
    }

    #[test]
    fn a_call_written_as_text_is_left_out() {
        let (_, doc) = from_wiki("API:FontString GetText", "{{widgetmethod}}\nReturns the text from a FontString widget.\n text = fontString:GetText()\n").unwrap();
        assert_eq!(doc.summary, "Returns the text from a FontString widget.");
    }

    #[test]
    fn markup_reads_as_markdown() {
        assert_eq!(
            markdown("'''Note:''' see [[API Foo|Foo]] and [https://x.y the site]"),
            "**Note:** see Foo and the site"
        );
        assert_eq!(markdown("a {{Patch 1.2|note=x}} b"), "a b");
    }
}

#[cfg(test)]
mod emit_tests {
    use super::*;
    use crate::model::{Decls, Namespace, Sig};

    #[test]
    fn declarations_carry_their_doc_comments() {
        let mut d = Decls::default();
        d.functions.insert("GetTime".into(), vec![Sig::default()]);
        let mut ns = Namespace::default();
        ns.functions.insert("After".into(), vec![Sig::default()]);
        d.namespaces.insert("C_Timer".into(), ns);
        d.docs.insert(
            "GetTime".into(),
            Doc {
                summary: "Returns the system uptime.".into(),
                page: Some("API_GetTime".into()),
                ..Doc::default()
            },
        );
        let mut written = BTreeMap::new();
        written.insert(
            "C_Timer.After".into(),
            Written {
                summary: "Calls the callback after the delay.".into(),
                ..Written::default()
            },
        );
        add_written(&mut d.docs, &written);
        let text = d.emit("header");
        assert!(
            text.contains("--- Returns the system uptime.\n---\n--- [warcraft.wiki.gg](https://warcraft.wiki.gg/wiki/API_GetTime)\ndeclare function GetTime()"),
            "{text}"
        );
        assert!(
            text.contains("    --- Calls the callback after the delay.\n    ---\n    --- *Description by wowlua"),
            "{text}"
        );
    }
}
