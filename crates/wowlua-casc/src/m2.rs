//! The client's 3D models (M2, the chunked `MD21` form): the mesh, its
//! textures and materials, bones and their animation tracks, cameras and
//! attachment points; and the skin profiles (`.skin`) that cut the mesh
//! into submeshes and batches (spec debugger §18.10).
//!
//! What a sequence animates may live in the model or, for most sequences
//! of big models, in an `.anim` file of its own: [`Model::needs_anim`]
//! names it and [`Model::load_anim`] reads it in.

use std::collections::HashMap;

pub type Vec3 = [f32; 3];
pub type Quat = [f32; 4];

/// A mesh vertex.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vertex {
    pub position: Vec3,
    pub weights: [u8; 4],
    pub bones: [u8; 4],
    pub normal: Vec3,
    pub uv: [f32; 2],
    pub uv2: [f32; 2],
}

/// A texture the model uses: a file of its own (kind 0, the file from the
/// model's `TXID` chunk) or one the display gives (1 the character's skin,
/// 2 an item's, 6 hair, 11 to 13 a creature's skins, …).
#[derive(Clone, Copy, Debug)]
pub struct Texture {
    pub kind: u32,
    pub flags: u32,
    pub file: u32,
}

/// How a batch is drawn: its flags (1 unlit, 4 two-sided, 0x10 no depth
/// write) and blend mode (0 opaque, 1 alpha key, 2 alpha, 3 no-alpha add,
/// 4 add, 5 mod, 6 mod 2x, 7 blend add).
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub flags: u16,
    pub blend: u16,
}

/// An animation sequence.
#[derive(Clone, Copy, Debug)]
pub struct Sequence {
    /// The animation (`AnimationData`: 0 stand, 4 walk, …).
    pub id: u16,
    pub variation: u16,
    pub duration: u32,
    pub flags: u32,
    /// The sequence whose data this one uses (flag 0x40), else none.
    pub alias: Option<u16>,
}

impl Sequence {
    /// Whether its data is in the model itself.
    pub fn in_model(&self) -> bool {
        self.flags & 0x20 != 0
    }
}

/// An animated value's keys in one sequence.
#[derive(Clone, Debug, Default)]
pub struct Keys<T> {
    pub times: Vec<u32>,
    pub values: Vec<T>,
}

/// An animated value: its keys per sequence (or one list, for a global
/// sequence).
#[derive(Clone, Debug)]
pub struct Track<T> {
    /// 0 none, 1 linear, 2 bezier, 3 hermite.
    pub interpolation: u16,
    pub global: Option<u16>,
    pub keys: Vec<Keys<T>>,
    /// Where each sequence's keys are, when they are in an `.anim` file:
    /// (timestamp count, offset, value count, offset).
    pending: Vec<Option<[u32; 4]>>,
}

impl<T> Default for Track<T> {
    fn default() -> Self {
        Track {
            interpolation: 0,
            global: None,
            keys: Vec::new(),
            pending: Vec::new(),
        }
    }
}

/// A bone.
#[derive(Clone, Debug, Default)]
pub struct Bone {
    pub key: i32,
    pub flags: u32,
    pub parent: Option<u16>,
    pub translation: Track<Vec3>,
    pub rotation: Track<Quat>,
    pub scale: Track<Vec3>,
    pub pivot: Vec3,
}

/// A camera (type 0 the portrait's, 1 the character info's).
#[derive(Clone, Debug)]
pub struct Camera {
    pub kind: i32,
    pub far: f32,
    pub near: f32,
    pub position: Vec3,
    pub target: Vec3,
    /// The diagonal field of view, in radians.
    pub fov: f32,
    pub roll: f32,
}

/// An attachment point (0 shield, 1 right hand, 2 left hand, 11 helm, …).
#[derive(Clone, Copy, Debug)]
pub struct Attachment {
    pub id: u32,
    pub bone: u16,
    pub position: Vec3,
}

/// A color's animated values.
#[derive(Clone, Debug, Default)]
pub struct Color {
    pub color: Track<Vec3>,
    pub alpha: Track<f32>,
}

/// A texture transform's animated values.
#[derive(Clone, Debug, Default)]
pub struct TextureTransform {
    pub translation: Track<Vec3>,
    pub rotation: Track<Quat>,
    pub scale: Track<Vec3>,
}

/// An `.anim` file a sequence's data is in.
#[derive(Clone, Copy, Debug)]
pub struct AnimFile {
    pub animation: u16,
    pub variation: u16,
    pub file: u32,
}

/// A model.
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub flags: u32,
    pub global_loops: Vec<u32>,
    pub sequences: Vec<Sequence>,
    /// The first sequence of each animation ID.
    pub sequence_of: HashMap<u16, usize>,
    pub bones: Vec<Bone>,
    /// Key bones (`key_bone_lookup`): 0 arm L, 1 arm R, …, 26 head.
    pub key_bones: Vec<i16>,
    pub vertices: Vec<Vertex>,
    pub colors: Vec<Color>,
    pub textures: Vec<Texture>,
    pub texture_weights: Vec<Track<f32>>,
    pub texture_transforms: Vec<TextureTransform>,
    /// Texture kinds to their index (`replaceable_texture_lookup`).
    pub replaceable: Vec<i16>,
    pub materials: Vec<Material>,
    pub bone_combos: Vec<u16>,
    pub texture_combos: Vec<u16>,
    pub texture_coord_combos: Vec<u16>,
    pub texture_weight_combos: Vec<u16>,
    pub texture_transform_combos: Vec<u16>,
    pub bounds: [Vec3; 2],
    pub radius: f32,
    pub attachments: Vec<Attachment>,
    /// Attachment IDs to their index.
    pub attachment_of: Vec<i16>,
    pub cameras: Vec<Camera>,
    pub skin_files: Vec<u32>,
    pub anim_files: Vec<AnimFile>,
    pub bone_files: Vec<u32>,
    /// The `.skel` file bones and sequences come from (newer character
    /// models), else none.
    pub skeleton_file: Option<u32>,
    pub skin_count: u32,
}

/// A part of a skin: a geoset (its ID is the group times 100 plus the
/// variant) and its triangles.
#[derive(Clone, Copy, Debug)]
pub struct Submesh {
    pub id: u16,
    pub index_start: u32,
    pub index_count: u32,
    pub bone_combo_start: u16,
    pub bone_count: u16,
    pub center: Vec3,
}

/// A batch: a submesh drawn with a material and textures.
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    pub flags: u8,
    pub priority: i8,
    pub shader: u16,
    pub submesh: u16,
    pub color: i16,
    pub material: u16,
    pub layer: u16,
    pub texture_count: u16,
    pub texture_combo: u16,
    pub texture_coord_combo: u16,
    pub texture_weight_combo: u16,
    pub texture_transform_combo: u16,
}

/// A skin profile: the triangles (as indices of the model's vertices),
/// its submeshes and batches.
#[derive(Clone, Debug, Default)]
pub struct Skin {
    pub indices: Vec<u32>,
    pub submeshes: Vec<Submesh>,
    pub batches: Vec<Batch>,
}

/// Little-endian reads that fail softly.
struct R<'a>(&'a [u8]);

impl R<'_> {
    fn u8(&self, at: usize) -> Option<u8> {
        self.0.get(at).copied()
    }
    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.0.get(at..at + 2)?.try_into().ok()?))
    }
    fn i16(&self, at: usize) -> Option<i16> {
        self.u16(at).map(|v| v as i16)
    }
    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.0.get(at..at + 4)?.try_into().ok()?))
    }
    fn i32(&self, at: usize) -> Option<i32> {
        self.u32(at).map(|v| v as i32)
    }
    fn f32(&self, at: usize) -> Option<f32> {
        self.u32(at).map(f32::from_bits)
    }
    fn vec3(&self, at: usize) -> Option<Vec3> {
        Some([self.f32(at)?, self.f32(at + 4)?, self.f32(at + 8)?])
    }
    /// An `M2Array`'s count and offset.
    fn array(&self, at: usize) -> (usize, usize) {
        (
            self.u32(at).unwrap_or(0) as usize,
            self.u32(at + 4).unwrap_or(0) as usize,
        )
    }
    /// An array of `size`-byte elements, each read by `f`, as far as the
    /// data holds.
    fn list<T>(&self, at: usize, size: usize, f: impl Fn(&Self, usize) -> Option<T>) -> Vec<T> {
        let (count, offset) = self.array(at);
        if offset.saturating_add(count.saturating_mul(size)) > self.0.len() {
            return Vec::new();
        }
        (0..count)
            .filter_map(|i| f(self, offset + i * size))
            .collect()
    }
    fn u16s(&self, at: usize) -> Vec<u16> {
        self.list(at, 2, |r, o| r.u16(o))
    }
    fn i16s(&self, at: usize) -> Vec<i16> {
        self.list(at, 2, |r, o| r.i16(o))
    }
}

/// A compressed quaternion's component.
fn unpack_quat(v: i16) -> f32 {
    let v = i32::from(v);
    (if v < 0 { v + 32768 } else { v - 32767 }) as f32 / 32767.0
}

/// A value an animated track holds: its size and how it reads.
trait Value: Sized + Clone + Default {
    const SIZE: usize;
    fn read(r: &R, at: usize) -> Option<Self>;
}

impl Value for Vec3 {
    const SIZE: usize = 12;
    fn read(r: &R, at: usize) -> Option<Self> {
        r.vec3(at)
    }
}

impl Value for Quat {
    const SIZE: usize = 8;
    fn read(r: &R, at: usize) -> Option<Self> {
        Some([
            unpack_quat(r.i16(at)?),
            unpack_quat(r.i16(at + 2)?),
            unpack_quat(r.i16(at + 4)?),
            unpack_quat(r.i16(at + 6)?),
        ])
    }
}

/// A fixed-point fraction (alpha, weight).
impl Value for f32 {
    const SIZE: usize = 2;
    fn read(r: &R, at: usize) -> Option<Self> {
        Some(f32::from(r.i16(at)?) / 32767.0)
    }
}

/// A track's keys at `(count, offset)` pairs.
fn keys_at<T: Value>(r: &R, k: [u32; 4]) -> Option<Keys<T>> {
    let [tc, to, vc, vo] = k.map(|v| v as usize);
    if to + tc * 4 > r.0.len() || vo + vc * T::SIZE > r.0.len() {
        return None;
    }
    Some(Keys {
        times: (0..tc).filter_map(|i| r.u32(to + i * 4)).collect(),
        values: (0..vc)
            .filter_map(|i| T::read(r, vo + i * T::SIZE))
            .collect(),
    })
}

/// A track (`M2Track<T>`, 20 bytes): each sequence's keys read where they
/// are in the model, the others left for their `.anim` file.
fn track<T: Value>(r: &R, at: usize, sequences: &[Sequence]) -> Track<T> {
    let interpolation = r.u16(at).unwrap_or(0);
    let global = r.i16(at + 2).filter(|g| *g >= 0).map(|g| g as u16);
    let (tn, to) = r.array(at + 4);
    let (vn, vo) = r.array(at + 12);
    let n = tn.min(vn);
    let mut keys = Vec::with_capacity(n);
    let mut pending = Vec::with_capacity(n);
    for s in 0..n {
        let (Some(tc), Some(tp), Some(vc), Some(vp)) = (
            r.u32(to + s * 8),
            r.u32(to + s * 8 + 4),
            r.u32(vo + s * 8),
            r.u32(vo + s * 8 + 4),
        ) else {
            break;
        };
        let k = [tc, tp, vc, vp];
        let here = global.is_some() || sequences.get(s).is_none_or(Sequence::in_model);
        if here {
            keys.push(keys_at(r, k).unwrap_or_default());
            pending.push(None);
        } else {
            keys.push(Keys::default());
            pending.push((tc > 0).then_some(k));
        }
    }
    Track {
        interpolation,
        global,
        keys,
        pending,
    }
}

/// A camera track (`M2Track<M2SplineKey<T>>`): its first key's value.
fn first_spline<T>(
    r: &R,
    at: usize,
    size: usize,
    read: impl Fn(&R, usize) -> Option<T>,
) -> Option<T> {
    let (vn, vo) = r.array(at + 12);
    if vn == 0 {
        return None;
    }
    let (count, offset) = (r.u32(vo)?, r.u32(vo + 4)? as usize);
    if count == 0 || offset + size > r.0.len() {
        return None;
    }
    read(r, offset)
}

fn sequence(r: &R, o: usize) -> Option<Sequence> {
    let flags = r.u32(o + 12)?;
    let alias_next = r.u16(o + 62)?;
    Some(Sequence {
        id: r.u16(o)?,
        variation: r.u16(o + 2)?,
        duration: r.u32(o + 4)?,
        flags,
        alias: (flags & 0x40 != 0).then_some(alias_next),
    })
}

fn bone(r: &R, o: usize, sequences: &[Sequence]) -> Option<Bone> {
    let parent = r.i16(o + 8)?;
    Some(Bone {
        key: r.i32(o)?,
        flags: r.u32(o + 4)?,
        parent: (parent >= 0).then_some(parent as u16),
        translation: track(r, o + 16, sequences),
        rotation: track(r, o + 36, sequences),
        scale: track(r, o + 56, sequences),
        pivot: r.vec3(o + 76)?,
    })
}

/// The chunks of a chunked file: (tag, body).
fn chunks(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 8 <= data.len() {
        let tag: [u8; 4] = data[p..p + 4].try_into().unwrap();
        let n = u32::from_le_bytes(data[p + 4..p + 8].try_into().unwrap()) as usize;
        let end = (p + 8).saturating_add(n).min(data.len());
        out.push((tag, &data[p + 8..end]));
        p = end;
    }
    out
}

fn u32s(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect()
}

impl Model {
    /// Reads a model file (`MD21` chunked, or a bare `MD20`).
    pub fn parse(data: &[u8]) -> Option<Model> {
        let mut md20: Option<&[u8]> = None;
        let mut m = Model::default();
        if data.starts_with(b"MD20") {
            md20 = Some(data);
        } else {
            for (tag, body) in chunks(data) {
                match &tag {
                    b"MD21" => md20 = Some(body),
                    b"SFID" => m.skin_files = u32s(body),
                    b"TXID" => {
                        let files = u32s(body);
                        m.textures = files
                            .iter()
                            .map(|&file| Texture {
                                kind: 0,
                                flags: 0,
                                file,
                            })
                            .collect();
                    }
                    b"AFID" => {
                        m.anim_files = body
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|c| AnimFile {
                                animation: u16::from_le_bytes([c[0], c[1]]),
                                variation: u16::from_le_bytes([c[2], c[3]]),
                                file: u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                            })
                            .collect()
                    }
                    b"BFID" => m.bone_files = u32s(body),
                    b"SKID" => m.skeleton_file = u32s(body).first().copied().filter(|f| *f != 0),
                    _ => {}
                }
            }
        }
        let d = md20?;
        if !d.starts_with(b"MD20") {
            return None;
        }
        let r = R(d);
        let (n, o) = r.array(8);
        m.name = d
            .get(o..o + n)
            .map(|b| String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or(b)).into_owned())
            .unwrap_or_default();
        m.flags = r.u32(16)?;
        m.global_loops = r.list(20, 4, |r, o| r.u32(o));
        m.sequences = r.list(28, 64, sequence);
        for (i, s) in m.sequences.iter().enumerate() {
            m.sequence_of.entry(s.id).or_insert(i);
        }
        let seqs = m.sequences.clone();
        m.bones = r.list(44, 88, |r, o| bone(r, o, &seqs));
        m.key_bones = r.i16s(52);
        m.vertices = r.list(60, 48, |r, o| {
            Some(Vertex {
                position: r.vec3(o)?,
                weights: [r.u8(o + 12)?, r.u8(o + 13)?, r.u8(o + 14)?, r.u8(o + 15)?],
                bones: [r.u8(o + 16)?, r.u8(o + 17)?, r.u8(o + 18)?, r.u8(o + 19)?],
                normal: r.vec3(o + 20)?,
                uv: [r.f32(o + 32)?, r.f32(o + 36)?],
                uv2: [r.f32(o + 40)?, r.f32(o + 44)?],
            })
        });
        m.skin_count = r.u32(68)?;
        m.colors = r.list(72, 40, |r, o| {
            Some(Color {
                color: track(r, o, &seqs),
                alpha: track(r, o + 20, &seqs),
            })
        });
        let textures: Vec<Texture> = r.list(80, 16, |r, o| {
            Some(Texture {
                kind: r.u32(o)?,
                flags: r.u32(o + 4)?,
                file: 0,
            })
        });
        // The files are the `TXID` chunk's, by index.
        m.textures = textures
            .into_iter()
            .enumerate()
            .map(|(i, t)| Texture {
                file: if t.kind == 0 {
                    m.textures.get(i).map_or(0, |f| f.file)
                } else {
                    0
                },
                ..t
            })
            .collect();
        m.texture_weights = r.list(88, 20, |r, o| Some(track(r, o, &seqs)));
        m.texture_transforms = r.list(96, 60, |r, o| {
            Some(TextureTransform {
                translation: track(r, o, &seqs),
                rotation: track(r, o + 20, &seqs),
                scale: track(r, o + 40, &seqs),
            })
        });
        m.replaceable = r.i16s(104);
        m.materials = r.list(112, 4, |r, o| {
            Some(Material {
                flags: r.u16(o)?,
                blend: r.u16(o + 2)?,
            })
        });
        m.bone_combos = r.u16s(120);
        m.texture_combos = r.u16s(128);
        m.texture_coord_combos = r.u16s(136);
        m.texture_weight_combos = r.u16s(144);
        m.texture_transform_combos = r.u16s(152);
        m.bounds = [r.vec3(160)?, r.vec3(172)?];
        m.radius = r.f32(184)?;
        m.attachments = r.list(240, 40, |r, o| {
            Some(Attachment {
                id: r.u32(o)?,
                bone: r.u16(o + 4)?,
                position: r.vec3(o + 8)?,
            })
        });
        m.attachment_of = r.i16s(248);
        m.cameras = r.list(272, 116, |r, o| {
            let pos = first_spline(r, o + 12, 12, |r, at| r.vec3(at)).unwrap_or_default();
            let base = r.vec3(o + 32)?;
            let target = first_spline(r, o + 44, 12, |r, at| r.vec3(at)).unwrap_or_default();
            let target_base = r.vec3(o + 64)?;
            let roll = first_spline(r, o + 76, 4, |r, at| r.f32(at)).unwrap_or(0.0);
            let fov = first_spline(r, o + 96, 4, |r, at| r.f32(at)).unwrap_or(0.7);
            Some(Camera {
                kind: r.i32(o)?,
                far: r.f32(o + 4)?,
                near: r.f32(o + 8)?,
                position: [pos[0] + base[0], pos[1] + base[1], pos[2] + base[2]],
                target: [
                    target[0] + target_base[0],
                    target[1] + target_base[1],
                    target[2] + target_base[2],
                ],
                fov,
                roll,
            })
        });
        Some(m)
    }

    /// The model's camera of `kind` (0 the portrait's).
    pub fn camera(&self, kind: i32) -> Option<&Camera> {
        self.cameras.iter().find(|c| c.kind == kind)
    }

    /// A sequence whose data is elsewhere: the `.anim` file to read for
    /// it, if one is listed and it is not read yet.
    pub fn needs_anim(&self, sequence: usize) -> Option<u32> {
        let s = self.sequences.get(sequence)?;
        if s.in_model() {
            return None;
        }
        let waiting = self.bones.iter().any(|b| {
            b.rotation
                .pending
                .get(sequence)
                .is_some_and(Option::is_some)
        });
        if !waiting {
            return None;
        }
        self.anim_files
            .iter()
            .find(|a| a.animation == s.id && a.variation == s.variation)
            .map(|a| a.file)
    }

    /// Reads a sequence's data from its `.anim` file (chunked: `AFM2`;
    /// else the bare data).
    pub fn load_anim(&mut self, sequence: usize, data: &[u8]) {
        let body = if data.len() >= 8 && matches!(&data[..4], b"AFM2" | b"AFSA" | b"AFSB") {
            // A model's own sequence data (`AFM2`), or a skeleton's bones'
            // (`AFSB`).
            let parts = chunks(data);
            parts
                .iter()
                .find(|(t, _)| t == b"AFM2")
                .or_else(|| parts.iter().find(|(t, _)| t == b"AFSB"))
                .map_or(&data[..0], |(_, b)| *b)
        } else {
            data
        };
        let r = R(body);
        fn fill<T: Value>(t: &mut Track<T>, s: usize, r: &R) {
            if let Some(Some(k)) = t.pending.get(s).copied() {
                if let Some(keys) = keys_at(r, k) {
                    t.keys[s] = keys;
                }
                t.pending[s] = None;
            }
        }
        for b in &mut self.bones {
            fill(&mut b.translation, sequence, &r);
            fill(&mut b.rotation, sequence, &r);
            fill(&mut b.scale, sequence, &r);
        }
        for c in &mut self.colors {
            fill(&mut c.color, sequence, &r);
            fill(&mut c.alpha, sequence, &r);
        }
        for w in &mut self.texture_weights {
            fill(w, sequence, &r);
        }
        for t in &mut self.texture_transforms {
            fill(&mut t.translation, sequence, &r);
            fill(&mut t.rotation, sequence, &r);
            fill(&mut t.scale, sequence, &r);
        }
    }

    /// Takes the bones and sequences of a skeleton file (`.skel`: `SKB1`
    /// the bones, `SKS1` the sequences, `SKA1` the attachments, `AFID` its
    /// `.anim` files), as newer character models keep them.
    pub fn load_skeleton(&mut self, data: &[u8]) {
        let parts = chunks(data);
        for (tag, body) in &parts {
            if tag == b"AFID" {
                self.anim_files = body
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| AnimFile {
                        animation: u16::from_le_bytes([c[0], c[1]]),
                        variation: u16::from_le_bytes([c[2], c[3]]),
                        file: u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                    })
                    .collect();
            }
        }
        if let Some((_, body)) = parts.iter().find(|(t, _)| t == b"SKS1") {
            let r = R(body);
            self.global_loops = r.list(0, 4, |r, o| r.u32(o));
            self.sequences = r.list(8, 64, sequence);
            self.sequence_of.clear();
            for (i, s) in self.sequences.iter().enumerate() {
                self.sequence_of.entry(s.id).or_insert(i);
            }
        }
        let seqs = self.sequences.clone();
        if let Some((_, body)) = parts.iter().find(|(t, _)| t == b"SKB1") {
            let r = R(body);
            self.bones = r.list(0, 88, |r, o| bone(r, o, &seqs));
            self.key_bones = r.i16s(8);
        }
        if let Some((_, body)) = parts.iter().find(|(t, _)| t == b"SKA1") {
            let r = R(body);
            self.attachments = r.list(0, 40, |r, o| {
                Some(Attachment {
                    id: r.u32(o)?,
                    bone: r.u16(o + 4)?,
                    position: r.vec3(o + 8)?,
                })
            });
            self.attachment_of = r.i16s(8);
        }
    }

    /// The skeleton file this one's bones come from, if it names a parent
    /// (`SKPD`).
    pub fn skeleton_parent(data: &[u8]) -> Option<u32> {
        chunks(data)
            .into_iter()
            .find(|(t, _)| t == b"SKPD")
            .and_then(|(_, b)| R(b).u32(8))
            .filter(|f| *f != 0)
    }
}

impl Skin {
    /// Reads a skin profile (`SKIN`).
    pub fn parse(data: &[u8]) -> Option<Skin> {
        if !data.starts_with(b"SKIN") {
            return None;
        }
        let r = R(data);
        let lookup = r.u16s(4);
        let local = r.u16s(12);
        let indices = local
            .iter()
            .map(|&i| u32::from(lookup.get(i as usize).copied().unwrap_or(0)))
            .collect();
        let submeshes = r.list(28, 48, |r, o| {
            let level = u32::from(r.u16(o + 2)?);
            Some(Submesh {
                id: r.u16(o)?,
                index_start: u32::from(r.u16(o + 8)?) + (level << 16),
                index_count: u32::from(r.u16(o + 10)?),
                bone_count: r.u16(o + 12)?,
                bone_combo_start: r.u16(o + 14)?,
                center: r.vec3(o + 20)?,
            })
        });
        let batches = r.list(36, 24, |r, o| {
            Some(Batch {
                flags: r.u8(o)?,
                priority: r.u8(o + 1)? as i8,
                shader: r.u16(o + 2)?,
                submesh: r.u16(o + 4)?,
                color: r.i16(o + 8)?,
                material: r.u16(o + 10)?,
                layer: r.u16(o + 12)?,
                texture_count: r.u16(o + 14)?,
                texture_combo: r.u16(o + 16)?,
                texture_coord_combo: r.u16(o + 18)?,
                texture_weight_combo: r.u16(o + 20)?,
                texture_transform_combo: r.u16(o + 22)?,
            })
        });
        Some(Skin {
            indices,
            submeshes,
            batches,
        })
    }
}

/// A 4×4 matrix, columns first.
pub type Mat4 = [f32; 16];

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            o[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    o
}

pub fn translation(t: Vec3) -> Mat4 {
    let mut m = IDENTITY;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

/// A rotation by a quaternion (x, y, z, w).
pub fn rotation(q: Quat) -> Mat4 {
    let [x, y, z, w] = q;
    [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        0.0,
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        0.0,
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

pub fn scaling(s: Vec3) -> Mat4 {
    let mut m = IDENTITY;
    m[0] = s[0];
    m[5] = s[1];
    m[10] = s[2];
    m
}

/// A point through a matrix.
pub fn transform(m: &Mat4, p: Vec3) -> Vec3 {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

/// A direction through a matrix (no translation).
pub fn transform_dir(m: &Mat4, p: Vec3) -> Vec3 {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2],
    ]
}

/// What a track's values blend as.
pub trait Blend: Copy {
    fn blend(a: Self, b: Self, t: f32) -> Self;
}

impl Blend for Vec3 {
    fn blend(a: Self, b: Self, t: f32) -> Self {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    }
}

impl Blend for f32 {
    fn blend(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }
}

impl Blend for Quat {
    /// Normalized linear blending, the shorter way round.
    fn blend(a: Self, b: Self, t: f32) -> Self {
        let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
        let s = if dot < 0.0 { -1.0 } else { 1.0 };
        let mut q = [0.0; 4];
        for i in 0..4 {
            q[i] = a[i] * (1.0 - t) + b[i] * s * t;
        }
        let n = (q.iter().map(|v| v * v).sum::<f32>()).sqrt().max(1e-6);
        q.map(|v| v / n)
    }
}

impl<T: Blend + Default> Track<T> {
    /// The value at `time` (milliseconds) of `sequence`, if the track has
    /// keys there.
    fn sample(&self, sequence: usize, time: u32, loops: &[u32]) -> Option<T> {
        let (keys, time) = match self.global {
            Some(g) => {
                let len = loops.get(g as usize).copied().unwrap_or(0);
                (self.keys.first()?, if len > 0 { time % len } else { 0 })
            }
            None => (self.keys.get(sequence)?, time),
        };
        let n = keys.times.len().min(keys.values.len());
        if n == 0 {
            return None;
        }
        if n == 1 || time <= keys.times[0] {
            return Some(keys.values[0]);
        }
        let i = keys.times[..n].partition_point(|&t| t <= time);
        if i >= n {
            return Some(keys.values[n - 1]);
        }
        let (t0, t1) = (keys.times[i - 1], keys.times[i]);
        if self.interpolation == 0 || t1 <= t0 {
            return Some(keys.values[i - 1]);
        }
        let f = (time - t0) as f32 / (t1 - t0) as f32;
        Some(T::blend(keys.values[i - 1], keys.values[i], f))
    }
}

/// What a batch's color and transparency are at a moment.
#[derive(Clone, Copy, Debug)]
pub struct Tint {
    pub color: Vec3,
    pub alpha: f32,
}

impl Model {
    /// The sequence of an animation (its first variation), following
    /// aliases.
    pub fn sequence(&self, animation: u16) -> Option<usize> {
        let mut s = *self.sequence_of.get(&animation)?;
        for _ in 0..8 {
            match self.sequences[s].alias {
                Some(a) if (a as usize) < self.sequences.len() && a as usize != s => s = a as usize,
                _ => break,
            }
        }
        Some(s)
    }

    /// Each bone's matrix at `time` of `sequence` (none: the bind pose).
    pub fn pose(&self, sequence: Option<usize>, time: u32) -> Vec<Mat4> {
        let n = self.bones.len();
        let mut out: Vec<Option<Mat4>> = vec![None; n];
        fn of(
            m: &Model,
            i: usize,
            seq: Option<usize>,
            time: u32,
            out: &mut Vec<Option<Mat4>>,
            depth: u32,
        ) -> Mat4 {
            if let Some(v) = out[i] {
                return v;
            }
            let b = &m.bones[i];
            let local = match seq {
                Some(s) => {
                    let t = b
                        .translation
                        .sample(s, time, &m.global_loops)
                        .unwrap_or([0.0; 3]);
                    let r = b
                        .rotation
                        .sample(s, time, &m.global_loops)
                        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
                    let sc = b.scale.sample(s, time, &m.global_loops).unwrap_or([1.0; 3]);
                    let p = b.pivot;
                    let mut l = translation([p[0] + t[0], p[1] + t[1], p[2] + t[2]]);
                    l = mul(&l, &rotation(r));
                    l = mul(&l, &scaling(sc));
                    mul(&l, &translation([-p[0], -p[1], -p[2]]))
                }
                None => IDENTITY,
            };
            let world = match b.parent {
                Some(p) if (p as usize) < m.bones.len() && p as usize != i && depth < 128 => {
                    let pm = of(m, p as usize, seq, time, out, depth + 1);
                    mul(&pm, &local)
                }
                _ => local,
            };
            out[i] = Some(world);
            world
        }
        for i in 0..n {
            of(self, i, sequence, time, &mut out, 0);
        }
        out.into_iter().map(|m| m.unwrap_or(IDENTITY)).collect()
    }

    /// The vertices posed by `bones` (positions and normals).
    pub fn skinned(&self, bones: &[Mat4]) -> Vec<(Vec3, Vec3)> {
        self.vertices
            .iter()
            .map(|v| {
                let mut p = [0.0f32; 3];
                let mut nrm = [0.0f32; 3];
                let mut total = 0.0;
                for k in 0..4 {
                    let w = f32::from(v.weights[k]) / 255.0;
                    if w == 0.0 {
                        continue;
                    }
                    let Some(m) = bones.get(v.bones[k] as usize) else {
                        continue;
                    };
                    let tp = transform(m, v.position);
                    let tn = transform_dir(m, v.normal);
                    for c in 0..3 {
                        p[c] += tp[c] * w;
                        nrm[c] += tn[c] * w;
                    }
                    total += w;
                }
                if total == 0.0 {
                    (v.position, v.normal)
                } else {
                    (p, nrm)
                }
            })
            .collect()
    }

    /// A batch's color and transparency at `time` of `sequence`.
    pub fn tint(&self, batch: &Batch, sequence: Option<usize>, time: u32) -> Tint {
        let s = sequence.unwrap_or(0);
        let mut tint = Tint {
            color: [1.0; 3],
            alpha: 1.0,
        };
        if let Some(c) = usize::try_from(batch.color)
            .ok()
            .and_then(|i| self.colors.get(i))
        {
            if let Some(v) = c.color.sample(s, time, &self.global_loops) {
                tint.color = v;
            }
            if let Some(a) = c.alpha.sample(s, time, &self.global_loops) {
                tint.alpha *= a;
            }
        }
        if let Some(w) = self
            .texture_weight_combos
            .get(batch.texture_weight_combo as usize)
            .and_then(|&i| self.texture_weights.get(i as usize))
            && let Some(a) = w.sample(s, time, &self.global_loops)
        {
            tint.alpha *= a;
        }
        tint
    }

    /// A batch's texture coordinate transform at `time` (a matrix for
    /// (u, v, 0)), if it has one.
    pub fn uv_transform(&self, batch: &Batch, sequence: Option<usize>, time: u32) -> Option<Mat4> {
        self.uv_transform_of(batch, 0, sequence, time)
    }

    /// The transform of a batch's `k`th texture.
    pub fn uv_transform_of(
        &self,
        batch: &Batch,
        k: usize,
        sequence: Option<usize>,
        time: u32,
    ) -> Option<Mat4> {
        let i = *self
            .texture_transform_combos
            .get(batch.texture_transform_combo as usize + k)?;
        let t = self.texture_transforms.get(i as usize)?;
        let s = sequence.unwrap_or(0);
        let tr = t
            .translation
            .sample(s, time, &self.global_loops)
            .unwrap_or([0.0; 3]);
        let r = t
            .rotation
            .sample(s, time, &self.global_loops)
            .unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let sc = t
            .scale
            .sample(s, time, &self.global_loops)
            .unwrap_or([1.0; 3]);
        // Rotated and scaled about the texture's middle.
        let mut m = translation([0.5 + tr[0], 0.5 + tr[1], tr[2]]);
        m = mul(&m, &rotation(r));
        m = mul(&m, &scaling(sc));
        Some(mul(&m, &translation([-0.5, -0.5, 0.0])))
    }

    /// An attachment point's position at a pose.
    pub fn attachment(&self, id: u32, bones: &[Mat4]) -> Option<Mat4> {
        let i = *self.attachment_of.get(id as usize)?;
        let a = self.attachments.get(usize::try_from(i).ok()?)?;
        let bone = bones.get(a.bone as usize).copied().unwrap_or(IDENTITY);
        Some(mul(&bone, &translation(a.position)))
    }
}
