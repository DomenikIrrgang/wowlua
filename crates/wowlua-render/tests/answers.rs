//! Answers from the client's tables (decision T464), against the answers
//! the client recorded (`wowlua-engine/data/camelot-answers.txt`), on the
//! WoW installation of this machine; skipped where there is none. Each
//! kind of call the tables answer has its test.

fn client() -> Option<wowlua_render::client::CascClientData> {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root };
    let product = wowlua_casc::Casc::product_of_folder(&storage, "_classic_beta_")?;
    let casc = std::sync::Arc::new(wowlua_casc::Casc::open(Box::new(storage), &product).ok()?);
    Some(wowlua_render::client::CascClientData::new(Box::new(
        move || Some(casc.clone()),
    )))
}

fn check(prefix: &str, at_least: usize) {
    let Some(data) = client() else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let (checked, wrong) = wowlua_engine::recorded::compare_with_client(&data, prefix);
    eprintln!("{prefix}: {checked} answers, {} differ", wrong.len());
    assert!(checked >= at_least, "{checked}");
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The world maps' static data.
#[test]
fn maps() {
    check("C_Map.", 700);
}
