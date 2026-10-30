//! A map's WDT (spec debugger §18.11): the files of each of its 64x64
//! terrain tiles, here their minimap textures.

/// Each tile's minimap texture (file ID, 0 for none), row by row from the
/// north-west: `tiles[y * 64 + x]`. From the `MAID` chunk, whose eight
/// file IDs a tile are its root, objects, textures, LOD, map textures
/// and, last, its minimap texture.
pub fn minimap_tiles(data: &[u8]) -> Option<Vec<u32>> {
    let mut o = 0;
    while o + 8 <= data.len() {
        let tag = &data[o..o + 4];
        let size = u32::from_le_bytes(data[o + 4..o + 8].try_into().ok()?) as usize;
        let body = data.get(o + 8..o + 8 + size)?;
        // Chunk tags are stored reversed.
        if tag == b"DIAM" {
            let per = size / 4096;
            if per < 32 {
                return None;
            }
            return Some(
                (0..4096)
                    .map(|i| {
                        let at = i * per + 28;
                        u32::from_le_bytes(body[at..at + 4].try_into().unwrap())
                    })
                    .collect(),
            );
        }
        o += 8 + size;
    }
    None
}

/// A map's terrain tiles (decision T522): each tile's files, row by row from
/// the north-west (`tiles[y * 64 + x]`, all 0 for none), and the map's
/// flags (`MPHD`: 0x4 or 0x80, the tiles' texture blends at 8 bits).
#[derive(Clone, Debug, PartialEq)]
pub struct Tiles {
    pub flags: u32,
    pub tiles: Vec<TileFiles>,
}

/// A terrain tile's files (`MAID`): its root (heights), objects, textures
/// and the rest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TileFiles {
    pub root: u32,
    pub obj0: u32,
    pub obj1: u32,
    pub tex0: u32,
    pub lod: u32,
    pub map_texture: u32,
    pub map_texture_n: u32,
    pub minimap: u32,
}

/// A WDT's tiles and flags, none without its `MAID` chunk (a WMO-only map).
pub fn tiles(data: &[u8]) -> Option<Tiles> {
    let mut o = 0;
    let mut flags = 0;
    while o + 8 <= data.len() {
        let tag = &data[o..o + 4];
        let size = u32::from_le_bytes(data[o + 4..o + 8].try_into().ok()?) as usize;
        let body = data.get(o + 8..o + 8 + size)?;
        let at = |i: usize| -> u32 {
            body.get(i..i + 4)
                .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
        };
        if tag == b"DHPM" {
            flags = at(0);
        }
        if tag == b"DIAM" {
            let per = size / 4096;
            if per < 32 {
                return None;
            }
            let tiles = (0..4096)
                .map(|i| {
                    let b = i * per;
                    TileFiles {
                        root: at(b),
                        obj0: at(b + 4),
                        obj1: at(b + 8),
                        tex0: at(b + 12),
                        lod: at(b + 16),
                        map_texture: at(b + 20),
                        map_texture_n: at(b + 24),
                        minimap: at(b + 28),
                    }
                })
                .collect();
            return Some(Tiles { flags, tiles });
        }
        o += 8 + size;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = tag.to_vec();
        v.extend((body.len() as u32).to_le_bytes());
        v.extend(body);
        v
    }

    /// The minimap texture is each tile's eighth file ID, after the chunks
    /// before `MAID`.
    #[test]
    fn minimap_tiles_are_each_tiles_last_file() {
        let mut maid = vec![0u8; 4096 * 32];
        let tile = (48 * 64 + 32) * 32;
        maid[tile..tile + 4].copy_from_slice(&7u32.to_le_bytes());
        maid[tile + 28..tile + 32].copy_from_slice(&204493u32.to_le_bytes());
        let mut wdt = chunk(b"REVM", &18u32.to_le_bytes());
        wdt.extend(chunk(b"DHPM", &[0; 32]));
        wdt.extend(chunk(b"DIAM", &maid));
        let tiles = minimap_tiles(&wdt).unwrap();
        assert_eq!(tiles.len(), 4096);
        assert_eq!(tiles[48 * 64 + 32], 204493);
        assert_eq!(tiles.iter().filter(|t| **t != 0).count(), 1);
        assert_eq!(minimap_tiles(&chunk(b"REVM", &[0; 4])), None);
        let all = super::tiles(&wdt).unwrap();
        assert_eq!(all.tiles[48 * 64 + 32].root, 7);
        assert_eq!(all.tiles[48 * 64 + 32].minimap, 204493);
        assert_eq!(all.flags, 0);
    }
}
