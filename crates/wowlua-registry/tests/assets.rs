//! The registry's copies of wowlua.dev's stylesheets are the website's own.
//! The website is another repository: the test compares them when it is
//! checked out at `WOWLUA_WEBSITE` (or next to this repository, as
//! `../wowlua-website`), and says so when it isn't.

use std::path::PathBuf;

const COPIES: &[(&str, &str)] = &[
    ("tokens.css", "styles/tokens.css"),
    ("design-system.css", "styles/design-system.css"),
    ("chrome.css", "styles/mobile/chrome.css"),
];

fn website() -> Option<PathBuf> {
    let dir = match std::env::var_os("WOWLUA_WEBSITE") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../wowlua-website"),
    };
    dir.join("styles").is_dir().then_some(dir)
}

#[test]
fn stylesheets_match_the_website() {
    let Some(site) = website() else {
        eprintln!("the website is not checked out; set WOWLUA_WEBSITE to compare the stylesheets");
        return;
    };
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    for (copy, original) in COPIES {
        let ours = std::fs::read(assets.join(copy)).expect("the registry's copy");
        let theirs = std::fs::read(site.join(original)).expect("the website's file");
        assert!(
            ours == theirs,
            "assets/{copy} differs from the website's {original}: copy it again ({})",
            site.join(original).display()
        );
    }
}

#[test]
fn every_asset_is_served() {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    for (name, _, bytes) in wowlua_registry::assets::FILES {
        assert_eq!(
            std::fs::read(assets.join(name)).expect("the file").len(),
            bytes.len(),
            "{name}"
        );
    }
    // The stylesheet names only fonts the server has.
    let css = std::fs::read_to_string(assets.join("registry.css")).unwrap();
    for url in css.split("url(\"").skip(1) {
        let file = url.split('"').next().unwrap();
        assert!(
            wowlua_registry::assets::FILES
                .iter()
                .any(|(n, _, _)| *n == file),
            "registry.css names {file}"
        );
    }
}
