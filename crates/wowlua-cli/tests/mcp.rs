//! `wowlua mcp` (spec debugger §18.12.2): an agent drives an addon session.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

const ADDON: &str = r#"local f = CreateFrame("Frame", "CounterFrame", UIParent)
f:SetSize(200, 100)
f:SetPoint("CENTER")
local b = CreateFrame("Button", "CounterButton", f)
b:SetSize(100, 30)
b:SetPoint("CENTER")
local label = b:CreateFontString(nil, "OVERLAY")
label:SetFont("Fonts\\FRIZQT__.TTF", 12)
label:SetAllPoints()
label:SetText("Count")
CounterClicks = 0
b:SetScript("OnClick", function(self, button)
    CounterClicks = CounterClicks + 1
    label:SetText("Count " .. CounterClicks)
end)
"#;

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    id: u64,
}

impl Mcp {
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params });
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"));
        assert_eq!(v["id"], self.id, "{v:#}");
        v["result"].clone()
    }

    /// A tool's text (all its text parts) and whether it failed.
    fn tool(&mut self, name: &str, args: Value) -> (String, bool, Value) {
        let r = self.call("tools/call", json!({ "name": name, "arguments": args }));
        let text: Vec<String> = r["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["text"].as_str().map(str::to_string))
            .collect();
        (text.join("\n"), r["isError"] == true, r)
    }

    fn ok(&mut self, name: &str, args: Value) -> String {
        let (text, failed, _) = self.tool(name, args.clone());
        assert!(!failed, "{name} {args}: {text}");
        text
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn an_agent_drives_and_debugs_an_addon() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("wowlua.json"),
        r#"{ "package": { "name": "counter", "version": "1.0.0" }, "flavors": { "standard": {} }, "addon": { "title": "Counter" } }"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/counter.lua"), ADDON).unwrap();
    let mut child = Command::new(assert_cmd::cargo::cargo_bin("wowlua"))
        .arg("mcp")
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", root.join(".wowlua-home"))
        .env("WOWLUA_WOW_PATH", root.join("no-wow-here"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut m = Mcp {
        stdin: child.stdin.take().unwrap(),
        stdout: BufReader::new(child.stdout.take().unwrap()),
        child,
        id: 0,
    };
    let init = m.call("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }));
    assert_eq!(init["serverInfo"]["name"], "wowlua");
    let tools = m.call("tools/list", json!({}));
    assert!(tools["tools"].as_array().unwrap().len() > 20);

    let started = m.ok("start_session", json!({ "addon": true }));
    assert!(
        started.contains("session of the addon: running"),
        "{started}"
    );

    // Where the button is, in screenshot pixels; a click there.
    let found = m.ok("find_frames", json!({ "query": "CounterButton" }));
    let data: Value = serde_json::from_str(found.split("\n(session").next().unwrap()).unwrap();
    let rect = &data[0]["rect"];
    let (x, y) = (
        rect["x"].as_f64().unwrap() + rect["width"].as_f64().unwrap() / 2.0,
        rect["y"].as_f64().unwrap() + rect["height"].as_f64().unwrap() / 2.0,
    );
    m.ok("click", json!({ "x": x, "y": y }));
    assert_eq!(
        m.ok("eval", json!({ "expression": "CounterClicks" })),
        "1 (number)"
    );
    let tree = m.ok("frame_tree", json!({ "root": "CounterFrame" }));
    assert!(tree.contains("Count 1"), "{tree}");

    // A breakpoint in the handler, hit by a click.
    m.ok(
        "set_breakpoints",
        json!({ "source": "src/counter.lua", "lines": [14] }),
    );
    let clicked = m.ok("click", json!({ "frame": "CounterButton" }));
    assert!(clicked.contains("stopped (breakpoint)"), "{clicked}");
    let stack = m.ok("stack", json!({}));
    assert!(stack.contains("src/counter.lua:14"), "{stack}");
    assert!(
        stack.contains("CounterButton:OnClick (dispatch)"),
        "{stack}"
    );
    let vars = m.ok("variables", json!({}));
    assert!(vars.contains("button = \"LeftButton\""), "{vars}");
    assert_eq!(
        m.ok("eval", json!({ "expression": "CounterClicks", "frame": 1 })),
        "2 (number)"
    );
    let ran = m.ok("continue", json!({}));
    assert!(ran.starts_with("running"), "{ran}");
    assert_eq!(
        m.ok("eval", json!({ "expression": "CounterClicks" })),
        "2 (number)"
    );
    m.ok(
        "set_breakpoints",
        json!({ "source": "src/counter.lua", "lines": [] }),
    );

    // The profiler: the handler's cost.
    m.ok("profile_start", json!({}));
    for _ in 0..3 {
        m.ok("click", json!({ "frame": "CounterButton" }));
    }
    m.ok("advance", json!({ "frames": 10 }));
    let speedscope = root.join("profile.json");
    let report = m.ok(
        "profile_stop",
        json!({ "speedscope": speedscope.to_string_lossy() }),
    );
    assert!(report.contains("script CounterButton:OnClick"), "{report}");
    assert!(report.contains("function src/counter.lua:12"), "{report}");
    let ss: Value = serde_json::from_slice(&fs::read(&speedscope).unwrap()).unwrap();
    assert_eq!(ss["profiles"].as_array().unwrap().len(), 3);

    // A screenshot, where there is a GPU.
    let (text, failed, raw) = m.tool("screenshot", json!({ "frame": "CounterFrame" }));
    if failed {
        assert!(text.contains("GPU"), "{text}");
    } else {
        assert_eq!(raw["content"][0]["type"], "image");
        assert_eq!(raw["content"][0]["mimeType"], "image/png");
    }
    assert_eq!(m.ok("stop_session", json!({})), "the session ended");
}
