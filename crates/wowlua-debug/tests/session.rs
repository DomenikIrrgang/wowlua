//! An addon session hosted in process, as wowlua studio's game page and
//! debugger run one (decisions T484, T494): the scene after each change,
//! the inspector, what draws at a point, the units, the event log and the
//! game's settings; pausing, hit counts, an error's details, the clock,
//! the world saved as a profile, a reload.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use wowlua_debug::protocol::{Event, LineBreakpoint, Request, Response, StopDetail, StopReason};
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

/// A package `demo` for `standard` with `main` as its source.
fn package(main: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        (
            "wowlua.json",
            r#"{ "package": { "name": "demo", "version": "1.0.0" }, "flavors": { "standard": {} }, "addon": { "title": "Demo" } }"#,
        ),
        ("src/main.wowlua", main),
    ] {
        let path = dir.path().join(path);
        wowlua_host::create_dir_all(path.parent().unwrap()).unwrap();
        wowlua_host::write(path, text).unwrap();
    }
    dir
}

/// The front-end's side: requests out, responses back, events kept.
struct Front {
    tx: Sender<(u64, Request)>,
    rx: Receiver<Outgoing>,
    next: u64,
    events: Vec<Event>,
}

impl Front {
    fn send(&mut self, req: Request) -> u64 {
        self.next += 1;
        self.tx.send((self.next, req)).unwrap();
        self.next
    }

    fn ask(&mut self, req: Request) -> Result<Response, String> {
        let id = self.send(req);
        self.answer(id)
    }

    fn answer(&mut self, id: u64) -> Result<Response, String> {
        loop {
            match self.rx.recv().expect("the session answers") {
                Outgoing::Response(r, out) if r == id => return out,
                Outgoing::Event(e) => self.events.push(e),
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

    /// The first event `is` takes, among those kept or coming.
    fn event(&mut self, is: impl Fn(&Event) -> bool) -> Event {
        if let Some(i) = self.events.iter().position(&is) {
            return self.events.remove(i);
        }
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left).expect("the event comes") {
                Outgoing::Event(e) if is(&e) => return e,
                Outgoing::Event(e) => self.events.push(e),
                _ => {}
            }
        }
    }
}

/// Builds the package at `root` and runs its addon on a thread, the
/// clock as given, `hook` its frame hook.
fn start(
    root: std::path::PathBuf,
    clock: ClockMode,
    hook: Option<session::FrameHook>,
    hook_maker: Option<Box<dyn FnOnce() -> session::FrameHook + Send>>,
) -> (Front, JoinHandle<()>) {
    let (req_tx, req_rx) = channel();
    let (out_tx, out_rx) = channel();
    drop(hook);
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
                clock,
                stop_on_entry: false,
                debug_blizzard_ui: false,
                flavor: None,
                profile: None,
            };
            session::run_hosted(
                &suite,
                &opts,
                Box::new(ChannelLink::new(req_rx, out_tx)),
                Arc::new(AtomicBool::new(false)),
                None,
                hook_maker.map(|make| make()),
            );
        })
        .unwrap();
    let front = Front {
        tx: req_tx,
        rx: out_rx,
        next: 0,
        events: Vec::new(),
    };
    (front, thread)
}

#[test]
fn a_hosted_session_draws_inspects_and_changes_its_settings() {
    let dir = package(MAIN);
    let (scene_tx, scene_rx) = channel::<(u32, u32, usize)>();
    let make: Box<dyn FnOnce() -> session::FrameHook + Send> = Box::new(move || {
        Box::new(move |vm| {
            let scene = wowlua_engine::scene::build(vm);
            let _ = scene_tx.send((scene.width, scene.height, scene.items.len()));
        })
    });
    let (mut front, thread) = start(
        dir.path().to_path_buf(),
        ClockMode::Manual,
        None,
        Some(make),
    );
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

    // A stop draws the screen as it is: the hook runs there too.
    front
        .ask(Request::SetExceptionBreakpoints {
            filters: vec!["all".into()],
        })
        .expect("filters");
    // (the manual clock's pass after that answer runs the hook once)
    scene_rx.recv().expect("a scene");
    while scene_rx.try_recv().is_ok() {}
    let raised = front.send(Request::World {
        script: "error('stop here')".into(),
    });
    front.event(|e| matches!(e, Event::Stopped { .. }));
    assert!(scene_rx.try_recv().is_ok(), "a scene at the stop");
    front
        .ask(Request::SetExceptionBreakpoints { filters: vec![] })
        .expect("filters");
    front.ask(Request::Continue).expect("continuing");
    let _ = front.answer(raised);

    front.ask(Request::Terminate).ok();
    drop(front);
    thread.join().unwrap();
}

const TICKS: &str = r#"
Count = 0
local f = CreateFrame("Frame")
f:SetScript("OnUpdate", function()
    Count = Count + 1
end)
function Boom()
    error("boom")
end
"#;

/// The debugger's page (decision T494): a breakpoint's hits, a pause and
/// the session going on, an error's details, the clock switched, the
/// world as a profile, a unit by its place, a reload told.
#[test]
fn a_hosted_session_pauses_counts_and_explains_its_stops() {
    let dir = package(TICKS);
    let (mut front, thread) = start(dir.path().to_path_buf(), ClockMode::Realtime, None, None);
    // A breakpoint that never stops (its hit condition) counts its hits.
    let set = front.ask(Request::SetBreakpoints {
        source: "src/main.wowlua".into(),
        breakpoints: vec![LineBreakpoint {
            line: 5,
            condition: None,
            hit_condition: Some(">1000000".into()),
            log_message: None,
        }],
    });
    assert!(matches!(set, Ok(Response::Breakpoints { .. })), "{set:?}");
    front.ask(Request::ConfigurationDone).expect("configured");
    std::thread::sleep(Duration::from_millis(300));
    let Ok(Response::Breakpoints { breakpoints }) = front.ask(Request::Breakpoints) else {
        panic!("the breakpoints");
    };
    let line = breakpoints
        .iter()
        .find(|b| b.source.as_deref() == Some("src/main.wowlua"))
        .expect("the line breakpoint");
    assert!(line.verified && line.hits > 0, "{breakpoints:?}");

    // A pause stops the running session; continuing tells it.
    front.ask(Request::Pause).expect("pausing");
    let stopped = front.event(|e| matches!(e, Event::Stopped { .. }));
    assert!(
        matches!(
            stopped,
            Event::Stopped {
                reason: StopReason::Pause,
                ..
            }
        ),
        "{stopped:?}"
    );
    front.ask(Request::Continue).expect("continuing");
    front.event(|e| matches!(e, Event::Continued));

    // An error, every one stopping: its message, stack, caught or not.
    front
        .ask(Request::SetExceptionBreakpoints {
            filters: vec!["all".into()],
        })
        .expect("filters");
    let boom = front.send(Request::World {
        script: "Boom()".into(),
    });
    let stopped = front.event(|e| matches!(e, Event::Stopped { .. }));
    let Event::Stopped {
        reason: StopReason::Exception,
        detail: Some(StopDetail::Error { message, stack, .. }),
        ..
    } = stopped
    else {
        panic!("{stopped:?}");
    };
    assert!(message.contains("boom"), "{message}");
    assert!(!stack.is_empty());
    front
        .ask(Request::SetExceptionBreakpoints { filters: vec![] })
        .expect("filters");
    front.ask(Request::Continue).expect("continuing");
    let _ = front.answer(boom);

    // The clock switched to manual: time moves when asked.
    front
        .ask(Request::SetClock { manual: true })
        .expect("manual");
    front
        .ask(Request::Advance {
            seconds: None,
            frames: Some(2),
        })
        .expect("advanced");

    // The world as a profile; a unit by its place in the list.
    let world = front.data(Request::SaveWorld);
    assert!(world["character"]["class"].is_string(), "{world:#}");
    assert!(world["units"].is_array(), "{world:#}");
    front
        .ask(Request::World {
            script: "assert(mock.unitAt(0) ~= nil and mock.unitAt(99999) == nil)".into(),
        })
        .expect("units by place");
    let units = front.data(Request::Units);
    assert!(units[0].get("threat").is_some(), "{units:#}");

    // A reload is told.
    // (`/reload` is Blizzard's UI's slash command: `C_UI.Reload` without it)
    front
        .ask(Request::World {
            script: "C_UI.Reload()".into(),
        })
        .expect("reloading");
    front
        .ask(Request::Advance {
            seconds: None,
            frames: Some(1),
        })
        .ok();
    front.event(|e| matches!(e, Event::Reloaded));

    front.ask(Request::Terminate).ok();
    drop(front);
    thread.join().unwrap();
}
