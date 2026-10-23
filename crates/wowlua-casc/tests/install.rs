//! The reader on the WoW installation of this machine (`WOWLUA_WOW_PATH`,
//! else the default place); skipped where there is none.

use wowlua_casc::{Casc, FsStorage, Storage};

fn install() -> Option<std::path::PathBuf> {
    let p = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    p.join(".build.info").exists().then_some(p)
}

#[test]
fn reads_every_products_textures_and_fonts() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    let camelot = Casc::product_of_folder(&storage, "_classic_beta_");
    eprintln!("_classic_beta_ is {camelot:?}");
    let products = Casc::products(&storage);
    assert!(!products.is_empty());
    for (product, version) in products {
        let started = std::time::Instant::now();
        let casc = match Casc::open(Box::new(FsStorage { root: root.clone() }), &product) {
            Ok(c) => c,
            Err(e) => panic!("{product} {version}: {e}"),
        };
        let opened = started.elapsed();
        let white = casc
            .file_by_path("Interface\\Buttons\\WHITE8X8")
            .unwrap_or_else(|| panic!("{product}: WHITE8X8"));
        let img = wowlua_casc::decode_image(&white).expect("a BLP");
        assert_eq!((img.width, img.height), (8, 8));
        // Near white: a DXT1 file stores 565 colors (era's green is 251).
        assert!(img.pixels.iter().all(|&c| c >= 248), "{product}: white");
        let font = casc
            .file_by_path("Fonts\\FRIZQT__.TTF")
            .unwrap_or_else(|| panic!("{product}: FRIZQT__.TTF"));
        assert_eq!(&font[..4], &[0, 1, 0, 0], "{product}: a TrueType font");
        let button = casc
            .file_by_path("Interface/Buttons/UI-Panel-Button-Up.blp")
            .and_then(|b| wowlua_casc::decode_image(&b));
        eprintln!(
            "{product} {version}: {} files, opened in {opened:?}, button {:?}",
            casc.file_count(),
            button.map(|b| (b.width, b.height))
        );
    }
    let _ = storage.list("Data");
}

#[test]
fn reads_the_builds_own_atlases() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, version) in Casc::products(&storage) {
        let casc = Casc::open(Box::new(FsStorage { root: root.clone() }), &product).expect("opens");
        let started = std::time::Instant::now();
        let atlases = wowlua_casc::atlas::read(&casc);
        eprintln!(
            "{product} {version}: {} atlases in {:?}",
            atlases.len(),
            started.elapsed()
        );
        // Some builds encrypt the tables (classic's): none then.
        if product == "wow_classic_beta" {
            assert!(atlases.len() > 1000, "{product}: {}", atlases.len());
            // An element's camelot variant at 1x: the level circle.
            let c = atlases["ui-hud-unitframe-smallcircle"];
            assert_eq!(c.file, 8036204, "{c:?}");
            assert_eq!((c.width, c.height), (39, 39));
            assert_eq!(
                c.coords,
                [979.0 / 1024.0, 1018.0 / 1024.0, 87.0 / 512.0, 126.0 / 512.0]
            );
            // A member's own name.
            assert_eq!(atlases["ui-hud-unitframe-smallcircle-c60-2x"].file, 8036209);
        }
    }
}
