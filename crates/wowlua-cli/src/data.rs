//! `wowlua data pull` (spec CLI §12.1, decision T203): a flavor's game data
//! read from the user's own WoW installation into wowlua's local cache,
//! `$WOWLUA_HOME/data/<flavor>/`, for `wowlua test --game-data`. Nothing is
//! distributed: the cache is the user's, like the installation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wowlua_casc::{Casc, FsStorage};
use wowlua_diagnostics::Diagnostic;
use wowlua_driver::Session;

use crate::output::Output;
use wowlua_test::screenshots::Camera;

/// The client's fonts the engine measures text with, copied with the
/// tables (the ones a flavor lacks are left out).
const FONTS: &[&str] = &[
    "Fonts\\FRIZQT__.TTF",
    "Fonts\\FRIZQT___CYR.TTF",
    "Fonts\\ARIALN.TTF",
    "Fonts\\MORPHEUS.TTF",
    "Fonts\\MORPHEUS_CYR.TTF",
    "Fonts\\SKURRI.TTF",
    "Fonts\\SKURRI_CYR.TTF",
    "Fonts\\2002.TTF",
    "Fonts\\2002B.TTF",
    "Fonts\\ARKai_C.ttf",
    "Fonts\\ARKai_T.ttf",
    "Fonts\\ARHei.ttf",
    "Fonts\\blei00d.TTF",
    "Fonts\\bHEI00M.ttf",
    "Fonts\\bHEI01B.ttf",
    "Fonts\\bKAI00M.ttf",
    "Fonts\\K_Damage.TTF",
    "Fonts\\K_Pagetext.TTF",
    "Fonts\\NIM_____.ttf",
];

/// The game data `wowlua data pull` read for `flavor`, if any.
pub fn open_pulled(flavor: &str) -> Option<Arc<Casc>> {
    let root = wowlua_driver::install::wowlua_home()?
        .join("data")
        .join(flavor);
    if !root.join(".pulled").is_file() {
        return None;
    }
    Casc::open_pulled(Box::new(FsStorage { root }))
        .ok()
        .map(Arc::new)
}

/// `wowlua test --game-data`: the camera's client data from the game data
/// pulled for `flavor`, else (none pulled) the installation's as before.
pub fn with_pulled(
    camera: Option<Arc<dyn Camera>>,
    flavor: &str,
) -> (Option<Arc<dyn Camera>>, bool) {
    let (Some(inner), Some(casc)) = (camera.clone(), open_pulled(flavor)) else {
        return (camera, false);
    };
    (Some(Arc::new(PulledCamera { inner, casc })), true)
}

/// A camera whose client data is pulled game data.
struct PulledCamera {
    inner: Arc<dyn Camera>,
    casc: Arc<Casc>,
}

impl Camera for PulledCamera {
    fn shoot(
        &self,
        vm: &mut wowlua_vm::Vm,
        frame: Option<wowlua_engine::widgets::WidgetId>,
        scale: f64,
    ) -> Result<Vec<u8>, String> {
        self.inner.shoot(vm, frame, scale)
    }

    fn compare(
        &self,
        expected: &[u8],
        actual: &[u8],
        threshold: f64,
    ) -> Result<(f64, Vec<u8>), String> {
        self.inner.compare(expected, actual, threshold)
    }

    fn out_dir(&self) -> PathBuf {
        self.inner.out_dir()
    }

    fn client_data(&self) -> Option<Arc<dyn wowlua_engine::clientdata::ClientData>> {
        let casc = self.casc.clone();
        Some(Arc::new(wowlua_render::client::CascClientData::new(
            Box::new(move || Some(casc.clone())),
        )))
    }
}

/// `wowlua data pull [--wow <path>] [--flavor <f>…]`: each flavor's game
/// data (by default the package's flavors, outside a package every flavor
/// whose client the installation has).
pub fn pull(out: &Output, wow: Option<&Path>, flavors: &[String]) -> bool {
    let mut session = Session::new();
    let install = match wowlua_driver::install::find_wow(wow) {
        Ok(path) => path,
        Err(d) => {
            session.push(d);
            out.flush(&mut session);
            return false;
        }
    };
    let table = wowlua_flavors::table();
    let mut selected: Vec<String> = Vec::new();
    for name in flavors {
        match table.resolve(name) {
            Some(f) => selected.push(f.id.clone()),
            None => session.push(Diagnostic::plain_error(format!("unknown flavor `{name}`"))),
        }
    }
    // Outside a package every flavor is tried: the ones whose client the
    // installation lacks are left out.
    let mut lenient = false;
    if flavors.is_empty() {
        selected = package_flavors().unwrap_or_else(|| {
            lenient = true;
            table.flavors.iter().map(|f| f.id.clone()).collect()
        });
    }
    let Some(home) = wowlua_driver::install::wowlua_home() else {
        session.push(Diagnostic::plain_error(
            "no home folder for wowlua's cache (set WOWLUA_HOME)",
        ));
        out.flush(&mut session);
        return false;
    };
    let mut pulled = 0;
    for flavor in selected {
        let folders: Vec<String> = table
            .clients_of(&flavor)
            .flat_map(|c| c.folders.iter().cloned())
            .collect();
        let casc = match wowlua_render::client::open_client(&install, &folders) {
            Ok(c) => c,
            Err(_) if lenient => continue,
            Err(e) => {
                session.push(Diagnostic::plain_error(format!("{flavor}: {e}")));
                continue;
            }
        };
        let dir = home.join("data").join(&flavor);
        match write(&dir, casc.pull(FONTS)) {
            Ok(bytes) => {
                pulled += 1;
                out.status(
                    "Pulled",
                    format!(
                        "{flavor} game data ({} {}, {} MiB) → {}",
                        casc.product,
                        casc.version,
                        bytes.div_ceil(1 << 20),
                        dir.display()
                    ),
                );
            }
            Err(e) => session.push(Diagnostic::plain_error(format!(
                "cannot write `{}`: {e}",
                dir.display()
            ))),
        }
    }
    if pulled == 0 && !session.has_errors() {
        session.push(Diagnostic::plain_error(format!(
            "the installation at {} has the client of none of the flavors",
            install.display()
        )));
    }
    out.flush(&mut session)
}

/// The flavors of the package in the current folder, if there is one.
fn package_flavors() -> Option<Vec<String>> {
    let cwd = std::env::current_dir().ok()?;
    let path = wowlua_driver::project::find_manifest(&cwd)?;
    let mut db = wowlua_span::SourceDb::new();
    let loaded = wowlua_driver::project::load_manifest(&mut db, &path).manifest?;
    Some(loaded.flavors)
}

/// Replaces `dir` with `files`; their size.
fn write(dir: &Path, files: Vec<(String, Vec<u8>)>) -> std::io::Result<usize> {
    let staging = dir.with_extension("pulling");
    let _ = std::fs::remove_dir_all(&staging);
    let mut bytes = 0;
    for (rel, data) in files {
        let path = staging.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        bytes += data.len();
        std::fs::write(path, data)?;
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&staging, dir)?;
    Ok(bytes)
}
