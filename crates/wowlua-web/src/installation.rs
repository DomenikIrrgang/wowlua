//! The user's WoW installation in the page (spec M14 S5, decision T398):
//! the folder the user picks is read in the browser, never uploaded. The
//! CASC reader reads it through a [`Storage`] the page backs
//! ([`PageStorage`]: the worker reads the picked files synchronously);
//! the game's textures, fonts and models are the renderer's, its tables
//! (spells, items, …) the engine's, as with `wowlua test --game-data`.

use std::path::PathBuf;
use std::sync::Arc;
use wowlua_casc::{Casc, Storage};

/// A flavor's client opened from the installation.
pub struct Installation {
    pub flavor: String,
    /// `wow_classic_beta`, …
    pub product: String,
    pub casc: Arc<Casc>,
}

/// Opens `flavor`'s client from `storage` (the installation's folder).
pub fn open(storage: Box<dyn Storage + Send + Sync>, flavor: &str) -> Result<Installation, String> {
    let folders: Vec<String> = wowlua_flavors::table()
        .clients_of(flavor)
        .flat_map(|c| c.folders.iter().cloned())
        .collect();
    let product = wowlua_render::client::product_of(&*storage, &folders).ok_or_else(|| {
        format!(
            "the folder has no {flavor} client (none of {}): pick the World of Warcraft folder",
            folders.join(", ")
        )
    })?;
    let casc = Casc::open(storage, &product).map_err(|e| e.to_string())?;
    Ok(Installation {
        flavor: flavor.to_string(),
        product,
        casc: Arc::new(casc),
    })
}

/// The test runner's camera for a run with the installation: its tables
/// (decision T321). The page draws itself; tests' screenshots are not run
/// here.
pub struct GameData(pub Arc<Casc>);

impl wowlua_test::screenshots::Camera for GameData {
    fn shoot(
        &self,
        _vm: &mut wowlua_vm::Vm,
        _frame: Option<wowlua_engine::widgets::WidgetId>,
        _scale: f64,
    ) -> Result<Vec<u8>, String> {
        Err("the playground takes no test screenshots".into())
    }

    fn compare(
        &self,
        expected: &[u8],
        actual: &[u8],
        threshold: f64,
    ) -> Result<(f64, Vec<u8>), String> {
        let a = wowlua_render::decode_png(expected)?;
        let b = wowlua_render::decode_png(actual)?;
        let d = wowlua_render::compare::compare(&a, &b, threshold);
        Ok((d.ratio(), wowlua_render::encode_png(&d.image)?))
    }

    fn out_dir(&self) -> PathBuf {
        PathBuf::from(crate::ROOT).join("screenshots")
    }

    fn client_data(&self) -> Option<Arc<dyn wowlua_engine::clientdata::ClientData>> {
        let casc = self.0.clone();
        Some(Arc::new(wowlua_render::client::CascClientData::new(
            Box::new(move || Some(casc.clone())),
        )))
    }
}

/// The installation the page picked, read by the worker (`web/worker.js`):
/// `wowluaInstallRead(path)`, `wowluaInstallReadAt(path, offset, len)` (a
/// `Uint8Array`, or `undefined`) and `wowluaInstallList(dir)` (the names,
/// as JSON); paths relative to the folder, `/`-separated.
#[cfg(target_arch = "wasm32")]
pub struct PageStorage;

#[cfg(target_arch = "wasm32")]
mod page {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = wowluaInstallRead)]
        pub fn read(path: &str) -> Option<Vec<u8>>;
        #[wasm_bindgen(js_name = wowluaInstallReadAt)]
        pub fn read_at(path: &str, offset: f64, len: f64) -> Option<Vec<u8>>;
        #[wasm_bindgen(js_name = wowluaInstallList)]
        pub fn list(dir: &str) -> String;
    }
}

#[cfg(target_arch = "wasm32")]
impl Storage for PageStorage {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        page::read(path)
    }

    fn read_at(&self, path: &str, offset: u64, len: usize) -> Option<Vec<u8>> {
        page::read_at(path, offset as f64, len as f64)
    }

    fn list(&self, dir: &str) -> Vec<String> {
        serde_json::from_str(&page::list(dir)).unwrap_or_default()
    }
}
