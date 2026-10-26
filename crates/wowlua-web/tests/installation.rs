//! The user's installation in the playground (spec M14 S5): opened from a
//! storage (here the disk's, in the page the picked folder's), a run
//! answers from its tables, and its pictures and fonts are there to draw.
//! Skipped without an installation with a camelot client (`WOWLUA_WOW_PATH`
//! or the usual folder). One test: the files' host is the program's.

use serde_json::json;
use std::path::PathBuf;
use wowlua_engine::scene::Image;
use wowlua_render::Assets;
use wowlua_web::Playground;
use wowlua_web::run::Options;

fn installation() -> Option<PathBuf> {
    let dir = std::env::var_os("WOWLUA_WOW_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from(r"C:\Program Files (x86)\World of Warcraft")
            } else {
                PathBuf::from("/Applications/World of Warcraft")
            }
        });
    #[allow(clippy::disallowed_methods)] // (the disk's, before any host)
    let there = dir.join("Data").is_dir();
    there.then_some(dir)
}

#[test]
fn a_run_answers_and_draws_from_the_installation() {
    let Some(dir) = installation() else {
        eprintln!("skipped: no WoW installation");
        return;
    };
    let mut p = Playground::new();
    let storage = wowlua_casc::FsStorage { root: dir };
    let opened = match p.use_installation(Box::new(storage), "camelot") {
        Ok(info) => info,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    assert!(
        opened["product"].as_str().unwrap().starts_with("wow"),
        "{opened}"
    );

    p.write_file(
        "src/main.wowlua",
        "local info = C_Spell.GetSpellInfo(133)\nprint(info and info.name)\n",
    );
    let started = p.run(Options {
        flavor: "camelot".into(),
        blizzard_ui: false,
        profile: None,
        debug: None,
        installation: None,
    });
    assert_eq!(started["ok"], true, "{started}");
    let game = p.game().unwrap();
    let printed: Vec<String> = game
        .take_output()
        .iter()
        .map(|o| o["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(printed, ["Fireball"], "{}", json!(printed));

    let mut assets = game.assets();
    let white = assets.image(&Image::File(r"Interface\Buttons\WHITE8X8".into()));
    assert!(
        white.is_some_and(|i| i.width == 8),
        "a texture of the client"
    );
    let font = assets.font(r"Fonts\FRIZQT__.TTF");
    assert!(
        font.is_some_and(|f| f.len() > 10_000),
        "a font of the client"
    );
}
