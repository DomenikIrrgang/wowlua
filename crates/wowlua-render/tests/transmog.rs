//! Transmog sources from the client's tables (decision T462), against the
//! sources the client answered (`wowlua-engine/data/camelot-answers.txt`,
//! `C_TransmogCollection.GetAppearanceSources`): item, appearance, slot,
//! category and source type, on the WoW installation of this machine;
//! skipped where there is none.

/// What the build changed since the recording (1.60.1.70425): on 70707
/// two sources show another appearance, on 70755 135 more (their items'
/// new appearances, nothing else of them changed) (until the sources are
/// recorded again: tests/ingame/PROBES.md, "The Camelot update").
const CHANGED: &[&str] = &[
    "source 299176:",
    "source 304856:",
    "source 308226:",
    "source 308227:",
    "source 308229:",
    "source 308250:",
    "source 308251:",
    "source 308252:",
    "source 308253:",
    "source 308262:",
    "source 308263:",
    "source 308264:",
    "source 308265:",
    "source 308266:",
    "source 308267:",
    "source 308268:",
    "source 308269:",
    "source 308270:",
    "source 308271:",
    "source 308272:",
    "source 308273:",
    "source 308274:",
    "source 308275:",
    "source 308276:",
    "source 308277:",
    "source 308294:",
    "source 308295:",
    "source 308296:",
    "source 308297:",
    "source 308298:",
    "source 308299:",
    "source 308300:",
    "source 308301:",
    "source 308360:",
    "source 308361:",
    "source 308362:",
    "source 308363:",
    "source 308489:",
    "source 308490:",
    "source 308491:",
    "source 308515:",
    "source 308516:",
    "source 308517:",
    "source 308518:",
    "source 308519:",
    "source 308520:",
    "source 308521:",
    "source 308522:",
    "source 308523:",
    "source 308524:",
    "source 308525:",
    "source 308526:",
    "source 308527:",
    "source 308528:",
    "source 308529:",
    "source 308530:",
    "source 308570:",
    "source 308693:",
    "source 308694:",
    "source 308695:",
    "source 308696:",
    "source 308697:",
    "source 308698:",
    "source 308699:",
    "source 308700:",
    "source 308701:",
    "source 308702:",
    "source 308703:",
    "source 308704:",
    "source 308705:",
    "source 308706:",
    "source 308707:",
    "source 308709:",
    "source 308710:",
    "source 308711:",
    "source 308712:",
    "source 308713:",
    "source 308714:",
    "source 308715:",
    "source 308716:",
    "source 308717:",
    "source 308718:",
    "source 308719:",
    "source 308720:",
    "source 308721:",
    "source 308722:",
    "source 308723:",
    "source 308724:",
    "source 308725:",
    "source 308726:",
    "source 308727:",
    "source 308728:",
    "source 308729:",
    "source 308730:",
    "source 308731:",
    "source 308732:",
    "source 308734:",
    "source 308735:",
    "source 308736:",
    "source 308737:",
    "source 308738:",
    "source 308739:",
    "source 308740:",
    "source 308741:",
    "source 308742:",
    "source 308743:",
    "source 308744:",
    "source 308745:",
    "source 308746:",
    "source 308747:",
    "source 308748:",
    "source 308749:",
    "source 308750:",
    "source 308751:",
    "source 308752:",
    "source 308753:",
    "source 308754:",
    "source 308755:",
    "source 308756:",
    "source 308757:",
    "source 308758:",
    "source 308761:",
    "source 308762:",
    "source 308763:",
    "source 308764:",
    "source 308765:",
    "source 308766:",
    "source 308767:",
    "source 308768:",
    "source 308769:",
    "source 308770:",
    "source 308771:",
    "source 308772:",
    "source 308773:",
    "source 308774:",
    "source 308775:",
    "source 308776:",
];

#[test]
fn the_client_sources_answer_as_recorded() {
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
    let (checked, wrong) = wowlua_engine::collections::compare_sources_with_recording(&data);
    eprintln!("{checked} sources, {} differ", wrong.len());
    let wrong: Vec<&String> = wrong
        .iter()
        .filter(|w| !CHANGED.iter().any(|c| w.starts_with(c)))
        .collect();
    assert!(checked > 5000, "{checked}");
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(20)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
