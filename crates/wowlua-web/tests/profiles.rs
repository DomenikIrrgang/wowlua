//! World profiles in the playground (spec M14, decision T401): the
//! package's listed, a run in one by name or from its text, their mistakes
//! found by a check, the running world changed, and a link that says how
//! the package runs. One test: the files' host is the program's.

use serde_json::json;
use wowlua_web::Playground;
use wowlua_web::run::{Options, Profile};

const MAIN: &str = r#"print(UnitName("player"), GetMoney())
local f = CreateFrame("Frame")
f:RegisterEvent("PLAYER_MONEY")
f:SetScript("OnEvent", function() print("money", GetMoney()) end)
"#;

fn run(p: &mut Playground, profile: Option<Profile>) -> Vec<String> {
    let started = p.run(Options {
        flavor: "camelot".into(),
        blizzard_ui: false,
        profile,
        debug: None,
        installation: None,
    });
    assert_eq!(started["ok"], true, "{started}");
    texts(&p.game().unwrap().take_output())
}

fn texts(output: &[serde_json::Value]) -> Vec<String> {
    output
        .iter()
        .map(|o| o["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn runs_start_in_world_profiles_and_the_world_changes() {
    let mut p = Playground::new();
    p.write_file("src/main.wowlua", MAIN);
    p.write_file(
        "tests/profiles/rich.json",
        r#"{ "character": { "name": "Thrall", "class": "SHAMAN", "race": "Orc", "level": 60 }, "money": 123456 }"#,
    );
    p.write_file(
        "tests/profiles/poor.json",
        r#"{ "character": { "name": "Peasant" }, "money": 1 }"#,
    );
    assert_eq!(
        wowlua_driver::profiles::list(std::path::Path::new(wowlua_web::ROOT))
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>(),
        ["poor", "rich"]
    );

    // by name, by text, and none (the fresh character)
    assert_eq!(
        run(&mut p, Some(Profile::Named("rich".into()))),
        ["Thrall 123456"]
    );
    assert_eq!(
        run(&mut p, Some(Profile::Text(r#"{ "money": 7 }"#.into())))[0]
            .split(' ')
            .nth(1),
        Some("7")
    );
    let started = p.run(Options {
        flavor: "camelot".into(),
        blizzard_ui: false,
        profile: Some(Profile::Named("stormwind".into())),
        debug: None,
        installation: None,
    });
    assert!(
        started["error"]
            .as_str()
            .unwrap()
            .contains("no world profile `stormwind`"),
        "{started}"
    );

    // the running world changes, the addon sees it
    run(&mut p, Some(Profile::Named("poor".into())));
    let game = p.game().unwrap();
    let out = game.world("mock.player:setMoney(500)").unwrap();
    assert_eq!(texts(&out), ["money 500"]);
    let err = game.world("mock.nothing()").unwrap_err();
    assert!(err.contains("[world]:1"), "{err}");

    // a check finds a profile's mistakes and a name no profile has
    p.write_file("tests/profiles/broken.json", r#"{ "monies": 5 }"#);
    p.write_file(
        "tests/main.test.wowlua",
        "import test, config from \"wowlua:test\"\nconfig({ profile = \"stormwind\" })\n\n@test(\"x\")\nlocal function x()\nend\n",
    );
    let found = p.check();
    let codes: Vec<(&str, &str)> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap_or(""),
                d["file"].as_str().unwrap_or(""),
            )
        })
        .collect();
    assert!(
        codes.contains(&("E4401", "tests/profiles/broken.json")),
        "{found}"
    );
    assert!(
        codes.contains(&("E4402", "tests/main.test.wowlua")),
        "{found}"
    );

    // a link says how the package runs
    let link = p.share(Some(&json!({ "flavor": "camelot", "profile": "rich" })));
    p.reset([("wowlua.json".to_string(), "{}".to_string())]);
    let run_settings = p.open_shared(&link).unwrap();
    assert_eq!(run_settings["profile"], "rich");
    assert!(
        p.files()
            .iter()
            .any(|(path, _)| path == "tests/profiles/rich.json")
    );
}
