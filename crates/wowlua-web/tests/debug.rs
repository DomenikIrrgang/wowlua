//! The debugger in the page (spec M14 S4): a breakpoint in a click
//! handler stops the run; the stopped stack, an expression's value, then
//! on. The page's mailbox stands in a queue of requests here. One test:
//! the files' host is the program's.

use serde_json::{Value as Json, json};
use wowlua_web::Playground;
use wowlua_web::debug::{QueueLink, Setup};
use wowlua_web::run::Options;

const MAIN: &str = r#"local count = 0
local button = CreateFrame("Button", "PlaygroundButton", UIParent)
button:SetSize(100, 40)
button:SetPoint("CENTER")
button:SetScript("OnClick", function()
    count = count + 41
    print("clicked", count + 1)
end)
"#;

fn request(id: u64, r: Json) -> (u64, wowlua_debug::Request) {
    (id, serde_json::from_value(r).expect("a request"))
}

#[test]
fn a_breakpoint_stops_a_click_and_shows_its_values() {
    let mut p = Playground::new();
    p.write_file("src/main.wowlua", MAIN);
    let link = QueueLink::default();
    link.requests.borrow_mut().extend([
        request(1, json!({ "command": "stackTrace" })),
        request(
            2,
            json!({ "command": "evaluate", "expression": "count", "frame": 1 }),
        ),
        request(3, json!({ "command": "next" })),
        request(
            4,
            json!({ "command": "evaluate", "expression": "count", "frame": 1 }),
        ),
        request(5, json!({ "command": "continue" })),
    ]);
    let started = p.run(Options {
        flavor: "camelot".into(),
        blizzard_ui: false,
        profile: None,
        debug: Some(Setup {
            link: Box::new(link.clone()),
            settings: json!({ "breakpoints": { "src/main.wowlua": [{ "line": 6 }] } }),
        }),
        installation: None,
    });
    assert_eq!(started["ok"], true, "{started}");
    let game = p.game().expect("running");
    game.advance(0.0).unwrap();
    let (w, h) = game.screen();
    game.input(&json!({ "action": "click", "x": w as f64 / 2.0, "y": h as f64 / 2.0 }))
        .unwrap();
    let printed: Vec<String> = game
        .take_output()
        .iter()
        .map(|o| o["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(printed, ["clicked 42"]);

    let sent = link.sent.borrow();
    let stopped: Vec<&Json> = sent.iter().filter(|m| m["event"] == "stopped").collect();
    assert_eq!(stopped[0]["reason"], "breakpoint", "{sent:?}");
    assert_eq!(stopped[1]["reason"], "step", "{sent:?}");
    let answer = |id: u64| {
        sent.iter()
            .find(|m| m["id"] == id)
            .unwrap_or_else(|| panic!("no answer {id}: {sent:?}"))
    };
    let top = &answer(1)["response"]["frames"][0];
    assert_eq!(top["source"], "src/main.wowlua", "{top}");
    assert_eq!(top["line"], 6, "{top}");
    assert_eq!(
        answer(2)["response"]["variable"]["value"],
        "0",
        "{:?}",
        answer(2)
    );
    assert_eq!(
        answer(4)["response"]["variable"]["value"],
        "41",
        "{:?}",
        answer(4)
    );
    drop(sent);

    // stopping the run ends the session
    p.stop();
    let sent = link.sent.borrow();
    assert_eq!(sent.last().unwrap()["event"], "terminated", "{sent:?}");
}
