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

/// Every product's creature displays resolve to models that read: the
/// tables (encrypted ones with the client's keys), the model and its
/// skin, posed in its first sequence.
#[test]
fn reads_every_products_creature_models() {
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, version) in Casc::products(&storage) {
        let Ok(casc) = Casc::open(Box::new(FsStorage { root: root.clone() }), &product) else {
            continue;
        };
        let mut lib = wowlua_casc::display::Library::default();
        let rows = lib
            .table(&casc, "CreatureDisplayInfo")
            .map_or(0, wowlua_casc::tables::Table::len);
        assert!(rows > 1000, "{product}: {rows} displays");
        // The first displays that name a model file.
        let mut read = 0;
        for id in 1..3000 {
            let Some(d) = lib.creature_display(&casc, id) else {
                continue;
            };
            let part = &d.parts[0];
            let Some(bytes) = casc.file_by_id(part.file) else {
                continue;
            };
            let m = wowlua_casc::m2::Model::parse(&bytes)
                .unwrap_or_else(|| panic!("{product}: display {id}'s model {}", part.file));
            let skin = casc
                .file_by_id(m.skin_files[0])
                .and_then(|b| wowlua_casc::m2::Skin::parse(&b))
                .unwrap_or_else(|| panic!("{product}: model {}'s skin", part.file));
            assert!(!skin.batches.is_empty() && !m.vertices.is_empty());
            let pose = m.pose(m.sequence(0), 0);
            assert_eq!(pose.len(), m.bones.len());
            read += 1;
            if read == 20 {
                break;
            }
        }
        eprintln!(
            "{product} {version}: {rows} displays, {} keys, {read} models read",
            casc.key_count()
        );
        assert_eq!(read, 20, "{product}");
    }
}

/// Every product's characters resolve: a body with its customizations'
/// skin composited and textures, what it wears as parts; and a humanoid
/// creature with its baked skin.
#[test]
fn resolves_every_products_characters() {
    use wowlua_casc::character::{Look, Worn};
    use wowlua_casc::display::Texture;
    let Some(root) = install() else {
        eprintln!("no WoW installation: skipped");
        return;
    };
    let storage = FsStorage { root: root.clone() };
    for (product, _) in Casc::products(&storage) {
        let Ok(casc) = Casc::open(Box::new(FsStorage { root: root.clone() }), &product) else {
            continue;
        };
        let mut lib = wowlua_casc::display::Library::default();
        // A human warrior in a recruit's shirt with a sword (the starting
        // items of every classic client).
        let look = Look {
            race: 1,
            sex: 0,
            class: 1,
            choices: Vec::new(),
            worn: vec![(4, Worn::Item(38)), (16, Worn::Item(25))],
            bake: None,
        };
        let d = lib
            .character(&casc, &look)
            .unwrap_or_else(|| panic!("{product}: the human"));
        let body = &d.parts[0];
        let Some(Texture::Composite(skin)) = body.textures.get(&1) else {
            panic!("{product}: no composited skin: {:?}", body.textures.keys());
        };
        assert!(skin.layers.len() > 2, "{product}: {skin:?}");
        assert!(
            d.parts.iter().any(|p| p.attachment == Some(1)),
            "{product}: the sword"
        );
        assert!(casc.file_by_id(body.file).is_some());
        // A humanoid creature: its baked skin.
        let humanoid = lib
            .table(&casc, "CreatureDisplayInfo")
            .and_then(|t| {
                t.rows()
                    .find(|r| r.int("ExtendedDisplayInfoID") != 0)
                    .map(|r| r.id())
            })
            .expect("a humanoid");
        let d = lib
            .creature_display(&casc, humanoid)
            .unwrap_or_else(|| panic!("{product}: display {humanoid}"));
        assert!(
            matches!(d.parts[0].textures.get(&1), Some(Texture::File(_))),
            "{product}: display {humanoid}'s bake"
        );
    }
}
