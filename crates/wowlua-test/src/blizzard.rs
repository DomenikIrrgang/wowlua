//! Blizzard's UI code for `test.blizzard-ui` (spec emulator §17.7, decision
//! T204): wow-ui-source at the commit the flavor's API declarations were
//! generated from, with the global strings of the same dump, fetched on
//! first use into `$WOWLUA_HOME/cache/blizzard-ui/`. `WOWLUA_UI_SOURCE`
//! names a folder laid out as `cargo xtask types fetch` makes it
//! (`<flavor>/Interface`, `strings/<commit>.lua`) to use instead.

use std::path::{Path, PathBuf};
use std::process::Command;

const UI_REPO: &str = "https://github.com/Gethe/wow-ui-source.git";

/// Where a flavor's UI code is: its `Interface` folder and its global
/// strings.
#[derive(Clone, Debug)]
pub struct UiSource {
    pub interface: PathBuf,
    pub strings: Option<PathBuf>,
}

/// The commits the flavor's declarations name: wow-ui-source's and the
/// global strings dump's.
fn commits(flavor: &str) -> Option<(String, Option<String>)> {
    let file = wowlua_api::own(flavor)?;
    let header: String = file.text.lines().take(3).collect::<Vec<_>>().join(" ");
    let ui = regex::Regex::new(r"wow-ui-source [^,]*, commit ([0-9a-f]{40})")
        .expect("a valid pattern")
        .captures(&header)?[1]
        .to_string();
    let strings = regex::Regex::new(r"BlizzardInterfaceResources \S+ ([0-9a-f]{40})")
        .expect("a valid pattern")
        .captures(&header)
        .map(|c| c[1].to_string());
    Some((ui, strings))
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd
        .output()
        .map_err(|e| format!("{what}: cannot run {:?}: {e}", cmd.get_program()))?;
    if !out.status.success() {
        return Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// The flavor's UI code, fetched when the cache lacks it.
/// The UI code in a `cargo xtask types fetch` directory (`target/types-src`):
/// the flavor's `Interface` and its global strings, when there.
pub fn ui_source_in(dir: &Path, flavor: &str) -> Option<UiSource> {
    let (_, strings_commit) = commits(flavor)?;
    let interface = dir.join(flavor).join("Interface");
    if !interface.exists() {
        return None;
    }
    Some(UiSource {
        interface,
        strings: strings_commit
            .map(|c| dir.join("strings").join(format!("{c}.lua")))
            .filter(|p| p.exists()),
    })
}

pub fn ui_source(flavor: &str, status: &dyn Fn(&str)) -> Result<UiSource, String> {
    let (commit, strings_commit) = commits(flavor).ok_or_else(|| {
        format!("the `{flavor}` declarations do not name their wow-ui-source commit")
    })?;
    if let Some(dir) = std::env::var_os("WOWLUA_UI_SOURCE") {
        return ui_source_in(&PathBuf::from(dir), flavor)
            .ok_or_else(|| format!("no `{flavor}` UI code in WOWLUA_UI_SOURCE"));
    }
    let cache = wowlua_driver::cache_root()
        .ok_or("cannot find the cache (set `WOWLUA_HOME`)")?
        .join("blizzard-ui");
    let dir = cache.join(format!("{flavor}-{}", &commit[..12]));
    if !dir.join(".wowlua-complete").exists() {
        status(&format!(
            "Blizzard's UI code for {flavor} (wow-ui-source {})",
            &commit[..12]
        ));
        fetch_ui(&dir, &commit)?;
    }
    let strings = match strings_commit {
        Some(c) => {
            let path = cache.join("strings").join(format!("{c}.lua"));
            if !path.exists() {
                fetch_strings(&path, &c)?;
            }
            Some(path)
        }
        None => None,
    };
    Ok(UiSource {
        interface: dir.join("Interface"),
        strings,
    })
}

fn fetch_ui(dir: &Path, commit: &str) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let git = |args: &[&str]| -> Result<(), String> {
        run(
            Command::new("git").arg("-C").arg(dir).args(args),
            "fetching Blizzard's UI code with git",
        )
    };
    git(&["init", "-q"])?;
    git(&["remote", "add", "origin", UI_REPO])?;
    git(&["sparse-checkout", "set", "Interface/AddOns"])?;
    git(&[
        "fetch",
        "-q",
        "--depth",
        "1",
        "--filter=blob:none",
        "origin",
        commit,
    ])?;
    git(&["checkout", "-q", "FETCH_HEAD"])?;
    // Only the files are needed.
    let _ = std::fs::remove_dir_all(dir.join(".git"));
    std::fs::write(dir.join(".wowlua-complete"), commit).map_err(|e| e.to_string())
}

fn fetch_strings(path: &Path, commit: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let url = format!(
        "https://raw.githubusercontent.com/Ketho/BlizzardInterfaceResources/{commit}/Resources/GlobalStrings/enUS.lua"
    );
    let part = path.with_extension("part");
    run(
        Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&part)
            .arg(&url),
        "downloading the global strings with curl",
    )?;
    std::fs::rename(&part, path).map_err(|e| e.to_string())
}
