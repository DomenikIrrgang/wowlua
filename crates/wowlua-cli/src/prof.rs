//! `wowlua prof report` (spec Tooling §16.5, decisions T175 and T179): the
//! recordings the dev window exported into `WowluaDevDB_<Addon>`
//! (Debugging §7.7.9), read from the SavedVariables of the WoW
//! installation, as one self-contained HTML page in the website's style.

use crate::ProfReportArgs;
use crate::commands::{cwd, load_options, load_project};
use crate::output::Output;
use crate::{GlobalArgs, commands};
use serde_json::Value;
use std::path::Path;
use wowlua_diagnostics::Diagnostic;
use wowlua_driver::recordings::{Found, db_name, find, map_locations};
use wowlua_driver::{LoadOptions, Session, install};

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
    let (mut found, warnings) = find(&wow, &db, args.client.as_deref(), args.account.as_deref());
    for w in warnings {
        session.push(Diagnostic::plain_warning(w));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_dates() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(civil(20_752), (2026, 10, 26));
    }
}
