//! `wowlua debug` (spec debugger §18.12.1): a DAP client stops a test at a
//! breakpoint in a script handler, inspects and steps it.

use std::fs;
use std::io::{BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::{Value, json};

const COUNTER: &str = r#"local count = 0
local button = CreateFrame("Button", "DemoButton", UIParent)
button:SetSize(100, 20)
button:SetPoint("CENTER")
button:SetScript("OnClick", function(self, mouse)
    local before = count
    count = count + 1
    DemoCount = count
    print("clicked", count)
end)
"#;

const TEST: &str = r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("a click counts")
local function clicks()
    mock.click(_G.DemoButton :: any)
    expect((_G :: any).DemoCount).toBe(1)
end
"#;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = r#"{ "package": { "name": "demo", "version": "1.0.0" }, "flavors": { "standard": {} },
  "addon": { "title": "Demo" } }"#;
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    for (path, text) in [
        ("src/counter.lua", COUNTER),
        ("tests/counter.test.wowlua", TEST),
    ] {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    seq: u64,
    /// Messages read but not yet asked for.
    seen: Vec<Value>,
}

impl Client {
    fn start(dir: &Path) -> Client {
        let mut child = Command::new(assert_cmd::cargo::cargo_bin("wowlua"))
            .arg("debug")
            .current_dir(dir)
            .env("NO_COLOR", "1")
            .env("WOWLUA_HOME", dir.join(".wowlua-home"))
            .env_remove("WOWLUA_WOW_PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            while let Some(m) = wowlua_debug::dap::read_message(&mut r) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        Client {
            child,
            stdin,
            rx,
            seq: 1,
            seen: Vec::new(),
        }
    }

    fn send(&mut self, command: &str, arguments: Value) -> u64 {
        let seq = self.seq;
        self.seq += 1;
        let msg =
            json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
        wowlua_debug::dap::write_message(&mut self.stdin, &msg).unwrap();
        self.stdin.flush().unwrap();
        seq
    }

    /// The first message (seen or coming) that `f` accepts.
    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self.seen.iter().position(&f) {
            return self.seen.remove(i);
        }
        loop {
            match self.rx.recv_timeout(Duration::from_secs(120)) {
                Ok(m) if f(&m) => return m,
                Ok(m) => self.seen.push(m),
                Err(_) => panic!("no {what}; seen: {:#?}", self.seen),
            }
        }
    }

    fn response(&mut self, seq: u64) -> Value {
        let r = self.wait(&format!("response to {seq}"), |m| {
            m["type"] == "response" && m["request_seq"] == seq
        });
        assert_eq!(r["success"], true, "{r:#}");
        r["body"].clone()
    }

    fn request(&mut self, command: &str, arguments: Value) -> Value {
        let seq = self.send(command, arguments);
        self.response(seq)
    }

    fn event(&mut self, name: &str) -> Value {
        let n = name.to_string();
        self.wait(&format!("{name} event"), move |m| {
            m["type"] == "event" && m["event"] == n.as_str()
        })["body"]
            .clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn stops_in_a_script_handler_inspects_and_steps() {
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    let mut c = Client::start(&root);
    let caps = c.request("initialize", json!({ "adapterID": "wowlua" }));
    assert_eq!(caps["supportsConfigurationDoneRequest"], true);
    let test = root.join("tests/counter.test.wowlua");
    c.request("launch", json!({ "test": test.to_string_lossy() }));
    c.event("initialized");
    let src = root.join("src/counter.lua");
    let bps = c.request(
        "setBreakpoints",
        json!({ "source": { "path": src.to_string_lossy() }, "breakpoints": [{ "line": 6 }] }),
    );
    assert_eq!(bps["breakpoints"][0]["line"], 6);
    c.request("setExceptionBreakpoints", json!({ "filters": [] }));
    c.request("configurationDone", json!({}));

    let stopped = c.event("stopped");
    assert_eq!(stopped["reason"], "breakpoint", "{stopped:#}");

    // The handler, then the script dispatch that ran it, then the test.
    let stack = c.request("stackTrace", json!({ "threadId": 1 }));
    let frames = stack["stackFrames"].as_array().unwrap();
    assert_eq!(frames[0]["line"], 6, "{stack:#}");
    assert!(
        frames[0]["source"]["path"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("src/counter.lua")
    );
    assert!(
        frames.iter().any(|f| f["name"] == "DemoButton:OnClick"),
        "a dispatch frame: {stack:#}"
    );
    assert!(
        frames.iter().any(|f| f["source"]["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("counter.test.wowlua"))),
        "the test below: {stack:#}"
    );
    let top = frames[0]["id"].as_u64().unwrap();

    // Its locals: the button and the mouse button.
    let scopes = c.request("scopes", json!({ "frameId": top }));
    let locals = scopes["scopes"][0]["variablesReference"].as_u64().unwrap();
    let vars = c.request("variables", json!({ "variablesReference": locals }));
    let vars = vars["variables"].as_array().unwrap();
    let mouse = vars.iter().find(|v| v["name"] == "mouse").expect("mouse");
    assert_eq!(mouse["value"], "\"LeftButton\"");
    let this = vars.iter().find(|v| v["name"] == "self").expect("self");
    assert!(
        this["value"]
            .as_str()
            .unwrap()
            .starts_with("Button \"DemoButton\""),
        "{this:#}"
    );
    // A widget expands to its layout first.
    let widget = c.request(
        "variables",
        json!({ "variablesReference": this["variablesReference"] }),
    );
    assert!(
        widget["variables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["name"] == "(rect)")
    );

    // Watches read locals and upvalues.
    let r = c.request(
        "evaluate",
        json!({ "expression": "count + 10", "frameId": top, "context": "watch" }),
    );
    assert_eq!(r["result"], "10");

    // A step to the next line, where `before` is set.
    c.request("next", json!({ "threadId": 1 }));
    let stopped = c.event("stopped");
    assert_eq!(stopped["reason"], "step");
    let stack = c.request("stackTrace", json!({ "threadId": 1 }));
    assert_eq!(stack["stackFrames"][0]["line"], 7);
    let top = stack["stackFrames"][0]["id"].as_u64().unwrap();
    let r = c.request(
        "evaluate",
        json!({ "expression": "before", "frameId": top }),
    );
    assert_eq!(r["result"], "0");

    // On to the end: the handler printed, the test passed.
    c.request("continue", json!({ "threadId": 1 }));
    let out = c.wait("printed output", |m| {
        m["event"] == "output"
            && m["body"]["output"]
                .as_str()
                .is_some_and(|o| o.contains("clicked 1"))
    });
    assert_eq!(out["body"]["category"], "stdout");
    let summary = c.wait("the summary", |m| {
        m["event"] == "output"
            && m["body"]["output"]
                .as_str()
                .is_some_and(|o| o.contains("passed"))
    });
    assert!(
        summary["body"]["output"]
            .as_str()
            .unwrap()
            .contains("1 passed, 0 failed"),
        "{summary:#}"
    );
    let exited = c.event("exited");
    assert_eq!(exited["exitCode"], 0);
    c.event("terminated");
}

/// A session names its flavor; an error in another addon's file
/// (`test.addons`) stops where it reached the error handler, its frame
/// carrying that file's path, so the editor opens it; a disconnect during
/// a stop ends the session there, not at the next breakpoint (decision
/// T358).
#[test]
fn names_its_flavor_opens_other_addons_files_and_ends_at_a_disconnect() {
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    // The first click calls into the other addon, which fails; a second
    // click would stop again after a disconnect that runs on.
    let counter = COUNTER.replace(
        "    print(\"clicked\", count)",
        "    print(\"clicked\", count)\n    if count == 1 then OtherBreak() end",
    );
    fs::write(root.join("src/counter.lua"), counter).unwrap();
    let twice = TEST.replace(
        "mock.click(_G.DemoButton :: any)",
        "mock.click(_G.DemoButton :: any)\n    mock.click(_G.DemoButton :: any)",
    );
    fs::write(
        root.join("tests/counter.test.wowlua"),
        twice.replace("toBe(1)", "toBe(2)"),
    )
    .unwrap();
    fs::create_dir_all(root.join("addons/Other")).unwrap();
    fs::write(
        root.join("addons/Other/Other.toc"),
        "## Title: Other\nother.lua\n",
    )
    .unwrap();
    fs::write(
        root.join("addons/Other/other.lua"),
        "function OtherBreak()\n    local x = nil\n    return x.field\nend\n",
    )
    .unwrap();
    let manifest = fs::read_to_string(root.join("wowlua.json")).unwrap();
    fs::write(
        root.join("wowlua.json"),
        manifest.replacen("{", r#"{ "test": { "addons": ["addons/Other"] },"#, 1),
    )
    .unwrap();

    let mut c = Client::start(&root);
    c.request("initialize", json!({ "adapterID": "wowlua" }));
    let test = root.join("tests/counter.test.wowlua");
    c.request(
        "launch",
        json!({ "test": test.to_string_lossy(), "flavor": "standard" }),
    );
    c.event("initialized");
    let src = root.join("src/counter.lua");
    c.request(
        "setBreakpoints",
        json!({ "source": { "path": src.to_string_lossy() }, "breakpoints": [{ "line": 7 }] }),
    );
    c.request(
        "setExceptionBreakpoints",
        json!({ "filters": ["uncaught"] }),
    );
    c.request("configurationDone", json!({}));

    let named = c.wait("the flavor's line", |m| {
        m["event"] == "output"
            && m["body"]["output"]
                .as_str()
                .is_some_and(|o| o.contains("on standard"))
    });
    assert_eq!(named["body"]["category"], "console");

    // The first click: the breakpoint, then the other addon's error in the
    // handler (it reaches the error handler: a stop), in its own file.
    assert_eq!(c.event("stopped")["reason"], "breakpoint");
    c.request("continue", json!({ "threadId": 1 }));
    let stopped = c.event("stopped");
    assert_eq!(stopped["reason"], "exception", "{stopped:#}");
    let stack = c.request("stackTrace", json!({ "threadId": 1 }));
    let path = stack["stackFrames"][0]["source"]["path"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    // (Windows: the root canonical, `\\?\C:\…`; the path as editors write it)
    let plain = |p: &Path| {
        let p = p.to_string_lossy().replace('\\', "/");
        p.strip_prefix("//?/").map(str::to_string).unwrap_or(p)
    };
    assert_eq!(
        plain(Path::new(&path)),
        plain(&root.join("addons/Other/other.lua")),
        "{stack:#}"
    );
    // Dimmed, the package's handler below it: the editor shows that.
    assert_eq!(
        stack["stackFrames"][0]["presentationHint"], "subtle",
        "{stack:#}"
    );
    assert!(
        stack["stackFrames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["source"]["path"]
                .as_str()
                .is_some_and(|p| p.replace('\\', "/").ends_with("src/counter.lua"))),
        "{stack:#}"
    );
    c.request("continue", json!({ "threadId": 1 }));

    // The second click's stop, then a disconnect: no stop after it.
    let stopped = c.event("stopped");
    assert_eq!(stopped["reason"], "breakpoint", "{stopped:#}");
    c.request("disconnect", json!({ "terminateDebuggee": true }));
    c.event("terminated");
    assert!(
        !c.seen.iter().any(|m| m["event"] == "stopped"),
        "stopped again after the disconnect: {:#?}",
        c.seen
    );
}

/// Debugging the addon itself (`addon: true`, no test): a console line
/// starting with `/` is a slash command, as typed in chat; other lines are
/// Lua, which sees what the command did.
#[test]
fn the_addons_session_takes_slash_commands_in_the_console() {
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("src/counter.lua"),
        "SLASH_DEMOCMD1 = \"/demo\"\nSlashCmdList.DEMOCMD = function(msg) DemoSlash = msg end\n",
    )
    .unwrap();
    let mut c = Client::start(&root);
    c.request("initialize", json!({ "adapterID": "wowlua" }));
    c.request("launch", json!({ "addon": true, "flavor": "standard" }));
    c.event("initialized");
    c.request("setExceptionBreakpoints", json!({ "filters": [] }));
    c.request("configurationDone", json!({}));
    let ran = c.request(
        "evaluate",
        json!({ "expression": "/demo hello there", "context": "repl" }),
    );
    assert_eq!(ran["result"], "", "{ran:#}");
    // What it prints shows as the chat frame shows it.
    c.request(
        "evaluate",
        json!({ "expression": "print('|cff33ff99Demo|r: 2 |4day:days;')", "context": "repl" }),
    );
    c.wait("the printed line, plain", |m| {
        m["event"] == "output" && m["body"]["output"] == "Demo: 2 days\n"
    });
    let value = c.request(
        "evaluate",
        json!({ "expression": "DemoSlash", "context": "repl" }),
    );
    assert_eq!(value["result"], "\"hello there\"", "{value:#}");
    c.request("disconnect", json!({ "terminateDebuggee": true }));
}

/// A `/reload` in the addon's session (`C_UI.Reload()`) loads the UI again as
/// the client does: the addon's SavedVariables kept, its breakpoints on the
/// new load too.
#[test]
fn the_addons_session_reloads_with_its_saved_variables() {
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("src/counter.lua"),
        "local f = CreateFrame(\"Frame\")\nf:RegisterEvent(\"ADDON_LOADED\")\nf:SetScript(\"OnEvent\", function(_, _, name)\n    if name:lower() ~= \"demo\" then return end\n    DemoDB = DemoDB or { loads = 0 }\n    DemoDB.loads = DemoDB.loads + 1\nend)\n",
    )
    .unwrap();
    let manifest = fs::read_to_string(root.join("wowlua.json")).unwrap();
    fs::write(
        root.join("wowlua.json"),
        manifest.replace(
            r#""addon": { "title": "Demo" }"#,
            r#""addon": { "title": "Demo", "saved-variables": ["DemoDB"] }"#,
        ),
    )
    .unwrap();
    let mut c = Client::start(&root);
    c.request("initialize", json!({ "adapterID": "wowlua" }));
    c.request("launch", json!({ "addon": true, "flavor": "standard" }));
    c.event("initialized");
    let src = root.join("src/counter.lua");
    c.request(
        "setBreakpoints",
        json!({ "source": { "path": src.to_string_lossy() }, "breakpoints": [{ "line": 6 }] }),
    );
    c.request("setExceptionBreakpoints", json!({ "filters": [] }));
    c.request("configurationDone", json!({}));
    // The first load stops at the breakpoint.
    assert_eq!(c.event("stopped")["reason"], "breakpoint");
    c.request("continue", json!({ "threadId": 1 }));
    let first = c.request(
        "evaluate",
        json!({ "expression": "DemoDB.loads", "context": "repl" }),
    );
    assert_eq!(first["result"], "1", "{first:#}");
    c.request(
        "evaluate",
        json!({ "expression": "C_UI.Reload()", "context": "repl" }),
    );
    // The reload's load stops there again, the saved count carried over.
    assert_eq!(c.event("stopped")["reason"], "breakpoint");
    c.request("continue", json!({ "threadId": 1 }));
    let second = c.request(
        "evaluate",
        json!({ "expression": "DemoDB.loads", "context": "repl" }),
    );
    assert_eq!(second["result"], "2", "{second:#}");
    assert!(
        c.seen.iter().any(|m| m["event"] == "output"
            && m["body"]["output"]
                .as_str()
                .is_some_and(|o| o.contains("Reloading the UI"))),
        "{:#?}",
        c.seen
    );
    c.request("disconnect", json!({ "terminateDebuggee": true }));
}

/// A launch's `profile` starts the session in that world profile
/// (`tests/profiles/<name>.json`): its character, named as it says.
#[test]
fn a_launch_starts_in_its_world_profile() {
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("tests/profiles")).unwrap();
    fs::write(
        root.join("tests/profiles/ann.json"),
        r#"{ "character": { "name": "Ann", "lastName": "Smith", "realm": "Realm", "level": 42 } }"#,
    )
    .unwrap();
    let mut c = Client::start(&root);
    c.request("initialize", json!({ "adapterID": "wowlua" }));
    c.request(
        "launch",
        json!({ "addon": true, "flavor": "standard", "profile": "ann" }),
    );
    c.event("initialized");
    c.request("setExceptionBreakpoints", json!({ "filters": [] }));
    c.request("configurationDone", json!({}));
    let name = c.request(
        "evaluate",
        json!({ "expression": "strjoin(' ', UnitName('player')) .. ' ' .. UnitLevel('player')", "context": "repl" }),
    );
    assert_eq!(name["result"], "\"Ann Smith 42\"", "{name:#}");
    c.request("disconnect", json!({ "terminateDebuggee": true }));
}

#[test]
fn test_profile_out_writes_a_speedscope_file_per_test_file() {
    let dir = project();
    let out = Command::new(assert_cmd::cargo::cargo_bin("wowlua"))
        .args(["test", "--profile-out", "prof"])
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.path().join(".wowlua-home"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let file = dir
        .path()
        .join("prof/tests_counter.test.wowlua.standard.speedscope.json");
    let json: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(json["exporter"], "wowlua");
    let frames = json["shared"]["frames"].as_array().unwrap();
    assert!(
        frames
            .iter()
            .any(|f| f["name"] == "script DemoButton:OnClick"),
        "{frames:?}"
    );
    assert!(
        frames.iter().any(|f| f["file"] == "src/counter.lua"),
        "{frames:?}"
    );
}
