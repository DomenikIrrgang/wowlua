//! The integration tests of `wowlua-engine`, one test binary: each
//! `tests/*.rs` is a module of it, so the crate and its dependencies are
//! linked once instead of once per file. A new file needs its `mod` line
//! here (`every_test_file_is_a_module` checks).

mod actions;
mod addon_answers;
mod argcheck;
mod auras;
mod bags_stack;
mod bars_auctions;
mod camera;
mod channels;
mod chat_log;
mod chat_windows;
mod clickbindings;
mod client;
mod client_services;
mod clock;
mod collected;
mod conditionals;
mod coverage;
mod curves;
mod cvars;
mod damagemeter;
mod elvui_answers;
mod encounters;
mod frames;
mod gamedata;
mod graphics;
mod guild;
mod guild_motd;
mod inspect;
mod install;
mod inventory;
mod lfg;
mod loot_rolls;
mod maps;
mod mouse_over;
mod movement;
mod npcs;
mod ns_actions2;
mod ns_collections;
mod ns_content;
mod ns_extras;
mod ns_globalsinfo;
mod ns_iteminfo;
mod ns_leftovers2;
mod ns_methods2;
mod ns_methods3;
mod ns_objects2;
mod ns_questinfo;
mod ns_social;
mod ns_spellinfo;
mod ns_systeminfo;
mod ns_unitinfo;
mod owed_probes;
mod places;
mod play;
mod player_changes;
mod powers;
mod protection;
mod racials;
mod raid_targets;
mod ranges;
mod restrictions;
mod standins;
mod stats;
mod suuui_probes;
mod suuui_trees;
mod taint;
mod targeting;
mod timeline;
mod trainers;
mod ui;
mod unit_actions;
mod unit_update;
mod units;
mod weather;
mod widgetops;

/// Each `tests/*.rs` file is a module here or a test target of its own:
/// `autotests = false` builds no other.
#[test]
fn every_test_file_is_a_module() {
    // (a Windows checkout ends its lines with CRLF)
    let main = include_str!("suite.rs").replace("\r\n", "\n");
    let own: &[&str] = &["suite"];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    #[allow(clippy::disallowed_methods)] // (the test reads its own directory)
    let entries = std::fs::read_dir(&dir).expect("tests/");
    for entry in entries {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let name = path
            .file_stem()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        assert!(
            own.contains(&name.as_str()) || main.contains(&format!("\nmod {name};\n")),
            "tests/{name}.rs is not a module of tests/suite.rs: add `mod {name};`"
        );
    }
}
