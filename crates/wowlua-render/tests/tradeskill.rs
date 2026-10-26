//! The professions' recipes from the client's tables (decision T470),
//! against the windows the client recorded (`wowlua-engine/data/
//! camelot-tradeskill.txt`: blacksmithing and mining), the character's part
//! of each answer kept, on the WoW installation of this machine; skipped
//! where there is none.

#[test]
fn the_client_recipes_answer_as_recorded() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root };
    let Some(product) = wowlua_casc::Casc::product_of_folder(&storage, "_classic_beta_") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let casc = std::sync::Arc::new(wowlua_casc::Casc::open(Box::new(storage), &product).unwrap());
    let data = wowlua_render::client::CascClientData::new(Box::new(move || Some(casc.clone())));
    let (checked, wrong) = wowlua_engine::tradeskill::compare_with_client(&data);
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
