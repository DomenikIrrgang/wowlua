//! A building (decision T523): a WMO's root file (its materials, its
//! groups' files, its doodads) and its groups (each a mesh: vertices,
//! triangles drawn in batches of a material, and which triangles collide).
//!
//! Places inside are the building's own (x north, y west, z up); its
//! placement turns and moves them onto the map.

/// A building's root.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Root {
    pub materials: Vec<Material>,
    /// Its groups' files (file IDs).
    pub groups: Vec<u32>,
    /// Its bounds (min, max).
    pub bounds: [[f32; 3]; 2],
    /// Its doodad sets: each one's first doodad and how many.
    pub doodad_sets: Vec<(u32, u32)>,
    pub doodads: Vec<Doodad>,
    /// The ambient light inside (RGBA).
    pub ambient: [u8; 4],
}

/// How a material draws.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Material {
    /// Its flags: 0x1 unlit, 0x2 unfogged, 0x4 both sides, 0x10 the
    /// exterior's light, 0x40 clamped.
    pub flags: u32,
    pub shader: u32,
    /// 0 opaque, 1 alpha keyed, 2 alpha blended, 3 added, …
    pub blend: u32,
    /// Its textures (file IDs, 0 for none).
    pub textures: [u32; 3],
}

/// A doodad inside a building.
#[derive(Clone, Debug, PartialEq)]
pub struct Doodad {
    pub file: u32,
    pub at: [f32; 3],
    /// A quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub scale: f32,
}

impl Doodad {
    /// A place of its model in the building's own space: scaled, turned
    /// and moved where it stands.
    pub fn local(&self) -> impl Fn([f32; 3]) -> [f32; 3] + '_ {
        move |v: [f32; 3]| {
            let s = f64::from(self.scale);
            let v = [
                f64::from(v[0]) * s,
                f64::from(v[1]) * s,
                f64::from(v[2]) * s,
            ];
            let r = turned(self.rotation, v);
            [
                (f64::from(self.at[0]) + r[0]) as f32,
                (f64::from(self.at[1]) + r[1]) as f32,
                (f64::from(self.at[2]) + r[2]) as f32,
            ]
        }
    }
}

/// A vector turned by a quaternion (x, y, z, w).
pub fn turned(q: [f32; 4], v: [f64; 3]) -> [f64; 3] {
    let (x, y, z, w) = (
        f64::from(q[0]),
        f64::from(q[1]),
        f64::from(q[2]),
        f64::from(q[3]),
    );
    let c = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let u = [x, y, z];
    let t = c(u, v);
    let t = [t[0] + w * v[0], t[1] + w * v[1], t[2] + w * v[2]];
    let t = c(u, t);
    [v[0] + 2.0 * t[0], v[1] + 2.0 * t[1], v[2] + 2.0 * t[2]]
}

/// A group: its mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    /// Its flags (0x8: outdoors, 0x2000: indoors).
    pub flags: u32,
    /// Its bounds in the building (min, max).
    pub bounds: [[f32; 3]; 2],
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Its vertex colors (BGRA), none where it has none.
    pub colors: Vec<[u8; 4]>,
    pub indices: Vec<u16>,
    /// Each triangle's flags and material (none, 0xFFFF: collision
    /// only): `MOPY`'s bytes, or the newer files' `MPY2` words.
    pub triangles: Vec<(u16, u16)>,
    pub batches: Vec<Batch>,
    /// Its collision tree (`MOBN`), whose leaves list triangles (`MOBR`).
    pub bsp: Vec<BspNode>,
    pub bsp_faces: Vec<u16>,
}

/// A node of a group's collision tree: a plane across one axis (0 x, 1 y,
/// 2 z) at `dist`, its children on either side (-1: none), or a leaf of
/// triangles (`faces` from `first` of the tree's list).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BspNode {
    pub flags: u16,
    pub negative: i16,
    pub positive: i16,
    pub faces: u16,
    pub first: u32,
    pub dist: f32,
}

/// A batch: a run of indices drawn with one material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Batch {
    pub start: u32,
    pub count: u32,
    pub material: u16,
}

fn chunks(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut o = 0;
    while o + 8 <= data.len() {
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&data[o..o + 4]);
        tag.reverse();
        let size = u32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()) as usize;
        let end = (o + 8 + size).min(data.len());
        out.push((tag, &data[o + 8..end]));
        o = end;
    }
    out
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

fn vec3s(b: &[u8]) -> Vec<[f32; 3]> {
    b.as_chunks::<12>()
        .0
        .iter()
        .map(|v| [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)])
        .collect()
}

impl Root {
    /// A root file; `by_path` gives the file ID of a name (the older
    /// files' doodads and textures are named). None for a file that is not
    /// a WMO root.
    pub fn read(data: &[u8], by_path: &dyn Fn(&str) -> Option<u32>) -> Option<Root> {
        let chunks = chunks(data);
        let find = |t: &[u8; 4]| chunks.iter().find(|(c, _)| c == t).map(|(_, b)| *b);
        let mohd = find(b"MOHD")?;
        let mut root = Root {
            // (after seven counts: the ambient light, the ID, the bounds)
            bounds: [
                [f32_at(mohd, 36), f32_at(mohd, 40), f32_at(mohd, 44)],
                [f32_at(mohd, 48), f32_at(mohd, 52), f32_at(mohd, 56)],
            ],
            ambient: {
                let c = u32_at(mohd, 28).to_le_bytes();
                [c[2], c[1], c[0], c[3]]
            },
            ..Default::default()
        };
        let groups = u32_at(mohd, 4) as usize;
        let motx = find(b"MOTX");
        let texture = |v: u32| -> u32 {
            // (the older files: an offset into the names, `MOTX`)
            match motx {
                Some(names) if !names.is_empty() => {
                    let rest = names.get(v as usize..).unwrap_or(&[]);
                    let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
                    let name = String::from_utf8_lossy(&rest[..end]);
                    if name.is_empty() {
                        0
                    } else {
                        by_path(&name).unwrap_or(0)
                    }
                }
                _ => v,
            }
        };
        if let Some(momt) = find(b"MOMT") {
            root.materials = momt
                .as_chunks::<64>()
                .0
                .iter()
                .map(|m| Material {
                    flags: u32_at(m, 0),
                    shader: u32_at(m, 4),
                    blend: u32_at(m, 8),
                    textures: [
                        texture(u32_at(m, 12)),
                        texture(u32_at(m, 24)),
                        texture(u32_at(m, 36)),
                    ],
                })
                .collect();
        }
        if let Some(gfid) = find(b"GFID") {
            root.groups = gfid
                .as_chunks::<4>()
                .0
                .iter()
                .take(groups)
                .map(|b| u32_at(b, 0))
                .collect();
        }
        if let Some(mods) = find(b"MODS") {
            root.doodad_sets = mods
                .as_chunks::<32>()
                .0
                .iter()
                .map(|s| (u32_at(s, 20), u32_at(s, 24)))
                .collect();
        }
        let modi = find(b"MODI");
        let modn = find(b"MODN");
        if let Some(modd) = find(b"MODD") {
            for d in modd.as_chunks::<40>().0.iter() {
                let index = u32_at(d, 0) & 0x00FF_FFFF;
                let file = match (modi, modn) {
                    (Some(ids), _) => u32_at(ids, index as usize * 4),
                    (None, Some(names)) => {
                        let rest = names.get(index as usize..).unwrap_or(&[]);
                        let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
                        let name = String::from_utf8_lossy(&rest[..end]).to_ascii_lowercase();
                        // (the old `.mdx` names are the `.m2` files)
                        let name = match name.rsplit_once('.') {
                            Some((stem, "mdx" | "mdl")) => format!("{stem}.m2"),
                            _ => name,
                        };
                        by_path(&name).unwrap_or(0)
                    }
                    _ => 0,
                };
                root.doodads.push(Doodad {
                    file,
                    at: [f32_at(d, 4), f32_at(d, 8), f32_at(d, 12)],
                    rotation: [f32_at(d, 16), f32_at(d, 20), f32_at(d, 24), f32_at(d, 28)],
                    scale: f32_at(d, 32),
                });
            }
        }
        Some(root)
    }
}

impl Group {
    /// A group file; none for a file that is not one.
    pub fn read(data: &[u8]) -> Option<Group> {
        let (_, mogp) = chunks(data).into_iter().find(|(t, _)| t == b"MOGP")?;
        let header = 0x44;
        let mut g = Group {
            flags: u32_at(mogp, 8),
            bounds: [
                [f32_at(mogp, 12), f32_at(mogp, 16), f32_at(mogp, 20)],
                [f32_at(mogp, 24), f32_at(mogp, 28), f32_at(mogp, 32)],
            ],
            ..Default::default()
        };
        // (its own chunks follow its header, inside it)
        for (tag, b) in chunks(mogp.get(header..)?) {
            match &tag {
                b"MOPY" => {
                    g.triangles = b
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|t| {
                            let material = if t[1] == 0xFF {
                                0xFFFF
                            } else {
                                u16::from(t[1])
                            };
                            (u16::from(t[0]), material)
                        })
                        .collect()
                }
                b"MPY2" => {
                    g.triangles = b
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|t| (u16_at(t, 0), u16_at(t, 2)))
                        .collect()
                }
                b"MOBN" => {
                    g.bsp = b
                        .as_chunks::<16>()
                        .0
                        .iter()
                        .map(|n| BspNode {
                            flags: u16_at(n, 0),
                            negative: u16_at(n, 2) as i16,
                            positive: u16_at(n, 4) as i16,
                            faces: u16_at(n, 6),
                            first: u32_at(n, 8),
                            dist: f32_at(n, 12),
                        })
                        .collect()
                }
                b"MOBR" => {
                    g.bsp_faces = b.as_chunks::<2>().0.iter().map(|i| u16_at(i, 0)).collect()
                }
                b"MOVI" => g.indices = b.as_chunks::<2>().0.iter().map(|i| u16_at(i, 0)).collect(),
                b"MOVT" => g.positions = vec3s(b),
                b"MONR" => g.normals = vec3s(b),
                // (the first set of coordinates; later ones for blends)
                b"MOTV" if g.uvs.is_empty() => {
                    g.uvs = b
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|t| [f32_at(t, 0), f32_at(t, 4)])
                        .collect()
                }
                b"MOCV" if g.colors.is_empty() => {
                    g.colors = b
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| [c[0], c[1], c[2], c[3]])
                        .collect()
                }
                b"MOBA" => {
                    g.batches = b
                        .as_chunks::<24>()
                        .0
                        .iter()
                        .map(|m| {
                            // (flag 2: a material ID too large for a byte,
                            // in place of the bounds' last two bytes)
                            let large = m[22] & 0x2 != 0;
                            Batch {
                                start: u32_at(m, 12),
                                count: u32::from(u16_at(m, 16)),
                                material: if large {
                                    u16_at(m, 10)
                                } else {
                                    u16::from(m[23])
                                },
                            }
                        })
                        .collect()
                }
                _ => {}
            }
        }
        Some(g)
    }

    /// Whether a triangle (by its index) collides: not detail, not one
    /// only drawn (`MOPY` 0x4 detail, 0x20 render without collision).
    pub fn collides(&self, triangle: usize) -> bool {
        let (flags, _) = self.triangles.get(triangle).copied().unwrap_or((0x20, 0));
        // (collision, or drawn and not detail)
        flags & 0x08 != 0 || flags & 0x20 != 0 && flags & 0x04 == 0
    }

    /// The triangles near a box of the group (min, max): its collision
    /// tree's leaves the box reaches, else every triangle.
    pub fn near(&self, lo: [f64; 3], hi: [f64; 3], out: &mut Vec<usize>) {
        out.clear();
        if self.bsp.is_empty() {
            out.extend(0..self.indices.len() / 3);
            return;
        }
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let Some(n) = self.bsp.get(i) else {
                continue;
            };
            if n.flags & 0x4 != 0 {
                let from = n.first as usize;
                for f in self.bsp_faces.iter().skip(from).take(n.faces as usize) {
                    out.push(*f as usize);
                }
                continue;
            }
            let axis = (n.flags & 0x3) as usize;
            let d = f64::from(n.dist);
            if axis < 3 {
                if lo[axis] <= d && n.negative >= 0 {
                    stack.push(n.negative as usize);
                }
                if hi[axis] >= d && n.positive >= 0 {
                    stack.push(n.positive as usize);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}
