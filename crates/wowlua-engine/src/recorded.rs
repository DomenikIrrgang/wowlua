//! Functions that answer from the client's own data (chat types, game
//! rules, class colors, chat windows…): the client's recorded answers
//! (`/stdtest engine answers`, `data/camelot-answers.txt` from `cargo xtask
//! ingest`), looked up by the call as the engine writes it
//! (`note_default`'s form). A call never recorded answers nothing.

use std::collections::HashMap;
use std::sync::OnceLock;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;

fn answers() -> &'static HashMap<&'static str, &'static str> {
    static ANSWERS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    ANSWERS.get_or_init(|| {
        include_str!("../data/camelot-answers.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .map(|(c, a)| (unescape(c), unescape(a)))
            .collect()
    })
}

/// The trainers' recorded answers (`/stdtest engine trainer`,
/// `data/camelot-trainers.txt`): by call, any trainer's.
fn trainer_answer(call: &str) -> Option<&'static str> {
    static BY_CALL: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    BY_CALL
        .get_or_init(|| {
            include_str!("../data/camelot-trainers.txt")
                .lines()
                .filter(|l| !l.starts_with('#'))
                .filter_map(|l| {
                    let (_, rest) = l.split_once('\t')?;
                    let (call, answer) = rest.split_once('\t')?;
                    Some((unescape(call), unescape(answer)))
                })
                .collect()
        })
        .get(call)
        .copied()
}

/// A call the probes of game data recorded (a profession window's, a
/// class's trees', a trainer's): its answer, any character's.
pub fn probed(call: &str) -> Option<&'static str> {
    crate::tradeskill::any_answer(call)
        .or_else(|| crate::traits::owner_answer(call).copied())
        .or_else(|| trainer_answer(call))
}

/// A data file's field as written (`cargo xtask ingest`): its line breaks,
/// tabs and carriage returns kept as their symbols (␤ ␉ ␍) back.
pub fn unescape(field: &'static str) -> &'static str {
    if !field.contains(['\u{2424}', '\u{2409}', '\u{240D}']) {
        return field;
    }
    let text = field
        .replace('\u{2424}', "\n")
        .replace('\u{2409}', "\t")
        .replace('\u{240D}', "\r");
    Box::leak(text.into_boxed_str())
}

/// The functions with recorded answers.
pub fn functions() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = answers()
        .keys()
        .filter_map(|call| call.split('(').next())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// A recorded value: `string text`, `number 1`, `boolean true`, `nil nil`,
/// or a table `{key=value,…}` (a color, with `ColorMixin`'s methods, made
/// with `CreateColor`; a vector, with `GetXY`, made with `CreateVector2D`
/// or `CreateVector3D`).
fn parse_value(vm: &mut Vm, text: &str) -> Value {
    // Not trimmed at the end: `string ` is the empty string.
    let text = text.trim_start();
    if let Some(inner) = text.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
        let t = vm.new_table();
        let mut color = false;
        let mut vector = false;
        for entry in split_top(inner, ',') {
            let Some((k, v)) = entry.split_once('=') else {
                continue;
            };
            if v.starts_with("function") {
                color |= k == "GetRGB";
                vector |= k == "GetXY";
                continue;
            }
            let key = match k.parse::<f64>() {
                Ok(n) => Value::Num(n),
                Err(_) => vm.str(k),
            };
            let value = parse_value(vm, v);
            let _ = vm.heap.table_mut(t).set(key, value);
        }
        if vector {
            let get = |vm: &mut Vm, k: &str| {
                let key = vm.str(k);
                vm.heap.table(t).get(key)
            };
            let z = get(vm, "z");
            let (create, args) = if z.is_nil() {
                ("CreateVector2D", vec![get(vm, "x"), get(vm, "y")])
            } else {
                ("CreateVector3D", vec![get(vm, "x"), get(vm, "y"), z])
            };
            let create = vm.get_global(create);
            if !create.is_nil()
                && let Ok(r) = vm.call(create, &args)
                && let Some(v @ Value::Table(_)) = r.first().copied()
            {
                return v;
            }
        }
        if color {
            let get = |vm: &mut Vm, k: &str| {
                let key = vm.str(k);
                vm.heap.table(t).get(key)
            };
            let rgba = [get(vm, "r"), get(vm, "g"), get(vm, "b"), get(vm, "a")];
            let create = vm.get_global("CreateColor");
            if !create.is_nil()
                && let Ok(r) = vm.call(create, &rgba)
                && let Some(Value::Table(color)) = r.first().copied()
            {
                // A color recorded without alpha has none (`CreateColor`
                // may default it).
                if rgba[3].is_nil() {
                    let a = vm.str("a");
                    let _ = vm.heap.table_mut(color).set(a, Value::Nil);
                }
                return Value::Table(color);
            }
        }
        return Value::Table(t);
    }
    match text.split_once(' ') {
        Some(("string", s)) => vm.str(s),
        Some(("number", n)) => n.parse().map_or(Value::Nil, Value::Num),
        Some(("boolean", b)) => Value::Bool(b == "true"),
        _ => Value::Nil,
    }
}

/// `text` (a table's entries, `key=value`) split at `sep` outside braces
/// where the next entry's key follows: a string value may hold the
/// separator ("Sentinel Hill, Westfall", a flight point's name).
pub(crate) fn split_top(text: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            c if c == sep && depth == 0 && key_follows(&text[i + c.len_utf8()..]) => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Whether `text` starts with a table key and its `=` (a name or a number).
fn key_follows(text: &str) -> bool {
    let key = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'))
        .unwrap_or(text.len());
    key > 0 && text[key..].starts_with('=')
}

/// An answer's values: `n: v1, v2, …` (a string may hold ", ": values
/// start with their type).
pub(crate) fn parse_answer(vm: &mut Vm, answer: &str) -> Vec<Value> {
    let Some((count, rest)) = answer.split_once(": ") else {
        return Vec::new();
    };
    if count == "0" {
        return Vec::new();
    }
    let starts = regex_starts(rest);
    let mut values = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(rest.len(), |&e| e - 2);
        values.push(parse_value(vm, &rest[s..end]));
    }
    values
}

/// Where the values of an answer start: at 0 and after each top-level
/// ", " followed by a type (or a table).
pub(crate) fn regex_starts(text: &str) -> Vec<usize> {
    const TYPES: &[&str] = &["string ", "number ", "boolean ", "nil ", "secret ", "{"];
    let mut out = vec![0];
    let mut depth = 0i32;
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b',' if depth == 0
                && bytes.get(i + 1) == Some(&b' ')
                && TYPES.iter().any(|t| text[i + 2..].starts_with(t)) =>
            {
                out.push(i + 2)
            }
            _ => {}
        }
    }
    out
}

/// The recorded functions' behaviour (camelot's data).
pub fn behaviours(flavor: &str) -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    if flavor != "camelot" {
        return Vec::new();
    }
    functions()
        .into_iter()
        .map(|name| (name, recorded as wowlua_vm::NativeFn))
        .collect()
}

/// The behaviour of a function with recorded answers; a call never
/// recorded gets what an unanswered call gets (a documented function's
/// empty values, an undocumented one's nothing), noted as unanswered.
pub fn recorded(vm: &mut Vm, c: NativeCall) -> Ret {
    let (name, documented) = match c.upvals.first().copied() {
        Some(Value::Num(i)) => {
            let api = engine(vm).api;
            match api.functions.get(i as usize) {
                Some(f) => (f.name.to_string(), Some(&f.doc)),
                None => return Ok(vec![]),
            }
        }
        Some(Value::Str(s)) => (String::from_utf8_lossy(vm.str_bytes(s)).into_owned(), None),
        _ => return Ok(vec![]),
    };
    if let Some(values) = lookup(vm, &name, &c) {
        return Ok(values);
    }
    crate::note_default(vm, &name, &c, 1);
    Ok(match documented {
        Some(doc) => {
            let api = engine(vm).api;
            crate::defaults(vm, api, doc)
        }
        None => Vec::new(),
    })
}

/// The name of the API function called (from its upvalue: its index in
/// the API, or its name).
pub fn called_name(vm: &Vm, c: &NativeCall) -> Option<String> {
    match c.upvals.first().copied() {
        Some(Value::Num(i)) => engine(vm)
            .api
            .functions
            .get(i as usize)
            .map(|f| f.name.to_string()),
        Some(Value::Str(s)) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// Game data the test did not define, as the client gave it for this call
/// (`/stdtest engine answers`); else nothing, noted as unanswered when
/// Blizzard's code asked.
pub fn client_answer(vm: &mut Vm, c: &NativeCall) -> Vec<Value> {
    let Some(name) = called_name(vm, c) else {
        return Vec::new();
    };
    if let Some(values) = lookup(vm, &name, c) {
        return values;
    }
    if crate::protection::blizzard_caller(vm) {
        crate::note_default(vm, &name, c, 1);
    }
    Vec::new()
}

/// The recorded answer to a call written as the engine writes calls
/// (`C_X.F(1)`), if there is one.
pub fn lookup_text(vm: &mut Vm, call: &str) -> Option<Vec<Value>> {
    let answer = answers().get(call)?;
    Some(parse_answer(vm, answer))
}

/// The recorded answer to this call of `name`, if there is one.
///
/// A call the recording lacks is asked again without its trailing tables
/// and nils (Blizzard's code passes a transmog location or a nil
/// variation that the probes leave out).
pub fn lookup(vm: &mut Vm, name: &str, c: &NativeCall) -> Option<Vec<Value>> {
    let answer = call_variants(vm, name, c)
        .iter()
        .find_map(|call| answers().get(call.as_str()))?;
    Some(parse_answer(vm, answer))
}

/// A call as the engine writes it, then without its trailing tables and
/// nils one by one: how a recording may hold it (Blizzard's code passes a
/// transmog location, a nil recipe level, that the probes leave out).
pub fn call_variants(vm: &Vm, name: &str, c: &NativeCall) -> Vec<String> {
    let call = crate::call_text(vm, name, c, 1);
    let mut out = vec![call.clone()];
    let Some(inner) = call
        .strip_prefix(name)
        .and_then(|r| r.strip_prefix('('))
        .and_then(|r| r.strip_suffix(')'))
    else {
        return out;
    };
    let mut args: Vec<&str> = inner.split(", ").filter(|a| !a.is_empty()).collect();
    while matches!(args.last(), Some(&"nil" | &"{}")) {
        args.pop();
        out.push(format!("{name}({})", args.join(", ")));
    }
    out
}
