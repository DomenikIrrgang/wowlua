//! `wowlua profile snapshot <name>` (spec Tooling §16.1.4, decision T498):
//! the newest snapshot the dev window took in the game (`/wl snapshot`,
//! Debugging §7.7.10), read from the SavedVariables of the WoW
//! installation, written as the world profile `tests/profiles/<name>.json`
//! with the character's and other players' names scrubbed.

use crate::ProfileSnapshotArgs;
use crate::commands::{cwd, load_options, load_project};
use crate::output::Output;
use crate::{GlobalArgs, commands};
use std::path::Path;
use wowlua_diagnostics::Diagnostic;
use wowlua_driver::recordings::db_name;
use wowlua_driver::snapshots::{Found, find, profile_text};
use wowlua_driver::{LoadOptions, Session, install};
use wowlua_manifest::world::PROFILES_DIR;

fn describe(found: &Found) -> String {
    let at = found
        .at
        .map(|t| format!("taken {}", crate::prof::date_time(t as i64)))
        .unwrap_or_else(|| "taken at an unknown time".into());
    format!("{}  {}  {at}", found.client, found.account)
}

/// Asks which of several snapshots to write, on a terminal.
fn ask(found: &[Found]) -> Option<usize> {
    use std::io::Write;
    eprintln!("Several accounts or clients have a snapshot:");
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

/// `wowlua profile snapshot`.
pub fn snapshot(out: &Output, global: &GlobalArgs, args: &ProfileSnapshotArgs) -> bool {
    let mut session = Session::new();
    if args.name.is_empty()
        || !args
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        session.push(
            Diagnostic::plain_error(format!("`{}` is no profile name", args.name))
                .with_help("a profile's name has letters, digits, `-` and `_`: `raid-pull`"),
        );
        out.flush(&mut session);
        return false;
    }
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
    let rel = Path::new(PROFILES_DIR).join(format!("{}.json", args.name));
    let path = project.root().join(&rel);
    if path.exists() && !args.force {
        session.push(
            Diagnostic::plain_error(format!("`{}` exists", rel.display()))
                .with_help("replace it with `--force`, or give another name"),
        );
        out.flush(&mut session);
        return false;
    }
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
            session.push(
                Diagnostic::plain_error(format!("no snapshot of {addon} in `{}`", wow.display()))
                    .with_note(format!(
                        "it is in `<client>/WTF/Account/<account>/SavedVariables/{db}.lua`"
                    ))
                    .with_help("in the game, with a dev build: /wl snapshot, then /reload"),
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
                    "several accounts or clients have a snapshot of {addon}"
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
    let text = match profile_text(&source.profile) {
        Ok((text, _)) => text,
        Err(e) => {
            session.push(Diagnostic::plain_error(format!(
                "cannot read the snapshot in `{}`: {e}",
                source.path.display()
            )));
            out.flush(&mut session);
            return false;
        }
    };
    for part in &source.failed {
        session.push(Diagnostic::plain_warning(format!(
            "the client refused a part of the snapshot: {part}"
        )));
    }
    // The profile as `wowlua check` checks it.
    let (_, found) = wowlua_manifest::world::check(&mut session.db, &rel, text.clone());
    for d in found.into_vec() {
        session.push(d);
    }
    if let Err(e) = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, &text))
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
            "the snapshot of {addon} from {} ({})",
            source.path.display(),
            describe(&source)
        ),
    );
    out.status("Wrote", commands::relative(&path));
    true
}
