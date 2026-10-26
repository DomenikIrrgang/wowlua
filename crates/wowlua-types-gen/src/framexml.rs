//! Globals of Blizzard's own UI code (FrameXML) and its global strings:
//! top-level functions, tables, mixins and values of the Lua files, named
//! frames, regions and fonts of the XML files (decision T84).

use crate::model::{Class, ParamDecl, Sig, ident};
use indexmap::IndexMap;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashSet;
use wowlua_syntax::view::*;

/// The aura filter words of `AuraUtil.AuraFilters` in the UI code (`!`
/// when the flavor negates words), sorted.
pub fn aura_filter_words(ui_lua: &[(String, String)]) -> Vec<String> {
    let mut words = std::collections::BTreeSet::new();
    for (_, text) in ui_lua {
        let Some(at) = text.find("AuraUtil.AuraFilters =") else {
            continue;
        };
        let rest = &text[at..];
        let (Some(open), Some(close)) = (rest.find('{'), rest.find("\n}")) else {
            continue;
        };
        for part in rest[open..close].split('"').skip(1).step_by(2) {
            if !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b == b'_' || b == b'!')
            {
                words.insert(part.trim_start_matches('!').to_string());
                if part.starts_with('!') {
                    words.insert("!".to_string());
                }
            }
        }
    }
    words.remove("");
    words.into_iter().collect()
}

/// The tooltip methods `TooltipDataHandler.lua` defines, each with the
/// `C_TooltipInfo` function it passes its arguments to: (`SetInboxItem`,
/// `GetInboxItem`), from its `local accessors = { … }`.
pub fn tooltip_accessors(ui_lua: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (_, text) in ui_lua {
        let Some(at) = text.find("local accessors = {") else {
            continue;
        };
        let rest = &text[at..];
        let Some(close) = rest.find('}') else {
            continue;
        };
        for line in rest[..close].lines().skip(1) {
            let Some((method, getter)) = line.split_once('=') else {
                continue;
            };
            let method = method.trim();
            let getter = getter.trim().trim_end_matches(',').trim_matches('"');
            if crate::model::is_ident(method) && crate::model::is_ident(getter) {
                out.push((method.to_string(), getter.to_string()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The globals of one flavor's UI code: name → type text.
pub fn globals(
    lua: &[(String, String)],
    xml: &[(String, String)],
    widget_classes: &HashSet<String>,
) -> IndexMap<String, String> {
    ui_globals(lua, xml, widget_classes).globals
}

/// The globals of one flavor's UI code, and the mixins its named frames
/// are typed with.
pub struct UiGlobals {
    /// Name → type text.
    pub globals: IndexMap<String, String>,
    /// The mixin classes the named frames' types name (and their bases),
    /// those the annotations declare left out.
    pub mixins: IndexMap<String, Class>,
    /// Where each class's method comes from: (class, method) → the mixin
    /// whose Lua defines it (its own, or one copied in).
    pub origins: std::collections::HashMap<(String, String), String>,
}

/// A mixin table of the UI code (`FooMixin = {}`, `FooMixin =
/// CreateFromMixins(BarMixin)`) and the functions its Lua defines on it.
#[derive(Clone, Debug, Default)]
struct MixinDef {
    /// `CreateFromMixins(A, B)`, `Mixin({}, A, B)`.
    bases: Vec<String>,
    /// `function M:F(a, b)`: name → (parameters, `...`).
    methods: IndexMap<String, (Vec<String>, bool)>,
    /// `function M.F(a)`: name → (parameters, `...`).
    functions: IndexMap<String, (Vec<String>, bool)>,
}

/// A template (`virtual="true"`) or intrinsic frame type of the XML.
#[derive(Clone, Debug, Default)]
struct Template {
    tag: String,
    /// `mixin` and `secureMixin`.
    mixins: Vec<String>,
    inherits: Vec<String>,
    intrinsic: bool,
}

/// A named frame (outside templates) of the XML.
struct NamedFrame {
    name: String,
    tag: String,
    mixins: Vec<String>,
    inherits: Vec<String>,
}

/// The globals of one flavor's UI code. A named frame is typed by its
/// mixins and its widget type (`ChatFrame1: PrimaryChatFrameMixin & … &
/// Frame`): those of its `mixin` and `secureMixin` attributes first, then
/// those of the templates it inherits, in order, and of its intrinsic type
/// (`<ScrollingMessageFrame>`). A member is looked up in the parts in order,
/// so a mixin's method wins over the widget's, as in the client.
pub fn ui_globals(
    lua: &[(String, String)],
    xml: &[(String, String)],
    widget_classes: &HashSet<String>,
) -> UiGlobals {
    let mut out: IndexMap<String, String> = IndexMap::new();
    let mut defs: IndexMap<String, MixinDef> = IndexMap::new();
    for (_, text) in lua {
        lua_globals(text, &mut out, &mut defs);
    }
    let templates = template_defs(xml);
    // Each frame's widget type and mixins; one defined differently in
    // several files keeps the mixins every definition has.
    let mut frames: IndexMap<String, Option<(String, Vec<String>)>> = IndexMap::new();
    for (_, text) in xml {
        let mut names = IndexMap::new();
        let mut named = Vec::new();
        let mut scripts = Vec::new();
        xml_globals(text, &mut names, &mut named, &mut scripts);
        for (frame, code) in scripts {
            match frame {
                None => lua_globals(&code, &mut out, &mut defs),
                // `self` is the frame; a handler defines no mixins.
                Some(frame) => {
                    let mut found = IndexMap::new();
                    lua_globals(&code, &mut found, &mut IndexMap::new());
                    for (name, ty) in found {
                        let ty = if ty == "alias:self" {
                            format!("alias:{frame}")
                        } else {
                            ty
                        };
                        add(&mut out, &name, &ty);
                    }
                }
            }
        }
        for frame in named {
            let (base, mut mixins) = frame_type(&frame, &templates, widget_classes);
            mixins.retain(|m| defs.contains_key(m) || widget_classes.contains(m));
            match frames.get_mut(&frame.name) {
                Some(Some((b, ms))) if *b == base => ms.retain(|m| mixins.contains(m)),
                Some(slot) => *slot = None,
                None => {
                    frames.insert(frame.name, Some((base, mixins)));
                }
            }
        }
        // Fonts and regions as before.
        for (name, ty) in names {
            if !frames.contains_key(&name) {
                add(&mut out, &name, &ty);
            }
        }
    }
    let mut used: Vec<String> = Vec::new();
    for (name, frame) in frames {
        let ty = match frame {
            Some((base, mixins)) => {
                let mut parts = mixins.clone();
                parts.push(base);
                used.extend(mixins);
                parts.join(" & ")
            }
            // Different widget types in different files: a region of the
            // screen still.
            None => "Region".into(),
        };
        add(&mut out, &name, &ty);
    }
    // Every mixin the code defines functions on is a class (decision T288),
    // not only those the named frames use.
    for (name, def) in &defs {
        if !def.methods.is_empty() || !def.functions.is_empty() {
            used.push(name.clone());
        }
    }
    let mut origins = std::collections::HashMap::new();
    let mixins = mixin_classes(&defs, used, widget_classes, &mut origins);
    UiGlobals {
        globals: out,
        mixins,
        origins,
    }
}

/// A frame's widget type and mixins, the most specific first.
fn frame_type(
    frame: &NamedFrame,
    templates: &std::collections::HashMap<String, Template>,
    widget_classes: &HashSet<String>,
) -> (String, Vec<String>) {
    fn inherit(
        name: &str,
        templates: &std::collections::HashMap<String, Template>,
        seen: &mut HashSet<String>,
        out: &mut Vec<String>,
    ) {
        if !seen.insert(name.to_string()) {
            return;
        }
        let Some(t) = templates.get(name) else { return };
        for m in &t.mixins {
            if !out.contains(m) {
                out.push(m.clone());
            }
        }
        for i in &t.inherits {
            inherit(i, templates, seen, out);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for m in &frame.mixins {
        if !out.contains(m) {
            out.push(m.clone());
        }
    }
    let mut seen = HashSet::new();
    for i in &frame.inherits {
        inherit(i, templates, &mut seen, &mut out);
    }
    // An intrinsic type brings its mixins and its own widget type.
    let mut tag = frame.tag.clone();
    while !widget_classes.contains(&tag)
        && let Some(t) = templates.get(&tag).filter(|t| t.intrinsic)
        && !seen.contains(&tag)
    {
        let next = t.tag.clone();
        inherit(&tag, templates, &mut seen, &mut out);
        tag = next;
    }
    let base = if widget_classes.contains(&tag) {
        tag
    } else {
        "Frame".to_string()
    };
    (base, out)
}

/// The classes of the mixins `used` and of their bases: methods take their
/// Lua parameters, untyped, and answer any values. The first declared base
/// is the class's base; the others' functions are copied in.
fn mixin_classes(
    defs: &IndexMap<String, MixinDef>,
    used: Vec<String>,
    widget_classes: &HashSet<String>,
    origins: &mut std::collections::HashMap<(String, String), String>,
) -> IndexMap<String, Class> {
    let sig = |(params, vararg): &(Vec<String>, bool)| Sig {
        params: params
            .iter()
            .map(|p| ParamDecl {
                name: p.clone(),
                ty: "any".into(),
                optional: false,
            })
            .collect(),
        vararg: vararg.then(|| "any".to_string()),
        ret_vararg: Some("any".into()),
        ..Sig::default()
    };
    let function_type = |f: &(Vec<String>, bool)| {
        let mut params: Vec<String> = f.0.iter().map(|p| format!("{}: any", ident(p))).collect();
        if f.1 {
            params.push("...: any".into());
        }
        format!("({}) -> ...any", params.join(", "))
    };
    // A mixin's first declared base.
    let first_base = |name: &str| -> Option<String> {
        defs.get(name)?
            .bases
            .iter()
            .find(|b| defs.contains_key(*b) || widget_classes.contains(*b))
            .cloned()
    };
    let mut out: IndexMap<String, Class> = IndexMap::new();
    let mut pending = used;
    pending.reverse();
    while let Some(name) = pending.pop() {
        if out.contains_key(&name) || widget_classes.contains(&name) {
            continue;
        }
        if !defs.contains_key(&name) {
            continue;
        }
        let mut class = Class::default();
        let mut base = first_base(&name);
        // No cycles of bases.
        let mut at = base.clone();
        let mut steps = 0;
        while let Some(b) = at {
            if b == name || steps > defs.len() {
                base = None;
                break;
            }
            at = first_base(&b);
            steps += 1;
        }
        // Members: the mixin's own, then those of the bases after the
        // first (and of their bases).
        let mut sources = vec![name.clone()];
        let mut seen: HashSet<String> = HashSet::new();
        let mut i = 0;
        while i < sources.len() {
            let current = sources[i].clone();
            i += 1;
            if !seen.insert(current.clone()) {
                continue;
            }
            let Some(d) = defs.get(&current) else {
                continue;
            };
            for (m, f) in &d.methods {
                if !class.methods.contains_key(m) && !class.fields.contains_key(m) {
                    class.methods.insert(m.clone(), vec![sig(f)]);
                    origins.insert((name.clone(), m.clone()), current.clone());
                }
            }
            for (m, f) in &d.functions {
                if !class.methods.contains_key(m) && !class.fields.contains_key(m) {
                    class.fields.insert(m.clone(), (function_type(f), false));
                }
            }
            // (the last mixin's functions win, as `CreateFromMixins(A, B)`
            // copies B's over A's: the later bases are read first)
            for b in d.bases.iter().rev() {
                if Some(b) != base.as_ref() || current != name {
                    sources.push(b.clone());
                }
            }
        }
        if let Some(b) = base {
            pending.push(b.clone());
            class.bases.push(b);
        }
        out.insert(name, class);
    }
    out
}

/// Globals that only files of load-on-demand addons define (`lod_files`:
/// path, lowercase with `/`, → addon): name → the addon (the first by name
/// when several do).
pub fn lod_origins(
    lua: &[(String, String)],
    xml: &[(String, String)],
    lod_files: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    let mut by_name: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let mut not_lod: HashSet<String> = HashSet::new();
    let files = lua
        .iter()
        .map(|f| (f, true))
        .chain(xml.iter().map(|f| (f, false)));
    for ((path, text), is_lua) in files {
        let mut g = IndexMap::new();
        if is_lua {
            lua_globals(text, &mut g, &mut IndexMap::new());
        } else {
            let mut named = Vec::new();
            let mut scripts = Vec::new();
            xml_globals(text, &mut g, &mut named, &mut scripts);
            for frame in named {
                g.insert(frame.name, String::new());
            }
            for (_, code) in scripts {
                lua_globals(&code, &mut g, &mut IndexMap::new());
            }
        }
        match lod_files.get(&path.replace('\\', "/").to_lowercase()) {
            Some(addon) => {
                for name in g.keys() {
                    by_name
                        .entry(name.clone())
                        .or_default()
                        .insert(addon.clone());
                }
            }
            None => not_lod.extend(g.into_keys()),
        }
    }
    by_name
        .into_iter()
        .filter(|(name, _)| !not_lod.contains(name))
        .filter_map(|(name, addons)| addons.into_iter().next().map(|a| (name, a)))
        .collect()
}

fn add(out: &mut IndexMap<String, String>, name: &str, ty: &str) {
    if !crate::model::is_ident(name) {
        return;
    }
    match out.get(name) {
        // Different kinds in different files: `unknown`.
        Some(existing) if existing != ty => {
            out.insert(name.to_string(), "unknown".into());
        }
        Some(_) => {}
        None => {
            out.insert(name.to_string(), ty.to_string());
        }
    }
}

/// `a.b.c` as text.
fn dotted(e: Expr<'_>) -> Option<String> {
    match e.kind() {
        ExprKind::Name(n) => Some(n.text().to_string()),
        ExprKind::Field { obj, name } => Some(format!("{}.{}", dotted(obj)?, name.text())),
        _ => None,
    }
}

/// What a global's value is: a literal's type, or what the generator
/// resolves later (decision T288): `call:F` (the result of `F(…)`),
/// `alias:X` (another global's value), `unknown`.
fn value_type(e: Expr<'_>) -> String {
    match e.kind() {
        ExprKind::Function(_) => "(...any) -> ...any".into(),
        ExprKind::Table(_) => "table".into(),
        ExprKind::String(_) | ExprKind::Interp(_) => "string".into(),
        ExprKind::Number(_) => "number".into(),
        ExprKind::True | ExprKind::False => "boolean".into(),
        ExprKind::Paren(inner) => value_type(inner),
        ExprKind::Name(n) => format!("alias:{}", n.text()),
        // `Constants.InventoryConstants.NumBagSlots`, `Enum.X.Y`
        ExprKind::Field { .. } => {
            dotted(e).map_or_else(|| "unknown".into(), |d| format!("alias:{d}"))
        }
        ExprKind::Call { callee, .. } => match callee.kind() {
            ExprKind::Name(n) => format!("call:{}", n.text()),
            ExprKind::Field { obj, name } => match obj.kind() {
                ExprKind::Name(o) => format!("call:{}.{}", o.text(), name.text()),
                _ => "unknown".into(),
            },
            _ => "unknown".into(),
        },
        ExprKind::Binary { op, .. } => match op {
            BinOp::Concat => "string".into(),
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                "number".into()
            }
            _ => "unknown".into(),
        },
        _ => "unknown".into(),
    }
}

fn lua_globals(
    text: &str,
    out: &mut IndexMap<String, String>,
    defs: &mut IndexMap<String, MixinDef>,
) {
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("ui.lua", text.to_string());
    let parse = wowlua_syntax::parse(text.as_bytes(), file);
    if parse.has_errors() {
        return;
    }
    let mut locals = HashSet::new();
    block(parse.view().block(), &mut locals, out, defs);
}

/// A function's parameters and whether it takes `...`.
fn params(func: Func<'_>) -> (Vec<String>, bool) {
    let names = func
        .params()
        .iter()
        .map(|p| p.name().text().to_string())
        .collect();
    (names, func.vararg().is_some())
}

/// The mixins a call creates a table from: `CreateFromMixins(A, B)`,
/// `Mixin({}, A, B)`.
fn mixin_bases(e: Expr<'_>) -> Option<Vec<String>> {
    let ExprKind::Call { callee, args } = e.kind() else {
        return None;
    };
    let ExprKind::Name(f) = callee.kind() else {
        return None;
    };
    let ArgsKind::Parens { args, .. } = args.kind() else {
        return None;
    };
    let args: Vec<Expr<'_>> = args.iter().collect();
    let rest = match f.text() {
        "CreateFromMixins" => &args[..],
        "Mixin" if matches!(args.first().map(|a| a.kind()), Some(ExprKind::Table(_))) => &args[1..],
        _ => return None,
    };
    Some(
        rest.iter()
            .filter_map(|a| match a.kind() {
                ExprKind::Name(n) => Some(n.text().to_string()),
                _ => None,
            })
            .collect(),
    )
}

/// Top-level statements (and those in top-level `do`/`if` blocks).
fn block(
    b: Block<'_>,
    locals: &mut HashSet<String>,
    out: &mut IndexMap<String, String>,
    defs: &mut IndexMap<String, MixinDef>,
) {
    for s in &b.stmts() {
        match s.kind() {
            StmtKind::Local { names, .. } => {
                for n in names {
                    locals.insert(n.name.text().to_string());
                }
            }
            StmtKind::LocalFunction { name, .. } => {
                locals.insert(name.text().to_string());
            }
            StmtKind::Function { name, func } => {
                let root = name.path().at(0).text().to_string();
                if locals.contains(&root) {
                    continue;
                }
                if name.path().len() == 1 && name.method().is_none() {
                    add(out, &root, "(...any) -> ...any");
                } else {
                    add(out, &root, "table");
                }
                // A table's functions (`function M:F(a)`, `function M.F(a)`).
                match (name.path().len(), name.method()) {
                    (1, Some(m)) => {
                        let def = defs.entry(root).or_default();
                        def.methods
                            .entry(m.text().to_string())
                            .or_insert_with(|| params(func));
                    }
                    (2, None) => {
                        let def = defs.entry(root).or_default();
                        def.functions
                            .entry(name.path().at(1).text().to_string())
                            .or_insert_with(|| params(func));
                    }
                    _ => {}
                }
            }
            StmtKind::Assign { targets, values } => {
                for (i, t) in targets.iter().enumerate() {
                    let ty = values
                        .get(i)
                        .map(value_type)
                        .unwrap_or_else(|| "unknown".into());
                    let ty = ty.as_str();
                    match t.kind() {
                        ExprKind::Name(n) if !locals.contains(n.text()) => {
                            add(out, n.text(), ty);
                            // A mixin table.
                            let value = values.get(i);
                            let bases = value.and_then(mixin_bases);
                            if bases.is_some()
                                || value.is_some_and(|v| matches!(v.kind(), ExprKind::Table(_)))
                            {
                                let def = defs.entry(n.text().to_string()).or_default();
                                if def.bases.is_empty() {
                                    def.bases = bases.unwrap_or_default();
                                }
                            }
                        }
                        // `_G.X = …` / `_G["X"] = …`
                        ExprKind::Field { obj, name } if matches!(obj.kind(), ExprKind::Name(g) if g.text() == "_G") => {
                            add(out, name.text(), ty)
                        }
                        ExprKind::Index { obj, key } if matches!(obj.kind(), ExprKind::Name(g) if g.text() == "_G") => {
                            if let ExprKind::String(k) = key.kind() {
                                add(out, &String::from_utf8_lossy(&k.value()), ty);
                            }
                        }
                        _ => {}
                    }
                }
            }
            StmtKind::Do(inner) => block(inner, locals, out, defs),
            StmtKind::If {
                then_block,
                elseifs,
                else_block,
                ..
            } => {
                let mut l = locals.clone();
                block(then_block, &mut l, out, defs);
                for (_, b) in elseifs {
                    let mut l = locals.clone();
                    block(b, &mut l, out, defs);
                }
                if let Some(b) = else_block {
                    let mut l = locals.clone();
                    block(b, &mut l, out, defs);
                }
            }
            _ => {}
        }
    }
}

/// The Lua of an XML file that defines globals: an inline `<Script>`
/// (top-level code, `None`), and a named frame's `OnLoad` (it runs once,
/// `self` the frame). Templates' handlers run for each frame made from
/// them: left out.
type Scripts = Vec<(Option<String>, String)>;

/// Named elements of an XML file: font objects (into `out`), frames and
/// regions outside templates (into `named`), and its Lua that defines
/// globals (into `scripts`).
fn xml_globals(
    text: &str,
    out: &mut IndexMap<String, String>,
    named: &mut Vec<NamedFrame>,
    scripts: &mut Scripts,
) {
    let mut reader = Reader::from_str(text);
    // Per open element: inside a template (`virtual="true"`), its tag and
    // its name (a frame's outside templates).
    let mut stack: Vec<(bool, String, Option<String>)> = Vec::new();
    // The script being read: the depth of its element, its frame, its code.
    let mut reading: Option<(usize, Option<String>, String)> = None;
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
                let a = attributes(e);
                // An intrinsic frame type (`DropdownButton`) is a template,
                // not a frame.
                let in_template = stack.iter().any(|v| v.0) || a.virtual_ || a.intrinsic;
                let name = a.name.clone().filter(|n| !n.contains('$'));
                if let Some(n) = name.clone() {
                    match tag.as_str() {
                        // Font objects are globals even when virtual.
                        "Font" | "FontFamily" => add(out, &n, "Font"),
                        _ if in_template => {}
                        _ => named.push(NamedFrame {
                            name: n,
                            tag: tag.clone(),
                            mixins: a.mixins,
                            inherits: a.inherits,
                        }),
                    }
                }
                if matches!(event, Event::Start(_)) {
                    if reading.is_none() && !in_template {
                        let frame = match stack.as_slice() {
                            [.., (_, _, Some(frame)), (_, scripts, _)]
                                if tag == "OnLoad" && scripts == "Scripts" =>
                            {
                                Some(Some(frame.clone()))
                            }
                            _ if tag == "Script" && !a.file => Some(None),
                            _ => None,
                        };
                        if let Some(frame) = frame {
                            reading = Some((stack.len(), frame, String::new()));
                        }
                    }
                    stack.push((in_template, tag, name.filter(|_| !in_template)));
                }
            }
            Event::Text(t) => {
                if let Some((_, _, code)) = reading.as_mut() {
                    code.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some((_, _, code)) = reading.as_mut() {
                    code.push_str(&t.into_inner());
                }
            }
            Event::GeneralRef(r) => {
                if let Some((_, _, code)) = reading.as_mut() {
                    let c = match r.resolve_char_ref() {
                        Ok(Some(c)) => Some(c),
                        _ => match &*r.into_inner() {
                            "lt" => Some('<'),
                            "gt" => Some('>'),
                            "amp" => Some('&'),
                            "quot" => Some('"'),
                            "apos" => Some('\''),
                            _ => None,
                        },
                    };
                    code.extend(c);
                }
            }
            Event::End(_) => {
                stack.pop();
                if reading
                    .as_ref()
                    .is_some_and(|(at, _, _)| *at == stack.len())
                    && let Some((_, frame, code)) = reading.take()
                {
                    scripts.push((frame, code));
                }
            }
            Event::Eof => return,
            _ => {}
        }
    }
}

/// The names the UI code reads: every name of its Lua (and its XML's
/// scripts) outside comments and strings that is no field (`a.b`, `a:b`).
pub fn names_read(lua: &[(String, String)], xml: &[(String, String)]) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("ui.lua", String::new());
    let mut read = |code: &str| {
        let lexed = wowlua_syntax::lexer::lex(code.as_bytes(), file);
        let tokens = &lexed.tokens;
        for i in 0..tokens.len() {
            let t = tokens.token(i);
            if t.kind != wowlua_syntax::token::TokenKind::Name {
                continue;
            }
            let field = i > 0
                && matches!(
                    tokens.kind(i - 1),
                    wowlua_syntax::token::TokenKind::Dot | wowlua_syntax::token::TokenKind::Colon
                );
            if !field {
                out.insert(code[t.start as usize..t.end as usize].to_string());
            }
        }
    };
    for (_, text) in lua {
        read(text);
    }
    for (_, text) in xml {
        let mut scripts = Vec::new();
        xml_globals(text, &mut IndexMap::new(), &mut Vec::new(), &mut scripts);
        for (_, code) in scripts {
            read(&code);
        }
    }
    out
}

/// The attributes of an XML element the globals depend on.
#[derive(Default)]
struct Attributes {
    name: Option<String>,
    virtual_: bool,
    intrinsic: bool,
    /// `mixin` and `secureMixin`.
    mixins: Vec<String>,
    inherits: Vec<String>,
    /// A `<Script file="…">`.
    file: bool,
}

fn attributes(e: &quick_xml::events::BytesStart<'_>) -> Attributes {
    let list = |v: &str| -> Vec<String> {
        v.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut out = Attributes::default();
    for a in e.attributes().flatten() {
        let key = a.key.local_name();
        let key: &str = key.as_ref();
        let value = a.value.to_string();
        match key {
            "name" => out.name = Some(value),
            "virtual" => out.virtual_ |= value.eq_ignore_ascii_case("true"),
            "intrinsic" => out.intrinsic |= value.eq_ignore_ascii_case("true"),
            "mixin" | "secureMixin" => out.mixins.extend(list(&value)),
            "inherits" => out.inherits.extend(list(&value)),
            "file" => out.file = true,
            _ => {}
        }
    }
    out
}

/// The templates and intrinsic frame types of the XML by name (the first
/// definition of a name).
fn template_defs(ui_xml: &[(String, String)]) -> std::collections::HashMap<String, Template> {
    let mut out = std::collections::HashMap::new();
    for (_, text) in ui_xml {
        let mut reader = Reader::from_str(text);
        loop {
            match reader.read_event() {
                Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                    let a = attributes(e);
                    if !(a.virtual_ || a.intrinsic) {
                        continue;
                    }
                    let Some(name) = a.name.filter(|n| !n.contains('$')) else {
                        continue;
                    };
                    let local = e.local_name();
                    let tag: &str = local.as_ref();
                    out.entry(name).or_insert_with(|| Template {
                        tag: tag.to_string(),
                        mixins: a.mixins,
                        inherits: a.inherits,
                        intrinsic: a.intrinsic,
                    });
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
    }
    out
}

/// The templates (`virtual="true"`) and intrinsic frame types the UI
/// code's XML defines, by name: what `CreateFrame` and `inherits` may name.
pub fn templates(ui_xml: &[(String, String)]) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for (_, text) in ui_xml {
        let mut reader = Reader::from_str(text);
        loop {
            match reader.read_event() {
                Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                    let mut name = None;
                    let mut template = false;
                    for a in e.attributes().flatten() {
                        let key = a.key.local_name();
                        let key: &str = key.as_ref();
                        let value = a.value.to_string();
                        match key {
                            "name" => name = Some(value),
                            "virtual" | "intrinsic" => {
                                template |= value.eq_ignore_ascii_case("true");
                            }
                            _ => {}
                        }
                    }
                    if let Some(n) = name.filter(|n| template && !n.contains('$')) {
                        out.insert(n);
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
    }
    out.into_iter().collect()
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
        let xml = r#"<Ui><Frame name="MyFrame"><Frames><Button name="$parentClose"/></Frames></Frame><Frame name="Tpl" virtual="true"/><Font name="GameFontNormal" virtual="true"/><Button name="Btn"/><Button name="Intrinsic" intrinsic="true"/></Ui>"#;
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
        assert!(!g.contains_key("Intrinsic"));
        assert_eq!(
            global_strings(
                "_G[\"OKAY\"] = \"Okay\";\n_G[\"1X\"] = \"\";\nUNKNOWN = \"Unknown\";\n"
            ),
            ["OKAY", "UNKNOWN"]
        );
    }

    /// A named frame is typed by its mixins (its own, its templates', its
    /// intrinsic type's) and its widget type; the mixins become classes
    /// with the functions their Lua defines.
    #[test]
    fn frames_typed_by_their_mixins() {
        let lua = "FooMixin = {}\nfunction FooMixin:Bar(a, b) end\nBazMixin = CreateFromMixins(FooMixin, OtherMixin)\nfunction BazMixin:Qux(...) end\nfunction BazMixin.Make(x) end\nOtherMixin = {}\nfunction OtherMixin:Other() end\nIntrMixin = {}\nfunction IntrMixin:AddLine(text) end\nlocal LocalMixin = {}\nfunction LocalMixin:Hidden() end\n";
        let xml = r#"<Ui><Frame name="Tpl" virtual="true" mixin="FooMixin"/><Frame name="Intr" intrinsic="true" mixin="IntrMixin"/><Button name="Mixed" inherits="Tpl" mixin="BazMixin, UnknownMixin"/><Intr name="ViaIntrinsic"/><Frame name="Plain"/><Frame name="Twice" mixin="FooMixin"/></Ui>"#;
        let other = r#"<Ui><Frame name="Twice" mixin="BazMixin"/></Ui>"#;
        let classes: HashSet<String> = ["Button".to_string(), "Frame".to_string()]
            .into_iter()
            .collect();
        let ui = ui_globals(
            &[("a.lua".into(), lua.into())],
            &[("a.xml".into(), xml.into()), ("b.xml".into(), other.into())],
            &classes,
        );
        let g = &ui.globals;
        assert_eq!(g["Mixed"], "BazMixin & FooMixin & Button");
        assert_eq!(g["ViaIntrinsic"], "IntrMixin & Frame");
        assert_eq!(g["Plain"], "Frame");
        // Defined differently in two files: the mixins both have.
        assert_eq!(g["Twice"], "Frame");
        assert_eq!(g["FooMixin"], "table");
        let names: Vec<&String> = ui.mixins.keys().collect();
        // every mixin with functions (the generator keeps those referred to)
        assert_eq!(names, ["BazMixin", "FooMixin", "IntrMixin", "OtherMixin"]);
        let baz = &ui.mixins["BazMixin"];
        assert_eq!(baz.bases, ["FooMixin"]);
        let methods: Vec<String> = baz
            .methods
            .iter()
            .map(|(n, s)| format!("{n}{}", crate::model::sig_text(&s[0])))
            .collect();
        // Its own, then those of the bases after the first.
        assert_eq!(methods, ["Qux(...: any): (...any)", "Other(): (...any)"]);
        assert_eq!(
            baz.fields["Make"],
            ("(x: any) -> ...any".to_string(), false)
        );
        let foo = &ui.mixins["FooMixin"].methods["Bar"][0];
        assert_eq!(crate::model::sig_text(foo), "(a: any, b: any): (...any)");
    }

    /// The Lua of the XML: an inline `<Script>` is a chunk of top-level
    /// code, a named frame's `OnLoad` runs once with `self` the frame
    /// (`GENERAL_CHAT_DOCK = self`); templates' handlers make no globals.
    #[test]
    fn globals_of_the_xml_scripts() {
        let xml = r#"<Ui><Script>
		if (LOCALE_koKR) then
			STANDARD_TEXT_FONT = "Fonts\\2002.TTF";
		else
			STANDARD_TEXT_FONT = "Fonts\\FRIZQT__.TTF";
		end
		local hidden = 1
		CMP = 1 &lt; 2
	</Script><Script file="Other.lua"/><Frame name="Dock" mixin="DockMixin"><Scripts><OnLoad>
		DockMixin.OnLoad(self);
		GENERAL_CHAT_DOCK = self;
		local list = self.list;
		COUNT = 3;
	</OnLoad><OnShow>SHOWN = self</OnShow></Scripts></Frame><Frame name="Tpl" virtual="true"><Scripts><OnLoad>TEMPLATE_GLOBAL = self</OnLoad></Scripts></Frame></Ui>"#;
        let lua = "DockMixin = {}\nfunction DockMixin:OnLoad() end\n";
        let classes: HashSet<String> = ["Frame".to_string()].into_iter().collect();
        let g = globals(
            &[("a.lua".into(), lua.into())],
            &[("a.xml".into(), xml.into())],
            &classes,
        );
        assert_eq!(g["STANDARD_TEXT_FONT"], "string");
        // (`&lt;` read as `<`)
        assert_eq!(g["CMP"], "unknown");
        assert_eq!(g["GENERAL_CHAT_DOCK"], "alias:Dock");
        assert_eq!(g["COUNT"], "number");
        assert_eq!(g["Dock"], "DockMixin & Frame");
        for n in ["hidden", "list", "SHOWN", "TEMPLATE_GLOBAL", "self"] {
            assert!(!g.contains_key(n), "{n}: {g:?}");
        }
        // what the code reads, outside comments and strings
        let read = names_read(
            &[("a.lua".into(), "-- LE_COMMENT\nif IsInGroup(LE_PARTY_CATEGORY_INSTANCE) then print(\"LE_STRING\") end\n".into())],
            &[("a.xml".into(), "<Ui><Script>x = LE_IN_SCRIPT</Script></Ui>".into())],
        );
        for n in ["IsInGroup", "LE_PARTY_CATEGORY_INSTANCE", "LE_IN_SCRIPT"] {
            assert!(read.contains(n), "{n}");
        }
        assert!(!read.contains("LE_COMMENT") && !read.contains("LE_STRING"));
    }

    #[test]
    fn tooltip_accessors_of_the_data_handler() {
        let lua = "do\n\tlocal accessors = {\n\t\tSetInboxItem = \"GetInboxItem\",\n\t\tSetSendMailItem = \"GetSendMailItem\",\n\t};\n\tfor a, g in pairs(accessors) do end\nend\n";
        assert_eq!(
            tooltip_accessors(&[("TooltipDataHandler.lua".into(), lua.into())]),
            [
                ("SetInboxItem".to_string(), "GetInboxItem".to_string()),
                ("SetSendMailItem".to_string(), "GetSendMailItem".to_string()),
            ]
        );
    }
}
