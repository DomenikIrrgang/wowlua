//! The addon running in the page (spec M14 S3, decision T396): built into
//! memory and loaded as `wowlua test` loads it, after Blizzard's UI when the
//! page fetched it ([`blizzard_ui`]), the player logged in; time moves when
//! the page says, input is the debugger's, and what it prints and raises is
//! kept for the page.

use serde_json::{Value as Json, json};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use wowlua_driver::BuildOutput;
use wowlua_vm::Vm;

/// The length of a game frame (60 a second).
const FRAME_MS: f64 = 1000.0 / 60.0;

/// The most frames one [`Game::advance`] runs: a page that was away (a tab
/// in the background) does not catch up on all of it.
const MAX_FRAMES: usize = 6;

/// The Blizzard UI files of each flavor the page can fetch.
const UI_LISTS: &[(&str, &str)] = &[("camelot", include_str!("../ui/camelot.txt"))];

/// Where the page keeps a flavor's Blizzard UI.
pub fn ui_root(flavor: &str, commit: &str) -> PathBuf {
    PathBuf::from(format!(
        "/blizzard-ui/{flavor}-{}",
        &commit[..12.min(commit.len())]
    ))
}

/// What the page fetches for `flavor`'s Blizzard UI: the commit, where the
/// files come from, where they go and which (`startup` first, then `later`,
/// those loaded on demand). `None` for a flavor the playground lacks.
pub fn blizzard_ui(flavor: &str) -> Option<Json> {
    let (_, text) = UI_LISTS.iter().find(|(f, _)| *f == flavor)?;
    let mut commit = String::new();
    let mut strings = None;
    let (mut startup, mut later) = (Vec::new(), Vec::new());
    let mut into_later = false;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        if let Some(c) = line.strip_prefix("commit ") {
            commit = c.to_string();
        } else if let Some(s) = line.strip_prefix("strings ") {
            strings = Some(s.to_string());
        } else if line == "startup" {
            into_later = false;
        } else if line == "later" {
            into_later = true;
        } else if into_later {
            later.push(line);
        } else {
            startup.push(line);
        }
    }
    let root = ui_root(flavor, &commit);
    Some(json!({
        "flavor": flavor,
        "commit": commit,
        "root": root,
        "base": format!("https://cdn.jsdelivr.net/gh/Gethe/wow-ui-source@{commit}/"),
        "strings": strings.map(|s| json!({
            "url": format!("https://raw.githubusercontent.com/Ketho/BlizzardInterfaceResources/{s}/Resources/GlobalStrings/enUS.lua"),
            "path": format!("/blizzard-ui/strings/{s}.lua"),
        })),
        "startup": startup,
        "later": later,
    }))
}

/// The flavor's Blizzard UI, when the page put its startup files there.
fn ui_source(flavor: &str) -> Option<wowlua_test::UiSource> {
    let ui = blizzard_ui(flavor)?;
    let root = PathBuf::from(ui["root"].as_str()?);
    let first = ui["startup"].as_array()?.first()?.as_str()?;
    if !wowlua_host::is_file(root.join(first)) {
        return None;
    }
    let strings = ui["strings"]["path"]
        .as_str()
        .map(PathBuf::from)
        .filter(|p| wowlua_host::is_file(p));
    Some(wowlua_test::UiSource {
        interface: root.join("Interface"),
        strings,
    })
}

/// How a run starts.
pub struct Options {
    pub flavor: String,
    /// Load Blizzard's UI first (the page fetched it).
    pub blizzard_ui: bool,
    /// The world profile's text (`tests/profiles/<name>.json`'s); none: the
    /// manifest's, else a fresh character's.
    pub profile: Option<String>,
}

/// The addon running.
pub struct Game {
    build: BuildOutput,
    project: wowlua_driver::Project,
    flavor: String,
    blizzard_ui: Option<wowlua_test::UiSource>,
    session: wowlua_test::AddonSession,
    output: Rc<RefCell<Vec<Json>>>,
    /// The page's time at the last frame, and what was left of a frame.
    last_ms: Option<f64>,
}

impl Game {
    /// Builds `project` for the flavor into memory and starts it.
    pub fn start(
        project: wowlua_driver::Project,
        build: BuildOutput,
        options: &Options,
    ) -> Result<Game, String> {
        let blizzard_ui = if options.blizzard_ui {
            Some(ui_source(&options.flavor).ok_or_else(|| {
                format!(
                    "Blizzard's UI for {} is not loaded (`loadBlizzardUI` first)",
                    options.flavor
                )
            })?)
        } else {
            None
        };
        if let Some(text) = &options.profile {
            // (the suite reads a profile by name from the package's tests)
            let path = project.root().join("tests/profiles/playground.json");
            wowlua_host::create_dir_all(path.parent().unwrap_or(Path::new("/")))
                .map_err(|e| e.to_string())?;
            wowlua_host::write(&path, text).map_err(|e| e.to_string())?;
        }
        let output: Rc<RefCell<Vec<Json>>> = Rc::default();
        let sink = output.clone();
        wowlua_test::set_tee(Some(Box::new(move |o| {
            sink.borrow_mut().push(match o {
                wowlua_test::Output::Printed(text) => {
                    let line = wowlua_engine::text::plain(&text);
                    json!({ "kind": "print", "text": line.trim_end_matches('\n') })
                }
                wowlua_test::Output::Error { message, stack } => {
                    json!({ "kind": "error", "text": message, "stack": stack })
                }
            });
        })));
        let session = {
            let suite = suite(
                &project,
                &build,
                &options.flavor,
                blizzard_ui.clone(),
                options.profile.is_some(),
            );
            wowlua_test::AddonSession::open(&suite)?
        };
        Ok(Game {
            build,
            project,
            flavor: options.flavor.clone(),
            blizzard_ui,
            session,
            output,
            last_ms: None,
        })
    }

    pub fn vm(&mut self) -> &mut Vm {
        &mut self.session.vm
    }

    /// Moves time to the page's `now_ms`: the frames due since the last
    /// call (at most a few), a `/reload` done. What was printed and raised
    /// since the last call.
    pub fn advance(&mut self, now_ms: f64) -> Result<Vec<Json>, String> {
        let frames = match self.last_ms {
            None => {
                self.last_ms = Some(now_ms);
                1
            }
            Some(last) => {
                let n = ((now_ms - last) / FRAME_MS).floor().max(0.0) as usize;
                self.last_ms = Some(if n > MAX_FRAMES {
                    now_ms
                } else {
                    last + n as f64 * FRAME_MS
                });
                n.min(MAX_FRAMES)
            }
        };
        if self.session.wants_reload() {
            let suite = suite(
                &self.project,
                &self.build,
                &self.flavor,
                self.blizzard_ui.clone(),
                wowlua_host::exists(self.project.root().join("tests/profiles/playground.json")),
            );
            self.session.reload(&suite)?;
        }
        if frames > 0 {
            wowlua_engine::clock::advance_frames(&mut self.session.vm, frames, FRAME_MS);
        }
        Ok(self.take_output())
    }

    /// What was printed and raised since the last time asked.
    pub fn take_output(&mut self) -> Vec<Json> {
        std::mem::take(&mut *self.output.borrow_mut())
    }

    /// Input as the user gives it (the debugger's `input` request's).
    pub fn input(&mut self, input: &Json) -> Result<(), String> {
        let input: wowlua_debug::protocol::Input =
            serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
        wowlua_debug::ui::input(&mut self.session.vm, &input)
    }

    /// The addon's folder, as its images' paths name it
    /// (`interface\addons\<name>\`, lower case).
    pub fn addon_folders(&self) -> Vec<(String, PathBuf)> {
        vec![(
            format!(
                "interface\\addons\\{}\\",
                self.build.addon_name.to_ascii_lowercase()
            ),
            self.build.dir.clone(),
        )]
    }

    /// The screen's size in pixels.
    pub fn screen(&mut self) -> (u32, u32) {
        let scene = wowlua_engine::scene::build(&mut self.session.vm);
        (scene.width, scene.height)
    }
}

impl Drop for Game {
    fn drop(&mut self) {
        wowlua_test::set_tee(None);
    }
}

fn suite<'a>(
    project: &wowlua_driver::Project,
    build: &'a BuildOutput,
    flavor: &str,
    blizzard_ui: Option<wowlua_test::UiSource>,
    own_profile: bool,
) -> wowlua_test::Suite<'a> {
    let test = &project.manifest.manifest.test;
    wowlua_test::Suite {
        build,
        flavor: flavor.to_string(),
        root: project.root().to_path_buf(),
        src: project.manifest.manifest.build.src.clone(),
        blizzard_ui,
        profile: if own_profile {
            Some("playground".into())
        } else {
            test.profile.clone()
        },
        platform: match test.platform {
            Some(wowlua_manifest::model::TestPlatform::Windows) => wowlua_test::Platform::Windows,
            _ => wowlua_test::Platform::Mac,
        },
        screen: match test.screen {
            Some(s) => wowlua_test::Screen {
                width: s.width,
                height: s.height,
                dpi_scale: s.dpi_scale.unwrap_or(1.0),
                ui_scale: s.ui_scale,
            },
            None => wowlua_test::Screen::default(),
        },
        addons: Vec::new(),
        update_snapshots: false,
        coverage: false,
        instrument: None,
        camera: None,
        game_data: false,
        after_test: None,
        after_file: None,
    }
}
