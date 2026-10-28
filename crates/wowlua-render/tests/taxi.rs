//! Flight paths from the client's tables (decision T463), against the
//! world maps' recorded answers (`wowlua-engine/data/camelot-answers.txt`,
//! `C_TaxiMap.GetTaxiNodesForMap` for every map of Forever's world), on the
//! WoW installation of this machine; skipped where there is none.

#[test]
fn the_client_flight_paths_answer_as_recorded() {
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
    let (checked, wrong) = wowlua_engine::taxi::compare_with_recording(&data);
    eprintln!("{checked} nodes, {} differ", wrong.len());
    assert!(checked > 1900, "{checked}");
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
