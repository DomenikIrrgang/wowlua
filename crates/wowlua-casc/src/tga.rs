//! TGA, the other image format addons ship: true color (24 or 32 bits)
//! and grayscale, raw or run-length encoded.

use crate::Rgba;

pub fn decode(b: &[u8]) -> Option<Rgba> {
    if b.len() < 18 {
        return None;
    }
    let id_len = b[0] as usize;
    let color_map = b[1];
    let kind = b[2];
    let w = u16::from_le_bytes([b[12], b[13]]) as usize;
    let h = u16::from_le_bytes([b[14], b[15]]) as usize;
    let bpp = b[16] as usize;
    let desc = b[17];
    if color_map != 0 || w == 0 || h == 0 {
        return None;
    }
    let gray = matches!(kind, 3 | 11);
    let rle = matches!(kind, 10 | 11);
    if !matches!(kind, 2 | 3 | 10 | 11) {
        return None;
    }
    let size = bpp / 8;
    if !(gray && size == 1 || !gray && (size == 3 || size == 4)) {
        return None;
    }
    let mut src = b.get(18 + id_len..)?;
    let n = w * h;
    let mut raw: Vec<u8> = Vec::with_capacity(n * size);
    if rle {
        while raw.len() < n * size {
            let (&head, rest) = src.split_first()?;
            src = rest;
            let count = (head & 0x7f) as usize + 1;
            if head & 0x80 != 0 {
                let px = src.get(..size)?;
                for _ in 0..count {
                    raw.extend_from_slice(px);
                }
                src = &src[size..];
            } else {
                raw.extend_from_slice(src.get(..count * size)?);
                src = &src[count * size..];
            }
        }
        raw.truncate(n * size);
    } else {
        raw.extend_from_slice(src.get(..n * size)?);
    }
    let top_first = desc & 0x20 != 0;
    let mut pixels = vec![0u8; n * 4];
    for y in 0..h {
        let sy = if top_first { y } else { h - 1 - y };
        for x in 0..w {
            let s = &raw[(sy * w + x) * size..][..size];
            let p = &mut pixels[(y * w + x) * 4..][..4];
            match size {
                1 => p.copy_from_slice(&[s[0], s[0], s[0], 255]),
                3 => p.copy_from_slice(&[s[2], s[1], s[0], 255]),
                _ => p.copy_from_slice(&[s[2], s[1], s[0], s[3]]),
            }
        }
    }
    Some(Rgba {
        width: w as u32,
        height: h as u32,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_bottom_up() {
        // 2x1, 32 bit, RLE: a run of 2 blue pixels.
        let mut b = vec![0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 8];
        b.extend_from_slice(&[0x81, 255, 0, 0, 128]);
        let img = decode(&b).unwrap();
        assert_eq!(img.pixels, [0, 0, 255, 128, 0, 0, 255, 128]);
    }
}
