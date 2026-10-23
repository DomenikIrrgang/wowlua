//! What a model frame shows, as the build's tables give it (spec debugger
//! §18.10): a creature display's model file, its skins and geosets; an
//! item's models.
//!
//! Display IDs resolve through `CreatureDisplayInfo` and
//! `CreatureModelData`; creatures (`SetCreature`) through `Creature`'s
//! first display; a model file is itself.

use std::collections::HashMap;

use crate::Casc;
use crate::tables::Table;

/// A texture a part is drawn with: a file, or a character's skin
/// composited from layers.
#[derive(Clone, Debug, PartialEq)]
pub enum Texture {
    File(u32),
    Composite(Composite),
}

/// A texture composited from files, each into its rectangle in order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Composite {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
}

/// A file drawn into a composite: its rectangle (x, y, width, height) and
/// blend mode (`ChrModelTextureLayer.BlendMode`: 0 and 1 copy, 4
/// multiply, 6 overlay, 7 screen, others blend by alpha).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub file: u32,
    pub rect: [u32; 4],
    pub blend: u32,
}

/// A model file and what it is drawn with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Part {
    /// The `.m2` file.
    pub file: u32,
    /// Textures by kind (11 to 13 a creature's skins, 2 an item's or a
    /// cape's, 1 the character's skin, 6 hair, …).
    pub textures: HashMap<u32, Texture>,
    /// Geoset groups the display decides: a group's submeshes shown
    /// (`group * 100 + variant`), the group's others hidden.
    pub groups: HashMap<u16, Vec<u16>>,
    /// Submeshes shown or hidden over the defaults (customizations).
    pub shown: Vec<u16>,
    pub hidden: Vec<u16>,
    /// Groups hidden whole (hair under a helm).
    pub hidden_groups: Vec<u16>,
    /// The attachment point of the first part this one sits at (an
    /// item's), else none.
    pub attachment: Option<u32>,
    /// Posed with the first part's bones (a model made for its skeleton:
    /// armor and customizations).
    pub skinned: bool,
}

impl Part {
    pub fn with_file(file: u32) -> Part {
        Part {
            file,
            ..Part::default()
        }
    }

    /// Whether a submesh shows.
    pub fn shows(&self, id: u16) -> bool {
        // The body always shows.
        if id == 0 {
            return true;
        }
        if self.hidden_groups.contains(&(id / 100)) {
            return false;
        }
        if let Some(ids) = self.groups.get(&(id / 100))
            && id >= 100
        {
            return ids.contains(&id);
        }
        if self.shown.contains(&id) {
            return true;
        }
        if self.hidden.contains(&id) {
            return false;
        }
        default_geoset(id)
    }

    /// Shows `group * 100 + variant` alone of its group.
    pub fn set_group(&mut self, group: u16, variant: u16) {
        self.groups.insert(group, vec![group * 100 + variant]);
    }
}

/// A display: its parts (the first the body), its scale and alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub parts: Vec<Part>,
    pub scale: f32,
    pub alpha: f32,
}

/// The tables, opened as first needed.
#[derive(Default)]
pub struct Library {
    pub(crate) tables: HashMap<&'static str, Option<Table>>,
    /// The client's creature cache: creatures' displays.
    creature_cache: Option<HashMap<u32, Vec<u32>>>,
}

impl Library {
    pub fn table(&mut self, casc: &Casc, name: &'static str) -> Option<&Table> {
        self.tables
            .entry(name)
            .or_insert_with(|| Table::open(casc, name))
            .as_ref()
    }

    /// A creature display (`CreatureDisplayInfo` ID).
    pub fn creature_display(&mut self, casc: &Casc, id: u32) -> Option<Display> {
        let (model_id, scale, alpha, extended, textures) = {
            let t = self.table(casc, "CreatureDisplayInfo")?;
            let r = t.row(id)?;
            let mut textures = r.ints("TextureVariationFileDataID");
            textures.resize(4, 0);
            (
                r.int("ModelID"),
                r.float("CreatureModelScale"),
                r.int("CreatureModelAlpha"),
                r.int("ExtendedDisplayInfoID"),
                textures,
            )
        };
        let (file, model_scale, has_geosets) = {
            let t = self.table(casc, "CreatureModelData")?;
            let r = t.row(model_id)?;
            (
                r.int("FileDataID"),
                r.float("ModelScale"),
                r.int("CreatureGeosetDataID") != 0,
            )
        };
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let model_scale = if model_scale > 0.0 { model_scale } else { 1.0 };
        // A humanoid: the character its extra info describes.
        if extended != 0
            && let Some(mut d) = self.npc(casc, extended, file)
        {
            d.scale *= scale;
            return Some(d);
        }
        let mut part = Part::with_file(file);
        // Skins: kinds 11 to 13 the display's first three; kinds 2 to 4
        // (object skins) the same, as model viewers take them.
        for (k, &f) in textures.iter().enumerate().take(3) {
            if f != 0 {
                part.textures.insert(11 + k as u32, Texture::File(f));
                part.textures.insert(2 + k as u32, Texture::File(f));
            }
        }
        if has_geosets && let Some(t) = self.table(casc, "CreatureDisplayInfoGeosetData") {
            for g in t.rows_where("CreatureDisplayInfoID", id) {
                let group = (g.int("GeosetIndex") + 1) as u16;
                let geoset = group * 100 + g.int("GeosetValue") as u16;
                part.groups.entry(group).or_default().push(geoset);
            }
        }
        Some(Display {
            parts: vec![part],
            scale: scale * model_scale,
            alpha: if alpha == 0 {
                1.0
            } else {
                alpha as f32 / 255.0
            },
        })
    }

    /// A creature's (`Creature` ID) first display: the client's table's,
    /// else its creature cache's (what the server told it).
    pub fn creature(&mut self, casc: &Casc, id: u32) -> Option<Display> {
        let own = self
            .table(casc, "Creature")
            .and_then(|t| t.row(id))
            .and_then(|r| r.ints("DisplayID").into_iter().find(|&d| d != 0));
        let display = match own {
            Some(d) => d,
            None => {
                let cache = self.creature_cache.get_or_insert_with(|| {
                    let mut all = HashMap::new();
                    for f in casc.cache_files("creaturecache.wdb") {
                        all.extend(crate::wdb::creature_displays(&f));
                    }
                    all
                });
                *cache.get(&id)?.first()?
            }
        };
        self.creature_display(casc, display)
    }

    /// A model file by itself.
    pub fn file(file: u32) -> Display {
        Display {
            parts: vec![Part::with_file(file)],
            scale: 1.0,
            alpha: 1.0,
        }
    }
}

/// Whether a submesh shows by default (no display naming its geosets):
/// the body (0), every group's first variant (`x01`) and group 32; not
/// the eye glows (17) nor group 35.
pub fn default_geoset(id: u16) -> bool {
    if id < 100 {
        return id == 0;
    }
    let group = id / 100;
    if group == 17 || group == 35 {
        return false;
    }
    id % 100 == 1 || group == 32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A group the display decides shows its named submeshes alone; the
    /// customizations' over the defaults; the body always.
    #[test]
    fn parts_show_their_geosets() {
        let mut p = Part::with_file(1);
        assert!(p.shows(0) && p.shows(401) && !p.shows(402) && !p.shows(1701));
        p.set_group(4, 3);
        assert!(p.shows(403) && !p.shows(401));
        p.shown.push(5);
        p.hidden.push(0);
        p.hidden.push(501);
        assert!(p.shows(5) && p.shows(0) && !p.shows(501));
    }
}
