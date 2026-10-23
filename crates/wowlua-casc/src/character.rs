//! Characters (spec debugger §18.10, decision T320): a race and sex's
//! model with its customizations (geosets, the skin composited from
//! layers, hair and other textures, models skinned to it) and what it
//! wears (geosets, textures in the skin's sections, models at attachment
//! points or skinned to it); and humanoid creatures, whose extra display
//! info gives the same with a baked skin.

use std::collections::{BTreeMap, HashMap};

use crate::Casc;
use crate::display::{Composite, Display, Layer, Library, Part, Texture};

/// What a character wears in a slot: an item, an item's appearance
/// (`ItemModifiedAppearance`), or an item display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Worn {
    Item(u32),
    Appearance(u32),
    Display(u32),
}

/// A character to resolve: race, sex (0 male, 1 female), class, the
/// customization choices (others take their option's first), and what it
/// wears by inventory slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub race: u32,
    pub sex: u32,
    pub class: u32,
    pub choices: Vec<u32>,
    pub worn: Vec<(u32, Worn)>,
    /// A baked skin (a humanoid creature's) in place of the composited
    /// one and the worn items' textures.
    pub bake: Option<u32>,
}

/// A composite being built: its size, and its layers by their order.
type Canvas = (u32, u32, BTreeMap<(u32, u32), Layer>);

/// Geoset groups (`CG_*`).
const SLEEVES: u16 = 8;
const KNEEPADS: u16 = 9;
const CHEST: u16 = 10;
const PANTS: u16 = 11;
const TABARD: u16 = 12;
const TROUSERS: u16 = 13;
const CLOAK: u16 = 15;
const BELT: u16 = 18;
const FEET: u16 = 20;
const SKULL: u16 = 21;
const TORSO: u16 = 22;
const HAND_ATTACHMENT: u16 = 23;
const GLOVES: u16 = 4;
const BOOTS: u16 = 5;
const SHOULDERS: u16 = 26;
const HELM: u16 = 27;
const ARM_UPPER: u16 = 28;

/// Each slot's geoset groups, by its display's `GeosetGroup` index.
fn slot_groups(slot: u32) -> &'static [u16] {
    match slot {
        1 => &[HELM, SKULL],
        3 => &[SHOULDERS],
        4 => &[SLEEVES, CHEST],
        5 => &[SLEEVES, CHEST, TROUSERS, TORSO, ARM_UPPER],
        6 => &[BELT],
        7 => &[PANTS, KNEEPADS, TROUSERS],
        8 => &[BOOTS, FEET],
        10 => &[GLOVES, HAND_ATTACHMENT],
        15 => &[CLOAK],
        19 => &[TABARD],
        _ => &[],
    }
}

/// Which slot decides a group when several would (the first listed).
fn group_priority(group: u16) -> &'static [u32] {
    match group {
        SLEEVES => &[10, 5, 4],
        CHEST => &[5, 4],
        TROUSERS => &[5, 7],
        _ => &[],
    }
}

/// The order worn items' textures are layered in the skin (later over
/// earlier).
const TEXTURE_ORDER: [u32; 9] = [4, 7, 8, 5, 9, 10, 6, 19, 15];

/// A humanoid creature's item slots (`NPCModelItemSlotDisplayInfo`) as
/// inventory slots.
const NPC_SLOTS: [u32; 11] = [1, 3, 4, 5, 6, 7, 8, 9, 10, 19, 15];

impl Library {
    /// A file of a material resource: the one for the race, sex and class
    /// where `ComponentTextureFileData` tells them apart, else the first.
    fn material_file(&mut self, casc: &Casc, resource: u32, look: &Look) -> Option<u32> {
        if resource == 0 {
            return None;
        }
        let files: Vec<u32> = self
            .table(casc, "TextureFileData")?
            .rows_where("MaterialResourcesID", resource)
            .iter()
            .filter(|r| r.int("UsageType") == 0)
            .map(|r| r.int("FileDataID"))
            .collect();
        if files.len() <= 1 {
            return files.first().copied();
        }
        let Some(info) = self.table(casc, "ComponentTextureFileData") else {
            return files.first().copied();
        };
        // Gender 0 male, 1 female, 3 either.
        let mut best: Option<(u32, (u8, u8, u8))> = None;
        for &f in &files {
            let rank = match info.row(f) {
                Some(r) => {
                    let g = r.int("GenderIndex");
                    let c = r.int("ClassID");
                    if !(g == look.sex || g == 3) || !(c == 0 || c == look.class) {
                        continue;
                    }
                    (
                        u8::from(g != look.sex),
                        u8::from(!(look.class != 0 && c == look.class)),
                        u8::from(r.int("RaceID") != look.race),
                    )
                }
                None => (2, 2, 2),
            };
            if best.is_none_or(|(_, b)| rank < b) {
                best = Some((f, rank));
            }
        }
        best.map(|(f, _)| f).or(files.first().copied())
    }

    /// A model resource's file for the race and sex (`position`: 0 left,
    /// 1 right, for shoulders).
    fn model_file(
        &mut self,
        casc: &Casc,
        resource: u32,
        look: &Look,
        position: Option<u32>,
    ) -> Option<u32> {
        if resource == 0 {
            return None;
        }
        let files: Vec<u32> = self
            .table(casc, "ModelFileData")?
            .rows_where("ModelResourcesID", resource)
            .iter()
            .map(|r| r.int("FileDataID"))
            .collect();
        if files.len() <= 1 && position.is_none() {
            return files.first().copied();
        }
        let info = self.table(casc, "ComponentModelFileData");
        let mut best: Option<(u32, u8)> = None;
        for &f in &files {
            let Some(r) = info.and_then(|t| t.row(f)) else {
                if best.is_none() {
                    best = Some((f, 9));
                }
                continue;
            };
            if let Some(p) = position
                && r.int("PositionIndex") != p
            {
                continue;
            }
            let (race, g) = (r.int("RaceID"), r.int("GenderIndex"));
            // Gender 2: either.
            let rank = if race == look.race && g == look.sex {
                0
            } else if race == look.race && g == 2 {
                1
            } else if race == 0 {
                2
            } else {
                8
            };
            if best.is_none_or(|(_, b)| rank < b) {
                best = Some((f, rank));
            }
        }
        best.map(|(f, _)| f)
    }

    /// An item display of what is worn.
    fn item_display(&mut self, casc: &Casc, worn: Worn) -> Option<u32> {
        let appearance = match worn {
            Worn::Display(d) => return Some(d),
            Worn::Appearance(a) => self
                .table(casc, "ItemModifiedAppearance")?
                .row(a)?
                .int("ItemAppearanceID"),
            Worn::Item(i) => {
                let rows = self
                    .table(casc, "ItemModifiedAppearance")?
                    .rows_where("ItemID", i);
                rows.iter()
                    .min_by_key(|r| (r.int("ItemAppearanceModifierID"), r.int("OrderIndex")))?
                    .int("ItemAppearanceID")
            }
        };
        let d = self
            .table(casc, "ItemAppearance")?
            .row(appearance)?
            .int("ItemDisplayInfoID");
        (d != 0).then_some(d)
    }

    /// The inventory slot an item goes in, by its inventory type.
    pub fn item_slot(&mut self, casc: &Casc, worn: Worn) -> Option<u32> {
        let item = match worn {
            Worn::Item(i) => i,
            Worn::Appearance(a) => self
                .table(casc, "ItemModifiedAppearance")?
                .row(a)?
                .int("ItemID"),
            Worn::Display(_) => return None,
        };
        let kind = self.table(casc, "Item")?.row(item)?.int("InventoryType");
        Some(match kind {
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            5 | 20 => 5,
            6 => 6,
            7 => 7,
            8 => 8,
            9 => 9,
            10 => 10,
            11 => 11,
            12 => 13,
            16 => 15,
            19 => 19,
            13 | 17 | 21 => 16,
            14 | 22 | 23 => 17,
            15 | 25 | 26 => 18,
            _ => return None,
        })
    }

    /// The model file and scale of a creature display.
    fn display_model(&mut self, casc: &Casc, display: u32) -> Option<u32> {
        let model = self
            .table(casc, "CreatureDisplayInfo")?
            .row(display)?
            .int("ModelID");
        let f = self
            .table(casc, "CreatureModelData")?
            .row(model)?
            .int("FileDataID");
        (f != 0).then_some(f)
    }

    /// A character as it looks.
    pub fn character(&mut self, casc: &Casc, look: &Look) -> Option<Display> {
        let chr_model = self
            .table(casc, "ChrRaceXChrModel")?
            .rows_where("ChrRacesID", look.race)
            .iter()
            .find(|r| r.int("Sex") == look.sex)?
            .int("ChrModelID");
        let (display, layout) = {
            let r = self.table(casc, "ChrModel")?.row(chr_model)?;
            (r.int("DisplayID"), r.int("CharComponentTextureLayoutID"))
        };
        let file = self.display_model(casc, display)?;
        self.dress(casc, look, chr_model, layout, file)
    }

    /// A humanoid creature (`CreatureDisplayInfoExtra` ID); `fallback` its
    /// display's own model.
    pub fn npc(&mut self, casc: &Casc, extra: u32, fallback: u32) -> Option<Display> {
        let (race, sex, class, bake) = {
            let r = self.table(casc, "CreatureDisplayInfoExtra")?.row(extra)?;
            let hd = r.int("HDBakeMaterialResourcesID");
            (
                r.int("DisplayRaceID"),
                r.int("DisplaySexID"),
                r.int("DisplayClassID"),
                if hd != 0 {
                    hd
                } else {
                    r.int("BakeMaterialResourcesID")
                },
            )
        };
        let choices: Vec<u32> = self
            .table(casc, "CreatureDisplayInfoOption")
            .map(|t| {
                t.rows_where("CreatureDisplayInfoExtraID", extra)
                    .iter()
                    .map(|r| r.int("ChrCustomizationChoiceID"))
                    .collect()
            })
            .unwrap_or_default();
        let worn: Vec<(u32, Worn)> = self
            .table(casc, "NPCModelItemSlotDisplayInfo")
            .map(|t| {
                t.rows_where("NpcModelID", extra)
                    .iter()
                    .filter_map(|r| {
                        let slot = *NPC_SLOTS.get(r.int("ItemSlot") as usize)?;
                        let d = r.int("ItemDisplayInfoID");
                        (d != 0).then_some((slot, Worn::Display(d)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut look = Look {
            race,
            sex,
            class,
            choices,
            worn,
            bake: None,
        };
        look.bake = self.material_file(casc, bake, &look);
        let found = self.table(casc, "ChrRaceXChrModel").and_then(|t| {
            t.rows_where("ChrRacesID", race)
                .iter()
                .find(|r| r.int("Sex") == sex)
                .map(|r| r.int("ChrModelID"))
        });
        let (chr_model, layout, file) = match found {
            Some(m) => {
                let (display, layout) = {
                    let r = self.table(casc, "ChrModel")?.row(m)?;
                    (r.int("DisplayID"), r.int("CharComponentTextureLayoutID"))
                };
                (
                    m,
                    layout,
                    self.display_model(casc, display).unwrap_or(fallback),
                )
            }
            None => (0, 0, fallback),
        };
        self.dress(casc, &look, chr_model, layout, file)
    }

    /// The chosen customization of each option of a model: the look's,
    /// else the option's first choice the class may have.
    fn chosen(&mut self, casc: &Casc, look: &Look, chr_model: u32) -> Vec<(u32, u32)> {
        let Some(options) = self.table(casc, "ChrCustomizationOption").map(|t| {
            let mut o: Vec<(u32, u32)> = t
                .rows_where("ChrModelID", chr_model)
                .iter()
                .map(|r| (r.int("OrderIndex"), r.id()))
                .collect();
            o.sort();
            o
        }) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (_, option) in options {
            let choices: Vec<(u32, u32, u32)> = self
                .table(casc, "ChrCustomizationChoice")
                .map(|t| {
                    let mut c: Vec<(u32, u32, u32)> = t
                        .rows_where("ChrCustomizationOptionID", option)
                        .iter()
                        .map(|r| (r.int("OrderIndex"), r.id(), r.int("ChrCustomizationReqID")))
                        .collect();
                    c.sort();
                    c
                })
                .unwrap_or_default();
            if let Some(&(_, c, _)) = choices.iter().find(|(_, c, _)| look.choices.contains(c)) {
                out.push((option, c));
                continue;
            }
            // The first choice without a requirement, or whose class mask
            // has the class.
            let class_bit = if look.class > 0 {
                1u32 << (look.class - 1)
            } else {
                0
            };
            let pick = choices
                .iter()
                .find(|(_, _, req)| *req == 0)
                .or_else(|| {
                    choices.iter().find(|(_, _, req)| {
                        self.tables_class_mask(casc, *req)
                            .is_some_and(|m| m == 0 || m & class_bit != 0)
                    })
                })
                .or(choices.first());
            if let Some(&(_, c, _)) = pick {
                out.push((option, c));
            }
        }
        out
    }

    fn tables_class_mask(&self, _casc: &Casc, req: u32) -> Option<u32> {
        let t = self.tables.get("ChrCustomizationReq")?.as_ref()?;
        Some(t.row(req)?.int("ClassMask"))
    }

    /// The body with its customizations and what it wears.
    fn dress(
        &mut self,
        casc: &Casc,
        look: &Look,
        chr_model: u32,
        layout: u32,
        file: u32,
    ) -> Option<Display> {
        let _ = self.table(casc, "ChrCustomizationReq");
        let mut body = Part::with_file(file);
        let mut parts: Vec<Part> = Vec::new();
        // Composites by texture kind: their size and layers by order.
        let mut canvases: HashMap<u32, Canvas> = HashMap::new();
        let chosen = self.chosen(casc, look, chr_model);
        let chosen_ids: Vec<u32> = chosen.iter().map(|(_, c)| *c).collect();
        // The layout's sections and materials.
        let sections: Vec<(u32, [u32; 4])> = self
            .table(casc, "CharComponentTextureSections")
            .map(|t| {
                t.rows_where("CharComponentTextureLayoutID", layout)
                    .iter()
                    .map(|r| {
                        (
                            r.int("SectionType"),
                            [r.int("X"), r.int("Y"), r.int("Width"), r.int("Height")],
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let materials: HashMap<u32, (u32, u32)> = self
            .table(casc, "ChrModelMaterial")
            .map(|t| {
                t.rows_where("CharComponentTextureLayoutsID", layout)
                    .iter()
                    .map(|r| (r.int("TextureType"), (r.int("Width"), r.int("Height"))))
                    .collect()
            })
            .unwrap_or_default();
        // Texture layers by their target.
        let layers: HashMap<u32, (u32, u32, u32)> = self
            .table(casc, "ChrModelTextureLayer")
            .map(|t| {
                t.rows_where("CharComponentTextureLayoutsID", layout)
                    .iter()
                    .filter_map(|r| {
                        let target = *r.ints("ChrModelTextureTargetID").first()?;
                        Some((
                            target,
                            (
                                r.int("TextureType"),
                                r.int("BlendMode"),
                                r.int("TextureSectionTypeBitMask"),
                            ),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let skin_size = materials.get(&1).copied();
        for (option, choice) in &chosen {
            // The option's other choices' geosets are hidden.
            let others: Vec<u32> = self
                .table(casc, "ChrCustomizationChoice")
                .map(|t| {
                    t.rows_where("ChrCustomizationOptionID", *option)
                        .iter()
                        .map(|r| r.id())
                        .filter(|c| c != choice)
                        .collect()
                })
                .unwrap_or_default();
            for other in others {
                for g in self.choice_geosets(casc, other) {
                    body.hidden.push(g);
                }
            }
            let elements: Vec<[u32; 4]> = self
                .table(casc, "ChrCustomizationElement")
                .map(|t| {
                    t.rows_where("ChrCustomizationChoiceID", *choice)
                        .iter()
                        .map(|r| {
                            [
                                r.int("RelatedChrCustomizationChoiceID"),
                                r.int("ChrCustomizationGeosetID"),
                                r.int("ChrCustomizationMaterialID"),
                                r.int("ChrCustomizationSkinnedModelID"),
                            ]
                        })
                        .collect()
                })
                .unwrap_or_default();
            for [related, geoset, material, skinned] in elements {
                if related != 0 && !chosen_ids.contains(&related) {
                    continue;
                }
                if geoset != 0
                    && let Some(g) = self.geoset(casc, geoset)
                {
                    body.hidden.retain(|h| *h != g);
                    body.shown.push(g);
                }
                if skinned != 0
                    && let Some(r) = self
                        .table(casc, "ChrCustomizationSkinnedModel")
                        .and_then(|t| t.row(skinned))
                {
                    let (f, ty, id) = (
                        r.int("CollectionsFileDataID"),
                        r.int("GeosetType") as u16,
                        r.int("GeosetID") as u16,
                    );
                    if f != 0 {
                        let mut p = Part::with_file(f);
                        p.skinned = true;
                        p.set_group(ty, id);
                        parts.push(p);
                    }
                }
                if material != 0 {
                    let Some((target, resource)) = self
                        .table(casc, "ChrCustomizationMaterial")
                        .and_then(|t| t.row(material))
                        .map(|r| {
                            (
                                r.int("ChrModelTextureTargetID"),
                                r.int("MaterialResourcesID"),
                            )
                        })
                    else {
                        continue;
                    };
                    let Some(&(kind, blend, mask)) = layers.get(&target) else {
                        continue;
                    };
                    let Some(f) = self.material_file(casc, resource, look) else {
                        continue;
                    };
                    if kind == 1 || kind == 8 {
                        if look.bake.is_some() && kind == 1 {
                            continue;
                        }
                        let Some(&(w, h)) = materials.get(&kind) else {
                            continue;
                        };
                        let rect = if mask == u32::MAX {
                            Some([0, 0, w, h])
                        } else {
                            sections
                                .iter()
                                .find(|(t, _)| t < &32 && (1u32 << t) & mask != 0)
                                .map(|(_, r)| *r)
                        };
                        if let Some(rect) = rect {
                            canvases
                                .entry(kind)
                                .or_insert((w, h, BTreeMap::new()))
                                .2
                                .insert(
                                    (0, target),
                                    Layer {
                                        file: f,
                                        rect,
                                        blend,
                                    },
                                );
                        }
                    } else {
                        body.textures.insert(kind, Texture::File(f));
                    }
                }
            }
        }
        // What it wears.
        let mut group_values: HashMap<u16, Vec<(u32, u16)>> = HashMap::new();
        let mut worn: Vec<(u32, Worn)> = look.worn.clone();
        worn.sort_by_key(|(slot, _)| {
            TEXTURE_ORDER
                .iter()
                .position(|s| s == slot)
                .unwrap_or(TEXTURE_ORDER.len())
        });
        for (rank, (slot, w)) in worn.iter().enumerate() {
            let Some(d) = self.item_display(casc, *w) else {
                continue;
            };
            let Some(info) = self.table(casc, "ItemDisplayInfo").and_then(|t| {
                let r = t.row(d)?;
                Some((
                    r.ints("ModelResourcesID"),
                    r.ints("ModelMaterialResourcesID"),
                    r.ints("GeosetGroup"),
                    r.ints("AttachmentGeosetGroup"),
                ))
            }) else {
                continue;
            };
            let (models, model_materials, geoset_groups, attachment_groups) = info;
            for (i, &group) in slot_groups(*slot).iter().enumerate() {
                let g = geoset_groups.get(i).copied().unwrap_or(0) as u16;
                let value = if group == FEET {
                    if g == 0 { 2 } else { g }
                } else {
                    1 + g
                };
                group_values.entry(group).or_default().push((*slot, value));
            }
            // The textures of the skin's sections.
            if look.bake.is_none() && skin_size.is_some() {
                let res: Vec<(u32, u32)> = self
                    .table(casc, "ItemDisplayInfoMaterialRes")
                    .map(|t| {
                        t.rows_where("ItemDisplayInfoID", d)
                            .iter()
                            .map(|r| (r.int("ComponentSection"), r.int("MaterialResourcesID")))
                            .collect()
                    })
                    .unwrap_or_default();
                for (section, resource) in res {
                    let Some(rect) = sections
                        .iter()
                        .find(|(t, _)| *t == section)
                        .map(|(_, r)| *r)
                    else {
                        continue;
                    };
                    let Some(f) = self.material_file(casc, resource, look) else {
                        continue;
                    };
                    let (w, h) = skin_size.unwrap_or((512, 512));
                    canvases
                        .entry(1)
                        .or_insert((w, h, BTreeMap::new()))
                        .2
                        .insert(
                            (1 + rank as u32, section),
                            Layer {
                                file: f,
                                rect,
                                blend: 15,
                            },
                        );
                }
            }
            // A cape: the body's object skin.
            if *slot == 15
                && let Some(f) = model_materials
                    .first()
                    .copied()
                    .and_then(|m| self.material_file(casc, m, look))
            {
                body.textures.insert(2, Texture::File(f));
            }
            // Its models: at attachment points, or made for the body.
            let attachments: &[u32] = match *slot {
                1 => &[11],
                3 => &[6, 5],
                16 => &[1],
                17 => &[2],
                18 => &[2],
                _ => &[],
            };
            let shoulders =
                *slot == 3 && models.len() >= 2 && models[0] != 0 && models[0] == models[1];
            for (i, &resource) in models.iter().enumerate() {
                if resource == 0 {
                    continue;
                }
                let position = shoulders.then_some(i as u32);
                let Some(f) = self.model_file(casc, resource, look, position) else {
                    continue;
                };
                let mut p = Part::with_file(f);
                let texture = model_materials
                    .get(i)
                    .copied()
                    .and_then(|m| self.material_file(casc, m, look))
                    .or_else(|| {
                        // Newer builds name it by the model's index.
                        let t = self.table(casc, "ItemDisplayInfoModelMatRes")?;
                        let r = t
                            .rows_where("ItemDisplayInfoID", d)
                            .into_iter()
                            .find(|r| r.int("ModelIndex") == i as u32)?
                            .int("MaterialResourcesID");
                        self.material_file(casc, r, look)
                    });
                if let Some(t) = texture {
                    p.textures.insert(2, Texture::File(t));
                }
                match attachments.get(i).or(attachments.first()) {
                    // A shield in the off hand.
                    Some(_) if *slot == 17 && self.is_shield(casc, *w) => p.attachment = Some(0),
                    Some(&a) => p.attachment = Some(a),
                    None => {
                        p.skinned = true;
                        for (k, &group) in slot_groups(*slot).iter().enumerate() {
                            let g = attachment_groups.get(k).copied().unwrap_or(0) as u16;
                            p.set_group(group, 1 + g);
                        }
                    }
                }
                parts.push(p);
            }
        }
        for (group, values) in group_values {
            let order = group_priority(group);
            let value = order
                .iter()
                .find_map(|s| values.iter().find(|(slot, _)| slot == s))
                .or(values.first())
                .map(|(_, v)| *v);
            if let Some(v) = value {
                body.set_group(group, v);
            }
        }
        // The skin: the bake, else the composite.
        if let Some(b) = look.bake {
            body.textures.insert(1, Texture::File(b));
        }
        for (kind, (w, h, layers)) in canvases {
            if look.bake.is_some() && kind == 1 {
                continue;
            }
            body.textures.insert(
                kind,
                Texture::Composite(Composite {
                    width: w,
                    height: h,
                    layers: layers.into_values().collect(),
                }),
            );
        }
        let mut all = vec![body];
        all.extend(parts);
        Some(Display {
            parts: all,
            scale: 1.0,
            alpha: 1.0,
        })
    }

    fn is_shield(&mut self, casc: &Casc, w: Worn) -> bool {
        let item = match w {
            Worn::Item(i) => i,
            Worn::Appearance(a) => match self
                .table(casc, "ItemModifiedAppearance")
                .and_then(|t| t.row(a))
            {
                Some(r) => r.int("ItemID"),
                None => return false,
            },
            Worn::Display(_) => return false,
        };
        self.table(casc, "Item")
            .and_then(|t| t.row(item))
            .is_some_and(|r| r.int("InventoryType") == 14)
    }

    /// A customization geoset's submesh ID.
    fn geoset(&mut self, casc: &Casc, id: u32) -> Option<u16> {
        let r = self.table(casc, "ChrCustomizationGeoset")?.row(id)?;
        Some((r.int("GeosetType") * 100 + r.int("GeosetID")) as u16)
    }

    /// The geosets a choice shows.
    fn choice_geosets(&mut self, casc: &Casc, choice: u32) -> Vec<u16> {
        let ids: Vec<u32> = self
            .table(casc, "ChrCustomizationElement")
            .map(|t| {
                t.rows_where("ChrCustomizationChoiceID", choice)
                    .iter()
                    .map(|r| r.int("ChrCustomizationGeosetID"))
                    .filter(|g| *g != 0)
                    .collect()
            })
            .unwrap_or_default();
        ids.into_iter()
            .filter_map(|g| self.geoset(casc, g))
            .collect()
    }
}
