//! Character skins composited from layers (spec debugger §18.11, decision
//! T320): each file resampled into its rectangle and blended over what is
//! there, in order.

use wowlua_casc::Rgba;
use wowlua_casc::display::Composite;

/// A pixel of an image at (u, v) in 0..1, bilinear, as floats 0..1.
fn sample(img: &Rgba, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (img.width as f32, img.height as f32);
    let x = (u * w - 0.5).clamp(0.0, w - 1.0);
    let y = (v * h - 0.5).clamp(0.0, h - 1.0);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(img.width - 1), (y0 + 1).min(img.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: u32, y: u32| {
        let o = ((y * img.width + x) * 4) as usize;
        [
            f32::from(img.pixels[o]),
            f32::from(img.pixels[o + 1]),
            f32::from(img.pixels[o + 2]),
            f32::from(img.pixels[o + 3]),
        ]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        out[k] = (top + (bottom - top) * fy) / 255.0;
    }
    out
}

/// The composite: black, then each layer by its blend mode (0 and 1 copy;
/// 4 multiply, 6 overlay, 7 screen with the texture's alpha; others the
/// texture over by its alpha).
pub fn composite(c: &Composite, image: &mut dyn FnMut(u32) -> Option<Rgba>) -> Rgba {
    let (w, h) = (c.width.max(1), c.height.max(1));
    let mut px = vec![[0.0f32, 0.0, 0.0, 1.0]; (w * h) as usize];
    for layer in &c.layers {
        let Some(img) = image(layer.file) else {
            continue;
        };
        if img.width == 0 || img.height == 0 {
            continue;
        }
        let [lx, ly, lw, lh] = layer.rect;
        for y in ly..(ly + lh).min(h) {
            for x in lx..(lx + lw).min(w) {
                let u = (x - lx) as f32 / lw as f32 + 0.5 / lw as f32;
                let v = (y - ly) as f32 / lh as f32 + 0.5 / lh as f32;
                let t = sample(&img, u, v);
                let base = &mut px[(y * w + x) as usize];
                let frag = match layer.blend {
                    0 | 1 => {
                        *base = t;
                        continue;
                    }
                    4 => [
                        base[0] * t[0],
                        base[1] * t[1],
                        base[2] * t[2],
                        base[3] * t[3],
                    ],
                    6 => {
                        let o = |b: f32, s: f32| {
                            if s < 0.5 {
                                2.0 * b * s
                            } else {
                                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
                            }
                        };
                        [o(base[0], t[0]), o(base[1], t[1]), o(base[2], t[2]), t[3]]
                    }
                    7 => {
                        let s = |b: f32, s: f32| 1.0 - (1.0 - b) * (1.0 - s);
                        [s(base[0], t[0]), s(base[1], t[1]), s(base[2], t[2]), t[3]]
                    }
                    _ => t,
                };
                let a = frag[3];
                for k in 0..3 {
                    base[k] = frag[k] * a + base[k] * (1.0 - a);
                }
                base[3] = a + base[3] * (1.0 - a);
            }
        }
    }
    Rgba {
        width: w,
        height: h,
        pixels: px
            .into_iter()
            .flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use wowlua_casc::display::{Composite, Layer};

    use super::*;

    fn solid(c: [u8; 4]) -> Rgba {
        Rgba {
            width: 2,
            height: 2,
            pixels: c.repeat(4),
        }
    }

    /// Layers go into their rectangles: copied, multiplied, or over by
    /// their alpha.
    #[test]
    fn layers_blend_into_their_rectangles() {
        let c = Composite {
            width: 4,
            height: 2,
            layers: vec![
                Layer {
                    file: 1,
                    rect: [0, 0, 4, 2],
                    blend: 1,
                },
                Layer {
                    file: 2,
                    rect: [0, 0, 2, 2],
                    blend: 4,
                },
                Layer {
                    file: 3,
                    rect: [2, 0, 2, 2],
                    blend: 15,
                },
            ],
        };
        let img = composite(&c, &mut |f| {
            Some(match f {
                1 => solid([200, 100, 50, 255]),
                2 => solid([128, 255, 0, 255]),
                _ => solid([0, 0, 255, 128]),
            })
        });
        let px = |x: usize| &img.pixels[x * 4..x * 4 + 4];
        assert_eq!(px(0), &[100, 100, 0, 255]);
        assert_eq!(px(3), &[100, 50, 153, 255]);
    }
}
