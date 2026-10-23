//! The client's texture atlases (spec debugger §18.10): each atlas's file
//! and its region of it, from the build's own tables, so they match the
//! build's files.
//!
//! A name the UI code uses (`UI-HUD-UnitFrame-SmallCircle`) is an element
//! (`UiTextureAtlasElement`); its members (`UiTextureAtlasMember`) are
//! variants in atlases (`UiTextureAtlas`) of a set (0 the default, others
//! a flavor's own: camelot's `-c60`) and a canvas (1 for 1x, 2 for 2x). A
//! name is looked up as an element, its variant the flavor's set's over
//! the default's, at 1x; or as a member's own name.

use std::collections::HashMap;

use crate::Casc;
use crate::db2::Db2;

/// The tables' file IDs (the same in every build).
const ATLAS_TABLE: u32 = 897470;
const MEMBER_TABLE: u32 = 897532;
const ELEMENT_TABLE: u32 = 1989276;
const SLICE_TABLE: u32 = 5319042;

/// An atlas: its file, its region (left, right, top, bottom, 0 to 1), its
/// size in the file's pixels, whether it tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atlas {
    pub file: u32,
    pub coords: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub tiles_horizontally: bool,
    pub tiles_vertically: bool,
    /// Its nine-slice margins in its pixels (left, top, right, bottom) and
    /// how the middle fills (0 stretched, 1 tiled), where the build's
    /// `UiTextureAtlasElementSliceData` gives them.
    pub slice: Option<([u32; 4], u8)>,
}

/// An atlas file's row: file, width, height, set, canvas.
struct AtlasFile {
    file: u32,
    width: u32,
    height: u32,
    set: u32,
    canvas: u32,
}

/// Every atlas of the build, by name in lower case (elements' and
/// members' names); none when the tables cannot be read (encrypted, or a
/// layout this does not know).
pub fn read(casc: &Casc) -> HashMap<String, Atlas> {
    let mut out = HashMap::new();
    let (Some(a), Some(m)) = (
        casc.file_by_id(ATLAS_TABLE).and_then(Db2::parse),
        casc.file_by_id(MEMBER_TABLE).and_then(Db2::parse),
    ) else {
        return out;
    };
    // The atlas table: an inline ID (its first field) or none; the file;
    // a set (newer builds); the width and height; the canvas.
    let inline_id = !a.is_empty() && a.int(0, 0) == Some(a.id(0));
    let base = usize::from(inline_id);
    let rest = a.field_count() - base;
    let (file_f, set_f, w_f, h_f, canvas_f) = if rest >= 5 {
        (base, Some(base + 1), base + 2, base + 3, Some(base + 4))
    } else {
        (
            base,
            None,
            base + 1,
            base + 2,
            (rest >= 4).then_some(base + 3),
        )
    };
    let mut files: HashMap<u32, AtlasFile> = HashMap::new();
    for i in 0..a.len() {
        if let (Some(file), Some(width), Some(height)) =
            (a.int(i, file_f), a.int(i, w_f), a.int(i, h_f))
        {
            files.insert(
                a.id(i),
                AtlasFile {
                    file,
                    width,
                    height,
                    set: set_f.and_then(|f| a.int(i, f)).unwrap_or(0),
                    canvas: canvas_f.and_then(|f| a.int(i, f)).unwrap_or(1),
                },
            );
        }
    }
    // Members: name, ID, atlas, (width, height in newer builds), left,
    // right, top, bottom, the element, …, flags.
    let (left_f, element_f, flags_f) = match m.field_count() {
        14 => (5, 9, 12),
        12 => (3, 7, 10),
        _ => return out,
    };
    let elements: HashMap<u32, String> = casc
        .file_by_id(ELEMENT_TABLE)
        .and_then(Db2::parse)
        .map(|e| {
            (0..e.len())
                .filter_map(|i| Some((e.id(i), e.string(i, 0)?.to_ascii_lowercase())))
                .collect()
        })
        .unwrap_or_default();
    // Each element's best variant so far: (its rank, the atlas).
    let mut best: HashMap<u32, ((bool, bool), Atlas)> = HashMap::new();
    for i in 0..m.len() {
        let Some(af) = m.int(i, 2).and_then(|id| files.get(&id)) else {
            continue;
        };
        if af.width == 0 || af.height == 0 {
            continue;
        }
        let px = |f: usize| m.int(i, f).map(|v| f64::from(v as u16));
        let (Some(l), Some(r), Some(t), Some(b)) =
            (px(left_f), px(left_f + 1), px(left_f + 2), px(left_f + 3))
        else {
            continue;
        };
        let flags = m.int(i, flags_f).unwrap_or(0);
        let no_tile = flags & 0x10 != 0;
        let atlas = Atlas {
            file: af.file,
            coords: [
                (l / f64::from(af.width)) as f32,
                (r / f64::from(af.width)) as f32,
                (t / f64::from(af.height)) as f32,
                (b / f64::from(af.height)) as f32,
            ],
            width: (r - l).max(0.0) as u32,
            height: (b - t).max(0.0) as u32,
            tiles_horizontally: !no_tile && flags & 4 != 0,
            tiles_vertically: !no_tile && flags & 2 != 0,
            slice: None,
        };
        if let Some(name) = m.string(i, 0).filter(|n| !n.is_empty()) {
            out.insert(name.to_ascii_lowercase(), atlas);
        }
        if let Some(element) = m.int(i, element_f) {
            // The flavor's own set first, then 1x.
            let rank = (af.set != 0, af.canvas == 1);
            match best.get(&element) {
                Some((r, _)) if *r >= rank => {}
                _ => {
                    best.insert(element, (rank, atlas));
                }
            }
        }
    }
    // Slice margins by element: (ID,) element, left, top, right, bottom,
    // mode.
    let slices: HashMap<u32, ([u32; 4], u8)> = casc
        .file_by_id(SLICE_TABLE)
        .and_then(Db2::parse)
        .map(|t| {
            let base = usize::from(t.field_count() >= 7);
            (0..t.len())
                .filter_map(|i| {
                    let f = |k: usize| t.int(i, base + k);
                    Some((
                        f(0)?,
                        ([f(1)?, f(2)?, f(3)?, f(4)?], f(5).unwrap_or(0) as u8),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    for (element, (_, mut atlas)) in best {
        atlas.slice = slices.get(&element).copied();
        if let Some(name) = elements.get(&element) {
            out.insert(name.clone(), atlas);
        }
    }
    out
}
