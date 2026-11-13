//! The WoW client's engine for the wowlua VM (spec emulator §17.6): the API
//! a flavor's documentation describes, as natives of the VM (decision T198),
//! with behaviour where tests need it; a function without behaviour fails
//! with "`<name>` is not mocked", so gaps are visible.
//!
//! The engine is two crates, so each builds in less time: its state (the
//! widgets, the units, the world), the API's arguments and the modules the
//! others build on are `wowlua-engine-core`'s, re-exported here as they
//! were; this crate has the other modules, and [`install`] puts them
//! together.

pub use wowlua_engine_core::*;

pub mod auction;
pub mod content;
pub mod controls;
pub mod coverage;
pub mod extras;
pub mod formatters;
pub mod gamedata;
pub mod globalsinfo;
pub mod iteminfo;
pub mod leftovers2;
pub mod lfg;
pub mod lootroll;
pub mod methods2;
pub mod methods3;
pub mod misc;
pub mod models;
pub mod objects2;
pub mod play;
pub mod queries;
pub mod questinfo;
pub mod scene;
pub mod social;
pub mod spellinfo;
pub mod standins;
pub mod systeminfo;
pub mod targeting;
pub mod taxi;
pub mod tooltip;
pub mod tradeskill;
pub mod trainer;
pub mod traits;
pub mod unitinfo;
pub mod unitnames;

use wowlua_vm::{NativeFn, Vm};

/// Installs the engine in the VM for a flavor (`wowlua_engine_core::install`,
/// with this crate's modules).
pub fn install(vm: &mut Vm, flavor: &str) -> Result<(), String> {
    hooks::set(&HOOKS);
    wowlua_engine_core::install(vm, flavor)
}

/// What the core calls of this crate's modules.
static HOOKS: hooks::Hooks = hooks::Hooks {
    own_behaviours,
    fill_ins,
    method_behaviours,
    standins_install: standins::install,
    silent_unchecked: extras::silent_unchecked,
    silent_checked: extras::silent_checked,
    macro_spell: spellinfo::macro_spell,
    spell_of: spellinfo::spell_of,
    in_range: spellinfo::in_range,
    range_checks_tick: spellinfo::range_checks_tick,
    item_data: gamedata::item_data,
    spell_data: gamedata::spell_data,
    spell_name: gamedata::spell_name,
    item_id: gamedata::item_id,
    scanned_spell: gamedata::scanned_spell,
    tracking_aura_changed: systeminfo::tracking_aura_changed,
    give_up_asked: tradeskill::give_up_asked,
    tradeskill_answer: tradeskill::any_answer,
    tradeskill_open: tradeskill::control_open,
    tradeskill_skill_ups: tradeskill::control_skill_ups,
    tradeskill_spell_focus: tradeskill::control_spell_focus,
    tradeskill_timings: tradeskill::control_timings,
    beast_training: trainer::beast_training,
    trainer_open: trainer::open,
    pet_left: trainer::pet_left,
    trainer_learn: trainer::control_learn,
    trainer_unlearn: trainer::control_unlearn,
    look_of: scene::look_of,
    format_with: formatters::format_with,
    navigation_frame: questinfo::navigation_frame,
    poi_map: questinfo::poi_map,
    quest_poi_arrived: questinfo::quest_poi_arrived,
    get_num_languages: globalsinfo::get_num_languages,
    taxi_open: taxi::open,
    file_id_of_path: misc::file_id_of_path,
    auction_has: auction::has,
    auction_interact: auction::interact,
    traits_answer: traits::owner_answer,
    player_config: traits::player_config,
};

/// The fill-in modules' behaviours: answers for functions no recording
/// or model of the engine answers, from Blizzard's documentation and UI
/// code, warcraft.wiki.gg and the functions' names (unverified).
pub fn fill_ins() -> Vec<Behaviours<NativeFn>> {
    vec![
        Behaviours::new("unitinfo", unitinfo::behaviours()),
        Behaviours::new("spellinfo", spellinfo::behaviours()),
        Behaviours::new("iteminfo", iteminfo::behaviours()),
        Behaviours::new("social", social::behaviours()),
        Behaviours::new("content", content::behaviours()),
        Behaviours::new("questinfo", questinfo::behaviours()),
        Behaviours::new("collections", collections::behaviours()),
        Behaviours::new("extras", extras::behaviours()),
        Behaviours::new("globalsinfo", globalsinfo::behaviours()),
        Behaviours::new("systeminfo", systeminfo::behaviours()),
        Behaviours::new("actions2", actions2::behaviours()),
        Behaviours::new("leftovers2", leftovers2::behaviours()),
        // Last: what the client answers with nothing there (the full
        // probe) takes the place of the guesses above.
        Behaviours::new("blanks", blanks::behaviours()),
    ]
}

/// The engine's own behaviours by module, in the order they take each
/// other's place (a later module's over an earlier one's): the recorded
/// answers first.
pub fn own_behaviours(flavor: &str) -> Vec<Behaviours<NativeFn>> {
    vec![
        Behaviours::recorded("recorded", recorded::behaviours(flavor)),
        Behaviours::recorded("traits", traits::recorded_behaviours(flavor)),
        // The client's tables over the recordings, where the host has them.
        Behaviours::new("clientdata", clientdata::behaviours()),
        Behaviours::new("clock", clock::behaviours()),
        Behaviours::new("units", units::behaviours()),
        Behaviours::new("units", units::more_behaviours()),
        Behaviours::new("clickbindings", clickbindings::behaviours()),
        Behaviours::new("targeting", targeting::behaviours()),
        Behaviours::new("raidtargets", raidtargets::behaviours()),
        Behaviours::new("tapping", tapping::behaviours()),
        Behaviours::new("leftovers3", leftovers3::behaviours()),
        Behaviours::new("curves", curves::behaviours()),
        Behaviours::new("healcalc", healcalc::behaviours()),
        Behaviours::new("damagemeter", damagemeter::behaviours()),
        Behaviours::new("formatters", formatters::behaviours()),
        Behaviours::new("widgets", widgets::functions()),
        Behaviours::new("auras", auras::behaviours()),
        Behaviours::new("gamedata", gamedata::behaviours()),
        Behaviours::new("character", character::behaviours()),
        Behaviours::new("stats", stats::behaviours()),
        Behaviours::new("actions", actions::behaviours()),
        Behaviours::new("editmode", editmode::behaviours()),
        Behaviours::new("channels", channels::behaviours()),
        Behaviours::new("macros", macros::behaviours()),
        Behaviours::new("cursor", cursor::behaviours()),
        Behaviours::new("input", input::behaviours()),
        Behaviours::new("casting", casting::behaviours()),
        Behaviours::new("merchant", merchant::behaviours()),
        Behaviours::new("loot", loot::behaviours()),
        Behaviours::new("lootroll", lootroll::behaviours()),
        Behaviours::new("quests", quests::behaviours()),
        Behaviours::new("gossip", gossip::behaviours()),
        Behaviours::new("trainer", trainer::behaviours()),
        Behaviours::new("taxi", taxi::behaviours()),
        Behaviours::new("maps", maps::behaviours()),
        Behaviours::new("misc", misc::behaviours()),
        Behaviours::new("mail", mail::behaviours()),
        Behaviours::new("bank", bank::behaviours()),
        Behaviours::new("auction", auction::behaviours()),
        Behaviours::new("itemlocation", itemlocation::behaviours()),
        Behaviours::new("death", death::behaviours()),
        Behaviours::new("inventory", inventory::behaviours()),
        Behaviours::new("queries", queries::behaviours()),
        Behaviours::new("objects2", objects2::behaviours()),
        Behaviours::new("party", party::behaviours()),
        Behaviours::new("inspect", inspect::behaviours()),
        Behaviours::new("traits", traits::behaviours(flavor)),
        Behaviours::new("classic", classic::behaviours(flavor)),
        Behaviours::new("achievements", achievements::behaviours()),
        Behaviours::new("tradeskill", tradeskill::behaviours(flavor)),
        Behaviours::new("tooltip", tooltip::behaviours()),
        Behaviours::new("chat", chat::behaviours()),
        Behaviours::new("movement", movement::behaviours()),
        Behaviours::new("camera", camera::behaviours()),
        Behaviours::new("replies", replies::behaviours()),
        Behaviours::new("bindings", bindings::behaviours()),
        Behaviours::new("facts", facts::behaviours()),
        Behaviours::new("equipsets", equipsets::behaviours()),
        Behaviours::new("encounters", encounters::behaviours()),
        Behaviours::new("pets", pets::behaviours()),
        Behaviours::new("conditionals", conditionals::behaviours()),
        Behaviours::new("lfg", lfg::behaviours()),
        Behaviours::new("timeline", timeline::behaviours()),
        Behaviours::new("chatwindows", chatwindows::behaviours()),
        Behaviours::new("guild", guild::behaviours()),
        Behaviours::new("collected", collected::behaviours()),
        Behaviours::new(
            "ui",
            vec![
                ("import", ui::build::import as NativeFn),
                (
                    "C_XMLUtil.GetTemplateInfo",
                    ui::build::template_info as NativeFn,
                ),
            ],
        ),
        Behaviours::new("framescript", framescript::functions()),
        Behaviours::new("standins", standins::behaviours()),
    ]
}

/// The widget methods' behaviours by module, in the order they take each
/// other's place: the fill-ins (`methods2`, `methods3`) first.
pub fn method_behaviours() -> Vec<Behaviours<widgets::MethodFn>> {
    vec![
        Behaviours::new("methods2", methods2::methods()),
        Behaviours::new("methods3", methods3::methods()),
        Behaviours::new("widgets", widgets::behaviours()),
        Behaviours::new("curves", curves::methods()),
        Behaviours::new("objects2", objects2::methods()),
        Behaviours::new("clock", clock::methods()),
        Behaviours::new("healcalc", healcalc::methods()),
        Behaviours::new("formatters", formatters::methods()),
        Behaviours::new("framescript", framescript::methods()),
        Behaviours::new("animations", animations::methods()),
        Behaviours::new("tooltip", tooltip::methods()),
        Behaviours::new("input", input::methods()),
        Behaviours::new("bars", bars::methods()),
        Behaviours::new("buttons", buttons::methods()),
        Behaviours::new("measures", measures::methods()),
        Behaviours::new("models", models::methods()),
        Behaviours::new("widgetops", widgetops::methods()),
    ]
}
