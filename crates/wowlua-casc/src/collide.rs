//! The world's solid parts (decision T524): the terrain and the buildings'
//! colliding triangles, for where one stands (the floor under a place),
//! what stops one walking (a wall between two places) and what one sees
//! (a line between two places, clear or not).
//!
//! A building's triangles are tested in its own space: a segment there is
//! the map's segment through the placement's inverse (a turn and a scale:
//! the fraction along it is the same).

use crate::Casc;
use crate::adt::TILE;
use crate::display::{Building, Library};

/// How high a step one walks up (yards; unverified).
pub const STEP: f64 = 2.0;
/// The steepest slope one stands on: a floor's normal at least this up.
pub const WALKABLE: f64 = 0.64;

/// A building where it stands: its geometry, and its placement's turn
/// (columns, model to map), scale and origin.
#[derive(Clone)]
pub struct Placed {
    pub building: std::sync::Arc<Building>,
    axes: [[f64; 3]; 3],
    scale: f64,
    origin: [f64; 3],
    /// Its bounds on the map.
    pub bounds: [[f64; 3]; 2],
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

impl Placed {
    pub fn new(building: std::sync::Arc<Building>, b: &crate::obj::Building) -> Placed {
        let f = crate::obj::placed(b.at, b.rotation, b.scale);
        let origin = f([0.0; 3]);
        let s = f64::from(b.scale.max(1e-6));
        let axis = |v: [f32; 3]| {
            let p = sub(f(v), origin);
            [p[0] / s, p[1] / s, p[2] / s]
        };
        Placed {
            building,
            axes: [
                axis([1.0, 0.0, 0.0]),
                axis([0.0, 1.0, 0.0]),
                axis([0.0, 0.0, 1.0]),
            ],
            scale: s,
            origin,
            bounds: b.bounds,
        }
    }

    /// A place of the map in the building's own space.
    fn inside(&self, p: [f64; 3]) -> [f64; 3] {
        let d = sub(p, self.origin);
        [
            dot(d, self.axes[0]) / self.scale,
            dot(d, self.axes[1]) / self.scale,
            dot(d, self.axes[2]) / self.scale,
        ]
    }

    /// A direction of the building's own space on the map.
    fn outside(&self, v: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0; 3];
        for (k, axis) in self.axes.iter().enumerate() {
            for i in 0..3 {
                out[i] += axis[i] * v[k];
            }
        }
        out
    }

    /// The first colliding triangle a segment meets: how far along it (0
    /// to 1) and the triangle's normal on the map.
    pub fn hit(&self, a: [f64; 3], b: [f64; 3]) -> Option<(f64, [f64; 3])> {
        self.first(a, b, |_| true)
    }

    /// The first colliding triangle a segment meets whose normal on the
    /// map is one `accept` takes (a floor under a wall, a wall behind a
    /// step).
    pub fn first(
        &self,
        a: [f64; 3],
        b: [f64; 3],
        accept: impl Fn([f64; 3]) -> bool,
    ) -> Option<(f64, [f64; 3])> {
        let (lo, hi) = (
            [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
            [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
        );
        if (0..3).any(|k| hi[k] < self.bounds[0][k] || lo[k] > self.bounds[1][k]) {
            return None;
        }
        let (ma, mb) = (self.inside(a), self.inside(b));
        let dir = sub(mb, ma);
        let (glo, ghi) = (
            [ma[0].min(mb[0]), ma[1].min(mb[1]), ma[2].min(mb[2])],
            [ma[0].max(mb[0]), ma[1].max(mb[1]), ma[2].max(mb[2])],
        );
        let mut best: Option<(f64, [f64; 3])> = None;
        let mut near = Vec::new();
        for g in &self.building.groups {
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
                let p = |i: u16| {
                    let v = g.positions[i as usize];
                    [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]
                };
                let (v0, v1, v2) = (p(tri[0]), p(tri[1]), p(tri[2]));
                // (Möller and Trumbore: the segment's fraction where it
                // crosses the triangle, either side)
                let (e1, e2) = (sub(v1, v0), sub(v2, v0));
                let h = cross(dir, e2);
                let det = dot(e1, h);
                if det.abs() < 1e-12 {
                    continue;
                }
                let s = sub(ma, v0);
                let u = dot(s, h) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = cross(s, e1);
                let v = dot(dir, q) / det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let f = dot(e2, q) / det;
                if (0.0..=1.0).contains(&f) && best.is_none_or(|(bf, _)| f < bf) {
                    let n = cross(e1, e2);
                    let n = self.outside(n);
                    let l = dot(n, n).sqrt().max(1e-12);
                    let n = [n[0] / l, n[1] / l, n[2] / l];
                    if accept(n) {
                        best = Some((f, n));
                    }
                }
            }
        }
        best
    }
}

impl Library {
    /// The buildings standing within `reach` yards of a place (north,
    /// west) of a map, placed.
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
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                let Some(objects) = self.objects(casc, map, x, y) else {
                    continue;
                };
                for b in &objects.buildings {
                    let dn = north - north.clamp(b.bounds[0][0], b.bounds[1][0]);
                    let dw = west - west.clamp(b.bounds[0][1], b.bounds[1][1]);
                    if dn.hypot(dw) > reach || !seen.insert(b.unique) {
                        continue;
                    }
                    if let Some(building) = self.building(casc, b.file) {
                        out.push(Placed::new(building, b));
                    }
                }
            }
        }
        out
    }

    /// The floor under a place of a map (north, west, up): the highest
    /// ground or building floor no more than a step over it (none: the
    /// terrain's, or nothing at all).
    pub fn floor(&mut self, casc: &Casc, map: u32, at: [f64; 3]) -> Option<f64> {
        let terrain = self.ground(casc, map, at[0], at[1]);
        // (a place under the ground stands on it, and on what is there)
        let from = at[2].max(terrain.unwrap_or(f64::MIN)) + STEP;
        let mut best = terrain;
        for p in self.placed_near(casc, map, at[0], at[1], 1.0) {
            let (a, b) = ([at[0], at[1], from], [at[0], at[1], from - 200.0]);
            // (the highest floor under: the first a falling line meets,
            // past the walls and steep slopes over it)
            if let Some((f, _)) = p.first(a, b, |n| n[2].abs() >= WALKABLE) {
                let z = from - 200.0 * f;
                if best.is_none_or(|t| z > t) {
                    best = Some(z);
                }
            }
        }
        best
    }

    /// Where the line from one place of a map to another first meets the
    /// world (0 to 1 along it): the terrain (sampled each yard) or a
    /// building; none for a clear line.
    pub fn hit(&mut self, casc: &Casc, map: u32, a: [f64; 3], b: [f64; 3]) -> Option<f64> {
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
            if let Some((t, _)) = p.hit(a, b)
                && first.is_none_or(|f| t < f)
            {
                first = Some(t);
            }
        }
        first
    }

    /// How far one walks from `a` toward `b` before a wall (0 to 1): a
    /// building's steep triangle met at the knee or the head.
    pub fn walk(&mut self, casc: &Casc, map: u32, a: [f64; 3], b: [f64; 3]) -> f64 {
        let d = sub(b, a);
        let len = dot(d, d).sqrt();
        if len < 1e-9 {
            return 1.0;
        }
        let mut f = 1.0f64;
        for p in self.placed_near(casc, map, a[0], a[1], len + 1.0) {
            for up in [0.6, 1.6] {
                let (ka, kb) = ([a[0], a[1], a[2] + up], [b[0], b[1], b[2] + up]);
                // (a wall, past the floors and steps before it)
                if let Some((t, _)) = p.first(ka, kb, |n| n[2].abs() < WALKABLE) {
                    // (stopping a hand short of it)
                    f = f.min((t - 0.3 / len).max(0.0));
                }
            }
        }
        f
    }
}
