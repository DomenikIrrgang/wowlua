//! warcraft.wiki.gg's API pages (CC BY-SA 4.0): the signatures of API
//! functions that Blizzard's documentation and Ketho's annotations leave
//! untyped, as LuaLS annotations for [`crate::luals::convert`]
//! (spec package manager §10.9, decision T288).
//!
//! A page names its call in `{{apisig|r1, r2 {{=}} F(a [, b])}}` and types
//! each value in its `==Arguments==` and `==Returns==` sections, as a list
//! (`:;name:{{apitype|T}}`) or a table (`| 1 || name || {{apitype|T}} || …`).
//! Records are `Struct_X` / `Structure X` pages, typed the same way.

use std::collections::BTreeSet;
use std::fmt::Write;

/// One value of a signature: its name, its type as the wiki writes it, and
/// whether it may be left out.
#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    pub name: String,
    pub ty: String,
    pub optional: bool,
}

/// What a page says of its function.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    /// `C_Garrison.GetFollowers`, `GetItemInfo`.
    pub name: String,
    pub params: Vec<Value>,
    /// `...` among the arguments.
    pub vararg: bool,
    pub returns: Vec<Value>,
    /// `...` among the results.
    pub ret_vararg: bool,
    pub deprecated: bool,
    /// The other calls the page shows (`{{=}} GetAchievementInfo(categoryID,
    /// index)`): their parameters and `...`, the results the same.
    pub overloads: Vec<(Vec<Value>, bool)>,
}

/// The function a page title names: `API:C Garrison.GetFollowers` →
/// `C_Garrison.GetFollowers`, `API:GetItemInfo` → `GetItemInfo`. A widget
/// method's page (`API:Frame SetPoint`) names none: Ketho's annotations type
/// the widgets.
pub fn function_name(title: &str) -> Option<String> {
    let rest = title.strip_prefix("API:")?;
    let name = match rest.split_once('.') {
        Some((ns, member)) => format!("{}.{member}", ns.replace(' ', "_")),
        None if rest.contains(' ') => return None,
        None => rest.to_string(),
    };
    let ident = |s: &str| {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !s.starts_with(|c: char| c.is_ascii_digit())
    };
    name.split('.').all(ident).then_some(name)
}

/// The text of a `==Name==` section, up to the next one of its level or
/// above.
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

/// `{{apitype|T|…}}` at the start of `s`: `T`, and whether a default
/// (`|default=…`) makes the value optional.
fn apitype(s: &str) -> Option<(String, bool)> {
    let start = s.find("{{apitype|")? + "{{apitype|".len();
    let end = start + s[start..].find("}}")?;
    let mut parts = s[start..end].split('|');
    let ty = parts.next()?.trim().to_string();
    let default = parts.any(|p| p.trim_start().starts_with("default"));
    (!ty.is_empty()).then_some((ty, default))
}

/// The old format: `; name : Number - …`.
fn old_type(word: &str) -> Option<&'static str> {
    Some(match word.to_ascii_lowercase().as_str() {
        "number" | "integer" | "float" => "number",
        "string" => "string",
        "boolean" | "bool" | "flag" => "boolean",
        "table" => "table",
        "function" => "function",
        _ => return None,
    })
}

/// Whether a value's description says it can be nil (`Returns nil if
/// Completed is false`).
fn says_nil(description: &str) -> bool {
    description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case("nil"))
}

/// The text after a cell's `{{apitype|…}}`.
fn after_apitype(s: &str) -> &str {
    s.find("{{apitype|")
        .and_then(|i| s[i..].find("}}").map(|j| &s[i + j + 2..]))
        .unwrap_or("")
}

/// The values a section lists, in order.
fn values(section: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for line in section.lines() {
        let t = line.trim_end();
        // A list entry at the top level: `:;name:` (`::;` is a field of a
        // record shown under it).
        if let Some(rest) = t.strip_prefix(":;") {
            let Some((name, tail)) = rest.split_once(':') else {
                continue;
            };
            // `1. itemName`
            let name = name
                .trim()
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
                .trim();
            if let Some((ty, default)) = apitype(tail) {
                out.push(Value {
                    name: name.to_string(),
                    ty,
                    optional: default || says_nil(after_apitype(tail)),
                });
            }
            continue;
        }
        // A table row: `| 1 || name || {{apitype|T}} || …`.
        if let Some(rest) = t.strip_prefix('|') {
            let cells: Vec<&str> = rest.split("||").map(str::trim).collect();
            if cells.len() >= 3
                && cells[0].chars().all(|c| c.is_ascii_digit())
                && let Some((ty, default)) = apitype(cells[2])
            {
                let description = cells[3..].join(" ");
                out.push(Value {
                    name: cells[1].to_string(),
                    ty,
                    optional: default || says_nil(&description),
                });
            }
            continue;
        }
        // The old format.
        if let Some(rest) = t.strip_prefix(';')
            && let Some((name, tail)) = rest.split_once(':')
        {
            let word = tail
                .trim()
                .split(|c: char| !c.is_ascii_alphabetic())
                .next()
                .unwrap_or("");
            if let Some(ty) = old_type(word) {
                out.push(Value {
                    name: name.trim().to_string(),
                    ty: ty.into(),
                    optional: false,
                });
            }
        }
    }
    out
}

/// A call's arguments' names, each with whether brackets make it optional.
type Args = Vec<(String, bool)>;

/// A call as `{{apisig|…}}` writes it: the results' names, and each form's
/// arguments (`r {{=}} F(a) {{=}} F(b, c)`: two).
type Call = (Vec<String>, Vec<Args>);

/// The `{{apisig|…}}` call.
fn apisig(text: &str) -> Option<Call> {
    let start = text.find("{{apisig|")? + "{{apisig|".len();
    // to the template's end: `}}` at depth 0 (`{{=}}` nests)
    let bytes = text.as_bytes();
    let (mut depth, mut i) = (0usize, start);
    let end = loop {
        if i + 1 >= bytes.len() {
            return None;
        }
        if &bytes[i..i + 2] == b"{{" {
            depth += 1;
            i += 2;
        } else if &bytes[i..i + 2] == b"}}" {
            if depth == 0 {
                break i;
            }
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    };
    let sig = &text[start..end];
    let parts: Vec<&str> = sig.split("{{=}}").map(str::trim).collect();
    let (lhs, calls) = match parts.as_slice() {
        [call] => ("", vec![*call]),
        [lhs, calls @ ..] => (*lhs, calls.to_vec()),
        [] => return None,
    };
    let results: Vec<String> = lhs
        .split(',')
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    let forms: Vec<Args> = calls.iter().filter_map(|c| call_args(c)).collect();
    if forms.is_empty() {
        return None;
    }
    Some((results, forms))
}

/// The arguments of one call `F(a, [b])`.
fn call_args(call: &str) -> Option<Args> {
    let open = call.find('(')?;
    let close = call.rfind(')')?;
    if close < open {
        return None;
    }
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut word = String::new();
    let flush = |word: &mut String, optional: bool, args: &mut Args| {
        let w = word
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        if !w.is_empty() {
            args.push((w, optional));
        }
        word.clear();
    };
    for c in call[open + 1..close].chars() {
        match c {
            '[' => {
                flush(&mut word, depth > 0, &mut args);
                depth += 1;
            }
            ']' => {
                flush(&mut word, depth > 0, &mut args);
                depth -= 1;
            }
            ',' => flush(&mut word, depth > 0, &mut args),
            _ => word.push(c),
        }
    }
    flush(&mut word, depth > 0, &mut args);
    Some(args)
}

/// Reads one page; `None` when it has no call to read.
pub fn function(title: &str, text: &str) -> Option<Function> {
    let name = function_name(title)?;
    if text.trim_start().starts_with("#REDIRECT") {
        return None;
    }
    let sig = apisig(text);
    let listed_args = section(text, "Arguments").map(values).unwrap_or_default();
    let listed_rets = section(text, "Returns").map(values).unwrap_or_default();
    if sig.is_none() && listed_args.is_empty() && listed_rets.is_empty() {
        return None;
    }
    let find = |list: &[Value], name: &str| list.iter().find(|v| v.name == name).cloned();
    // A form's parameters: each listed by name, else (the first form's) by
    // position.
    let form = |args: &Args, by_position: bool| -> (Vec<Value>, bool) {
        let mut params = Vec::new();
        let mut vararg = false;
        for (i, (arg, bracketed)) in args.iter().enumerate() {
            if arg == "..." {
                vararg = true;
                continue;
            }
            let listed = find(&listed_args, arg)
                .or_else(|| by_position.then(|| listed_args.get(i).cloned()).flatten());
            let mut v = listed.unwrap_or(Value {
                name: arg.clone(),
                ty: "unknown".into(),
                optional: false,
            });
            v.name = arg.clone();
            v.optional |= *bracketed || v.ty.ends_with('?');
            params.push(v);
        }
        (params, vararg)
    };
    let mut params = Vec::new();
    let mut vararg = false;
    let mut overloads = Vec::new();
    match &sig {
        Some((_, forms)) => {
            (params, vararg) = form(&forms[0], true);
            overloads = forms[1..].iter().map(|f| form(f, false)).collect();
        }
        None => {
            for v in &listed_args {
                let mut v = v.clone();
                v.optional |= v.ty.ends_with('?');
                params.push(v);
            }
        }
    }
    let mut returns = Vec::new();
    let mut ret_vararg = false;
    match &sig {
        Some((results, _)) if !results.is_empty() => {
            for (i, r) in results.iter().enumerate() {
                if r == "..." {
                    ret_vararg = true;
                    continue;
                }
                let listed = find(&listed_rets, r).or_else(|| listed_rets.get(i).cloned());
                returns.push(listed.unwrap_or(Value {
                    name: r.clone(),
                    ty: "unknown".into(),
                    optional: false,
                }));
            }
        }
        _ => returns = listed_rets,
    }
    // A parameter's type keeps no `?`: optionality is the parameter's.
    for p in params
        .iter_mut()
        .chain(overloads.iter_mut().flat_map(|(o, _)| o.iter_mut()))
    {
        if let Some(t) = p.ty.strip_suffix('?') {
            p.ty = t.to_string();
        }
    }
    Some(Function {
        name,
        params,
        vararg,
        returns,
        ret_vararg,
        deprecated: text.contains("{{deprecatedapi"),
        overloads,
    })
}

/// A record page's name: `Struct GarrFollowerInfo` / `Structure X` → the
/// type's name.
pub fn struct_name(title: &str) -> Option<String> {
    let rest = title
        .strip_prefix("Struct ")
        .or_else(|| title.strip_prefix("Structure "))
        .or_else(|| title.strip_prefix("Struct_"))?;
    let name = rest.trim().replace(' ', "_");
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then_some(name)
}

/// The records a page shows under its values (`{{:Struct_X|…}}`,
/// `{{:Structure X|…}}`): their page titles.
pub fn struct_refs(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(i) = rest.find("{{:") {
        let after = &rest[i + 3..];
        let end = after.find(['|', '}']).unwrap_or(after.len());
        let title = after[..end].replace('_', " ").trim().to_string();
        if struct_name(&title).is_some() {
            out.insert(title);
        }
        rest = &after[end..];
    }
    out
}

/// A record page's fields: rows `| {{apiname|field}} || {{apitype|T}} || …`
/// (or a list, as a function's values).
pub fn struct_fields(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim_end().strip_prefix('|') else {
            continue;
        };
        let cells: Vec<&str> = rest.split("||").map(str::trim).collect();
        if cells.len() < 2 {
            continue;
        }
        let Some(start) = cells[0].find("{{apiname|") else {
            continue;
        };
        let after = &cells[0][start + "{{apiname|".len()..];
        let name = after[..after.find(['|', '}']).unwrap_or(after.len())].trim();
        if let Some((ty, default)) = apitype(cells[1]) {
            out.push(Value {
                name: name.to_string(),
                optional: default || ty.ends_with('?'),
                ty: ty.trim_end_matches('?').to_string(),
            });
        }
    }
    if out.is_empty() { values(text) } else { out }
}

/// A wiki type as LuaLS writes it (`number,string` is a union).
fn luals_type(ty: &str) -> String {
    ty.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("|")
}

/// A result's type as LuaLS writes it, `?` when it can be nil.
fn result_type(r: &Value) -> String {
    let ty = luals_type(r.ty.trim_end_matches('?'));
    let optional = r.optional || r.ty.ends_with('?');
    match (optional, ty.contains('|')) {
        (true, true) => format!("({ty})?"),
        (true, false) => format!("{ty}?"),
        (false, _) => ty,
    }
}

/// The functions and records as one LuaLS file.
pub fn to_luals(functions: &[Function], structs: &[(String, Vec<Value>)]) -> String {
    let mut out = String::new();
    for (name, fields) in structs {
        let _ = writeln!(out, "---@class {name}");
        for f in fields {
            let opt = if f.optional || f.ty.ends_with('?') {
                "?"
            } else {
                ""
            };
            let ty = luals_type(f.ty.trim_end_matches('?'));
            let _ = writeln!(out, "---@field {}{opt} {ty}", f.name);
        }
        out.push('\n');
    }
    // Namespaces as tables, so their functions are namespace functions.
    let namespaces: BTreeSet<&str> = functions
        .iter()
        .filter_map(|f| f.name.split_once('.').map(|(ns, _)| ns))
        .collect();
    for ns in namespaces {
        let _ = writeln!(out, "{ns} = {{}}");
    }
    out.push('\n');
    for f in functions {
        if f.deprecated {
            out.push_str("---@deprecated\n");
        }
        // The other forms, with the same results.
        let results: Vec<String> = f
            .returns
            .iter()
            .map(result_type)
            .chain(f.ret_vararg.then(|| "...".to_string()))
            .collect();
        for (params, vararg) in &f.overloads {
            let list: Vec<String> = params
                .iter()
                .map(|p| {
                    let opt = if p.optional { "?" } else { "" };
                    format!("{}{opt}: {}", p.name, luals_type(&p.ty))
                })
                .chain(vararg.then(|| "...: unknown".to_string()))
                .collect();
            let ret = if results.is_empty() {
                String::new()
            } else {
                format!(": {}", results.join(", "))
            };
            let _ = writeln!(out, "---@overload fun({}){ret}", list.join(", "));
        }
        for p in &f.params {
            let opt = if p.optional { "?" } else { "" };
            let _ = writeln!(out, "---@param {}{opt} {}", p.name, luals_type(&p.ty));
        }
        if f.vararg {
            out.push_str("---@param ... unknown\n");
        }
        for r in &f.returns {
            let _ = writeln!(out, "---@return {} {}", result_type(r), r.name);
        }
        if f.ret_vararg {
            out.push_str("---@return unknown ...\n");
        }
        let names: Vec<&str> = f
            .params
            .iter()
            .map(|p| p.name.as_str())
            .chain(f.vararg.then_some("..."))
            .collect();
        let _ = writeln!(out, "function {}({}) end\n", f.name, names.join(", "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            function_name("API:C Garrison.GetFollowers").as_deref(),
            Some("C_Garrison.GetFollowers")
        );
        assert_eq!(
            function_name("API:GetItemInfo").as_deref(),
            Some("GetItemInfo")
        );
        assert_eq!(function_name("API:Frame SetPoint"), None);
        assert_eq!(
            struct_name("Struct GarrFollowerInfo").as_deref(),
            Some("GarrFollowerInfo")
        );
    }

    #[test]
    fn a_list_page() {
        let text = "{{wowapi}}\n{{apisig|info {{=}} C_Garrison.GetFollowers([followerType])}}\n\n==Arguments==\n:;followerType:{{apitype|Enum.GarrisonFollowerType?}}\n{{:Enum.GarrisonFollowerType|nocaption=1}}\n\n==Returns==\n:;info:{{apitype|GarrFollowerInfo[]}}\n{{:Struct_GarrFollowerInfo|nocaption=1}}\n\n==Details==\n* x\n";
        let f = function("API:C Garrison.GetFollowers", text).unwrap();
        assert_eq!(f.name, "C_Garrison.GetFollowers");
        assert_eq!(
            f.params,
            [Value {
                name: "followerType".into(),
                ty: "Enum.GarrisonFollowerType".into(),
                optional: true
            }]
        );
        assert_eq!(f.returns[0].ty, "GarrFollowerInfo[]");
        assert_eq!(
            struct_refs(text).into_iter().collect::<Vec<_>>(),
            ["Struct GarrFollowerInfo"]
        );
    }

    #[test]
    fn a_table_page_numbered_values_and_defaults() {
        let text = "{{apisig|petID, speciesID {{=}} C_PetJournal.GetPetInfoByIndex(index)}}\n==Arguments==\n:;index:{{apitype|number}} - x\n==Returns==\n{| class=\"darktable\"\n! Index !! Value !! Type !! Details\n|-\n| 1 || petID || {{apitype|string}} || GUID\n|-\n| 2 || speciesID || {{apitype|number}} || id\n|}\n";
        let f = function("API:C PetJournal.GetPetInfoByIndex", text).unwrap();
        assert_eq!(f.returns.len(), 2);
        assert_eq!(f.returns[1].ty, "number");
        let text = "{{apisig|name, ... {{=}} UnitAura(unit, index [, filter])}}\n==Arguments==\n:;unit:{{apitype|UnitToken}}\n:;index:{{apitype|number}}\n:;filter:{{apitype|AuraFilters?|default=HELPFUL}}\n==Returns==\n:;1. name:{{apitype|string}}\n";
        let f = function("API:UnitAura", text).unwrap();
        assert!(f.params[2].optional);
        assert_eq!(f.params[2].ty, "AuraFilters");
        assert_eq!(f.returns[0].ty, "string");
        assert!(f.ret_vararg);
        let lua = to_luals(&[f], &[]);
        assert!(lua.contains("---@param filter? AuraFilters\n"), "{lua}");
        assert!(
            lua.contains("---@return unknown ...\nfunction UnitAura(unit, index, filter) end"),
            "{lua}"
        );
    }

    #[test]
    fn a_record_page() {
        let text = "{{wowapitype}}\n<onlyinclude>{| class=\"sortable\"\n! Field !! Type !! Description\n|-\n| {{apiname|reagent}} || {{apitype|CraftingReagent}} || {{apiname.added|12.0.0}} x\n|-\n| {{apiname|quantity}} || {{apitype|number?}} || \n|}\n{{:Structure CraftingReagent}}</onlyinclude>";
        let f = struct_fields(text);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].name, "reagent");
        assert!(f[1].optional && f[1].ty == "number");
        assert!(struct_refs(text).contains("Structure CraftingReagent"));
    }

    #[test]
    fn untyped_values_are_unknown() {
        let f = function("API:Foo", "{{apisig|a {{=}} Foo(x)}}\n").unwrap();
        assert_eq!(f.params[0].ty, "unknown");
        assert_eq!(f.returns[0].ty, "unknown");
        assert_eq!(function("API:Foo", "#REDIRECT [[API:Bar]]"), None);
    }

    #[test]
    fn a_page_of_two_forms() {
        let text = "{{wowapi}}\n{{apisig|id, month\n  {{=}} GetAchievementInfo(achievementID)\n  {{=}} GetAchievementInfo(categoryID, index)}}\n\n==Arguments==\n:;achievementID:{{apitype|number}} : The achievement.\n----\n:;categoryID:{{apitype|number}} - A category.\n:;index:{{apitype|number}} - An offset.\n\n==Returns==\n{| class=\"darktable\"\n! Index !! Param !! Type !! Description\n|-\n|1 || id || {{apitype|number}} || Achievement ID.\n|-\n|2 || month || {{apitype|number}} || Returns nil if Completed is false.\n|}\n";
        let f = function("API:GetAchievementInfo", text).expect("a function");
        let names = |ps: &[Value]| ps.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&f.params), ["achievementID"]);
        assert_eq!(f.overloads.len(), 1);
        assert_eq!(names(&f.overloads[0].0), ["categoryID", "index"]);
        assert_eq!(f.overloads[0].0[1].ty, "number");
        assert!(!f.returns[0].optional);
        assert!(f.returns[1].optional);
        let lua = to_luals(&[f], &[]);
        assert!(
            lua.contains("---@overload fun(categoryID: number, index: number): number, number?\n"),
            "{lua}"
        );
        assert!(lua.contains("---@return number? month\n"), "{lua}");
    }
}
