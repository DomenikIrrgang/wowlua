//! The professions' recipes from the client's tables (decision T470),
//! against the windows the client recorded (`wowlua-engine/data/
//! camelot-tradeskill.txt`: blacksmithing and mining), the character's part
//! of each answer kept, on the WoW installation of this machine; skipped
//! where there is none.

/// What the build changed since the recording (1.60.1.70425): on 70888
/// Coarse Grinding Stone (recipe 3326) goes grey at skill 90, where the
/// recording has 75 (`maxTrivialLevel`; nothing else of it changed) (until
/// the windows are recorded again: tests/ingame/PROBES.md, "The Camelot
/// update").
const NEWER: &[&str] = &["BLACKSMITHING C_TradeSkillUI.GetRecipeInfo(3326)\n"];

#[test]
fn the_client_recipes_answer_as_recorded() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root };
    let Some(product) = wowlua_casc::Casc::product_of_folder(&storage, "_forever_") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let casc = std::sync::Arc::new(wowlua_casc::Casc::open(Box::new(storage), &product).unwrap());
    let data = wowlua_render::client::CascClientData::new(Box::new(move || Some(casc.clone())));
    let (checked, wrong) = wowlua_engine::tradeskill::compare_with_client(&data);
    let wrong: Vec<String> = wrong
        .into_iter()
        .filter(|w| !NEWER.iter().any(|n| w.starts_with(n)))
        .collect();
    eprintln!("{checked} answers, {} differ", wrong.len());
    assert!(checked >= 1600, "{checked}");
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
