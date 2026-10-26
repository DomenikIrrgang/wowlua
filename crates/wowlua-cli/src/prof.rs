//! `wowlua prof report` (spec Tooling §16.5, decisions T175 and T179): the
//! recordings the dev window exported into `WowluaDevDB_<Addon>`
//! (Debugging §7.7.9), read from the SavedVariables of the WoW
//! installation, as one self-contained HTML page in the website's style.

use crate::ProfReportArgs;
use crate::commands::{cwd, load_options, load_project};
use crate::output::Output;
use crate::{GlobalArgs, commands};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_driver::{LoadOptions, Project, Session, install};

/// One SavedVariables file with exported recordings.
struct Found {
    client: String,
    account: String,
    path: PathBuf,
    exports: Vec<Value>,
}

/// The SavedVariables name of an addon's dev data (as dev builds add it to
/// the TOC).
fn db_name(addon: &str) -> String {
    let safe: String = addon
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("WowluaDevDB_{safe}")
}

fn folder_matches(folder: &str, wanted: &str) -> bool {
    folder
        .trim_matches('_')
        .eq_ignore_ascii_case(wanted.trim_matches('_'))
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every client folder and account of the installation with exported
/// recordings of `db` (the `--client` and `--account` given only).
fn find(
    session: &mut Session,
    wow: &Path,
    db: &str,
    client: Option<&str>,
    account: Option<&str>,
) -> Vec<Found> {
    let mut out = Vec::new();
    for folder in sorted_dirs(wow) {
        let client_name = file_name(&folder);
        if client.is_some_and(|c| !folder_matches(&client_name, c)) {
            continue;
        }
        for dir in sorted_dirs(&folder.join("WTF").join("Account")) {
            let account_name = file_name(&dir);
            if account_name.eq_ignore_ascii_case("SavedVariables")
                || account.is_some_and(|a| !a.eq_ignore_ascii_case(&account_name))
            {
                continue;
            }
            let path = dir.join("SavedVariables").join(format!("{db}.lua"));
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let value = match parse_saved(&bytes) {
                Ok(vars) => vars.into_iter().find(|(n, _)| n == db).map(|(_, v)| v),
                Err(e) => {
                    session.push(Diagnostic::plain_warning(format!(
                        "cannot read `{}`: {e}",
                        path.display()
                    )));
                    continue;
                }
            };
            let exports: Vec<Value> = value
                .as_ref()
                .and_then(|v| v.get("exports"))
                .map(list)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.get("format").and_then(Value::as_f64) == Some(1.0))
                .collect();
            if !exports.is_empty() {
                out.push(Found {
                    client: client_name.clone(),
                    account: account_name,
                    path,
                    exports,
                });
            }
        }
    }
    out
}

/// The items of a Lua array (an array, or an object of numbered keys).
fn list(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.iter().filter(|v| !v.is_null()).cloned().collect(),
        Value::Object(o) => o.values().cloned().collect(),
        _ => Vec::new(),
    }
}

fn describe(found: &Found) -> String {
    let names: Vec<String> = found
        .exports
        .iter()
        .filter_map(|e| e.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    format!(
        "{}  {}  {} ({})",
        found.client,
        found.account,
        commands::plural(found.exports.len(), "recording", "recordings"),
        names.join(", ")
    )
}

/// Asks which of several places to read, on a terminal.
fn ask(found: &[Found]) -> Option<usize> {
    use std::io::Write;
    eprintln!("Several accounts or clients have exported recordings:");
    for (i, f) in found.iter().enumerate() {
        eprintln!("  {}  {}", i + 1, describe(f));
    }
    loop {
        eprint!("Which one? [1-{}]: ", found.len());
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).ok()? == 0 {
            return None;
        }
        match line.trim().parse::<usize>() {
            Ok(n) if (1..=found.len()).contains(&n) => return Some(n - 1),
            _ => continue,
        }
    }
}

/// `wowlua prof report`.
pub fn report(out: &Output, global: &GlobalArgs, args: &ProfReportArgs) -> bool {
    let mut session = Session::new();
    let Some(project) = load_project(
        global,
        &mut session,
        &LoadOptions {
            skip_dependencies: true,
            ..load_options(out, global)
        },
    ) else {
        out.flush(&mut session);
        return false;
    };
    let addon = project.addon_name();
    let db = db_name(&addon);
    let wow = match install::find_wow(args.wow.as_ref().map(|w| cwd().join(w)).as_deref()) {
        Ok(wow) => wow,
        Err(d) => {
            session.push(d);
            out.flush(&mut session);
            return false;
        }
    };
    let mut found = find(
        &mut session,
        &wow,
        &db,
        args.client.as_deref(),
        args.account.as_deref(),
    );
    let chosen = match found.len() {
        0 => {
            let mut where_ = String::new();
            if let Some(c) = &args.client {
                where_.push_str(&format!(" client folder `{c}`"));
            }
            if let Some(a) = &args.account {
                where_.push_str(&format!(" account `{a}`"));
            }
            session.push(
                Diagnostic::plain_error(format!(
                    "no exported recordings of {addon} in `{}`{}",
                    wow.display(),
                    if where_.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", where_.trim())
                    }
                ))
                .with_note(format!(
                    "they are in `<client>/WTF/Account/<account>/SavedVariables/{db}.lua`"
                ))
                .with_help(
                    "in the game, right-click a recording in the dev window's performance tab, \
                     choose Mark for export, then /reload",
                ),
            );
            out.flush(&mut session);
            return false;
        }
        1 => 0,
        _ => {
            use std::io::IsTerminal;
            if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
                match ask(&found) {
                    Some(i) => i,
                    None => {
                        session.push(Diagnostic::plain_error("no account or client chosen"));
                        out.flush(&mut session);
                        return false;
                    }
                }
            } else {
                let mut d = Diagnostic::plain_error(format!(
                    "several accounts or clients have exported recordings of {addon}"
                ));
                for f in &found {
                    d = d.with_note(describe(f));
                }
                session.push(
                    d.with_help("choose one with `--client <folder>` and `--account <name>`"),
                );
                out.flush(&mut session);
                return false;
            }
        }
    };
    let source = found.swap_remove(chosen);
    let mut recordings = source.exports.clone();
    if !args.recording.is_empty() {
        let names: Vec<&str> = recordings
            .iter()
            .filter_map(|r| r.get("name").and_then(Value::as_str))
            .collect();
        let missing: Vec<&String> = args
            .recording
            .iter()
            .filter(|n| !names.contains(&n.as_str()))
            .collect();
        if !missing.is_empty() {
            let quoted: Vec<String> = missing.iter().map(|n| format!("`{n}`")).collect();
            session.push(
                Diagnostic::plain_error(format!(
                    "no exported recording named {}",
                    quoted.join(", ")
                ))
                .with_note(format!("exported: {}", names.join(", "))),
            );
            out.flush(&mut session);
            return false;
        }
        recordings.retain(|r| {
            r.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| args.recording.iter().any(|w| w == n))
        });
    }
    for rec in &mut recordings {
        map_locations(&project, rec);
    }
    let date = today();
    let path = match &args.out {
        Some(p) => cwd().join(p),
        None => project
            .manifest
            .out_dir()
            .join("reports")
            .join(format!("{addon}-{date}.html")),
    };
    let page = html(
        &addon,
        &date,
        &source.client,
        &source.account,
        &source.path,
        &recordings,
    );
    if let Err(e) = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, page))
    {
        session.push(Diagnostic::plain_error(format!(
            "cannot write `{}`: {e}",
            path.display()
        )));
        out.flush(&mut session);
        return false;
    }
    if !out.flush(&mut session) {
        return false;
    }
    out.status(
        "Read",
        format!(
            "{} of {addon} from {} ({}, {})",
            commands::plural(recordings.len(), "recording", "recordings"),
            source.path.display(),
            source.client,
            source.account
        ),
    );
    out.status("Wrote", commands::relative(&path));
    if !args.no_open {
        match open(&path) {
            Ok(()) => out.status("Opened", "the report in the browser"),
            Err(e) => out.status("Note", format!("cannot open the browser ({e})")),
        }
    }
    true
}

/// Opens a file in the default browser.
fn open(path: &Path) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Today as `YYYY-MM-DD` (UTC).
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 → (year, month, day).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

// ------------------------------------------------------------ locations --

/// The recording's game locations mapped to source lines as `wowlua trace`
/// does, with the build id it was recorded with: the slow calls' stacks
/// (each line followed by its `→ src/x.wowlua:12 in name`), and locations
/// of built files.
fn map_locations(project: &Project, rec: &mut Value) {
    let build = rec.get("build").and_then(Value::as_str).map(str::to_string);
    // Every text to map, by where it goes.
    let mut texts: Vec<String> = Vec::new();
    let mut slots: Vec<(&'static str, usize, &'static str)> = Vec::new();
    if let Some(Value::Array(slow)) = rec.get("slow") {
        for (i, s) in slow.iter().enumerate() {
            for field in ["stack", "location"] {
                if let Some(t) = s.get(field).and_then(Value::as_str)
                    && !t.is_empty()
                {
                    texts.push(t.to_string());
                    slots.push(("slow", i, field));
                }
            }
        }
    }
    if texts.is_empty() {
        return;
    }
    const SEP: &str = "\u{1}wowlua-prof-report\u{1}";
    let input = texts.join(&format!("\n{SEP}\n"));
    let decoded = wowlua_driver::trace::decode(project, &input, build.as_deref());
    let parts: Vec<String> = decoded
        .split(&format!("{SEP}\n"))
        .map(|p| p.trim_end_matches('\n').to_string())
        .collect();
    if parts.len() != texts.len() {
        return;
    }
    for ((list, i, field), text) in slots.into_iter().zip(parts) {
        let Some(entry) = rec
            .get_mut(list)
            .and_then(|l| l.get_mut(i))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        if field == "location" {
            // A built file's location: its source line.
            if let Some(mapped) = text.lines().find_map(|l| l.trim().strip_prefix("→ ")) {
                let place = mapped.split(" in ").next().unwrap_or(mapped);
                entry.insert("location".into(), Value::String(place.to_string()));
            }
        } else {
            entry.insert(field.into(), Value::String(text));
        }
    }
}

// ----------------------------------------------------------------- page --

const STYLE: &str = include_str!("prof_html/style.css");
const APP: &str = include_str!("prof_html/app.js");
/// wowlua's mark as the tab's icon, in the dark or light colours.
const ICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 48 48'%3E%3Cstyle%3Epath%7Bstroke:%23ece6d6%7Dg%7Bfill:%23ffb547%7D@media (prefers-color-scheme:light)%7Bpath%7Bstroke:%231a1813%7Dg%7Bfill:%23975200%7D%7D%3C/style%3E%3Cpath d='M9.5 5H4V43H9.5M38.5 5H44V43H38.5' fill='none' stroke-width='3.5' stroke-linecap='square'/%3E%3Cg%3E%3Cpath style='stroke:none' transform='translate(12.14 31.63)' d='M0.36 -11.16H3.33L3.73 -6.48L3.87 -2.72H4.23L4.9 -11.16H8.08L8.69 -2.72H9.05L9.18 -6.48L9.56 -11.16H12.24L11 0.0H7.11L6.52 -8.41H6.16L5.51 0.0H1.6ZM20.12 -15.26V-2.84H23.36V0.0H13.64V-2.84H16.88V-12.42H13.64V-15.26Z'/%3E%3C/g%3E%3C/svg%3E";

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The design system's fonts (SIL Open Font License), embedded so the page
/// looks the same everywhere without the network.
fn font_faces() -> String {
    let font = |file: &str| {
        wowlua_runtime::DEV_FONTS
            .iter()
            .find(|(n, _)| *n == file)
            .map(|(_, b)| base64(b))
            .unwrap_or_default()
    };
    [
        ("Martian Mono", 600, "MartianMono-StdBd.ttf"),
        ("IBM Plex Mono", 400, "IBMPlexMono-Regular.ttf"),
        ("IBM Plex Mono", 600, "IBMPlexMono-SemiBold.ttf"),
    ]
    .iter()
    .map(|(family, weight, file)| {
        format!(
            "@font-face {{ font-family: \"{family}\"; font-weight: {weight}; font-style: normal; \
             src: url(data:font/ttf;base64,{}) format(\"truetype\"); }}\n",
            font(file)
        )
    })
    .collect()
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The page: the recordings as JSON, drawn by its script.
fn html(
    addon: &str,
    date: &str,
    client: &str,
    account: &str,
    path: &Path,
    recordings: &[Value],
) -> String {
    let data = serde_json::json!({
        "addon": addon,
        "date": date,
        "client": client,
        "account": account,
        "file": path.display().to_string(),
        "recordings": recordings,
    });
    // inside <script>: no `</script>` (nor `<!--`) can end it early
    let data = serde_json::to_string(&data)
        .unwrap_or_default()
        .replace('<', "\\u003c");
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{t} performance</title><link rel=\"icon\" href=\"{ICON}\"><style>{fonts}{STYLE}</style></head><body>\n<div id=\"app\"><header><h1>{t} performance</h1></header><main><p>The report needs JavaScript.</p></main></div>\n<script type=\"application/json\" id=\"data\">{data}</script>\n<script>{APP}</script>\n</body></html>\n",
        t = xml(addon),
        fonts = font_faces(),
    )
}

// ------------------------------------------------------ SavedVariables --

/// A Lua key as the client writes it.
#[derive(Debug)]
enum Key {
    Int(i64),
    Str(String),
    Other(String),
}

/// The variables of a SavedVariables file (`Name = value` statements, the
/// data part of Lua the client writes), as JSON: tables with the keys 1..n
/// are arrays (holes are null), other tables objects.
fn parse_saved(bytes: &[u8]) -> Result<Vec<(String, Value)>, String> {
    let mut p = Parser { s: bytes, i: 0 };
    let mut out = Vec::new();
    loop {
        p.skip();
        if p.i >= p.s.len() {
            return Ok(out);
        }
        let name = p.name().ok_or_else(|| p.error("a variable name"))?;
        p.skip();
        p.expect(b'=')?;
        let value = p.value()?;
        out.push((name, value));
        p.skip();
        if p.peek() == Some(b';') {
            p.i += 1;
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn line(&self) -> usize {
        1 + self.s[..self.i.min(self.s.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    }

    fn error(&self, wanted: &str) -> String {
        format!("line {}: expected {wanted}", self.line())
    }

    /// Whitespace and comments.
    fn skip(&mut self) {
        loop {
            while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"--") {
                self.i += 2;
                if let Some(level) = self.long_open() {
                    self.long_body(level);
                } else {
                    while self.peek().is_some_and(|b| b != b'\n') {
                        self.i += 1;
                    }
                }
            } else {
                return;
            }
        }
    }

    /// `[[` or `[==[` at the cursor: consumed, its level.
    fn long_open(&mut self) -> Option<usize> {
        let rest = &self.s[self.i..];
        if rest.first() != Some(&b'[') {
            return None;
        }
        let level = rest[1..].iter().take_while(|&&b| b == b'=').count();
        (rest.get(1 + level) == Some(&b'[')).then(|| {
            self.i += 2 + level;
            level
        })
    }

    /// A long bracket's text up to its closing bracket.
    fn long_body(&mut self, level: usize) -> Vec<u8> {
        let close: Vec<u8> = std::iter::once(b']')
            .chain(std::iter::repeat_n(b'=', level))
            .chain(std::iter::once(b']'))
            .collect();
        let start = self.i;
        while self.i < self.s.len() && !self.s[self.i..].starts_with(&close) {
            self.i += 1;
        }
        let mut text = self.s[start..self.i].to_vec();
        self.i = (self.i + close.len()).min(self.s.len());
        if text.first() == Some(&b'\n') {
            text.remove(0);
        }
        text
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.peek() == Some(b) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.error(&format!("`{}`", b as char)))
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.i;
        if !self
            .peek()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        {
            return None;
        }
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.i += 1;
        }
        Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip();
        match self.peek() {
            Some(b'{') => self.table(),
            Some(b'"' | b'\'') => Ok(Value::String(self.string()?)),
            Some(b'[') => match self.long_open() {
                Some(level) => Ok(Value::String(
                    String::from_utf8_lossy(&self.long_body(level)).into_owned(),
                )),
                None => Err(self.error("a value")),
            },
            Some(b'-' | b'.' | b'0'..=b'9') => self.number(),
            _ => match self.name().as_deref() {
                Some("true") => Ok(Value::Bool(true)),
                Some("false") => Ok(Value::Bool(false)),
                Some("nil" | "inf" | "nan") => Ok(Value::Null),
                _ => Err(self.error("a value")),
            },
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let negative = self.peek() == Some(b'-');
        if negative {
            self.i += 1;
            self.skip();
        }
        let start = self.i;
        while self.peek().is_some_and(|b| {
            b.is_ascii_alphanumeric()
                || b == b'.'
                || ((b == b'-' || b == b'+')
                    && matches!(self.s.get(self.i.wrapping_sub(1)), Some(b'e' | b'E')))
        }) {
            self.i += 1;
        }
        let text = String::from_utf8_lossy(&self.s[start..self.i]).to_ascii_lowercase();
        let n = if let Some(hex) = text.strip_prefix("0x") {
            i64::from_str_radix(hex, 16).ok().map(|n| n as f64)
        } else if text == "inf" || text == "nan" {
            None
        } else {
            text.parse::<f64>().ok()
        };
        let Some(n) = n else {
            // `-nan`, `inf`: no number JSON can hold.
            return if text.chars().all(|c| c.is_ascii_alphabetic()) {
                Ok(Value::Null)
            } else {
                Err(self.error("a number"))
            };
        };
        let n = if negative { -n } else { n };
        Ok(json_number(n))
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.s[self.i];
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(b) = self.peek() else {
                return Err(self.error("the end of a string"));
            };
            self.i += 1;
            if b == quote {
                break;
            }
            if b != b'\\' {
                out.push(b);
                continue;
            }
            let Some(e) = self.peek() else {
                return Err(self.error("an escape"));
            };
            self.i += 1;
            match e {
                b'n' | b'\n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(7),
                b'b' => out.push(8),
                b'f' => out.push(12),
                b'v' => out.push(11),
                b'\r' => {
                    out.push(b'\n');
                    if self.peek() == Some(b'\n') {
                        self.i += 1;
                    }
                }
                b'0'..=b'9' => {
                    let mut n = u32::from(e - b'0');
                    for _ in 0..2 {
                        match self.peek() {
                            Some(d @ b'0'..=b'9') => {
                                n = n * 10 + u32::from(d - b'0');
                                self.i += 1;
                            }
                            _ => break,
                        }
                    }
                    out.push(n.min(255) as u8);
                }
                other => out.push(other),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn table(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut entries: Vec<(Key, Value)> = Vec::new();
        let mut next = 1i64;
        loop {
            self.skip();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                None => return Err(self.error("`}`")),
                _ => {}
            }
            let save = self.i;
            let key = if self.peek() == Some(b'[') && self.long_open().is_none() {
                self.i += 1;
                let k = self.value()?;
                self.skip();
                self.expect(b']')?;
                self.skip();
                self.expect(b'=')?;
                Some(match k {
                    Value::String(s) => Key::Str(s),
                    Value::Number(n) if n.as_i64().is_some() => Key::Int(n.as_i64().unwrap_or(0)),
                    other => Key::Other(other.to_string()),
                })
            } else {
                self.i = save;
                // `name = value`, or a positional value.
                let named = self.name().and_then(|n| {
                    self.skip();
                    (self.peek() == Some(b'=') && self.s.get(self.i + 1) != Some(&b'=')).then(
                        || {
                            self.i += 1;
                            n
                        },
                    )
                });
                match named {
                    Some(n) => Some(Key::Str(n)),
                    None => {
                        self.i = save;
                        None
                    }
                }
            };
            let value = self.value()?;
            let key = key.unwrap_or_else(|| {
                next += 1;
                Key::Int(next - 1)
            });
            if !value.is_null() {
                entries.push((key, value));
            }
            self.skip();
            if matches!(self.peek(), Some(b',' | b';')) {
                self.i += 1;
            }
        }
        Ok(table_value(entries))
    }
}

fn json_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

fn table_value(entries: Vec<(Key, Value)>) -> Value {
    let ints: Option<Vec<i64>> = entries
        .iter()
        .map(|(k, _)| match k {
            Key::Int(i) if *i >= 1 => Some(*i),
            _ => None,
        })
        .collect();
    if let Some(ints) = ints
        && !entries.is_empty()
    {
        let max = ints.iter().copied().max().unwrap_or(0);
        if max as usize <= 2 * entries.len() + 8 {
            let mut items = vec![Value::Null; max as usize];
            for (k, v) in entries {
                if let Key::Int(i) = k {
                    items[i as usize - 1] = v;
                }
            }
            return Value::Array(items);
        }
    }
    let mut map = Map::new();
    for (k, v) in entries {
        let k = match k {
            Key::Int(i) => i.to_string(),
            Key::Str(s) | Key::Other(s) => s,
        };
        map.insert(k, v);
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_the_client_writes() {
        let text = br#"
WowluaDevDB_Probe = {
	["exports"] = {
		{
			["name"] = "pull 1",
			["detail"] = "Ulgrax",
			["duration"] = 12.5,
			["frames"] = 600,
			["neg"] = -3,
			["small"] = 1.5e-05,
			["stack"] = "a\nb \"c\" \\ d\
e",
			["seconds"] = {
				["frames"] = {
					60, -- [1]
					59, -- [2]
				},
			},
			["functions"] = {
			},
			[2] = true,
		}, -- [1]
	},
	["settings"] = {
		["callPaths"] = true,
	},
}
Other = "x"
"#;
        let vars = parse_saved(text).unwrap();
        assert_eq!(vars.len(), 2);
        let db = &vars[0].1;
        let rec = &db["exports"][0];
        assert_eq!(rec["name"], "pull 1");
        assert_eq!(rec["duration"], 12.5);
        assert_eq!(rec["frames"], 600);
        assert_eq!(rec["neg"], -3);
        assert_eq!(rec["small"], 1.5e-05);
        assert_eq!(rec["stack"], "a\nb \"c\" \\ d\ne");
        assert_eq!(rec["seconds"]["frames"], serde_json::json!([60, 59]));
        assert_eq!(rec["functions"], serde_json::json!({}));
        assert_eq!(rec["2"], true);
        assert_eq!(db["settings"]["callPaths"], true);
        assert_eq!(vars[1], ("Other".to_string(), Value::from("x")));
    }

    #[test]
    fn arrays_with_holes_and_errors() {
        let vars = parse_saved(b"X = { [1] = 1, [3] = 3 }").unwrap();
        assert_eq!(vars[0].1, serde_json::json!([1, null, 3]));
        let vars = parse_saved(b"X = { [1] = 1, [1000] = 3 }").unwrap();
        assert_eq!(vars[0].1, serde_json::json!({ "1": 1, "1000": 3 }));
        let err = parse_saved(b"X = {\n[\"a\"] = }").unwrap_err();
        assert!(err.starts_with("line 2"), "{err}");
    }

    #[test]
    fn names_and_base64() {
        assert_eq!(db_name("My-Addon"), "WowluaDevDB_My_Addon");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(civil(20_752), (2026, 10, 26));
    }
}
