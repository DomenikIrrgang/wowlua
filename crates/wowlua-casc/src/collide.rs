//! The world's solid parts (decisions T524, T532): the terrain, the
//! buildings' colliding triangles and the props' collision meshes, for
//! where one stands (the floor under a place), what stops one walking (a
//! wall a body meets on its way) and what one sees (a line between two
//! places, clear or not).
//!
//! A solid's triangles are tested in its own space: a segment there is the
//! map's segment through the placement's inverse (a turn and a scale: the
//! fraction along it is the same).

use std::collections::HashMap;
use std::sync::Arc;

use crate::Casc;
use crate::adt::{CHUNK, TILE};
use crate::display::{Building, Library};

/// How high a step one walks up: to the knee, where walls begin (yards;
/// unverified).
pub const STEP: f64 = 0.6;
/// The steepest slope a wall is not: a triangle's normal at least this
/// up.
pub const WALKABLE: f64 = 0.64;
/// The heights over one's feet a wall stops one at (yards; unverified):
/// from the knee (what is lower one steps onto) to the head, a hand
/// apart (a beam or a sill between them stops one too).
pub const BODY: [f64; 6] = [STEP, 0.8, 1.0, 1.2, 1.4, 1.6];
/// How many map tiles keep their solids gathered.
const SOLIDS_KEPT: usize = 49;
/// The farthest a prop reaches from where it stands, at its own scale
/// (yards; more than the game's models do).
const PROP_REACH: f64 = 50.0;

/// A prop's collision mesh (an M2 model's, decision T532): its corners
/// and its triangles, in its own space.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub corners: Vec<[f32; 3]>,
    pub indices: Vec<u16>,
}

/// What a solid is made of.
#[derive(Clone)]
enum Shape {
    Building(Arc<Building>),
    Prop(Arc<Prop>),
}

/// A placement: its turn (columns, its own space to the map's), scale and
/// origin.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    axes: [[f64; 3]; 3],
    scale: f64,
    origin: [f64; 3],
}

/// A tile's buildings and props, none read yet: a building when one
/// first comes within its bounds, a prop when one first comes near where
/// it stands (a tile places thousands).
pub(crate) struct Solids {
    buildings: Vec<TileBuilding>,
    props: Props,
}

/// A building, read with its props when one first comes within its
/// bounds.
struct TileBuilding {
    placement: crate::obj::Building,
    read: std::sync::OnceLock<Option<(Placed, Props)>>,
}

/// Props by where they stand, in cells of a chunk (33⅓ yards): a query
/// looks only at the cells around it.
#[derive(Default)]
struct Props {
    cells: HashMap<(i32, i32), Vec<TileProp>>,
    /// The largest scale among them (how far one may reach).
    scale: f64,
}

/// A prop, its model read when one first comes near it.
struct TileProp {
    file: u32,
    frame: Frame,
    key: (u8, u32, u32),
    read: std::sync::OnceLock<Option<Placed>>,
}

impl Props {
    fn cell(north: f64, west: f64) -> (i32, i32) {
        (
            (north / CHUNK).floor() as i32,
            (west / CHUNK).floor() as i32,
        )
    }

    fn add(&mut self, file: u32, frame: Frame, key: (u8, u32, u32)) {
        self.scale = self.scale.max(frame.scale);
        let cell = Props::cell(frame.origin[0], frame.origin[1]);
        self.cells.entry(cell).or_default().push(TileProp {
            file,
            frame,
            key,
            read: std::sync::OnceLock::new(),
        });
    }

    /// The props standing within `reach` yards of a place, give or take
    /// a model's size.
    fn near(&self, north: f64, west: f64, reach: f64) -> impl Iterator<Item = &TileProp> {
        let r = reach + PROP_REACH * self.scale;
        let (lo, hi) = (
            Props::cell(north - r, west - r),
            Props::cell(north + r, west + r),
        );
        (lo.0..=hi.0)
            .flat_map(move |n| (lo.1..=hi.1).map(move |w| (n, w)))
            .filter_map(|c| self.cells.get(&c))
            .flatten()
            .filter(move |p| {
                (north - p.frame.origin[0]).hypot(west - p.frame.origin[1])
                    <= reach + PROP_REACH * p.frame.scale
            })
    }
}

/// A solid where it stands: a building or a prop, and its placement.
#[derive(Clone)]
pub struct Placed {
    shape: Shape,
    frame: Frame,
    /// Its bounds on the map.
    pub bounds: [[f64; 3]; 2],
    /// Which placement it is, once across the tiles that list it: a
    /// building (0, its ID), a building's prop (1, the building's ID, the
    /// prop's index), a map's prop (2, its ID).
    key: (u8, u32, u32),
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Frame {
    /// The placement `f` is (a corner of its own space to the map's),
    /// which scales its own space by `scale`.
    fn of(f: &dyn Fn([f32; 3]) -> [f64; 3], scale: f64) -> Frame {
        let origin = f([0.0; 3]);
        let s = scale.max(1e-6);
        let axis = |v: [f32; 3]| {
            let p = sub(f(v), origin);
            [p[0] / s, p[1] / s, p[2] / s]
        };
        Frame {
            axes: [
                axis([1.0, 0.0, 0.0]),
                axis([0.0, 1.0, 0.0]),
                axis([0.0, 0.0, 1.0]),
            ],
            scale: s,
            origin,
        }
    }

    /// A place of the map in its own space.
    fn inside(&self, p: [f64; 3]) -> [f64; 3] {
        let d = sub(p, self.origin);
        [
            dot(d, self.axes[0]) / self.scale,
            dot(d, self.axes[1]) / self.scale,
            dot(d, self.axes[2]) / self.scale,
        ]
    }

    /// A direction of its own space on the map.
    fn outside(&self, v: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0; 3];
        for (k, axis) in self.axes.iter().enumerate() {
            for i in 0..3 {
                out[i] += axis[i] * v[k];
            }
        }
        out
    }

    /// A place of its own space on the map.
    fn place(&self, v: [f32; 3]) -> [f64; 3] {
        let s = self.scale;
        let d = self.outside([
            f64::from(v[0]) * s,
            f64::from(v[1]) * s,
            f64::from(v[2]) * s,
        ]);
        [
            self.origin[0] + d[0],
            self.origin[1] + d[1],
            self.origin[2] + d[2],
        ]
    }
}

impl Placed {
    pub fn new(building: Arc<Building>, b: &crate::obj::Building) -> Placed {
        let f = crate::obj::placed(b.at, b.rotation, b.scale);
        Placed {
            shape: Shape::Building(building),
            frame: Frame::of(&f, f64::from(b.scale)),
            bounds: b.bounds,
            key: (0, b.unique, 0),
        }
    }

    /// A prop where a frame places it.
    fn prop(prop: Arc<Prop>, frame: Frame, key: (u8, u32, u32)) -> Placed {
        let mut bounds = [[f64::MAX; 3], [f64::MIN; 3]];
        for v in &prop.corners {
            let p = frame.place(*v);
            for k in 0..3 {
                bounds[0][k] = bounds[0][k].min(p[k]);
                bounds[1][k] = bounds[1][k].max(p[k]);
            }
        }
        Placed {
            shape: Shape::Prop(prop),
            frame,
            bounds,
            key,
        }
    }

    /// Whether it is a prop (a model's collision mesh), not a building.
    pub fn is_prop(&self) -> bool {
        matches!(self.shape, Shape::Prop(_))
    }

    /// The first colliding triangle a segment meets: how far along it (0
    /// to 1) and the triangle's normal on the map.
    pub fn hit(&self, a: [f64; 3], b: [f64; 3]) -> Option<(f64, [f64; 3])> {
        self.first(a, b, |_, _| true)
    }

    /// The first colliding triangle a segment meets that `accept` takes,
    /// by how far along it is and its normal on the map (a floor under a
    /// wall, a wall behind a step).
    pub fn first(
        &self,
        a: [f64; 3],
        b: [f64; 3],
        accept: impl Fn(f64, [f64; 3]) -> bool,
    ) -> Option<(f64, [f64; 3])> {
        let (lo, hi) = (
            [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
            [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
        );
        if (0..3).any(|k| hi[k] < self.bounds[0][k] || lo[k] > self.bounds[1][k]) {
            return None;
        }
        let (ma, mb) = (self.frame.inside(a), self.frame.inside(b));
        let dir = sub(mb, ma);
        let mut best: Option<(f64, [f64; 3])> = None;
        let mut test = |v0: [f64; 3], v1: [f64; 3], v2: [f64; 3]| {
            // (Möller and Trumbore: the segment's fraction where it
            // crosses the triangle, either side)
            let (e1, e2) = (sub(v1, v0), sub(v2, v0));
            let h = cross(dir, e2);
            let det = dot(e1, h);
            if det.abs() < 1e-12 {
                return;
            }
            let s = sub(ma, v0);
            let u = dot(s, h) / det;
            if !(0.0..=1.0).contains(&u) {
                return;
            }
            let q = cross(s, e1);
            let v = dot(dir, q) / det;
            if v < 0.0 || u + v > 1.0 {
                return;
            }
            let f = dot(e2, q) / det;
            if (0.0..=1.0).contains(&f) && best.is_none_or(|(bf, _)| f < bf) {
                let n = self.frame.outside(cross(e1, e2));
                let l = dot(n, n).sqrt().max(1e-12);
                let n = [n[0] / l, n[1] / l, n[2] / l];
                if accept(f, n) {
                    best = Some((f, n));
                }
            }
        };
        let wide = |v: [f32; 3]| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])];
        match &self.shape {
            Shape::Building(building) => {
                let (glo, ghi) = (
                    [ma[0].min(mb[0]), ma[1].min(mb[1]), ma[2].min(mb[2])],
                    [ma[0].max(mb[0]), ma[1].max(mb[1]), ma[2].max(mb[2])],
                );
                let mut near = Vec::new();
                for g in &building.groups {
                    if (0..3).any(|k| {
                        ghi[k] < f64::from(g.bounds[0][k]) - 0.01
                            || glo[k] > f64::from(g.bounds[1][k]) + 0.01
                    }) {
                        continue;
                    }
                    g.near(glo, ghi, &mut near);
                    for &t in &near {
                        let Some(tri) = g.indices.get(t * 3..t * 3 + 3) else {
                            continue;
                        };
                        if !g.collides(t) {
                            continue;
                        }
                        let p = |i: u16| wide(g.positions[i as usize]);
                        test(p(tri[0]), p(tri[1]), p(tri[2]));
                    }
                }
            }
            Shape::Prop(prop) => {
                for tri in prop.indices.as_chunks::<3>().0 {
                    let p = |i: u16| prop.corners.get(i as usize).copied().map(wide);
                    if let (Some(v0), Some(v1), Some(v2)) = (p(tri[0]), p(tri[1]), p(tri[2])) {
                        test(v0, v1, v2);
                    }
                }
            }
        }
        best
    }
}

impl Library {
    /// A prop's collision mesh (an M2 file's), read once; none for a model
    /// without one (decision T532).
    pub fn prop(&mut self, casc: &Casc, file: u32) -> Option<Arc<Prop>> {
        if let Some(p) = self.props.get(&file) {
            return p.clone();
        }
        let prop = casc
            .file_by_id(file)
            .and_then(|data| crate::m2::collision(&data))
            .map(|(corners, indices)| Arc::new(Prop { corners, indices }));
        self.props.insert(file, prop.clone());
        prop
    }

    /// A map tile's solids, listed once (the last [`SOLIDS_KEPT`] tiles
    /// asked for), none read yet: its buildings and its own props.
    fn solids(&mut self, casc: &Casc, map: u32, x: i32, y: i32) -> Arc<Solids> {
        self.solids_asked += 1;
        let asked = self.solids_asked;
        if let Some((when, s)) = self.solids.get_mut(&(map, x, y)) {
            *when = asked;
            return s.clone();
        }
        let mut out = Solids {
            buildings: Vec::new(),
            props: Props::default(),
        };
        if let Some(objects) = self.objects(casc, map, x, y) {
            for b in &objects.buildings {
                out.buildings.push(TileBuilding {
                    placement: b.clone(),
                    read: std::sync::OnceLock::new(),
                });
            }
            for d in &objects.doodads {
                let f = crate::obj::placed(d.at, d.rotation, d.scale);
                let frame = Frame::of(&f, f64::from(d.scale));
                out.props.add(d.file, frame, (2, d.unique, 0));
            }
        }
        if self.solids.len() >= SOLIDS_KEPT
            && let Some(old) = self
                .solids
                .iter()
                .min_by_key(|(_, (when, _))| *when)
                .map(|(k, _)| *k)
        {
            self.solids.remove(&old);
        }
        let out = Arc::new(out);
        self.solids.insert((map, x, y), (asked, out.clone()));
        out
    }

    /// A building placed, and its props (its first set and the one
    /// placed, as they are drawn), not read yet.
    fn building_solids(
        &mut self,
        casc: &Casc,
        b: &crate::obj::Building,
    ) -> Option<(Placed, Props)> {
        let building = self.building(casc, b.file)?;
        let place = crate::obj::placed(b.at, b.rotation, b.scale);
        let mut sets = vec![0usize];
        if b.doodad_set != 0 {
            sets.push(b.doodad_set as usize);
        }
        let mut props = Props::default();
        for set in sets {
            let Some(&(first, count)) = building.root.doodad_sets.get(set) else {
                continue;
            };
            let doodads = building.root.doodads.iter().enumerate();
            for (i, d) in doodads.skip(first as usize).take(count as usize) {
                if d.file == 0 {
                    continue;
                }
                let local = d.local();
                let f = |v: [f32; 3]| place(local(v));
                let frame = Frame::of(&f, f64::from(d.scale) * f64::from(b.scale));
                props.add(d.file, frame, (1, b.unique, i as u32));
            }
        }
        Some((Placed::new(building, b), props))
    }

    /// The solids standing within `reach` yards of a place (north, west)
    /// of a map, placed (each read the first time one comes near it).
    pub fn placed_near(
        &mut self,
        casc: &Casc,
        map: u32,
        north: f64,
        west: f64,
        reach: f64,
    ) -> Vec<Placed> {
        let (cx, cy) = crate::adt::tile_of(north, west);
        let r = (reach / TILE).ceil() as i32;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // (how far a place of the map is from a box's, across)
        let away = |bounds: &[[f64; 3]; 2]| {
            let dn = north - north.clamp(bounds[0][0], bounds[1][0]);
            let dw = west - west.clamp(bounds[0][1], bounds[1][1]);
            dn.hypot(dw)
        };
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                let tile = self.solids(casc, map, x, y);
                let mut props = vec![&tile.props];
                for b in &tile.buildings {
                    let key = (0, b.placement.unique, 0);
                    if away(&b.placement.bounds) > reach || !seen.insert(key) {
                        continue;
                    }
                    let read = b
                        .read
                        .get_or_init(|| self.building_solids(casc, &b.placement));
                    if let Some((p, its)) = read {
                        out.push(p.clone());
                        props.push(its);
                    }
                }
                for group in props {
                    for prop in group.near(north, west, reach) {
                        if seen.contains(&prop.key) {
                            continue;
                        }
                        let read = prop.read.get_or_init(|| {
                            self.prop(casc, prop.file)
                                .map(|mesh| Placed::prop(mesh, prop.frame, prop.key))
                        });
                        if let Some(p) = read
                            && away(&p.bounds) <= reach
                        {
                            seen.insert(p.key);
                            out.push(p.clone());
                        }
                    }
                }
            }
        }
        out
    }

    /// The floor under a place of a map (north, west, up): the highest
    /// ground or solid no more than a step over it, steep or not (one
    /// steps onto what is lower than the knee, and does not fall through
    /// a roof). None: the terrain's, or nothing at all.
    pub fn floor(&mut self, casc: &Casc, map: u32, at: [f64; 3]) -> Option<f64> {
        let terrain = self.ground(casc, map, at[0], at[1]);
        // (a place under the ground stands on it, and on what is there)
        let from = at[2].max(terrain.unwrap_or(f64::MIN)) + STEP;
        let mut best = terrain;
        for p in self.placed_near(casc, map, at[0], at[1], 1.0) {
            let (a, b) = ([at[0], at[1], from], [at[0], at[1], from - 200.0]);
            // (the highest floor under: the first a falling line meets)
            if let Some((f, _)) = p.hit(a, b) {
                let z = from - 200.0 * f;
                if best.is_none_or(|t| z > t) {
                    best = Some(z);
                }
            }
        }
        best
    }

    /// Where the line from one place of a map to another first meets the
    /// world (0 to 1 along it): the terrain (sampled each yard), a
    /// building or, with `props`, a prop; none for a clear line.
    pub fn hit(
        &mut self,
        casc: &Casc,
        map: u32,
        a: [f64; 3],
        b: [f64; 3],
        props: bool,
    ) -> Option<f64> {
        let d = sub(b, a);
        let len = dot(d, d).sqrt();
        let steps = len.ceil().max(1.0) as usize;
        let mut first = None;
        for i in 1..steps {
            let t = i as f64 / steps as f64;
            let p = [a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t];
            if self.ground(casc, map, p[0], p[1]).is_some_and(|g| g > p[2]) {
                first = Some((i - 1) as f64 / steps as f64);
                break;
            }
        }
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        for p in self.placed_near(casc, map, mid[0], mid[1], len / 2.0 + 1.0) {
            if p.is_prop() && !props {
                continue;
            }
            if let Some((t, _)) = p.hit(a, b)
                && first.is_none_or(|f| t < f)
            {
                first = Some(t);
            }
        }
        first
    }

    /// How far one walks from `a` toward `b` before a wall (0 to 1): a
    /// solid's steep triangle one's body meets, `radius` yards wide, from
    /// the knee to the head ([`BODY`]); one stops with one's side at it.
    pub fn walk(&mut self, casc: &Casc, map: u32, a: [f64; 3], b: [f64; 3], radius: f64) -> f64 {
        let d = sub(b, a);
        let len = d[0].hypot(d[1]);
        if len < 1e-9 {
            return 1.0;
        }
        let (ahead, side) = ([d[0] / len, d[1] / len], [-d[1] / len, d[0] / len]);
        // (the body's middle and its two sides, each looking its reach
        // past the way)
        let rays = [
            (0.0, radius),
            (0.7 * radius, 0.7 * radius),
            (-0.7 * radius, 0.7 * radius),
        ];
        let mut f = 1.0f64;
        for p in self.placed_near(casc, map, a[0], a[1], len + 2.0 * radius + 1.0) {
            for up in BODY {
                for (off, reach) in rays {
                    let s = [a[0] + side[0] * off, a[1] + side[1] * off, a[2] + up];
                    let e = [
                        s[0] + d[0] + ahead[0] * reach,
                        s[1] + d[1] + ahead[1] * reach,
                        b[2] + up,
                    ];
                    // (a wall, past the floors and steps before it)
                    if let Some((t, _)) = p.first(s, e, |_, n| n[2].abs() < WALKABLE) {
                        f = f.min(((t * (len + reach) - reach) / len).max(0.0));
                    }
                }
            }
        }
        f
    }
}
