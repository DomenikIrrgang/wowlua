//! Text's glyphs: rasterized once per font, pixel size and character into
//! an atlas (white, the coverage as alpha), placed where the engine's
//! measures put them.

use std::collections::HashMap;

/// The atlas's side in pixels.
pub const ATLAS: u32 = 2048;

/// A glyph in the atlas.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    /// Its rectangle in the atlas, in pixels.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// From the pen position on the baseline to its bitmap's left and top.
    pub left: f32,
    pub top: f32,
}

pub struct Atlas {
    pub pixels: Vec<u8>,
    glyphs: HashMap<(usize, u32, char), Option<Glyph>>,
    /// The shelf being filled: its top, height and next x.
    shelf_y: u32,
    shelf_h: u32,
    next_x: u32,
    /// Rows written since the last upload (a dirty band).
    pub dirty: Option<(u32, u32)>,
}

impl Default for Atlas {
    fn default() -> Self {
        Atlas {
            pixels: vec![0; (ATLAS * ATLAS * 4) as usize],
            glyphs: HashMap::new(),
            shelf_y: 0,
            shelf_h: 0,
            next_x: 0,
            dirty: None,
        }
    }
}

impl Atlas {
    /// The glyph of `c` in font `font` (an index into the renderer's
    /// fonts) at `px` pixels, rasterized when first asked.
    pub fn glyph(
        &mut self,
        fonts: &[fontdue::Font],
        font: usize,
        px: f32,
        c: char,
    ) -> Option<Glyph> {
        let key = (font, (px * 4.0).round() as u32, c);
        if let Some(g) = self.glyphs.get(&key) {
            return *g;
        }
        let g = self.rasterize(&fonts[font], px, c);
        self.glyphs.insert(key, g);
        g
    }

    fn rasterize(&mut self, font: &fontdue::Font, px: f32, c: char) -> Option<Glyph> {
        let (m, bitmap) = font.rasterize(c, px);
        if m.width == 0 || m.height == 0 {
            return None;
        }
        let (w, h) = (m.width as u32, m.height as u32);
        if self.next_x + w + 1 > ATLAS {
            self.shelf_y += self.shelf_h + 1;
            self.shelf_h = 0;
            self.next_x = 0;
        }
        if self.shelf_y + h > ATLAS {
            // Full: start again (the glyphs are drawn again as needed).
            self.glyphs.clear();
            self.pixels.iter_mut().for_each(|p| *p = 0);
            self.shelf_y = 0;
            self.shelf_h = 0;
            self.next_x = 0;
            self.dirty = Some((0, ATLAS));
        }
        let (x, y) = (self.next_x, self.shelf_y);
        for row in 0..h {
            for col in 0..w {
                let a = bitmap[(row * w + col) as usize];
                let o = (((y + row) * ATLAS + x + col) * 4) as usize;
                self.pixels[o..o + 4].copy_from_slice(&[255, 255, 255, a]);
            }
        }
        self.next_x += w + 1;
        self.shelf_h = self.shelf_h.max(h);
        self.dirty = Some(match self.dirty {
            Some((a, b)) => (a.min(y), b.max(y + h)),
            None => (y, y + h),
        });
        Some(Glyph {
            x,
            y,
            w,
            h,
            left: m.xmin as f32,
            top: (m.height as i32 + m.ymin) as f32,
        })
    }
}
