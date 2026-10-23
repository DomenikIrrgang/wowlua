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
