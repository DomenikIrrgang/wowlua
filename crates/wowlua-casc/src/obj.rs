//! What stands on a terrain tile (decision T523): its objects file
//! (`_obj0`), the buildings (WMOs, `MODF`) and the doodads (M2 models,
//! `MDDF`) placed there.
//!
//! A placement's position is the files' own: `[0]` is 32 tiles less the
//! west coordinate, `[1]` the height, `[2]` 32 tiles less the north one;
//! its rotation is in degrees about those axes.

use crate::adt::TILE;

/// A building placed on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Building {
    /// Its root file (a file ID).
    pub file: u32,
    /// The same building on several tiles has one ID.
    pub unique: u32,
    /// Where (north, west, up).
    pub at: [f64; 3],
    /// Its turn, in degrees, as the file gives it.
    pub rotation: [f32; 3],
    pub scale: f32,
    pub doodad_set: u16,
    /// Its bounds on the map (min, max; north, west, up).
    pub bounds: [[f64; 3]; 2],
}

/// Turns a model's own place (x north, y west, z up in the model) into
/// the map's, by a placement's position (the file's), rotation (degrees)
/// and scale: the model's axes made the files' (x, z, -y), turned about
/// the files' up axis by the yaw less a quarter turn, then the others.
pub fn placed(at: [f64; 3], rotation: [f32; 3], scale: f32) -> impl Fn([f32; 3]) -> [f64; 3] {
    let r = |d: f32| f64::from(d).to_radians();
    let (yaw, pitch, roll) = (
        r(rotation[1]) - std::f64::consts::FRAC_PI_2,
        r(-rotation[0]),
        r(rotation[2]),
    );
    let edge = 32.0 * TILE;
    // (the placement's position in the files' axes)
    let p = [edge - at[1], at[2], edge - at[0]];
    move |v: [f32; 3]| {
        let s = f64::from(scale);
        let (x, y, z) = (
            f64::from(v[0]) * s,
            f64::from(v[2]) * s,
            -f64::from(v[1]) * s,
        );
        // about x (roll)
        let (y, z) = (
            y * roll.cos() - z * roll.sin(),
            y * roll.sin() + z * roll.cos(),
        );
        // about z (pitch)
        let (x, y) = (
            x * pitch.cos() - y * pitch.sin(),
            x * pitch.sin() + y * pitch.cos(),
        );
        // about y (yaw)
        let (x, z) = (
            x * yaw.cos() + z * yaw.sin(),
            -x * yaw.sin() + z * yaw.cos(),
        );
        let (px, py, pz) = (p[0] + x, p[1] + y, p[2] + z);
        [edge - pz, edge - px, py]
    }
}

/// A doodad (an M2 model) placed on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Doodad {
    pub file: u32,
    pub unique: u32,
    pub at: [f64; 3],
    pub rotation: [f32; 3],
    pub scale: f32,
}

/// A tile's objects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Objects {
    pub buildings: Vec<Building>,
    pub doodads: Vec<Doodad>,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .map_or(0, |x| u32::from_le_bytes(x.try_into().unwrap()))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2)
        .map_or(0, |x| u16::from_le_bytes(x.try_into().unwrap()))
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(b, at))
}

/// A placement's position as a place (north, west, up).
pub fn place_of(p: [f32; 3]) -> [f64; 3] {
    let edge = 32.0 * TILE;
    [
        edge - f64::from(p[2]),
        edge - f64::from(p[0]),
        f64::from(p[1]),
    ]
}

/// The names a file lists (`MWMO`, `MMDX`: strings one after another) by
/// their offsets (`MWID`, `MMID`).
fn named(names: &[u8], offsets: &[u8], i: u32) -> Option<String> {
    let at = u32_at(offsets, i as usize * 4) as usize;
    let rest = names.get(at..)?;
    let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// A tile's objects from its `_obj0` file: `by_path` gives the file ID of
/// a name the older files use instead of IDs.
pub fn read(data: &[u8], by_path: &dyn Fn(&str) -> Option<u32>) -> Objects {
    let mut out = Objects::default();
    let mut chunks: Vec<([u8; 4], &[u8])> = Vec::new();
    let mut o = 0;
    while o + 8 <= data.len() {
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&data[o..o + 4]);
        tag.reverse();
        let size = u32_at(data, o + 4) as usize;
        let end = (o + 8 + size).min(data.len());
        chunks.push((tag, &data[o + 8..end]));
        o = end;
    }
    let find = |t: &[u8; 4]| chunks.iter().find(|(c, _)| c == t).map(|(_, b)| *b);
    let (mwmo, mwid) = (find(b"MWMO").unwrap_or(&[]), find(b"MWID").unwrap_or(&[]));
    let (mmdx, mmid) = (find(b"MMDX").unwrap_or(&[]), find(b"MMID").unwrap_or(&[]));
    let vec3 = |b: &[u8], at: usize| [f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8)];
    if let Some(modf) = find(b"MODF") {
        for e in modf.as_chunks::<64>().0.iter() {
            let (name, flags) = (u32_at(e, 0), u16_at(e, 56));
            // (flag 8: the name is a file ID)
            let file = if flags & 0x8 != 0 {
                Some(name)
            } else {
                named(mwmo, mwid, name).and_then(|n| by_path(&n))
            };
            let Some(file) = file.filter(|f| *f != 0) else {
                continue;
            };
            // (flag 4: the scale is given, 1024 a whole)
            let scale = if flags & 0x4 != 0 {
                f32::from(u16_at(e, 62)) / 1024.0
            } else {
                1.0
            };
            out.buildings.push(Building {
                file,
                unique: u32_at(e, 4),
                at: place_of(vec3(e, 8)),
                rotation: vec3(e, 20),
                scale,
                doodad_set: u16_at(e, 58),
                bounds: {
                    let (a, b) = (place_of(vec3(e, 32)), place_of(vec3(e, 44)));
                    [
                        [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
                        [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
                    ]
                },
            });
        }
    }
    if let Some(mddf) = find(b"MDDF") {
        for e in mddf.as_chunks::<36>().0.iter() {
            let (name, flags) = (u32_at(e, 0), u16_at(e, 34));
            // (flag 0x40: the name is a file ID)
            let file = if flags & 0x40 != 0 {
                Some(name)
            } else {
                named(mmdx, mmid, name).and_then(|n| by_path(&n))
            };
            let Some(file) = file.filter(|f| *f != 0) else {
                continue;
            };
            out.doodads.push(Doodad {
                file,
                unique: u32_at(e, 4),
                at: place_of(vec3(e, 8)),
                rotation: vec3(e, 20),
                scale: f32::from(u16_at(e, 32)) / 1024.0,
            });
        }
    }
    out
}
