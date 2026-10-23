//! BLP2, the client's texture format: palettized (with 0, 1, 4 or 8 bit
//! alpha), DXT1, DXT3 and DXT5, and plain BGRA. Only the largest mipmap is
//! decoded.

use crate::Rgba;

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// RGB565 to 8-bit channels.
fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [
        ((r * 527 + 23) >> 6) as u8,
        ((g * 259 + 33) >> 6) as u8,
        ((b * 527 + 23) >> 6) as u8,
    ]
}

/// A DXT color block's four colors (`dxt1`: the three-color mode with
/// transparent black when the first color is not greater).
fn colors(block: &[u8], dxt1: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |w0: u32, w1: u32, d: u32| -> [u8; 4] {
        [
            ((u32::from(a[0]) * w0 + u32::from(b[0]) * w1) / d) as u8,
            ((u32::from(a[1]) * w0 + u32::from(b[1]) * w1) / d) as u8,
            ((u32::from(a[2]) * w0 + u32::from(b[2]) * w1) / d) as u8,
            255,
        ]
    };
    if c0 > c1 || !dxt1 {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(2, 1, 3),
            mix(1, 2, 3),
        ]
    } else {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(1, 1, 2),
            [0, 0, 0, 0],
        ]
    }
}

/// Decodes DXT blocks (`kind`: 1, 3 or 5).
fn dxt(data: &[u8], w: usize, h: usize, kind: u8) -> Option<Vec<u8>> {
    let block_size = if kind == 1 { 8 } else { 16 };
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    if data.len() < bw * bh * block_size {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    for by in 0..bh {
        for bx in 0..bw {
            let block = &data[(by * bw + bx) * block_size..][..block_size];
            let (alpha, color) = if kind == 1 {
                (&block[..0], block)
            } else {
                (&block[..8], &block[8..])
            };
            let pal = colors(color, kind == 1);
            let idx = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
            let mut alphas = [255u8; 16];
            if kind == 3 {
                for (i, a) in alphas.iter_mut().enumerate() {
                    let nib = (alpha[i / 2] >> ((i % 2) * 4)) & 0xf;
                    *a = nib * 17;
                }
            } else if kind == 5 {
                let (a0, a1) = (u32::from(alpha[0]), u32::from(alpha[1]));
                let mut table = [0u32; 8];
                table[0] = a0;
                table[1] = a1;
                if a0 > a1 {
                    for (i, t) in table.iter_mut().enumerate().skip(2) {
                        *t = ((8 - i as u32) * a0 + (i as u32 - 1) * a1) / 7;
                    }
                } else {
                    for (i, t) in table.iter_mut().enumerate().take(6).skip(2) {
                        *t = ((6 - i as u32) * a0 + (i as u32 - 1) * a1) / 5;
                    }
                    table[6] = 0;
                    table[7] = 255;
                }
                let bits = alpha[2..8]
                    .iter()
                    .rev()
                    .fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
                for (i, a) in alphas.iter_mut().enumerate() {
                    *a = table[((bits >> (i * 3)) & 7) as usize] as u8;
                }
            }
            for py in 0..4 {
                for px in 0..4 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x >= w || y >= h {
                        continue;
                    }
                    let i = py * 4 + px;
                    let mut c = pal[((idx >> (i * 2)) & 3) as usize];
                    if kind != 1 {
                        c[3] = alphas[i];
                    }
                    out[(y * w + x) * 4..][..4].copy_from_slice(&c);
                }
            }
        }
    }
    Some(out)
}

/// Decodes a BLP2 file's first mipmap.
pub fn decode(b: &[u8]) -> Option<Rgba> {
    if b.len() < 148 || &b[..4] != b"BLP2" {
        return None;
    }
    let encoding = b[8];
    let alpha_depth = b[9];
    let alpha_type = b[10];
    let w = le32(b, 12) as usize;
    let h = le32(b, 16) as usize;
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let offset = le32(b, 20) as usize;
    let size = le32(b, 20 + 64) as usize;
    let data = b.get(offset..offset.checked_add(size)?)?;
    let pixels = match encoding {
        1 => {
            let palette = b.get(148..148 + 1024)?;
            let n = w * h;
            let idx = data.get(..n)?;
            let alpha = &data[n..];
            let mut out = vec![0u8; n * 4];
            for i in 0..n {
                let p = idx[i] as usize * 4;
                let a = match alpha_depth {
                    1 => {
                        if alpha.get(i / 8)? >> (i % 8) & 1 == 1 {
                            255
                        } else {
                            0
                        }
                    }
                    4 => (alpha.get(i / 2)? >> ((i % 2) * 4) & 0xf) * 17,
                    8 => *alpha.get(i)?,
                    _ => 255,
                };
                out[i * 4..i * 4 + 4].copy_from_slice(&[
                    palette[p + 2],
                    palette[p + 1],
                    palette[p],
                    a,
                ]);
            }
            out
        }
        2 => {
            let kind = match alpha_type {
                1 => 3,
                7 => 5,
                _ => 1,
            };
            let mut px = dxt(data, w, h, kind)?;
            if kind == 1 && alpha_depth == 0 {
                for p in px.chunks_mut(4) {
                    p[3] = 255;
                }
            }
            px
        }
        3 => {
            let mut out = data.get(..w * h * 4)?.to_vec();
            for p in out.chunks_mut(4) {
                p.swap(0, 2);
            }
            out
        }
        _ => return None,
    };
    Some(Rgba {
        width: w as u32,
        height: h as u32,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blp(
        encoding: u8,
        alpha_depth: u8,
        alpha_type: u8,
        w: u32,
        h: u32,
        palette: &[u8],
        data: &[u8],
    ) -> Vec<u8> {
        let mut b = b"BLP2".to_vec();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[encoding, alpha_depth, alpha_type, 0]);
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        let offset = 148 + 1024;
        let mut offsets = [0u32; 16];
        let mut sizes = [0u32; 16];
        offsets[0] = offset;
        sizes[0] = data.len() as u32;
        for o in offsets {
            b.extend_from_slice(&o.to_le_bytes());
        }
        for s in sizes {
            b.extend_from_slice(&s.to_le_bytes());
        }
        let mut pal = palette.to_vec();
        pal.resize(1024, 0);
        b.extend_from_slice(&pal);
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn palettized_with_8_bit_alpha() {
        // Palette: 0 red, 1 blue (BGRA).
        let pal = [0, 0, 255, 0, 255, 0, 0, 0];
        let data = [0, 1, 0, 1, 255, 128, 0, 64];
        let img = decode(&blp(1, 8, 8, 2, 2, &pal, &data)).unwrap();
        assert_eq!(&img.pixels[..8], &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert_eq!(img.pixels[15], 64);
    }

    #[test]
    fn dxt1_block() {
        // c0 = white, c1 = black, every pixel index 0, then the last 1.
        let block = [0xff, 0xff, 0, 0, 0, 0, 0, 0x40];
        let img = decode(&blp(2, 0, 0, 4, 4, &[], &block)).unwrap();
        assert_eq!(&img.pixels[..4], &[255, 255, 255, 255]);
        assert_eq!(&img.pixels[15 * 4..], &[0, 0, 0, 255]);
    }

    #[test]
    fn dxt5_alpha() {
        // Alpha 255 → 0, all index 1 (= a1 = 0); colors white.
        let mut block = vec![255, 0];
        let bits: u64 = (0..16).fold(0, |acc, i| acc | (1u64 << (i * 3)));
        block.extend_from_slice(&bits.to_le_bytes()[..6]);
        block.extend_from_slice(&[0xff, 0xff, 0, 0, 0, 0, 0, 0]);
        let img = decode(&blp(2, 8, 7, 4, 4, &[], &block)).unwrap();
        assert_eq!(&img.pixels[..4], &[255, 255, 255, 0]);
    }
}
