//! The Model Context Protocol (spec debugger §18.12.2, decision T314):
//! `wowlua mcp` serves tools for AI agents on stdio: sessions of a test
//! file or the addon (manually clocked unless asked), screenshots as
//! images, the frame tree, input, time, evaluation, breakpoints, stepping
//! and profiles. A tool that runs code waits until the session stops or
//! settles, and answers with its state and what it printed.

use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::dap::Launcher;
use crate::protocol::*;
use crate::session::{ChannelLink, ClockMode, Options, Outgoing};

/// How long a tool waits for the session to stop.
const WAIT: Duration = Duration::from_secs(120);

struct Session {
    tx: Sender<(u64, Request)>,
    rx: Receiver<Outgoing>,
    interrupt: Arc<AtomicBool>,
    next: u64,
    /// The session's state as last told: running, stopped (why, where), ended.
    state: String,
    ended: bool,
    output: Vec<String>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The addon (it runs on until stopped), not a test file.
    addon: bool,
}

impl Session {
    /// Takes in what the session told, until `until` holds or the time is up.
    fn pump(
        &mut self,
        timeout: Duration,
        until: &mut dyn FnMut(&Outgoing) -> bool,
    ) -> Option<Outgoing> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let o = match self.rx.recv_timeout(left) {
                Ok(o) => o,
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => {
                    self.ended = true;
                    return None;
                }
            };
            if let Outgoing::Event(e) = &o {
                self.note(e);
            }
            if until(&o) {
                return Some(o);
            }
        }
    }

    fn note(&mut self, e: &Event) {
        match e {
            Event::Stopped { reason, text, .. } => {
                self.state = format!(
                    "stopped ({}{})",
                    reason.dap(),
                    text.as_ref().map(|t| format!(": {t}")).unwrap_or_default()
                );
            }
            Event::Continued => self.state = "running".into(),
            Event::Output { line } => {
                let text = line.text.trim_end().to_string();
                if !text.is_empty() {
                    self.output.push(format!("[{}] {text}", line.category));
                }
                if self.output.len() > 2000 {
                    self.output.drain(..1000);
                }
            }
            Event::Terminated { summary, failed } => {
                self.ended = true;
                self.state = format!(
                    "ended{}{}",
                    if *failed { " (failed)" } else { "" },
                    summary
                        .as_ref()
                        .map(|s| format!(": {s}"))
                        .unwrap_or_default()
                );
            }
            Event::Initialized | Event::Breakpoint { .. } => {}
        }
    }

    fn request(&mut self, req: Request) -> Result<Response, String> {
        self.request_until(req, false)
    }

    /// A request; with `or_stop`, the answer may wait for a stop it ran
    /// into (a click that hits a breakpoint): the stop answers then (the
    /// request's own answer comes when the session goes on, and is
    /// dropped).
    fn request_until(&mut self, req: Request, or_stop: bool) -> Result<Response, String> {
        if self.ended {
            return Err(format!("the session {}", self.state));
        }
        let id = self.next;
        self.next += 1;
        self.tx
            .send((id, req))
            .map_err(|_| "the session ended".to_string())?;
        self.interrupt.store(true, Ordering::Relaxed);
        let got = self.pump(WAIT, &mut |o| match o {
            Outgoing::Response(i, _) => *i == id,
            Outgoing::Event(Event::Stopped { .. }) => or_stop,
            _ => false,
        });
        match got {
            Some(Outgoing::Event(_)) => Ok(Response::Ok),
            Some(Outgoing::Response(_, r)) => r,
            _ if self.ended => Err(format!("the session {}", self.state)),
            _ => Err("the session did not answer in time (it may be busy; try pause)".into()),
        }
    }

    /// Waits until the session stops or ends (or settles: nothing told
    /// for `quiet`).
    fn wait_stop(&mut self, quiet: Duration) {
        let _ = self.pump(quiet, &mut |o| {
            matches!(
                o,
                Outgoing::Event(Event::Stopped { .. } | Event::Terminated { .. })
            )
        });
    }

    fn take_output(&mut self) -> String {
        if self.output.is_empty() {
            return String::new();
        }
        let lines = std::mem::take(&mut self.output);
        format!("\noutput:\n{}", lines.join("\n"))
    }
}

pub struct Server {
    launcher: Arc<dyn Launcher>,
    session: Option<Session>,
}

fn text(s: impl Into<String>) -> Json {
    json!({ "content": [{ "type": "text", "text": s.into() }] })
}

fn error(s: impl Into<String>) -> Json {
    json!({ "content": [{ "type": "text", "text": s.into() }], "isError": true })
}

fn str_arg(a: &Json, k: &str) -> Option<String> {
    a.get(k).and_then(Json::as_str).map(str::to_string)
}

/// The tools, with their input schemas.
fn tools() -> Json {
    let t = |name: &str, desc: &str, props: Json, required: &[&str]| {
        json!({
            "name": name,
            "description": desc,
            "inputSchema": { "type": "object", "properties": props, "required": required },
        })
    };
    let frame = json!({ "type": "string", "description": "A frame by its global name, or a path from one (MyFrame.title)" });
    let point = json!({ "type": "number", "description": "Screen pixels from the top left, as screenshots show them" });
    json!([
        t(
            "start_session",
            "Starts the emulated WoW client with the package: a test file (its tests run, stopping at breakpoints) or the addon itself (logged in, its UI live; time moves only with advance unless clock is realtime). Answers with the session's state.",
            json!({
                "test": { "type": "string", "description": "A test file, relative to the package (tests/x.test.wowlua)" },
                "filter": { "type": "string", "description": "Only the tests whose names contain this" },
                "addon": { "type": "boolean", "description": "The addon itself instead of a test file" },
                "flavor": { "type": "string", "description": "The flavor (standard, camelot, vanilla, …); the package's first by default" },
                "profile": { "type": "string", "description": "A world profile (tests/profiles/<name>.json) to start in, over the manifest's test.profile" },
                "clock": { "type": "string", "enum": ["manual", "realtime"] },
                "stopOnEntry": { "type": "boolean" },
                "breakpoints": { "type": "array", "items": { "type": "object", "properties": { "source": { "type": "string" }, "line": { "type": "integer" }, "condition": { "type": "string" } }, "required": ["source", "line"] } },
                "functionBreakpoints": { "type": "array", "items": { "type": "string" }, "description": "Functions (Addon.module:method), API calls (C_Timer.After), event:NAME, script:OnClick, script:Frame:OnUpdate, timer" },
                "exceptions": { "type": "array", "items": { "type": "string", "enum": ["uncaught", "all", "taint", "blocked"] } },
            }),
            &[]
        ),
        t("stop_session", "Ends the session.", json!({}), &[]),
        t(
            "screenshot",
            "The UI as the client draws it (PNG): the whole screen, or one frame and what it holds.",
            json!({ "frame": frame, "scale": { "type": "number" } }),
            &[]
        ),
        t(
            "frame_tree",
            "Frames with their type, name, shown and visible state, rectangle (screen pixels) and text.",
            json!({ "root": frame, "hidden": { "type": "boolean", "description": "Hidden frames too" } }),
            &[]
        ),
        t(
            "find_frames",
            "Frames whose name, type or text contains the query.",
            json!({ "query": { "type": "string" } }),
            &["query"]
        ),
        t(
            "click",
            "Clicks a frame (by name) or a point, as the user would.",
            json!({ "frame": frame, "x": point, "y": point, "button": { "type": "string" } }),
            &[]
        ),
        t(
            "hover",
            "Moves the mouse onto a frame or point (none: away).",
            json!({ "frame": frame, "x": point, "y": point }),
            &[]
        ),
        t(
            "drag",
            "Drags one frame onto another.",
            json!({ "from": frame, "to": frame }),
            &["from", "to"]
        ),
        t(
            "type_text",
            "Types into the edit box with the keyboard focus (click one first).",
            json!({ "text": { "type": "string" } }),
            &["text"]
        ),
        t(
            "press_key",
            "Presses a key (ENTER, ESCAPE, CTRL-S, …).",
            json!({ "key": { "type": "string" } }),
            &["key"]
        ),
        t(
            "slash",
            "Runs a slash command as the user types it in chat (/wdemo).",
            json!({ "command": { "type": "string" } }),
            &["command"]
        ),
        t(
            "advance",
            "Moves the game's time on: frames (1/60 s each) or seconds; OnUpdate and timers run.",
            json!({ "frames": { "type": "integer" }, "seconds": { "type": "number" } }),
            &[]
        ),
        t(
            "fire_event",
            "Fires a documented game event at the addon's frames.",
            json!({ "event": { "type": "string" }, "args": { "type": "array" } }),
            &["event"]
        ),
        t(
            "world",
            "Changes the game world while the addon runs: Lua with `mock` the engine's controls, as tests drive it (`mock.player:setMoney(500)`, `mock.player:target(mock.spawnUnit({ name = \"Hogger\", reaction = \"hostile\" }))`, `mock.player:enterCombat()`).",
            json!({ "script": { "type": "string" } }),
            &["script"]
        ),
        t(
            "eval",
            "Evaluates Lua in the stopped frame (or the globals): locals and upvalues are readable and assignable.",
            json!({ "expression": { "type": "string" }, "frame": { "type": "integer", "description": "A stack frame's id (stopped)" } }),
            &["expression"]
        ),
        t(
            "set_breakpoints",
            "Sets a source's line breakpoints (replacing its others).",
            json!({ "source": { "type": "string" }, "lines": { "type": "array", "items": { "type": "integer" } }, "condition": { "type": "string" } }),
            &["source", "lines"]
        ),
        t(
            "set_function_breakpoints",
            "Sets the function, API, event, script and timer breakpoints.",
            json!({ "names": { "type": "array", "items": { "type": "string" } } }),
            &["names"]
        ),
        t(
            "continue",
            "Runs on until the next stop or the end.",
            json!({}),
            &[]
        ),
        t(
            "step",
            "Steps: over (the next line here), into (into calls and handlers), out (to the caller or the next handler).",
            json!({ "kind": { "type": "string", "enum": ["over", "into", "out"] } }),
            &[]
        ),
        t(
            "pause",
            "Pauses at the next line run, or between frames.",
            json!({}),
            &[]
        ),
        t(
            "stack",
            "The stopped stack: Lua functions, natives and the engine's dispatches (events, scripts, OnUpdate, timers).",
            json!({}),
            &[]
        ),
        t(
            "variables",
            "A stack frame's locals, upvalues and varargs (the top frame's by default), or a reference's children.",
            json!({ "frame": { "type": "integer" }, "reference": { "type": "integer" } }),
            &[]
        ),
        t(
            "profile_start",
            "Starts the profiler (calls, instructions, bytes allocated, host time per function and dispatch).",
            json!({}),
            &[]
        ),
        t(
            "profile_stop",
            "Stops the profiler: the costliest functions; optionally writes a speedscope file.",
            json!({ "top": { "type": "integer" }, "speedscope": { "type": "string", "description": "A path to write the speedscope JSON to" } }),
            &[]
        ),
        t(
            "output",
            "What the session printed and the errors it raised since asked last.",
            json!({}),
            &[]
        ),
    ])
}

impl Server {
    pub fn new(launcher: Arc<dyn Launcher>) -> Server {
        Server {
            launcher,
            session: None,
        }
    }

    fn session(&mut self) -> Result<&mut Session, Json> {
        self.session
            .as_mut()
            .ok_or_else(|| error("no session: start_session first"))
    }

    fn start(&mut self, a: &Json) -> Json {
        if let Some(s) = &mut self.session
            && !s.ended
        {
            let _ = s.request(Request::Terminate);
        }
        self.session = None;
        let root = self.launcher.root();
        let mut opts_json = a.clone();
        if opts_json.get("test").is_none() && opts_json.get("addon").is_none() {
            opts_json["addon"] = json!(true);
        }
        let opts = match Options::from_launch(&opts_json, &root, ClockMode::Manual) {
            Ok(o) => o,
            Err(e) => return error(e),
        };
        let (tx, rx) = channel::<(u64, Request)>();
        let (otx, orx) = channel::<Outgoing>();
        let interrupt = Arc::new(AtomicBool::new(false));
        let launcher = self.launcher.clone();
        let flag = interrupt.clone();
        let err_tx = otx.clone();
        let o2 = opts.clone();
        let thread = std::thread::Builder::new()
            .name("wowlua session".into())
            .stack_size(64 << 20)
            .spawn(move || {
                let mut rx = Some(rx);
                let mut otx = Some(otx);
                let r = launcher.run(&o2, &mut |suite, shot| {
                    let (Some(rx), Some(otx)) = (rx.take(), otx.take()) else {
                        return;
                    };
                    crate::session::run(
                        suite,
                        &o2,
                        Box::new(ChannelLink::new(rx, otx)),
                        flag.clone(),
                        shot,
                    );
                });
                if let Err(e) = r {
                    let _ = err_tx.send(Outgoing::Event(Event::Terminated {
                        summary: Some(e),
                        failed: true,
                    }));
                }
            })
            .ok();
        let mut s = Session {
            tx,
            rx: orx,
            interrupt,
            next: 1,
            state: "starting".into(),
            ended: false,
            output: Vec::new(),
            thread,
            addon: matches!(opts.target, crate::session::Target::Addon),
        };
        // Built and waiting for its configuration (a build takes a while).
        let ready = s.pump(Duration::from_secs(900), &mut |o| {
            matches!(
                o,
                Outgoing::Event(Event::Initialized | Event::Terminated { .. })
            )
        });
        if s.ended || ready.is_none() {
            let state = s.state.clone();
            let out = s.take_output();
            return error(format!("the session did not start: {state}{out}"));
        }
        let mut problems = Vec::new();
        if let Some(bps) = a.get("breakpoints").and_then(Json::as_array) {
            let mut by_source: std::collections::BTreeMap<String, Vec<LineBreakpoint>> =
                Default::default();
            for b in bps {
                let (Some(src), Some(line)) =
                    (str_arg(b, "source"), b.get("line").and_then(Json::as_u64))
                else {
                    continue;
                };
                by_source.entry(src).or_default().push(LineBreakpoint {
                    line: line as u32,
                    condition: str_arg(b, "condition"),
                    ..Default::default()
                });
            }
            for (source, breakpoints) in by_source {
                if let Err(e) = s.request(Request::SetBreakpoints {
                    source,
                    breakpoints,
                }) {
                    problems.push(e);
                }
            }
        }
        if let Some(names) = a.get("functionBreakpoints").and_then(Json::as_array) {
            let breakpoints = names
                .iter()
                .filter_map(|n| n.as_str())
                .map(|n| FunctionBreakpoint {
                    name: n.to_string(),
                    condition: None,
                })
                .collect();
            if let Err(e) = s.request(Request::SetFunctionBreakpoints { breakpoints }) {
                problems.push(e);
            }
        }
        let filters: Vec<String> = a
            .get("exceptions")
            .and_then(Json::as_array)
            .map(|f| {
                f.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_else(|| vec!["uncaught".into(), "blocked".into()]);
        let _ = s.request(Request::SetExceptionBreakpoints { filters });
        let _ = s.request(Request::ConfigurationDone);
        s.state = "running".into();
        // A test file runs to a stop or its end; the addon logs in and waits.
        let quiet = if matches!(opts.target, crate::session::Target::Addon) {
            Duration::from_secs(3)
        } else {
            WAIT
        };
        s.wait_stop(quiet);
        let state = s.state.clone();
        let out = s.take_output();
        let what = match &opts.target {
            crate::session::Target::Test { file, .. } => format!("test file {file}"),
            crate::session::Target::Addon => "the addon".into(),
        };
        self.session = Some(s);
        let problems = if problems.is_empty() {
            String::new()
        } else {
            format!("\nproblems: {}", problems.join("; "))
        };
        text(format!("session of {what}: {state}{problems}{out}"))
    }

    /// A request answered as text (its data as JSON).
    fn simple(&mut self, req: Request, wait: bool) -> Json {
        let s = match self.session() {
            Ok(s) => s,
            Err(e) => return e,
        };
        let r = s.request_until(req, wait);
        if wait && !s.state.starts_with("stopped") {
            s.wait_stop(Duration::from_millis(500));
        }
        let state = s.state.clone();
        let out = s.take_output();
        match r {
            Ok(Response::Data { data }) => text(format!(
                "{}\n(session {state}){out}",
                serde_json::to_string_pretty(&data).unwrap_or_default()
            )),
            Ok(Response::Value { variable }) => text(format!(
                "{} ({}{}){out}",
                variable.value,
                variable.ty,
                if variable.taint == "secure" {
                    String::new()
                } else {
                    format!(", tainted by {}", variable.taint)
                }
            )),
            Ok(_) => text(format!("ok (session {state}){out}")),
            Err(e) => error(format!("{e}{out}")),
        }
    }

    fn run_on(&mut self, req: Request) -> Json {
        let s = match self.session() {
            Ok(s) => s,
            Err(e) => return e,
        };
        if let Err(e) = s.request(req) {
            return error(e);
        }
        s.state = "running".into();
        // The addon runs on: it settles when nothing stops it for a while.
        let wait = if s.addon {
            Duration::from_secs(2)
        } else {
            WAIT
        };
        s.wait_stop(wait);
        let state = s.state.clone();
        let out = s.take_output();
        let mut where_ = String::new();
        if state.starts_with("stopped")
            && let Ok(Response::StackTrace { frames }) = s.request(Request::StackTrace)
            && let Some(f) = frames.iter().find(|f| f.kind == "lua")
        {
            where_ = format!(
                " at {}:{} in {}",
                f.source.as_deref().or(f.chunk.as_deref()).unwrap_or("?"),
                f.line,
                f.name
            );
        }
        text(format!("{state}{where_}{out}"))
    }

    fn call(&mut self, name: &str, a: &Json) -> Json {
        let input = |i: Input| Request::Input { input: i };
        match name {
            "start_session" => self.start(a),
            "stop_session" => match self.session.take() {
                Some(mut s) => {
                    let _ = s.request(Request::Terminate);
                    drop(s.tx);
                    if let Some(t) = s.thread.take() {
                        let _ = t.join();
                    }
                    text("the session ended")
                }
                None => text("no session"),
            },
            "screenshot" => {
                let s = match self.session() {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                match s.request(Request::Screenshot {
                    frame: str_arg(a, "frame"),
                    scale: a.get("scale").and_then(Json::as_f64),
                }) {
                    Ok(Response::Image {
                        png_base64,
                        width,
                        height,
                    }) => json!({
                        "content": [
                            { "type": "image", "data": png_base64, "mimeType": "image/png" },
                            { "type": "text", "text": format!("{width}x{height} pixels; positions in frame_tree are in these pixels (at scale 1)") },
                        ],
                    }),
                    Ok(_) => error("no image"),
                    Err(e) => error(e),
                }
            }
            "frame_tree" => self.simple(
                Request::FrameTree {
                    root: str_arg(a, "root"),
                    hidden: a.get("hidden").and_then(Json::as_bool).unwrap_or(false),
                },
                false,
            ),
            "find_frames" => self.simple(
                Request::FindFrames {
                    query: str_arg(a, "query").unwrap_or_default(),
                },
                false,
            ),
            "click" => self.simple(
                input(Input::Click {
                    frame: str_arg(a, "frame"),
                    x: a.get("x").and_then(Json::as_f64),
                    y: a.get("y").and_then(Json::as_f64),
                    button: str_arg(a, "button").unwrap_or_else(|| "LeftButton".into()),
                }),
                true,
            ),
            "hover" => self.simple(
                input(Input::Hover {
                    frame: str_arg(a, "frame"),
                    x: a.get("x").and_then(Json::as_f64),
                    y: a.get("y").and_then(Json::as_f64),
                }),
                true,
            ),
            "drag" => self.simple(
                input(Input::Drag {
                    from: str_arg(a, "from").unwrap_or_default(),
                    to: str_arg(a, "to").unwrap_or_default(),
                }),
                true,
            ),
            "type_text" => self.simple(
                input(Input::Type {
                    text: str_arg(a, "text").unwrap_or_default(),
                }),
                true,
            ),
            "press_key" => self.simple(
                input(Input::Key {
                    key: str_arg(a, "key").unwrap_or_default(),
                }),
                true,
            ),
            "slash" => self.simple(
                input(Input::Slash {
                    text: str_arg(a, "command").unwrap_or_default(),
                }),
                true,
            ),
            "advance" => self.simple(
                Request::Advance {
                    seconds: a.get("seconds").and_then(Json::as_f64),
                    frames: a.get("frames").and_then(Json::as_u64).map(|f| f as u32),
                },
                true,
            ),
            "fire_event" => self.simple(
                Request::FireEvent {
                    event: str_arg(a, "event").unwrap_or_default(),
                    args: a
                        .get("args")
                        .and_then(Json::as_array)
                        .cloned()
                        .unwrap_or_default(),
                },
                true,
            ),
            "world" => self.simple(
                Request::World {
                    script: str_arg(a, "script").unwrap_or_default(),
                },
                true,
            ),
            "eval" => self.simple(
                Request::Evaluate {
                    expression: str_arg(a, "expression").unwrap_or_default(),
                    frame: a.get("frame").and_then(Json::as_u64).map(|f| f as u32),
                    context: EvalContext::Repl,
                },
                false,
            ),
            "set_breakpoints" => {
                let lines: Vec<LineBreakpoint> = a
                    .get("lines")
                    .and_then(Json::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(Json::as_u64)
                            .map(|line| LineBreakpoint {
                                line: line as u32,
                                condition: str_arg(a, "condition"),
                                ..Default::default()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let s = match self.session() {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                match s.request(Request::SetBreakpoints {
                    source: str_arg(a, "source").unwrap_or_default(),
                    breakpoints: lines,
                }) {
                    Ok(Response::Breakpoints { breakpoints }) => text(
                        breakpoints
                            .iter()
                            .map(|b| {
                                format!(
                                    "line {}: {}",
                                    b.line.unwrap_or(0),
                                    if b.verified {
                                        "set".to_string()
                                    } else {
                                        b.message.clone().unwrap_or_default()
                                    }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    Ok(_) => text("ok"),
                    Err(e) => error(e),
                }
            }
            "set_function_breakpoints" => {
                let names: Vec<FunctionBreakpoint> = a
                    .get("names")
                    .and_then(Json::as_array)
                    .map(|n| {
                        n.iter()
                            .filter_map(Json::as_str)
                            .map(|n| FunctionBreakpoint {
                                name: n.into(),
                                condition: None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.simple(
                    Request::SetFunctionBreakpoints { breakpoints: names },
                    false,
                )
            }
            "continue" => self.run_on(Request::Continue),
            "step" => self.run_on(match str_arg(a, "kind").as_deref() {
                Some("into") => Request::StepIn,
                Some("out") => Request::StepOut,
                _ => Request::Next,
            }),
            "pause" => self.run_on(Request::Pause),
            "stack" => {
                let s = match self.session() {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                match s.request(Request::StackTrace) {
                    Ok(Response::StackTrace { frames }) => text(
                        frames
                            .iter()
                            .map(|f| {
                                let place = match (&f.source, &f.chunk) {
                                    (Some(s), _) => format!(" {s}:{}", f.line),
                                    (None, Some(c)) => format!(" {c}:{}", f.line),
                                    _ => String::new(),
                                };
                                let taint = if f.taint == "secure" {
                                    String::new()
                                } else {
                                    format!(" [tainted by {}]", f.taint)
                                };
                                format!("#{} {} ({}){place}{taint}", f.id, f.name, f.kind)
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    Ok(_) => text("no stack"),
                    Err(e) => error(e),
                }
            }
            "variables" => {
                let s = match self.session() {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let reference = match a.get("reference").and_then(Json::as_u64) {
                    Some(r) => r as u32,
                    None => {
                        let frame = a.get("frame").and_then(Json::as_u64).unwrap_or(1) as u32;
                        match s.request(Request::Scopes { frame }) {
                            Ok(Response::Scopes { scopes }) => {
                                let mut out = Vec::new();
                                for sc in scopes.iter().filter(|s| !s.expensive) {
                                    if let Ok(Response::Variables { variables }) =
                                        s.request(Request::Variables {
                                            reference: sc.reference,
                                        })
                                    {
                                        out.push(format!("{}:", sc.name));
                                        for v in variables {
                                            out.push(variable_line(&v));
                                        }
                                    }
                                }
                                return text(out.join("\n"));
                            }
                            Ok(_) => return text("no scopes"),
                            Err(e) => return error(e),
                        }
                    }
                };
                match s.request(Request::Variables { reference }) {
                    Ok(Response::Variables { variables }) => text(
                        variables
                            .iter()
                            .map(variable_line)
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    Ok(_) => text("none"),
                    Err(e) => error(e),
                }
            }
            "profile_start" => self.simple(Request::ProfileStart, false),
            "profile_stop" => {
                let s = match self.session() {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                match s.request(Request::ProfileStop) {
                    Ok(Response::Data { data }) => {
                        let top = a.get("top").and_then(Json::as_u64).unwrap_or(30) as usize;
                        let mut lines = vec![format!(
                            "{:>12} {:>12} {:>10} {:>8}  function",
                            "total instr", "self instr", "bytes", "calls"
                        )];
                        if let Some(fs) = data["functions"].as_array() {
                            for f in fs.iter().take(top) {
                                lines.push(format!(
                                    "{:>12} {:>12} {:>10} {:>8}  {}",
                                    f["total"]["instructions"],
                                    f["self"]["instructions"],
                                    f["total"]["bytes"],
                                    f["calls"],
                                    f["name"].as_str().unwrap_or("")
                                ));
                            }
                        }
                        if let Some(path) = str_arg(a, "speedscope") {
                            match std::fs::write(
                                &path,
                                serde_json::to_vec(&data["speedscope"]).unwrap_or_default(),
                            ) {
                                Ok(()) => lines.push(format!("speedscope file: {path}")),
                                Err(e) => lines.push(format!("writing {path}: {e}")),
                            }
                        }
                        text(lines.join("\n"))
                    }
                    Ok(_) => text("no profile"),
                    Err(e) => error(e),
                }
            }
            "output" => {
                let s = match self.session() {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let _ = s.pump(Duration::from_millis(50), &mut |_| false);
                let out = std::mem::take(&mut s.output);
                text(if out.is_empty() {
                    "(nothing)".into()
                } else {
                    out.join("\n")
                })
            }
            other => error(format!("no tool {other}")),
        }
    }

    /// Answers one JSON-RPC message (`None` for notifications).
    pub fn handle(&mut self, msg: &Json) -> Option<Json> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Json::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let result = match method {
            "initialize" => json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "wowlua", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Drives the package's addon in wowlua's emulated WoW client. start_session first (the addon, or a test file); screenshot shows the UI; frame_tree and find_frames give frames and their pixel rectangles; click/type_text/press_key/advance act; breakpoints, step, stack, variables and eval debug; profile_start/profile_stop measure.",
            }),
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tools() }),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                self.call(&name, &args)
            }
            _ if id.is_none() => return None,
            _ => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("no method {method}") },
                }));
            }
        };
        id.map(|id| json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    /// Serves newline-delimited JSON-RPC until the input ends.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) {
        for line in input.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Json>(&line) {
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(
                    json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }),
                ),
            };
            if let Some(r) = reply {
                let _ = writeln!(output, "{r}");
                let _ = output.flush();
            }
        }
        if let Some(mut s) = self.session.take() {
            let _ = s.request(Request::Terminate);
        }
    }
}

fn variable_line(v: &Variable) -> String {
    let taint = if v.taint == "secure" {
        String::new()
    } else {
        format!(" [tainted by {}]", v.taint)
    };
    let secret = if v.secret { " [secret]" } else { "" };
    let more = if v.reference > 0 {
        format!(" (reference {})", v.reference)
    } else {
        String::new()
    };
    format!("  {} = {}{taint}{secret}{more}", v.name, v.value)
}
