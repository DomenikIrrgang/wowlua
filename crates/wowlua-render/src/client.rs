//! Assets from this machine (spec debugger §18.10): the addons' own
//! folders, then the WoW installation's storage for the flavor's client.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use wowlua_casc::display::{Display, Library};
use wowlua_casc::{Casc, FsStorage, Rgba};
use wowlua_engine::scene::{Image, ModelSource};

use crate::Assets;

/// The storages opened, by installation and product.
type Opened = Mutex<HashMap<(PathBuf, String), Arc<Casc>>>;

/// Each product's storage, opened once per process.
fn opened() -> &'static Opened {
    static OPEN: OnceLock<Opened> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

/// The storage of the first of `folders` (a flavor's client folders) the
/// installation has, opened once per process.
pub fn open_client(install: &Path, folders: &[String]) -> Result<Arc<Casc>, String> {
    let storage = FsStorage {
        root: install.to_path_buf(),
    };
    let product = folders
        .iter()
        .find_map(|f| Casc::product_of_folder(&storage, f))
        .ok_or_else(|| {
            format!(
                "the installation at {} has none of the clients {}",
                install.display(),
                folders.join(", ")
            )
        })?;
    let key = (install.to_path_buf(), product.clone());
    if let Some(c) = opened().lock().ok().and_then(|m| m.get(&key).cloned()) {
        return Ok(c);
    }
    let casc = Arc::new(Casc::open(Box::new(storage), &product).map_err(|e| e.to_string())?);
    if let Ok(mut m) = opened().lock() {
        m.insert(key, casc.clone());
    }
    Ok(casc)
}

/// The build's atlases, by lower-case name.
pub type Atlases = HashMap<String, wowlua_casc::atlas::Atlas>;

/// The atlases of a storage, read once per storage.
fn atlases_of(casc: &Arc<Casc>) -> Arc<Atlases> {
    type Read = Mutex<HashMap<usize, Arc<Atlases>>>;
    static READ: OnceLock<Read> = OnceLock::new();
    let key = Arc::as_ptr(casc) as usize;
    let map = READ.get_or_init(Default::default);
    if let Some(a) = map.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return a;
    }
    let a = Arc::new(wowlua_casc::atlas::read(casc));
    if let Ok(mut m) = map.lock() {
        m.insert(key, a.clone());
    }
    a
}

/// The tables of a storage, opened once per storage.
fn library_of(casc: &Arc<Casc>) -> Arc<Mutex<Library>> {
    type Libraries = Mutex<HashMap<usize, Arc<Mutex<Library>>>>;
    static LIBS: OnceLock<Libraries> = OnceLock::new();
    let key = Arc::as_ptr(casc) as usize;
    let map = LIBS.get_or_init(Default::default);
    let Ok(mut m) = map.lock() else {
        return Arc::new(Mutex::new(Library::default()));
    };
    m.entry(key).or_default().clone()
}

/// The addons' folders and the client's storage.
#[derive(Clone, Default)]
pub struct ClientAssets {
    pub casc: Option<Arc<Casc>>,
    /// The build's atlases (none when its tables are encrypted).
    pub atlases: Option<Arc<Atlases>>,
    /// `interface\addons\<name>\` (lower case) → the addon's folder.
    pub addons: Vec<(String, PathBuf)>,
}

fn norm(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

impl ClientAssets {
    pub fn new(casc: Option<Arc<Casc>>, addons: &[(String, PathBuf)]) -> ClientAssets {
        let atlases = casc.as_ref().map(atlases_of).filter(|a| !a.is_empty());
        ClientAssets {
            casc,
            atlases,
            addons: addons
                .iter()
                .map(|(name, dir)| {
                    (
                        format!("interface\\addons\\{}\\", name.to_ascii_lowercase()),
                        dir.clone(),
                    )
                })
                .collect(),
        }
    }

    /// A file of an addon's folder (case as on disk, any case asked).
    fn addon_file(&self, path: &str, exts: &[&str]) -> Option<Vec<u8>> {
        let n = norm(path);
        let (prefix, dir) = self
            .addons
            .iter()
            .find(|(p, _)| n.starts_with(p.as_str()))?;
        let rel = &path[prefix.len()..];
        let mut cur = dir.clone();
        let parts: Vec<&str> = rel.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let entries = std::fs::read_dir(&cur).ok()?;
            let mut found = None;
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let lower = name.to_ascii_lowercase();
                let want = part.to_ascii_lowercase();
                let hit = if last {
                    lower == want || exts.iter().any(|x| lower == format!("{want}{x}"))
                } else {
                    lower == want
                };
                if hit {
                    found = Some(cur.join(name));
                    break;
                }
            }
            cur = found?;
        }
        std::fs::read(cur).ok()
    }
}

impl Assets for ClientAssets {
    fn image(&mut self, image: &Image) -> Option<Rgba> {
        let bytes = match image {
            Image::Color => return None,
            Image::File(path) => self
                .addon_file(path, &[".blp", ".tga", ".png"])
                .or_else(|| self.casc.as_ref()?.file_by_path(path)),
            Image::FileId(id) => self.casc.as_ref()?.file_by_id(*id),
        }?;
        if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            return crate::decode_png(&bytes).ok();
        }
        wowlua_casc::decode_image(&bytes)
    }

    fn atlas(&mut self, name: &str) -> Option<(Image, [f32; 4])> {
        let a = self.atlases.as_ref()?.get(&name.to_ascii_lowercase())?;
        Some((Image::FileId(a.file), a.coords))
    }

    fn file(&mut self, id: u32) -> Option<Vec<u8>> {
        self.casc.as_ref()?.file_by_id(id)
    }

    fn model(&mut self, source: &ModelSource) -> Option<Display> {
        let casc = self.casc.as_ref()?;
        let lib = library_of(casc);
        let mut lib = lib.lock().ok()?;
        match source {
            ModelSource::File(f) => Some(Library::file(*f)),
            ModelSource::Path(p) => {
                // The old `.mdx` and `.mdl` names are the `.m2` files.
                let lower = p.to_ascii_lowercase();
                let path = match lower.rsplit_once('.') {
                    Some((stem, "mdx" | "mdl")) => format!("{stem}.m2"),
                    _ => lower,
                };
                Some(Library::file(casc.id_of(&path)?))
            }
            ModelSource::Display(d) => lib.creature_display(casc, *d),
            ModelSource::Creature(c) => lib.creature(casc, *c),
            ModelSource::Character(_) | ModelSource::Item(_) | ModelSource::Appearance(_) => None,
        }
    }

    fn font(&mut self, path: &str) -> Option<Vec<u8>> {
        self.addon_file(path, &[])
            .or_else(|| self.casc.as_ref()?.file_by_path(path))
    }
}
