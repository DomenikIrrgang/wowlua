//! Sessions (spec debugger §18.3): a test file run as `wowlua test` runs
//! it, or the addon loaded with the player logged in and the game's frames
//! running, in real time or as asked; each with a [`Controller`] on every
//! VM it makes. Native: a session runs on its own thread.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use wowlua_test::{Output, Suite};
use wowlua_vm::Vm;

use crate::controller::{Controller, Link, Screenshotter, Shared, SharedRef};
use crate::protocol::*;

/// What a session runs.
#[derive(Clone, Debug)]
pub enum Target {
    /// A test file (its path relative to the package root) and the tests
    /// whose names contain any of `filters`.
    Test {
        file: String,
        filters: Vec<String>,
    },
    Addon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockMode {
    /// A game frame every 1/60 s of wall time.
    Realtime,
    /// Time moves when asked (`Advance`).
    Manual,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub target: Target,
    pub clock: ClockMode,
    pub stop_on_entry: bool,
    pub debug_blizzard_ui: bool,
    pub flavor: Option<String>,
}

impl Options {
    /// From DAP's `launch` arguments: `test` (a path) and `filter`, or
    /// `addon: true`; `clock`, `stopOnEntry`, `debugBlizzardUI`, `flavor`.
    pub fn from_launch(
        args: &Json,
        root: &std::path::Path,
        default_clock: ClockMode,
    ) -> Result<Options, String> {
        let target = match args.get("test").and_then(Json::as_str) {
            Some(test) => {
                let path = std::path::Path::new(test);
                let rel = path.strip_prefix(root).unwrap_or(path);
                Target::Test {
                    file: rel.to_string_lossy().replace('\\', "/"),
                    filters: match args.get("filter") {
                        Some(Json::String(s)) => vec![s.clone()],
                        Some(Json::Array(a)) => a
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect(),
                        _ => Vec::new(),
                    },
                }
            }
            None if args.get("addon").and_then(Json::as_bool) == Some(true) => Target::Addon,
            None => return Err("launch needs `test` (a test file) or `addon: true`".into()),
        };
        let clock = match args.get("clock").and_then(Json::as_str) {
            Some("manual") => ClockMode::Manual,
            Some("realtime") => ClockMode::Realtime,
            Some(other) => {
                return Err(format!(
                    "clock is \"realtime\" or \"manual\", not \"{other}\""
                ));
            }
            None => default_clock,
        };
        Ok(Options {
            target,
            clock,
            stop_on_entry: args
                .get("stopOnEntry")
                .and_then(Json::as_bool)
                .unwrap_or(false),
            debug_blizzard_ui: args
                .get("debugBlizzardUI")
                .and_then(Json::as_bool)
                .unwrap_or(false),
            flavor: args
                .get("flavor")
                .and_then(Json::as_str)
                .map(str::to_string),
        })
    }
}

/// What a session sends to its front-end.
#[derive(Debug)]
pub enum Outgoing {
    Response(u64, Result<Response, String>),
    Event(Event),
}

/// A [`Link`] over channels: the front-end's thread on the other side.
pub struct ChannelLink {
    pub rx: Receiver<(u64, Request)>,
    pub tx: Sender<Outgoing>,
    gone: bool,
}

impl ChannelLink {
    pub fn new(rx: Receiver<(u64, Request)>, tx: Sender<Outgoing>) -> ChannelLink {
        ChannelLink {
            rx,
            tx,
            gone: false,
        }
    }
}

impl Link for ChannelLink {
    fn recv(&mut self, timeout_ms: Option<u64>) -> Option<(u64, Request)> {
        if self.gone {
            return None;
        }
        let r = match timeout_ms {
            None => self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(0) => self.rx.try_recv().map_err(|e| match e {
                std::sync::mpsc::TryRecvError::Empty => RecvTimeoutError::Timeout,
                std::sync::mpsc::TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            }),
            Some(ms) => self.rx.recv_timeout(Duration::from_millis(ms)),
        };
        match r {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.gone = true;
                None
            }
        }
    }

    fn respond(&mut self, id: u64, r: Result<Response, String>) {
        let _ = self.tx.send(Outgoing::Response(id, r));
    }

    fn event(&mut self, e: Event) {
        let _ = self.tx.send(Outgoing::Event(e));
    }

    fn gone(&self) -> bool {
        self.gone
    }
}

thread_local! {
    static SESSION: RefCell<Option<SharedRef>> = const { RefCell::new(None) };
    /// Output while the session's state was busy, sent with the next.
    static PENDING: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// The test runner's `Suite::instrument` for debug sessions: attaches the
/// running session's controller to each VM it makes.
pub fn instrument(vm: &mut Vm) {
    SESSION.with(|s| {
        if let Some(shared) = s.borrow().as_ref() {
            Controller::attach(vm, shared);
        }
    });
}

fn send_output(shared: &SharedRef, category: &str, text: String) {
    match shared.try_borrow_mut() {
        Ok(mut s) => {
            let pending = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
            for (c, t) in pending {
                s.output(&c, &t);
            }
            s.output(category, &text);
        }
        Err(_) => PENDING.with(|p| p.borrow_mut().push((category.to_string(), text))),
    }
}

/// The host's monotonic clock in nanoseconds.
fn host_clock() -> crate::controller::Clock {
    let start = Instant::now();
    Rc::new(move || start.elapsed().as_nanos() as u64)
}

/// Serves the requests that come before the session runs: breakpoints,
/// until `ConfigurationDone`. False when the front-end went away.
fn configure(shared: &SharedRef) -> bool {
    shared.borrow_mut().link.event(Event::Initialized);
    loop {
        let next = shared.borrow_mut().link.recv(None);
        let Some((id, req)) = next else {
            if shared.borrow().link.gone() {
                return false;
            }
            continue;
        };
        let mut s = shared.borrow_mut();
        let r = match req {
            Request::SetBreakpoints {
                source,
                breakpoints,
            } => Ok(Response::Breakpoints {
                breakpoints: s.record_lines(&source, breakpoints),
            }),
            Request::SetFunctionBreakpoints { breakpoints } => Ok(Response::Breakpoints {
                breakpoints: s.record_functions(breakpoints),
            }),
            Request::SetExceptionBreakpoints { filters } => {
                s.record_filters(&filters);
                Ok(Response::Ok)
            }
            Request::ConfigurationDone => {
                s.configured = true;
                s.link.respond(id, Ok(Response::Ok));
                return true;
            }
            Request::Terminate => {
                s.terminate = true;
                s.link.respond(id, Ok(Response::Ok));
                return false;
            }
            Request::StackTrace => Ok(Response::StackTrace { frames: Vec::new() }),
            _ => Err("the session has not started".into()),
        };
        s.link.respond(id, r);
    }
}

/// Runs a session on this thread until it ends; the suite's `instrument`
/// must be [`instrument`]. Tells the front-end `Terminated` at the end.
pub fn run(
    suite: &Suite,
    opts: &Options,
    link: Box<dyn Link>,
    interrupt: Arc<AtomicBool>,
    screenshot: Option<Screenshotter>,
) {
    let shared = Shared::new(link, wowlua_test::chunk_mapper(suite), host_clock());
    {
        let mut s = shared.borrow_mut();
        s.files = Some(wowlua_test::chunk_file_mapper(suite));
        s.interrupt = interrupt;
        s.screenshot = screenshot;
        s.debug_blizzard_ui = opts.debug_blizzard_ui;
        s.drives_time = matches!(opts.target, Target::Addon);
    }
    if !configure(&shared) {
        shared.borrow_mut().link.event(Event::Terminated {
            summary: None,
            failed: false,
        });
        return;
    }
    if opts.stop_on_entry {
        shared.borrow_mut().stop_on_entry();
    }
    // Which flavor runs: a package has several, the session one.
    let what = match &opts.target {
        Target::Test { file, .. } => file.clone(),
        Target::Addon => "the addon".into(),
    };
    send_output(
        &shared,
        "console",
        format!(
            "Debugging {what} on {} (the launch configuration's \"flavor\" chooses)\n",
            suite.flavor
        ),
    );
    SESSION.with(|s| *s.borrow_mut() = Some(shared.clone()));
    let tee = shared.clone();
    wowlua_test::set_tee(Some(Box::new(move |o: Output| match o {
        Output::Printed(text) => send_output(&tee, "stdout", text),
        Output::Error { message, stack } => {
            send_output(&tee, "stderr", format!("{message}\n{stack}\n"))
        }
    })));
    let (summary, failed) = match &opts.target {
        Target::Test { file, filters } => run_test(suite, file, filters),
        Target::Addon => run_addon(suite, opts, &shared),
    };
    wowlua_test::set_tee(None);
    SESSION.with(|s| *s.borrow_mut() = None);
    let mut s = shared.borrow_mut();
    if let Some(text) = &summary {
        s.output("console", &format!("{text}\n"));
    }
    s.link.event(Event::Terminated { summary, failed });
}

fn run_test(suite: &Suite, file: &str, filters: &[String]) -> (Option<String>, bool) {
    let Some(test) = suite.build.tests.iter().find(|t| t.rel == file) else {
        return (
            Some(format!("{file} is not a test file of the package")),
            true,
        );
    };
    let result = wowlua_test::run_file(suite, test, filters);
    let mut lines = Vec::new();
    if let Some(e) = &result.error {
        lines.push(format!("{file}: {e}"));
    }
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    for t in &result.tests {
        match &t.outcome {
            wowlua_test::Outcome::Pass => passed += 1,
            wowlua_test::Outcome::Skip(_) => skipped += 1,
            wowlua_test::Outcome::Fail { message, .. } => {
                failed += 1;
                lines.push(format!("✗ {}: {message}", t.name));
            }
        }
    }
    lines.push(format!(
        "{passed} passed, {failed} failed, {skipped} skipped"
    ));
    (Some(lines.join("\n")), result.failed())
}

const FRAME_MS: f64 = 1000.0 / 60.0;

fn run_addon(suite: &Suite, opts: &Options, shared: &SharedRef) -> (Option<String>, bool) {
    let mut vm = match wowlua_test::open_addon(suite) {
        Ok(vm) => vm,
        Err(e) => return (Some(e), true),
    };
    let started = Instant::now();
    let mut frames_run: u64 = 0;
    loop {
        vm.with_debugger(|d, vm| d.idle(vm));
        if shared.borrow().terminate || vm.aborted() {
            break;
        }
        // Input and events run here, the debugger attached.
        let queued = std::mem::take(&mut shared.borrow_mut().run_queue);
        for (id, req) in queued {
            let r = match &req {
                Request::Input { input } => crate::ui::input(&mut vm, input).map(|()| Response::Ok),
                Request::FireEvent { event, args } => {
                    crate::ui::fire_event(&mut vm, event, args).map(|()| Response::Ok)
                }
                _ => Err("not a request that runs code".into()),
            };
            shared.borrow_mut().link.respond(id, r);
        }
        if shared.borrow().terminate || vm.aborted() {
            break;
        }
        let advance = shared.borrow_mut().advance.take();
        if let Some((id, seconds, frames)) = advance {
            let n = match (frames, seconds) {
                (Some(f), _) => f as usize,
                (None, Some(s)) => (s * 1000.0 / FRAME_MS).ceil() as usize,
                (None, None) => 1,
            };
            wowlua_engine::clock::advance_frames(&mut vm, n, FRAME_MS);
            shared.borrow_mut().link.respond(id, Ok(Response::Ok));
            continue;
        }
        match opts.clock {
            ClockMode::Realtime => {
                wowlua_engine::clock::advance_frames(&mut vm, 1, FRAME_MS);
                frames_run += 1;
                let deadline =
                    started + Duration::from_secs_f64(frames_run as f64 * FRAME_MS / 1000.0);
                // Requests that come before the next frame wait for it.
                loop {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    let ms = (deadline - now).as_millis().max(1) as u64;
                    let next = shared.borrow_mut().link.recv(Some(ms));
                    match next {
                        Some(r) => {
                            shared.borrow_mut().deferred.push(r);
                            break;
                        }
                        None if shared.borrow().link.gone() => {
                            shared.borrow_mut().terminate = true;
                            break;
                        }
                        None => {}
                    }
                }
            }
            ClockMode::Manual => {
                // Nothing moves until asked.
                let next = shared.borrow_mut().link.recv(None);
                match next {
                    Some(r) => shared.borrow_mut().deferred.push(r),
                    None if shared.borrow().link.gone() => shared.borrow_mut().terminate = true,
                    None => {}
                }
            }
        }
    }
    (Some("the session ended".into()), false)
}
