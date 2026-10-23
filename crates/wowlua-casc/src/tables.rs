//! The client's tables by name: each build's layout (which field is
//! which) known by its layout hash, from [`crate::schema`]; a layout not
//! listed is taken as a listed one of the same field count.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::Casc;
use crate::db2::Db2;

/// The tables' file IDs (the same in every build).
const FILES: &[(&str, u32)] = &[
    ("CharComponentTextureLayouts", 1360262),
    ("CharComponentTextureSections", 1360263),
    ("ChrCustomizationChoice", 3450554),
    ("ChrCustomizationElement", 3512765),
    ("ChrCustomizationGeoset", 3456171),
    ("ChrCustomizationMaterial", 3459652),
    ("ChrCustomizationOption", 3384247),
    ("ChrCustomizationReq", 3450453),
    ("ChrCustomizationSkinnedModel", 3460183),
    ("ChrModel", 3384313),
    ("ChrModelMaterial", 3566562),
    ("ChrModelTextureLayer", 3548976),
    ("ChrRaces", 1305311),
    ("ChrRaceXChrModel", 3490304),
    ("ComponentModelFileData", 1349053),
    ("ComponentTextureFileData", 1278239),
    ("Creature", 841631),
    ("CreatureDisplayInfo", 1108759),
    ("CreatureDisplayInfoExtra", 1264997),
    ("CreatureDisplayInfoGeosetData", 1720141),
    ("CreatureDisplayInfoOption", 3692043),
    ("CreatureModelData", 1365368),
    ("HelmetGeosetData", 2821752),
    ("Item", 841626),
    ("ItemAppearance", 982462),
    ("ItemDisplayInfo", 1266429),
    ("ItemDisplayInfoMaterialRes", 1280614),
    ("ItemDisplayInfoModelMatRes", 4050937),
    ("ItemModifiedAppearance", 982457),
    ("ModelFileData", 1337833),
    ("NPCModelItemSlotDisplayInfo", 1340661),
    ("TextureFileData", 982459),
    ("UiModelScene", 1548215),
    ("UiModelSceneActor", 1548214),
    ("UiModelSceneActorDisplay", 1548216),
    ("UiModelSceneCamera", 1548213),
    ("UiCamera", 1120287),
];

/// A column: a field of the records, their ID, or the parent ID of the
/// relationship map.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Col {
    Field(usize),
    Id,
    Relation,
}

/// Records by a column's value.
type Index = std::sync::Arc<HashMap<u32, Vec<usize>>>;

/// A table of the build, its fields by name.
pub struct Table {
    db: Db2,
    names: &'static [&'static str],
    /// Records by a column's value, built on first use.
    by: Mutex<HashMap<&'static str, Index>>,
}

/// A record of a table.
#[derive(Clone, Copy)]
pub struct Row<'a> {
    table: &'a Table,
    index: usize,
}

impl Table {
    /// The build's table `name`, if it can be read and its layout is
    /// known.
    pub fn open(casc: &Casc, name: &str) -> Option<Table> {
        let file = FILES.iter().find(|(n, _)| *n == name)?.1;
        let db = Db2::parse(casc.file_by_id(file)?)?;
        let layouts = crate::schema::LAYOUTS.iter().find(|(n, _)| *n == name)?.1;
        let fields = |names: &[&str]| names.iter().filter(|n| !n.starts_with('@')).count();
        let names = layouts
            .iter()
            .find(|(h, _)| *h == db.layout_hash)
            .or_else(|| layouts.iter().find(|(_, n)| fields(n) == db.field_count()))?
            .1;
        Some(Table {
            db,
            names,
            by: Mutex::new(HashMap::new()),
        })
    }

    fn col(&self, name: &str) -> Option<Col> {
        let mut i = 0;
        for n in self.names {
            if let Some(rel) = n.strip_prefix('@') {
                if rel == name {
                    return Some(Col::Relation);
                }
                continue;
            }
            if *n == name {
                return Some(Col::Field(i));
            }
            i += 1;
        }
        (name == "ID").then_some(Col::Id)
    }

    /// Whether the layout has a column.
    pub fn has(&self, name: &str) -> bool {
        self.col(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.db.len()
    }

    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    /// The record with ID `id`.
    pub fn row(&self, id: u32) -> Option<Row<'_>> {
        self.db.find(id).map(|index| Row { table: self, index })
    }

    /// Every record.
    pub fn rows(&self) -> impl Iterator<Item = Row<'_>> {
        (0..self.db.len()).map(move |index| Row { table: self, index })
    }

    /// The records whose column `name` is `value` (a parent's children).
    pub fn rows_where(&self, name: &'static str, value: u32) -> Vec<Row<'_>> {
        let map = {
            let Ok(mut by) = self.by.lock() else {
                return Vec::new();
            };
            by.entry(name)
                .or_insert_with(|| {
                    let mut m: HashMap<u32, Vec<usize>> = HashMap::new();
                    for r in self.rows() {
                        m.entry(r.int(name)).or_default().push(r.index);
                    }
                    std::sync::Arc::new(m)
                })
                .clone()
        };
        map.get(&value)
            .map(|v| v.iter().map(|&index| Row { table: self, index }).collect())
            .unwrap_or_default()
    }
}

impl Row<'_> {
    pub fn id(&self) -> u32 {
        self.table.db.id(self.index)
    }

    /// A column as an integer (0 when the layout lacks it).
    pub fn int(&self, name: &str) -> u32 {
        match self.table.col(name) {
            Some(Col::Field(f)) => self.table.db.int(self.index, f).unwrap_or(0),
            Some(Col::Id) => self.id(),
            Some(Col::Relation) => self.table.db.relation(self.index),
            None => 0,
        }
    }

    /// A column as a signed integer.
    pub fn signed(&self, name: &str) -> i32 {
        match self.table.col(name) {
            Some(Col::Field(f)) => self.table.db.signed(self.index, f).unwrap_or(0),
            _ => self.int(name) as i32,
        }
    }

    /// A float column (0 when the layout lacks it).
    pub fn float(&self, name: &str) -> f32 {
        match self.table.col(name) {
            Some(Col::Field(f)) => self.table.db.float(self.index, f).unwrap_or(0.0),
            _ => 0.0,
        }
    }

    /// An array column's elements.
    pub fn ints(&self, name: &str) -> Vec<u32> {
        match self.table.col(name) {
            Some(Col::Field(f)) => self.table.db.ints(self.index, f),
            Some(_) => vec![self.int(name)],
            None => Vec::new(),
        }
    }

    /// An array of floats.
    pub fn floats(&self, name: &str) -> Vec<f32> {
        self.ints(name).into_iter().map(f32::from_bits).collect()
    }

    pub fn string(&self, name: &str) -> Option<String> {
        match self.table.col(name)? {
            Col::Field(f) => self.table.db.string(self.index, f),
            _ => None,
        }
    }
}
