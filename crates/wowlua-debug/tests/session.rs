//! An addon session hosted in process, as wowlua studio's game page runs
//! one (decision T484): the scene after each change, the inspector, what
//! draws at a point, the units, the event log and the game's settings.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, channel};

use serde_json::Value as Json;
use wowlua_debug::protocol::{Request, Response};
use wowlua_debug::session::{self, ChannelLink, ClockMode, Options, Outgoing, Target};
use wowlua_driver::project::{LoadOptions, load_projects};
use wowlua_test::runs::{self, Switches};

const MAIN: &str = r#"
local f = CreateFrame("Frame", "HookDemo", UIParent)
f:SetSize(200, 100)
f:SetPoint("CENTER")
f:SetFrameStrata("HIGH")
local t = f:CreateTexture(nil, "ARTWORK")
t:SetAllPoints()
t:SetColorTexture(1, 0, 0)
f:RegisterEvent("PLAYER_TARGET_CHANGED")
f:SetScript("OnEvent", function() end)
"#;

/// A package `demo` for `standard` with the frame above.
fn package() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        (
            "wowlua.json",
            r#"{ "package": { "name": "demo", "version": "1.0.0" }, "flavors": { "standard": {} }, "addon": { "title": "Demo" } }"#,
        ),
        ("src/main.wowlua", MAIN),
    ] {
        let path = dir.path().join(path);
        wowlua_host::create_dir_all(path.parent().unwrap()).unwrap();
        wowlua_host::write(path, text).unwrap();
    }
    dir
}

/// The front-end's side: requests out, responses back (events skipped).
struct Front {
    tx: Sender<(u64, Request)>,
    rx: Receiver<Outgoing>,
    next: u64,
}

impl Front {
    fn ask(&mut self, req: Request) -> Result<Response, String> {
        self.next += 1;
        let id = self.next;
        self.tx.send((id, req)).unwrap();
        loop {
            match self.rx.recv().expect("the session answers") {
                Outgoing::Response(r, out) if r == id => return out,
                _ => {}
            }
        }
    }

    fn data(&mut self, req: Request) -> Json {
        match self.ask(req) {
            Ok(Response::Data { data }) => data,
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_hosted_session_draws_inspects_and_changes_its_settings() {
    let dir = package();
    let root = dir.path().to_path_buf();
    let (req_tx, req_rx) = channel();
    let (out_tx, out_rx) = channel();
    let (scene_tx, scene_rx) = channel::<(u32, u32, usize)>();
    let thread = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut driver = wowlua_driver::Session::new();
            let options = LoadOptions {
                cwd: root,
                lock_in_memory: true,
                offline: true,
                ..LoadOptions::default()
            };
            let projects = load_projects(&mut driver, &options).expect("the package loads");
            let project = &projects[0];
            let (build, _) = runs::build(
                &mut driver,
                project,
                vec!["standard".into()],
                "standard",
                "test",
                wowlua_driver::parallel::Jobs::default(),
                &mut |_| {},
            )
            .expect("the build");
            let instrument: &(dyn Fn(&mut wowlua_vm::Vm) + Sync) = &session::instrument;
            let suite = runs::suite(
                project,
                &build,
                "standard",
                None,
                Switches::default(),
                Some(instrument),
                None,
            );
            let opts = Options {
                target: Target::Addon,
                clock: ClockMode::Manual,
                stop_on_entry: false,
                debug_blizzard_ui: false,
                flavor: None,
                profile: None,
            };
            let hook: session::FrameHook = Box::new(move |vm| {
                let scene = wowlua_engine::scene::build(vm);
                let _ = scene_tx.send((scene.width, scene.height, scene.items.len()));
            });
            session::run_hosted(
                &suite,
                &opts,
                Box::new(ChannelLink::new(req_rx, out_tx)),
                Arc::new(AtomicBool::new(false)),
                None,
                Some(hook),
            );
        })
        .unwrap();
    let mut front = Front {
        tx: req_tx,
        rx: out_rx,
        next: 0,
    };
    front.ask(Request::ConfigurationDone).expect("configured");
    // The first scene: the default screen, the frame's red square drawn.
    let (w, h, items) = scene_rx.recv().expect("a scene");
    assert_eq!((w, h), (1920, 1080));
    assert!(items >= 1, "{items}");

    // What draws at the screen's centre: the frame's texture.
    let picked = front.data(Request::Pick { x: 960.0, y: 540.0 });
    let frame = picked["frame"].as_u64().expect("a frame") as usize;
    assert!(picked["region"].is_u64(), "{picked}");
    let info = front.data(Request::Inspect { frame });
    assert_eq!(info["name"], "HookDemo");
    assert_eq!(info["strata"], "HIGH");
    assert_eq!(info["points"][0]["point"], "CENTER");
    assert_eq!(info["events"][0], "PLAYER_TARGET_CHANGED");
    assert_eq!(info["scripts"][0]["name"], "OnEvent");
    assert_eq!(info["regions"][0]["layer"], "ARTWORK");
    assert_eq!(info["created"]["line"], 2, "{info:#}");

    // The units, and the target the player picks: told to the frame.
    front.ask(Request::LogEvents { on: true }).expect("logging");
    let units = front.data(Request::Units);
    let player = &units[0];
    assert_eq!(player["tokens"][0], "player", "{units:#}");
    front
        .ask(Request::Target { unit: Some(0) })
        .expect("targets");
    let units = front.data(Request::Units);
    assert_eq!(units[0]["target"], true);
    let events = front.data(Request::Events);
    let told = events
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event"] == "PLAYER_TARGET_CHANGED")
        .expect("the target told");
    assert!(
        told["frames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "HookDemo"),
        "{told}"
    );

    // The game's settings: the next scene is the new size.
    front
        .ask(Request::SetScreen {
            width: 1280.0,
            height: 720.0,
            ui_scale: Some(0.9),
        })
        .expect("set");
    let last = loop {
        let s = scene_rx.recv().expect("a scene");
        if (s.0, s.1) == (1280, 720) {
            break s;
        }
    };
    assert_eq!((last.0, last.1), (1280, 720));

    front.ask(Request::Terminate).ok();
    drop(front);
    thread.join().unwrap();
}
