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

fn answer_table(text: &'static str) -> HashMap<&'static str, &'static str> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(c, a)| (unescape(c), unescape(a)))
        .collect()
}

/// The answers the same on every machine.
pub(crate) fn answers() -> &'static HashMap<&'static str, &'static str> {
    static ANSWERS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    ANSWERS.get_or_init(|| answer_table(include_str!("../data/camelot-answers.txt")))
}

/// The answers that differ by machine, the platform's (decision T236).
fn platform_answers(platform: crate::Platform) -> &'static HashMap<&'static str, &'static str> {
    static MAC: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static WINDOWS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    match platform {
        crate::Platform::Mac => {
            MAC.get_or_init(|| answer_table(include_str!("../data/camelot-answers-mac.txt")))
        }
        crate::Platform::Windows => WINDOWS
            .get_or_init(|| answer_table(include_str!("../data/camelot-answers-windows.txt"))),
    }
}

/// A call's recorded answer on this VM's machine: the platform's, else the
/// one every machine gave.
pub(crate) fn answer(vm: &Vm, call: &str) -> Option<&'static str> {
    platform_answers(engine(vm).platform)
        .get(call)
        .or_else(|| answers().get(call))
        .copied()
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
        .chain(platform_answers(crate::Platform::Mac).keys())
        .chain(platform_answers(crate::Platform::Windows).keys())
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
    // A call the recording lacks: the client's answer with nothing there
    // (an unknown chat type's index 0, the full probe).
    if let Some(answer) = crate::blanks::answer(&name) {
        return Ok(parse_answer(vm, answer));
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
    let answer = answer(vm, call)?;
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
        .find_map(|call| answer(vm, call))?;
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
    // Arguments past those the function takes: the client ignores them
    // (the full probe: `IsPartyLFG("player")`, `AntiAliasingSupported(1,
    // 1)` answered as without them, every function taking none).
    if let Some(n) = arity(vm, name)
        && args.len() > n
    {
        args.truncate(n);
        out.push(format!("{name}({})", args.join(", ")));
    }
    // A number with a fraction as its whole part, as the client takes an
    // integer (the full probe: `GetCombatRating(1.5)`,
    // `C_SpellBook.GetSpellBookItemInfo(1.5, 0)` answered as with 1).
    let whole: Vec<String> = args
        .iter()
        .map(|a| match a.parse::<f64>() {
            Ok(n) if n.fract() != 0.0 && n.is_finite() => format!("{}", n.trunc() as i64),
            _ => (*a).to_string(),
        })
        .collect();
    if whole.iter().zip(&args).any(|(w, a)| w != a) {
        out.push(format!("{name}({})", whole.join(", ")));
    }
    // An enumeration's value that is no number read as 0, as the client's
    // bindings take it (the full probe: `C_CombatAudioAlert.
    // GetCategoryVolume("x")`, `C_TradeSkillUI.GetProfessionSkillLineID("x")`,
    // `C_Item.GetWeaponEnchantInfo("x")` answered as with 0).
    if let Some(kinds) = enum_arguments(vm, name) {
        let read: Vec<String> = whole
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if !kinds.get(i).copied().unwrap_or(false) || a == "nil" || a.parse::<f64>().is_ok()
                {
                    return a.clone();
                }
                let quoted = a.strip_prefix('"').and_then(|s| s.strip_suffix('"'));
                match quoted.and_then(|s| s.trim().parse::<f64>().ok()) {
                    Some(n) => format!("{}", n.trunc() as i64),
                    None => "0".to_string(),
                }
            })
            .collect();
        if read != whole {
            out.push(format!("{name}({})", read.join(", ")));
        }
    }
    out
}

/// Which of a documented function's arguments are enumerations' values.
fn enum_arguments(vm: &Vm, name: &str) -> Option<&'static [bool]> {
    static ENUMS: OnceLock<HashMap<&'static str, Vec<bool>>> = OnceLock::new();
    let engine = crate::Engine::of(vm)?;
    // The recorded answers are camelot's: its documentation's.
    if engine.flavor != "camelot" {
        return None;
    }
    let api = engine.api;
    let all = ENUMS.get_or_init(|| {
        api.functions
            .iter()
            .filter_map(|f| {
                let kinds: Vec<bool> = f
                    .doc
                    .get("Arguments")
                    .and_then(serde_json::Value::as_array)?
                    .iter()
                    .map(|a| {
                        a.get("Type")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|t| api.enum_names.contains(t))
                    })
                    .collect();
                kinds.contains(&true).then_some((f.name, kinds))
            })
            .collect()
    });
    all.get(name).map(Vec::as_slice)
}

/// How many arguments a function with recorded answers takes: its
/// documentation's, else none when every recorded call of it has none
/// (the answers battery asks a function with arguments with them) or
/// every one, without and with arguments, got the same answer
/// (`GetCategoryList()` and `GetCategoryList(true)`: the full probe's
/// `GetCategoryList(1)` answered as they).
fn arity(vm: &Vm, name: &str) -> Option<usize> {
    static ARGLESS: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
    let argless = ARGLESS.get_or_init(|| {
        // On each machine (its answers over the shared ones), each
        // function's argless call and whether its calls with arguments all
        // answered as it; argless where both machines say so.
        let on = |platform: crate::Platform| {
            let mut all: HashMap<&'static str, &'static str> = answers().clone();
            all.extend(platform_answers(platform).iter().map(|(k, v)| (*k, *v)));
            let mut answer: HashMap<&'static str, &'static str> = HashMap::new();
            for (&call, &a) in &all {
                if let Some(f) = call.strip_suffix("()") {
                    answer.insert(f, a);
                }
            }
            let mut with_args = std::collections::HashSet::new();
            for (&call, &a) in &all {
                if call.ends_with("()") {
                    continue;
                }
                if let Some((f, _)) = call.split_once('(')
                    && answer.get(f) != Some(&a)
                {
                    with_args.insert(f);
                }
            }
            answer
                .into_keys()
                .filter(|f| !with_args.contains(f))
                .collect::<std::collections::HashSet<&'static str>>()
        };
        let mac = on(crate::Platform::Mac);
        let windows = on(crate::Platform::Windows);
        mac.intersection(&windows).copied().collect()
    });
    // The recorded answers are camelot's: its documentation's arities.
    static DOCUMENTED: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    let engine = crate::Engine::of(vm)?;
    if engine.flavor != "camelot" {
        return None;
    }
    let api = engine.api;
    let documented = DOCUMENTED.get_or_init(|| {
        api.functions
            .iter()
            .map(|f| {
                let n = f
                    .doc
                    .get("Arguments")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                (f.name, n)
            })
            .collect()
    });
    if let Some(&n) = documented.get(name) {
        return Some(n);
    }
    if argless.contains(name) {
        return Some(0);
    }
    // Else as many as the most any recorded call passed (the full probe:
    // `GetChatWindowInfo(1, 1)` answered as the recorded `(1)`).
    static MOST: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    MOST.get_or_init(|| {
        let mut most: HashMap<&'static str, usize> = HashMap::new();
        for call in answers().keys() {
            let Some((f, rest)) = call.split_once('(') else {
                continue;
            };
            let inner = rest.strip_suffix(')').unwrap_or(rest);
            let n = if inner.is_empty() {
                0
            } else {
                inner.split(", ").count()
            };
            let m = most.entry(f).or_insert(0);
            *m = (*m).max(n);
        }
        most
    })
    .get(name)
    .copied()
}
