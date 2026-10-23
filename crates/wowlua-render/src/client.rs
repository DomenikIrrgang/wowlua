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

/// The item of an item link (`|Hitem:6125:…|h`, `item:6125`).
fn item_of_link(link: &str) -> Option<u32> {
    let at = link.find("item:")?;
    let rest = &link[at + 5..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
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
            Image::Color | Image::Portrait { .. } => return None,
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

    fn atlas(&mut self, name: &str) -> Option<crate::AtlasRegion> {
        let a = self.atlases.as_ref()?.get(&name.to_ascii_lowercase())?;
        Some(crate::AtlasRegion {
            image: Image::FileId(a.file),
            coords: a.coords,
            size: (a.width, a.height),
            slice: a.slice,
            tiles: (a.tiles_horizontally, a.tiles_vertically),
        })
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
            ModelSource::Character(look) => {
                use wowlua_casc::character::{Look, Worn};
                let mut worn: Vec<(u32, Worn)> = look
                    .items
                    .iter()
                    .map(|&(slot, item)| (slot, Worn::Item(item)))
                    .collect();
                // Items tried on: a link's item, or an item appearance ID,
                // over what is worn in their slot.
                for t in &look.tried {
                    let w = match item_of_link(t) {
                        Some(i) => Worn::Item(i),
                        None => match t.trim().parse::<f64>() {
                            Ok(n) if n > 0.0 => Worn::Appearance(n as u32),
                            _ => continue,
                        },
                    };
                    if let Some(slot) = lib.item_slot(casc, w) {
                        worn.retain(|(s, _)| *s != slot);
                        worn.push((slot, w));
                    }
                }
                lib.character(
                    casc,
                    &Look {
                        race: look.race,
                        sex: look.sex,
                        class: look.class,
                        choices: Vec::new(),
                        worn,
                        bake: None,
                    },
                )
            }
            ModelSource::Item(i) => lib.item_model(casc, wowlua_casc::character::Worn::Item(*i)),
            ModelSource::Appearance(a) => {
                // An item appearance (`ItemAppearance`): its display.
                let d = lib
                    .table(casc, "ItemAppearance")?
                    .row(*a)?
                    .int("ItemDisplayInfoID");
                lib.item_model(casc, wowlua_casc::character::Worn::Display(d))
            }
        }
    }

    fn font(&mut self, path: &str) -> Option<Vec<u8>> {
        self.addon_file(path, &[])
            .or_else(|| self.casc.as_ref()?.file_by_path(path))
    }
}

/// The client's tables for the engine (decision T321): model scenes,
/// their cameras, actors and actors' displays.
/// The storage is opened at the first question, so sessions that ask
/// none do not open it.
pub struct CascClientData {
    open: Box<dyn Fn() -> Option<Arc<Casc>> + Send + Sync>,
    casc: OnceLock<Option<Arc<Casc>>>,
}

impl CascClientData {
    pub fn new(open: Box<dyn Fn() -> Option<Arc<Casc>> + Send + Sync>) -> CascClientData {
        CascClientData {
            open,
            casc: OnceLock::new(),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Library, &Casc) -> Option<T>) -> Option<T> {
        let casc = self.casc.get_or_init(|| (self.open)()).as_ref()?;
        let lib = library_of(casc);
        let mut lib = lib.lock().ok()?;
        f(&mut lib, casc)
    }
}

fn vec3(v: Vec<f32>) -> [f32; 3] {
    [
        v.first().copied().unwrap_or(0.0),
        v.get(1).copied().unwrap_or(0.0),
        v.get(2).copied().unwrap_or(0.0),
    ]
}

impl wowlua_engine::clientdata::ClientData for CascClientData {
    fn minimap_tile(&self, map: u32, x: u32, y: u32) -> Option<u32> {
        self.with(|lib, casc| lib.minimap_tile(casc, map, x, y))
    }

    fn model_scene(&self, id: u32) -> Option<wowlua_engine::clientdata::ModelScene> {
        self.with(|lib, casc| {
            let (kind, flags) = {
                let r = lib.table(casc, "UiModelScene")?.row(id)?;
                (r.int("UiSystemType"), r.int("Flags"))
            };
            let ids = |lib: &mut Library, t: &'static str| -> Vec<u32> {
                let mut v: Vec<u32> = lib
                    .table(casc, t)
                    .map(|t| {
                        t.rows_where("UiModelSceneID", id)
                            .iter()
                            .map(|r| r.id())
                            .collect()
                    })
                    .unwrap_or_default();
                v.sort_unstable();
                v
            };
            let cameras = ids(lib, "UiModelSceneCamera");
            let actors = ids(lib, "UiModelSceneActor");
            Some(wowlua_engine::clientdata::ModelScene {
                kind,
                cameras,
                actors,
                flags,
            })
        })
    }

    fn scene_camera(&self, id: u32) -> Option<wowlua_engine::clientdata::SceneCamera> {
        self.with(|lib, casc| {
            let r = lib.table(casc, "UiModelSceneCamera")?.row(id)?;
            Some(wowlua_engine::clientdata::SceneCamera {
                id,
                tag: r.string("ScriptTag").unwrap_or_default(),
                kind: r.int("CameraType"),
                target: vec3(r.floats("Target")),
                zoomed_target_offset: vec3(r.floats("ZoomedTargetOffset")),
                yaw: r.float("Yaw"),
                pitch: r.float("Pitch"),
                roll: r.float("Roll"),
                zoomed_yaw_offset: r.float("ZoomedYawOffset"),
                zoomed_pitch_offset: r.float("ZoomedPitchOffset"),
                zoomed_roll_offset: r.float("ZoomedRollOffset"),
                zoom_distance: r.float("ZoomDistance"),
                min_zoom_distance: r.float("MinZoomDistance"),
                max_zoom_distance: r.float("MaxZoomDistance"),
                flags: r.int("Flags"),
            })
        })
    }

    fn scene_actor(&self, id: u32) -> Option<wowlua_engine::clientdata::SceneActor> {
        self.with(|lib, casc| {
            let r = lib.table(casc, "UiModelSceneActor")?.row(id)?;
            Some(wowlua_engine::clientdata::SceneActor {
                id,
                tag: r.string("ScriptTag").unwrap_or_default(),
                position: vec3(r.floats("Position")),
                yaw: r.float("OrientationYaw"),
                pitch: r.float("OrientationPitch"),
                roll: r.float("OrientationRoll"),
                normalized_scale: r.float("NormalizedScale"),
                flags: r.int("Flags"),
                display: r.int("UiModelSceneActorDisplayID"),
            })
        })
    }

    fn ui_camera(&self, id: u32) -> Option<wowlua_engine::clientdata::UiCamera> {
        self.with(|lib, casc| {
            let r = lib.table(casc, "UiCamera")?.row(id)?;
            Some(wowlua_engine::clientdata::UiCamera {
                position: vec3(r.floats("Pos")),
                angles: vec3(r.floats("LookAt")),
                animation: r.signed("AnimID"),
                frame: r.signed("AnimFrame"),
                variation: r.signed("AnimVariation"),
                flags: r.int("Flags"),
            })
        })
    }

    fn item_inventory_type(&self, id: u32) -> Option<u32> {
        self.with(|lib, casc| Some(lib.table(casc, "Item")?.row(id)?.int("InventoryType")))
    }

    fn actor_display(&self, id: u32) -> Option<wowlua_engine::clientdata::ActorDisplay> {
        self.with(|lib, casc| {
            let r = lib.table(casc, "UiModelSceneActorDisplay")?.row(id)?;
            Some(wowlua_engine::clientdata::ActorDisplay {
                animation: r.int("AnimationID"),
                variation: r.int("SequenceVariation"),
                anim_kit: r.int("AnimKitID"),
                spell_visual_kit: r.int("SpellVisualKitID"),
                alpha: r.float("Alpha"),
                scale: r.float("Scale"),
                anim_speed: r.float("AnimSpeed"),
            })
        })
    }
}
