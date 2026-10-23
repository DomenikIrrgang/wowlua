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

/// A model file and what it is drawn with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Part {
    /// The `.m2` file.
    pub file: u32,
    /// Textures by kind (11 to 13 a creature's skins, 2 an item's, 1 the
    /// character's skin, 6 hair, …): their files.
    pub textures: HashMap<u32, u32>,
    /// The geosets shown (submesh IDs), where the display names them;
    /// else the model's defaults.
    pub geosets: Option<Vec<u16>>,
    /// Geosets added to the defaults (a creature display's extra ones).
    pub extra_geosets: Vec<u16>,
    /// The attachment point of the first part this one sits at (an
    /// item's), else none.
    pub attachment: Option<u32>,
}

/// A display: its parts (the first the body), its scale and alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub parts: Vec<Part>,
    pub scale: f32,
    pub alpha: f32,
    /// A humanoid creature display's character (race, sex), for the
    /// character resolution; none for plain creatures.
    pub character: Option<(u32, u32)>,
}

/// The tables, opened as first needed.
#[derive(Default)]
pub struct Library {
    tables: HashMap<&'static str, Option<Table>>,
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
        let mut part = Part {
            file,
            ..Part::default()
        };
        // Skins: kinds 11 to 13 the display's first three; kinds 2 to 4
        // (object skins) the same, as model viewers take them.
        for (k, &f) in textures.iter().enumerate().take(3) {
            if f != 0 {
                part.textures.insert(11 + k as u32, f);
                part.textures.insert(2 + k as u32, f);
            }
        }
        if has_geosets && let Some(t) = self.table(casc, "CreatureDisplayInfoGeosetData") {
            part.extra_geosets = t
                .rows_where("CreatureDisplayInfoID", id)
                .iter()
                .map(|g| ((g.int("GeosetIndex") + 1) * 100 + g.int("GeosetValue")) as u16)
                .collect();
        }
        let character = if extended != 0 {
            self.table(casc, "CreatureDisplayInfoExtra")
                .and_then(|t| t.row(extended))
                .map(|r| (r.int("DisplayRaceID"), r.int("DisplaySexID")))
        } else {
            None
        };
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let model_scale = if model_scale > 0.0 { model_scale } else { 1.0 };
        Some(Display {
            parts: vec![part],
            scale: scale * model_scale,
            alpha: if alpha == 0 {
                1.0
            } else {
                alpha as f32 / 255.0
            },
            character,
        })
    }

    /// A creature's (`Creature` ID) first display.
    pub fn creature(&mut self, casc: &Casc, id: u32) -> Option<Display> {
        let display = {
            let t = self.table(casc, "Creature")?;
            let r = t.row(id)?;
            r.ints("DisplayID").into_iter().find(|&d| d != 0)?
        };
        self.creature_display(casc, display)
    }

    /// A model file by itself.
    pub fn file(file: u32) -> Display {
        Display {
            parts: vec![Part {
                file,
                ..Part::default()
            }],
            scale: 1.0,
            alpha: 1.0,
            character: None,
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
