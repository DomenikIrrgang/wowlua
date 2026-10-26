//! The client's answers to calls that its tables answer (decision T464),
//! written as the recordings write answers (`1: {flags=number 2,…}`), so
//! that the engine takes them where it would take a recorded one: the
//! world maps' static data (`C_Map.GetMapInfo`, `GetMapChildrenInfo`,
//! `GetMapRectOnMap`, `GetMapWorldSize`, the map art's ID, layers and
//! tiles, its text positions); the achievements' own data and their
//! categories (decision T465: the character's part of an answer the
//! engine keeps from the recordings); the item classes' and subclasses'
//! names.

use std::collections::{BTreeMap, HashMap};

use wowlua_casc::Casc;
use wowlua_casc::tables::Table;

/// A map (`UiMap`).
struct UiMap {
    name: String,
    parent: u32,
    flags: u32,
    kind: u32,
    help_text: u32,
    zone_text: u32,
    background_atlas: u32,
}

/// Where a part of a map lies in the world (`UiMapAssignment`).
struct Assignment {
    order: u32,
    ui_map: u32,
    continent: u32,
    ui_min: [f64; 2],
    ui_max: [f64; 2],
    /// The world's minimum X, Y, then maximum X, Y.
    region: [f64; 4],
}

impl Assignment {
    /// Yards across a whole map (its UI's 0 to 1): east-west, north-south.
    fn size(&self) -> (f64, f64) {
        (
            (self.region[3] - self.region[1]) / (self.ui_max[0] - self.ui_min[0]),
            (self.region[2] - self.region[0]) / (self.ui_max[1] - self.ui_min[1]),
        )
    }

    /// The world's Y and X at a point of its map (the world's X runs north,
    /// its Y west).
    fn world(&self, x: f64, y: f64) -> (f64, f64) {
        let (w, h) = self.size();
        (
            self.region[3] - (x - self.ui_min[0]) * w,
            self.region[2] - (y - self.ui_min[1]) * h,
        )
    }

    /// The point of its map at the world's Y and X.
    fn map(&self, wy: f64, wx: f64) -> (f64, f64) {
        let (w, h) = self.size();
        (
            self.ui_min[0] + (self.region[3] - wy) / w,
            self.ui_min[1] + (self.region[2] - wx) / h,
        )
    }
}

/// A map art style's layer (`UiMapArtStyleLayer`).
struct Layer {
    index: u32,
    width: u32,
    height: u32,
    tile_width: u32,
    tile_height: u32,
    min_scale: f32,
    max_scale: f32,
    zoom_steps: u32,
}

/// The world maps' tables.
struct Maps {
    maps: BTreeMap<u32, UiMap>,
    /// By ID.
    assignments: Vec<Assignment>,
    /// A map's art (`UiMapXMapArt`).
    art: HashMap<u32, u32>,
    /// An art's style (`UiMapArt`).
    style: HashMap<u32, u32>,
    /// A style's layers, by index.
    layers: HashMap<u32, Vec<Layer>>,
    /// An art's tiles: (layer, row, column, file).
    tiles: HashMap<u32, Vec<(u32, u32, u32, u32)>>,
}

impl Maps {
    fn open(casc: &Casc) -> Option<Maps> {
        let open = |name: &str| Table::open(casc, name);
        let maps = open("UiMap")?
            .rows()
            .map(|r| {
                (
                    r.id(),
                    UiMap {
                        name: r.string("Name_lang").unwrap_or_default(),
                        parent: r.int("ParentUiMapID"),
                        flags: r.int("Flags"),
                        kind: r.int("Type"),
                        help_text: r.int("HelpTextPosition"),
                        zone_text: r.int("MapArtZoneTextPosition"),
                        background_atlas: r.int("BkgAtlasID"),
                    },
                )
            })
            .collect();
        let mut assignments: Vec<(u32, Assignment)> = open("UiMapAssignment")?
            .rows()
            .map(|r| {
                let f = |c: &str| -> Vec<f64> { r.floats(c).into_iter().map(f64::from).collect() };
                let (min, max, region) = (f("UiMin"), f("UiMax"), f("Region"));
                let at = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
                (
                    r.id(),
                    Assignment {
                        order: r.int("OrderIndex"),
                        ui_map: r.int("UiMapID"),
                        continent: r.int("MapID"),
                        ui_min: [at(&min, 0), at(&min, 1)],
                        ui_max: [at(&max, 0), at(&max, 1)],
                        region: [
                            at(&region, 0),
                            at(&region, 1),
                            at(&region, 3),
                            at(&region, 4),
                        ],
                    },
                )
            })
            .collect();
        assignments.sort_by_key(|(id, _)| *id);
        let mut art = HashMap::new();
        if let Some(t) = open("UiMapXMapArt") {
            for r in t.rows() {
                if r.int("PhaseID") == 0 {
                    art.entry(r.int("UiMapID")).or_insert(r.int("UiMapArtID"));
                }
            }
        }
        let style = open("UiMapArt")
            .map(|t| {
                t.rows()
                    .map(|r| (r.id(), r.int("UiMapArtStyleID")))
                    .collect()
            })
            .unwrap_or_default();
        let mut layers: HashMap<u32, Vec<Layer>> = HashMap::new();
        if let Some(t) = open("UiMapArtStyleLayer") {
            for r in t.rows() {
                layers
                    .entry(r.int("UiMapArtStyleID"))
                    .or_default()
                    .push(Layer {
                        index: r.int("LayerIndex"),
                        width: r.int("LayerWidth"),
                        height: r.int("LayerHeight"),
                        tile_width: r.int("TileWidth"),
                        tile_height: r.int("TileHeight"),
                        min_scale: r.float("MinScale"),
                        max_scale: r.float("MaxScale"),
                        zoom_steps: r.int("AdditionalZoomSteps"),
                    });
            }
        }
        for l in layers.values_mut() {
            l.sort_by_key(|l| l.index);
        }
        let mut tiles: HashMap<u32, Vec<(u32, u32, u32, u32)>> = HashMap::new();
        if let Some(t) = open("UiMapArtTile") {
            for r in t.rows() {
                tiles.entry(r.int("UiMapArtID")).or_default().push((
                    r.int("LayerIndex"),
                    r.int("RowIndex"),
                    r.int("ColIndex"),
                    r.int("FileDataID"),
                ));
            }
        }
        for t in tiles.values_mut() {
            t.sort_unstable();
        }
        Some(Maps {
            maps,
            assignments: assignments.into_iter().map(|(_, a)| a).collect(),
            art,
            style,
            layers,
            tiles,
        })
    }

    /// A map's first part in the world.
    fn assignment(&self, map: u32) -> Option<&Assignment> {
        self.assignments.iter().find(|a| a.ui_map == map)
    }

    fn info(&self, id: u32) -> Option<String> {
        let m = self.maps.get(&id)?;
        Some(format!(
            "{{flags=number {},mapID=number {id},mapType=number {},name=string {},parentMapID=number {}}}",
            m.flags, m.kind, m.name, m.parent
        ))
    }

    /// A map's children (by ID), of a type or any; with `all`, theirs too
    /// after each.
    fn children(&self, map: u32, kind: Option<u32>, all: bool, out: &mut Vec<u32>) {
        for (id, m) in &self.maps {
            if m.parent != map {
                continue;
            }
            if kind.is_none_or(|k| k == m.kind) {
                out.push(*id);
            }
            if all {
                self.children(*id, kind, all, out);
            }
        }
    }

    fn answer(&self, name: &str, args: &[&str]) -> Option<String> {
        let id = |i: usize| -> Option<u32> { args.get(i)?.parse().ok() };
        let map = id(0)?;
        Some(match name {
            "C_Map.GetMapInfo" => match self.info(map) {
                Some(info) => format!("1: {info}"),
                None => "0: ".into(),
            },
            "C_Map.GetMapChildrenInfo" if !self.maps.contains_key(&map) => "0: ".into(),
            "C_Map.GetMapChildrenInfo" => {
                let kind = id(1);
                let all = args.get(2) == Some(&"true");
                let mut ids = Vec::new();
                self.children(map, kind, all, &mut ids);
                list(ids.iter().filter_map(|c| self.info(*c)))
            }
            "C_Map.GetMapRectOnMap" => {
                let top = id(1)?;
                let rect = (|| {
                    let a = self.assignment(map)?;
                    let b = self
                        .assignments
                        .iter()
                        .find(|b| b.ui_map == top && b.continent == a.continent)?;
                    let (y0, x0) = a.world(0.0, 0.0);
                    let (y1, x1) = a.world(1.0, 1.0);
                    let (left, top) = b.map(y0, x0);
                    let (right, bottom) = b.map(y1, x1);
                    Some([left, right, top, bottom])
                })()
                .unwrap_or([0.0; 4]);
                format!(
                    "4: number {}, number {}, number {}, number {}",
                    rect[0], rect[1], rect[2], rect[3]
                )
            }
            // The first part by its order, each extent over the other
            // axis's part of the map (recorded: Azeroth's 19731.83 by
            // 68042.07 yards, Kalimdor's part's; the same for a map in one
            // part).
            "C_Map.GetMapWorldSize" => match self
                .assignments
                .iter()
                .filter(|a| a.ui_map == map)
                .min_by_key(|a| a.order)
            {
                Some(a) => {
                    let w = (a.region[3] - a.region[1]) / (a.ui_max[1] - a.ui_min[1]);
                    let h = (a.region[2] - a.region[0]) / (a.ui_max[0] - a.ui_min[0]);
                    format!("2: number {w}, number {h}")
                }
                None => "2: number 0, number 0".into(),
            },
            "C_Map.GetMapArtID" => match self.art.get(&map) {
                Some(art) => format!("1: number {art}"),
                None => "0: ".into(),
            },
            "C_Map.GetMapArtLayers" => {
                let Some(layers) = self
                    .art
                    .get(&map)
                    .and_then(|a| self.style.get(a))
                    .and_then(|s| self.layers.get(s))
                else {
                    return Some("0: ".into());
                };
                list(layers.iter().map(|l| {
                    format!(
                        "{{additionalZoomSteps=number {},layerHeight=number {},layerWidth=number {},\
                         maxScale=number {},minScale=number {},tileHeight=number {},tileWidth=number {}}}",
                        l.zoom_steps,
                        l.height,
                        l.width,
                        l.max_scale,
                        l.min_scale,
                        l.tile_height,
                        l.tile_width
                    )
                }))
            }
            "C_Map.GetMapArtLayerTextures" => {
                let layer = id(1)?.checked_sub(1)?;
                let tiles = self.art.get(&map).and_then(|a| self.tiles.get(a))?;
                list(
                    tiles
                        .iter()
                        .filter(|t| t.0 == layer)
                        .map(|t| format!("number {}", t.3)),
                )
            }
            "C_Map.MapHasArt" => format!("1: boolean {}", self.art.contains_key(&map)),
            "C_Map.GetMapArtHelpTextPosition" => {
                format!("1: number {}", self.maps.get(&map)?.help_text)
            }
            "C_Map.GetMapArtZoneTextPosition" => {
                format!("1: number {}", self.maps.get(&map)?.zone_text)
            }
            // (an atlas's name: the recordings' where a map has one)
            "C_Map.GetMapArtBackgroundAtlas" if self.maps.get(&map)?.background_atlas == 0 => {
                "1: nil nil".into()
            }
            _ => return None,
        })
    }
}

/// An achievement (`Achievement`).
struct Achievement {
    title: String,
    description: String,
    reward: String,
    flags: u32,
    supercedes: u32,
    category: u32,
    points: u32,
    icon: u32,
}

/// `ACHIEVEMENT_FLAGS_COUNTER` (a statistic), `ACHIEVEMENT_FLAGS_GUILD`.
const STATISTIC: u32 = 0x1;
const GUILD: u32 = 0x4000;
/// The statistics' root category.
const STATISTICS: u32 = 1;

/// The achievements, their categories and the trait currency they give.
struct Achievements {
    achievements: HashMap<u32, Achievement>,
    /// Name, parent (none: 0), display order.
    categories: BTreeMap<u32, (String, u32, u32)>,
    /// The achievement following one in its series.
    next: HashMap<u32, u32>,
    /// (currency, achievement) → amount (`TraitCurrencySource`).
    currency: HashMap<(u32, u32), u32>,
}

impl Achievements {
    fn open(casc: &Casc) -> Option<Achievements> {
        let open = |name: &str| Table::open(casc, name);
        let mut achievements = HashMap::new();
        let mut next = HashMap::new();
        for r in open("Achievement")?.rows() {
            let text = |c: &str| r.string(c).unwrap_or_default();
            let supercedes = r.int("Supercedes");
            if supercedes != 0 {
                next.entry(supercedes).or_insert(r.id());
            }
            achievements.insert(
                r.id(),
                Achievement {
                    title: text("Title_lang"),
                    description: text("Description_lang"),
                    reward: text("Reward_lang"),
                    flags: r.int("Flags"),
                    supercedes,
                    category: r.int("Category"),
                    points: r.int("Points"),
                    icon: r.int("IconFileID"),
                },
            );
        }
        let rows: Vec<(u32, String, u32, u32)> = open("Achievement_Category")?
            .rows()
            .map(|r| {
                (
                    r.id(),
                    r.string("Name_lang").unwrap_or_default(),
                    r.int("Parent"),
                    r.int("Ui_order"),
                )
            })
            .collect();
        let ids: Vec<u32> = rows.iter().map(|r| r.0).collect();
        let categories = rows
            .into_iter()
            .map(|(id, name, parent, order)| {
                // (none: -1, as its field's bits hold it)
                let parent = if ids.contains(&parent) { parent } else { 0 };
                (id, (name, parent, order))
            })
            .collect();
        let mut currency = HashMap::new();
        if let Some(t) = open("TraitCurrencySource") {
            for r in t.rows() {
                if r.int("AchievementID") != 0 {
                    *currency
                        .entry((r.int("TraitCurrencyID"), r.int("AchievementID")))
                        .or_insert(0) += r.int("Amount");
                }
            }
        }
        Some(Achievements {
            achievements,
            categories,
            next,
            currency,
        })
    }

    /// Whether a category is under the statistics' root.
    fn statistic(&self, mut id: u32) -> bool {
        for _ in 0..16 {
            match self.categories.get(&id) {
                Some((_, parent, _)) if *parent == STATISTICS => return true,
                Some((_, parent, _)) if *parent != 0 => id = *parent,
                _ => return false,
            }
        }
        false
    }

    /// The achievements' categories or the statistics', by display order,
    /// then ID.
    fn category_list(&self, statistics: bool) -> String {
        let mut ids: Vec<(u32, u32)> = self
            .categories
            .iter()
            .filter(|(id, _)| **id != STATISTICS && self.statistic(**id) == statistics)
            .map(|(id, (_, _, order))| (*order, *id))
            .collect();
        ids.sort_unstable();
        list(ids.into_iter().map(|(_, id)| format!("number {id}")))
    }

    fn answer(&self, name: &str, args: &[&str]) -> Option<String> {
        let id = |i: usize| -> Option<u32> { args.get(i)?.parse().ok() };
        let a = |i: usize| self.achievements.get(&id(i)?);
        Some(match name {
            // By ID; the character's part (completed, its date, by whom)
            // the engine's (decision T465).
            "GetAchievementInfo" if args.len() == 1 => {
                let x = a(0)?;
                let icon = match x.icon {
                    0 => "nil nil".to_string(),
                    i => format!("number {i}"),
                };
                format!(
                    "15: number {}, string {}, number {}, boolean false, nil nil, nil nil, nil nil, \
                     string {}, number {}, {icon}, string {}, boolean {}, boolean false, nil nil, \
                     boolean {}",
                    id(0)?,
                    x.title,
                    x.points,
                    x.description,
                    x.flags,
                    x.reward,
                    x.flags & GUILD != 0,
                    x.flags & STATISTIC != 0
                )
            }
            "GetAchievementCategory" => format!("1: number {}", a(0)?.category),
            "C_AchievementInfo.IsValidAchievement" => format!("1: boolean {}", a(0).is_some()),
            "C_AchievementInfo.IsGuildAchievement" => {
                format!("1: boolean {}", a(0).is_some_and(|x| x.flags & GUILD != 0))
            }
            "GetPreviousAchievement" => match a(0)?.supercedes {
                0 => "0: ".into(),
                s => format!("1: number {s}"),
            },
            "GetNextAchievement" => match self.next.get(&id(0)?) {
                Some(n) => format!("2: number {n}, boolean false"),
                None => "0: ".into(),
            },
            "GetCategoryInfo" => {
                let (name, parent, _) = self.categories.get(&id(0)?)?;
                let parent = match *parent {
                    0 | STATISTICS => -1,
                    p => i64::from(p),
                };
                format!("3: string {name}, number {parent}, number 0")
            }
            "GetCategoryList" => self.category_list(false),
            "GetStatisticsCategoryList" => self.category_list(true),
            "C_Traits.GetTraitCurrencyForAchievement" => format!(
                "1: number {}",
                self.currency.get(&(id(0)?, id(1)?)).copied().unwrap_or(0)
            ),
            _ => return None,
        })
    }
}

/// The item classes and subclasses' names (`ItemClass`, `ItemSubClass`).
struct ItemKinds {
    classes: HashMap<u32, String>,
    /// (class, subclass) → name (the verbose one, else the shown one),
    /// whether it is an armor type (its flag 512).
    subclasses: HashMap<(u32, u32), (String, bool)>,
}

impl ItemKinds {
    fn open(casc: &Casc) -> Option<ItemKinds> {
        let classes = Table::open(casc, "ItemClass")?
            .rows()
            .map(|r| {
                (
                    r.int("ClassID"),
                    r.string("ClassName_lang").unwrap_or_default(),
                )
            })
            .collect();
        let subclasses = Table::open(casc, "ItemSubClass")?
            .rows()
            .map(|r| {
                let verbose = r.string("VerboseName_lang").unwrap_or_default();
                let name = if verbose.is_empty() {
                    r.string("DisplayName_lang").unwrap_or_default()
                } else {
                    verbose
                };
                (
                    (r.int("ClassID"), r.int("SubClassID")),
                    (name, r.int("Flags") & 512 != 0),
                )
            })
            .collect();
        Some(ItemKinds {
            classes,
            subclasses,
        })
    }

    fn answer(&self, name: &str, args: &[&str]) -> Option<String> {
        let id = |i: usize| -> Option<u32> { args.get(i)?.parse().ok() };
        Some(match name {
            "C_Item.GetItemClassInfo" | "GetItemClassInfo" => match self.classes.get(&id(0)?) {
                Some(n) => format!("1: string {n}"),
                None => "1: nil nil".into(),
            },
            "C_Item.GetItemSubClassInfo" | "GetItemSubClassInfo" => {
                match self.subclasses.get(&(id(0)?, id(1)?)) {
                    Some((n, armor)) => format!("2: string {n}, boolean {armor}"),
                    None => "0: ".into(),
                }
            }
            _ => return None,
        })
    }
}

/// A list's answer (`1: {1=…,2=…}`).
fn list(items: impl Iterator<Item = String>) -> String {
    let items: Vec<String> = items
        .enumerate()
        .map(|(i, v)| format!("{}={v}", i + 1))
        .collect();
    format!("1: {{{}}}", items.join(","))
}

/// The answers the client's tables give.
pub(crate) struct Answers {
    maps: Option<Maps>,
    achievements: Option<Achievements>,
    items: Option<ItemKinds>,
}

impl Answers {
    pub(crate) fn open(casc: &Casc) -> Answers {
        Answers {
            maps: Maps::open(casc),
            achievements: Achievements::open(casc),
            items: ItemKinds::open(casc),
        }
    }

    /// A call's answer (`C_Map.GetMapInfo(1411)`), if the tables give it.
    pub(crate) fn answer(&self, call: &str) -> Option<String> {
        let (name, rest) = call.split_once('(')?;
        let args: Vec<&str> = rest
            .strip_suffix(')')?
            .split(", ")
            .filter(|a| !a.is_empty())
            .collect();
        if name.starts_with("C_Map.") {
            return self.maps.as_ref()?.answer(name, &args);
        }
        if let Some(a) = self.items.as_ref().and_then(|i| i.answer(name, &args)) {
            return Some(a);
        }
        self.achievements.as_ref()?.answer(name, &args)
    }
}
