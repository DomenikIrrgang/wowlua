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
        let file = wowlua_engine::bindings::find_addon_file(dir, &path[prefix.len()..], exts)?;
        std::fs::read(file).ok()
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
    /// The spell tables, opened on first use.
    spells: OnceLock<Option<wowlua_casc::spells::SpellTables>>,
    /// The client's fonts by their file's name (lower case), read on first
    /// use; `None` for one the storage lacks.
    fonts: std::sync::Mutex<std::collections::HashMap<String, Option<Arc<fontdue::Font>>>>,
    /// Glyphs measured: (font, pixel size, character) → advance, ink.
    glyphs: std::sync::Mutex<Measured>,
}

/// Glyphs measured, by font, pixel size and character: advance and ink.
type Measured = std::collections::HashMap<(String, u32, char), Option<(f32, f32)>>;

impl CascClientData {
    pub fn new(open: Box<dyn Fn() -> Option<Arc<Casc>> + Send + Sync>) -> CascClientData {
        CascClientData {
            open,
            casc: OnceLock::new(),
            spells: OnceLock::new(),
            fonts: Default::default(),
            glyphs: Default::default(),
        }
    }

    fn font(&self, file: &str) -> Option<Arc<fontdue::Font>> {
        let name = file.to_ascii_lowercase();
        let mut fonts = self.fonts.lock().ok()?;
        if let Some(f) = fonts.get(&name) {
            return f.clone();
        }
        let font = self
            .casc
            .get_or_init(|| (self.open)())
            .as_ref()
            .and_then(|casc| casc.file_by_path(&format!("Fonts\\{file}")))
            .and_then(|bytes| {
                fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
            })
            .map(Arc::new);
        fonts.insert(name, font.clone());
        font
    }

    fn spells(&self) -> Option<&wowlua_casc::spells::SpellTables> {
        self.spells
            .get_or_init(|| {
                let casc = self.casc.get_or_init(|| (self.open)()).as_ref()?;
                wowlua_casc::spells::SpellTables::open(casc)
            })
            .as_ref()
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

    fn creature_display(&self, creature: u32) -> Option<u32> {
        self.with(|lib, casc| lib.creature_display_id(casc, creature))
    }

    fn game_file_id(&self, path: &str) -> Option<Option<u32>> {
        let casc = self.casc.get_or_init(|| (self.open)()).as_ref()?;
        Some(casc.id_of_texture(path))
    }

    fn area_name(&self, area: u32) -> Option<String> {
        self.with(|lib, casc| {
            lib.table(casc, "AreaTable")?
                .row(area)?
                .string("AreaName_lang")
        })
    }

    fn appearance_source(&self, source: u32) -> Option<(u32, u32)> {
        self.with(|lib, casc| {
            let item = lib
                .table(casc, "ItemModifiedAppearance")?
                .row(source)?
                .int("ItemID");
            let slot = lib.item_slot(casc, wowlua_casc::character::Worn::Appearance(source))?;
            Some((item, slot))
        })
    }

    fn item_source(&self, item: u32) -> Option<(u32, u32)> {
        self.with(|lib, casc| {
            let source = lib
                .table(casc, "ItemModifiedAppearance")?
                .rows_where("ItemID", item)
                .iter()
                .min_by_key(|r| (r.int("ItemAppearanceModifierID"), r.int("OrderIndex")))?
                .id();
            let slot = lib.item_slot(casc, wowlua_casc::character::Worn::Item(item))?;
            Some((source, slot))
        })
    }

    fn transmog_camera(&self, race: u32, gender: u32, slot: u32, variation: u32) -> Option<u32> {
        self.with(|lib, casc| {
            let t = lib.table(casc, "UiCamFbackTransmogChrRace")?;
            let find = |variation: u32| {
                t.rows()
                    .find(|r| {
                        r.int("ChrRaceID") == race
                            && r.int("Gender") == gender
                            && r.int("InventoryType") == slot
                            && r.int("Variation") == variation
                    })
                    .map(|r| r.int("UiCameraID"))
            };
            // A variation the slot lacks: its first.
            find(variation).or_else(|| find(0))
        })
    }

    fn weapon_camera(&self, class: u32, subclass: u32, slot: u32) -> Option<u32> {
        self.with(|lib, casc| {
            let t = lib.table(casc, "UiCamFbackTransmogWeapon")?;
            let kind = |r: &wowlua_casc::tables::Row<'_>| {
                r.int("ItemClass") == class && r.int("ItemSubclass") == subclass
            };
            t.rows()
                .find(|r| kind(r) && r.int("InventoryType") == slot)
                .or_else(|| t.rows().find(|r| kind(r)))
                .map(|r| r.int("UiCameraID"))
        })
    }

    fn glyph(&self, font: &str, px: u32, c: char) -> Option<(f32, f32)> {
        let key = (font.to_ascii_lowercase(), px, c);
        if let Some(g) = self.glyphs.lock().ok()?.get(&key) {
            return *g;
        }
        let measured = self.font(font).and_then(|f| {
            // (a character the font lacks is not measured)
            if f.lookup_glyph_index(c) == 0 && c != ' ' {
                return None;
            }
            let m = f.metrics(c, px as f32);
            Some((m.advance_width, m.xmin as f32 + m.width as f32))
        });
        self.glyphs.lock().ok()?.insert(key, measured);
        measured
    }

    fn spell(&self, id: u32) -> Option<wowlua_engine::clientdata::ClientSpell> {
        let s = self.spells()?.info(id)?;
        Some(wowlua_engine::clientdata::ClientSpell {
            name: s.name,
            subtext: s.subtext,
            icon: s.icon,
            cast_ms: s.cast_ms,
            min_range: s.min_range,
            max_range: s.max_range,
            school_mask: s.school_mask,
            cooldown_ms: s.cooldown_ms.max(s.category_cooldown_ms),
            gcd_ms: s.gcd_ms,
            costs: s
                .costs
                .iter()
                .map(|c| wowlua_engine::clientdata::SpellCost {
                    power_type: c.power_type,
                    cost: c.shown(),
                    min_cost: c.shown_min(),
                    percent: c.percent,
                })
                .collect(),
        })
    }

    fn spell_description(
        &self,
        id: u32,
        caster: &wowlua_engine::clientdata::SpellCaster,
    ) -> Option<String> {
        let caster = wowlua_casc::spelltext::Caster {
            level: caster.level,
            spell_power: caster.spell_power,
            healing_power: caster.healing_power,
            attack_power: caster.attack_power,
            damage_done: 1.0,
        };
        self.spells()?.describe(id, &caster)
    }

    fn item(&self, id: u32) -> Option<wowlua_engine::clientdata::ClientItem> {
        self.with(|lib, casc| {
            let (icon, class, subclass, inventory_type) = {
                let r = lib.table(casc, "Item")?.row(id)?;
                (
                    r.int("IconFileDataID"),
                    r.int("ClassID"),
                    r.int("SubclassID"),
                    r.int("InventoryType"),
                )
            };
            let mut item = wowlua_engine::clientdata::ClientItem {
                icon,
                class,
                subclass,
                inventory_type,
                ..Default::default()
            };
            // Its name, quality and levels: `ItemSparse`'s, else the search
            // table's (a part of the items).
            for table in ["ItemSparse", "ItemSearchName"] {
                if let Some(r) = lib.table(casc, table).and_then(|t| t.row(id)) {
                    item.name = r.string("Display_lang").unwrap_or_default();
                    item.quality = r.int("OverallQualityID");
                    item.item_level = r.int("ItemLevel");
                    item.required_level = r.int("RequiredLevel");
                    item.expansion = r.int("ExpansionID");
                    if table == "ItemSparse" {
                        item.stack = r.int("Stackable");
                        item.sell_price = r.int("SellPrice");
                        item.bind = r.int("Bonding");
                        item.description = r.string("Description_lang").unwrap_or_default();
                    }
                    if !item.name.is_empty() {
                        break;
                    }
                }
            }
            Some(item)
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
