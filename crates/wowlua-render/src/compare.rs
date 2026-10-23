//! Comparing screenshots (spec debugger §18.11, decision T312): pixels
//! differ when their YIQ distance passes the threshold, as pixelmatch
//! measures it.

use wowlua_casc::Rgba;

/// The largest YIQ distance two colors can have.
const MAX_DELTA: f64 = 35215.0;

fn yiq(p: &[u8]) -> (f64, f64, f64) {
    // Blended onto white by alpha, as pixelmatch does.
    let a = f64::from(p[3]) / 255.0;
    let blend = |c: u8| 255.0 + (f64::from(c) - 255.0) * a;
    let (r, g, b) = (blend(p[0]), blend(p[1]), blend(p[2]));
    (
        r * 0.298_895_31 + g * 0.586_622_47 + b * 0.114_482_23,
        r * 0.595_977_99 - g * 0.274_176_10 - b * 0.321_801_89,
        r * 0.211_470_19 - g * 0.522_617_55 + b * 0.311_147_36,
    )
}

fn delta(a: &[u8], b: &[u8]) -> f64 {
    let (y1, i1, q1) = yiq(a);
    let (y2, i2, q2) = yiq(b);
    let (y, i, q) = (y1 - y2, i1 - i2, q1 - q2);
    0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q
}

/// The outcome of a comparison.
pub struct Diff {
    pub different: usize,
    pub total: usize,
    /// The pixels that differ red, the others faded.
    pub image: Rgba,
}

impl Diff {
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.different as f64 / self.total as f64
        }
    }
}

/// Compares two images (`threshold` 0 to 1); images of different sizes
/// differ everywhere.
pub fn compare(a: &Rgba, b: &Rgba, threshold: f64) -> Diff {
    let (w, h) = (a.width.max(b.width), a.height.max(b.height));
    let total = (w * h) as usize;
    let same_size = a.width == b.width && a.height == b.height;
    let limit = MAX_DELTA * threshold * threshold;
    let mut out = vec![0u8; total * 4];
    let mut different = 0;
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            let differs = if same_size {
                let pa = &a.pixels[o..o + 4];
                let pb = &b.pixels[o..o + 4];
                let d = delta(pa, pb) > limit;
                if !d {
                    // Faded gray of the original.
                    let (yy, _, _) = yiq(pa);
                    let g = (255.0 + (yy - 255.0) * 0.1) as u8;
                    out[o..o + 4].copy_from_slice(&[g, g, g, 255]);
                }
                d
            } else {
                true
            };
            if differs {
                different += 1;
                out[o..o + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
    }
    Diff {
        different,
        total,
        image: Rgba {
            width: w,
            height: h,
            pixels: out,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(px: &[[u8; 4]]) -> Rgba {
        Rgba {
            width: px.len() as u32,
            height: 1,
            pixels: px.iter().flatten().copied().collect(),
        }
    }

    #[test]
    fn small_differences_pass_the_threshold() {
        let a = img(&[[100, 100, 100, 255], [0, 0, 0, 255]]);
        let b = img(&[[102, 101, 100, 255], [255, 255, 255, 255]]);
        let d = compare(&a, &b, 0.1);
        assert_eq!(d.different, 1);
        assert_eq!(d.ratio(), 0.5);
    }
}
