//! Globals of Blizzard's own UI code (FrameXML) and its global strings:
//! top-level functions, tables, mixins and values of the Lua files, named
//! frames, regions and fonts of the XML files (decision T84).

use indexmap::IndexMap;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashSet;
use wowlua_syntax::ast::*;

/// The globals of one flavor's UI code: name → type text.
pub fn globals(
    lua: &[(String, String)],
    xml: &[(String, String)],
    widget_classes: &HashSet<String>,
) -> IndexMap<String, String> {
    let mut out: IndexMap<String, String> = IndexMap::new();
    for (_, text) in lua {
        lua_globals(text, &mut out);
    }
    for (_, text) in xml {
        xml_globals(text, widget_classes, &mut out);
    }
    out
}

fn add(out: &mut IndexMap<String, String>, name: &str, ty: &str) {
    if !crate::model::is_ident(name) {
        return;
    }
    match out.get(name) {
        // Different kinds in different files: `any`.
        Some(existing) if existing != ty => {
            out.insert(name.to_string(), "any".into());
        }
        Some(_) => {}
        None => {
            out.insert(name.to_string(), ty.to_string());
        }
    }
}

fn value_type(e: &Expr) -> &'static str {
    match &e.kind {
        ExprKind::Function(_) => "(...any) -> ...any",
        ExprKind::Table(_) => "table",
        ExprKind::String(_) | ExprKind::Interp(_) => "string",
        ExprKind::Number(_) => "number",
        ExprKind::True | ExprKind::False => "boolean",
        _ => "any",
    }
}

fn lua_globals(text: &str, out: &mut IndexMap<String, String>) {
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("ui.lua", text.to_string());
    let parse = wowlua_syntax::parse(text.as_bytes(), file);
    if parse.has_errors() {
        return;
    }
    let mut locals = HashSet::new();
    block(&parse.chunk.block, &mut locals, out);
}

/// Top-level statements (and those in top-level `do`/`if` blocks).
fn block(b: &Block, locals: &mut HashSet<String>, out: &mut IndexMap<String, String>) {
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Local { names, .. } => {
                for n in names {
                    locals.insert(n.name.text.to_string());
                }
            }
            StmtKind::LocalFunction { name, .. } => {
                locals.insert(name.text.to_string());
            }
            StmtKind::Function { name, .. } => {
                let root = name.path[0].text.to_string();
                if locals.contains(&root) {
                    continue;
                }
                if name.path.len() == 1 && name.method.is_none() {
                    add(out, &root, "(...any) -> ...any");
                } else {
                    add(out, &root, "table");
                }
            }
            StmtKind::Assign { targets, values } => {
                for (i, t) in targets.iter().enumerate() {
                    let ty = values.get(i).map(value_type).unwrap_or("any");
                    match &t.kind {
                        ExprKind::Name(n) if !locals.contains(n.text.as_ref()) => {
                            add(out, &n.text, ty)
                        }
                        // `_G.X = …` / `_G["X"] = …`
                        ExprKind::Field { obj, name } if matches!(&obj.kind, ExprKind::Name(g) if g.text.as_ref() == "_G") => {
                            add(out, &name.text, ty)
                        }
                        ExprKind::Index { obj, key } if matches!(&obj.kind, ExprKind::Name(g) if g.text.as_ref() == "_G") => {
                            if let ExprKind::String(k) = &key.kind {
                                add(out, &String::from_utf8_lossy(&k.value), ty);
                            }
                        }
                        _ => {}
                    }
                }
            }
            StmtKind::Do(inner) => block(inner, locals, out),
            StmtKind::If {
                then_block,
                elseifs,
                else_block,
                ..
            } => {
                let mut l = locals.clone();
                block(then_block, &mut l, out);
                for (_, b) in elseifs {
                    let mut l = locals.clone();
                    block(b, &mut l, out);
                }
                if let Some(b) = else_block {
                    let mut l = locals.clone();
                    block(b, &mut l, out);
                }
            }
            _ => {}
        }
    }
}

/// Named elements of an XML file: frames (outside templates), regions and
/// font objects.
fn xml_globals(text: &str, widget_classes: &HashSet<String>, out: &mut IndexMap<String, String>) {
    let mut reader = Reader::from_str(text);
    // Depth inside templates (`virtual="true"`).
    let mut stack: Vec<bool> = Vec::new();
    loop {
        let event = match reader.read_event() {
            Ok(e) => e,
            Err(_) => return,
        };
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let local = e.local_name();
                let tag: &str = local.as_ref();
                let tag = tag.to_string();
                let mut name = None;
                let mut virtual_ = false;
                for a in e.attributes().flatten() {
                    let key = a.key.local_name();
                    let key: &str = key.as_ref();
                    let value = a.value.to_string();
                    match key {
                        "name" => name = Some(value),
                        "virtual" => virtual_ = value.eq_ignore_ascii_case("true"),
                        _ => {}
                    }
                }
                let in_template = stack.iter().any(|v| *v) || virtual_;
                if let Some(n) = name.filter(|n| !n.contains('$')) {
                    match tag.as_str() {
                        // Font objects are globals even when virtual.
                        "Font" | "FontFamily" => add(out, &n, "Font"),
                        _ if in_template => {}
                        _ => {
                            let ty = if widget_classes.contains(&tag) {
                                tag.as_str()
                            } else {
                                "Frame"
                            };
                            add(out, &n, ty);
                        }
                    }
                }
                if matches!(event, Event::Start(_)) {
                    stack.push(in_template);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => return,
            _ => {}
        }
    }
}

/// The names of a `GlobalAPI` dump (every quoted name).
pub fn quoted_names(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        })
        .collect()
}

/// The names of a `GlobalStrings` file (`NAME = "…";` and
/// `_G["NAME"] = "…";` lines).
pub fn global_strings(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let name = match l.strip_prefix("_G[\"") {
                Some(rest) => &rest[..rest.find("\"]")?],
                None => l.split_once(" = ")?.0,
            };
            crate::model::is_ident(name).then(|| name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_and_xml_globals() {
        let lua = "local x = 1\nFooMixin = {}\nfunction FooMixin:Bar() end\nfunction GlobalFn() end\nNAME = \"x\"\nlocal function hidden() end\ndo GameTooltipText = 1 end\n_G.Other = {}\n";
        let xml = r#"<Ui><Frame name="MyFrame"><Frames><Button name="$parentClose"/></Frames></Frame><Frame name="Tpl" virtual="true"/><Font name="GameFontNormal" virtual="true"/><Button name="Btn"/></Ui>"#;
        let classes: HashSet<String> = ["Button".to_string()].into_iter().collect();
        let g = globals(
            &[("a.lua".into(), lua.into())],
            &[("a.xml".into(), xml.into())],
            &classes,
        );
        assert_eq!(g["FooMixin"], "table");
        assert_eq!(g["GlobalFn"], "(...any) -> ...any");
        assert_eq!(g["NAME"], "string");
        assert_eq!(g["GameTooltipText"], "number");
        assert_eq!(g["Other"], "table");
        assert_eq!(g["MyFrame"], "Frame");
        assert_eq!(g["Btn"], "Button");
        assert_eq!(g["GameFontNormal"], "Font");
        assert!(!g.contains_key("x") && !g.contains_key("Tpl") && !g.contains_key("hidden"));
        assert_eq!(
            global_strings(
                "_G[\"OKAY\"] = \"Okay\";\n_G[\"1X\"] = \"\";\nUNKNOWN = \"Unknown\";\n"
            ),
            ["OKAY", "UNKNOWN"]
        );
    }
}
