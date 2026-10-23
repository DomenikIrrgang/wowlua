//! The client's M2 shaders (spec debugger §18.11): which texture
//! coordinates a batch's textures take (the vertex shader) and how they
//! combine (the pixel shader), by the batch's shader ID and texture count.

/// Vertex shaders: where each texture's coordinates come from (all the
/// client's, some only named here).
#[allow(dead_code)]
pub(crate) mod vs {
    pub const T1: u32 = 0;
    pub const ENV: u32 = 1;
    pub const T1_T2: u32 = 2;
    pub const T1_ENV: u32 = 3;
    pub const ENV_T1: u32 = 4;
    pub const ENV_ENV: u32 = 5;
    pub const T1_ENV_T1: u32 = 6;
    pub const T1_T1: u32 = 7;
    pub const T1_T1_T1: u32 = 8;
    pub const EDGE_FADE_T1: u32 = 9;
    pub const T2: u32 = 10;
    pub const T1_ENV_T2: u32 = 11;
    pub const EDGE_FADE_T1_T2: u32 = 12;
    pub const EDGE_FADE_ENV: u32 = 13;
    pub const T1_T2_T1: u32 = 14;
    pub const T1_T2_T3: u32 = 15;
    pub const COLOR_T1_T2_T3: u32 = 16;
    pub const BW_T1: u32 = 17;
    pub const BW_T1_T2: u32 = 18;
}

/// Pixel shaders (the combiners), as numbered in `model.wgsl`.
#[allow(dead_code)]
pub(crate) mod ps {
    pub const OPAQUE: u32 = 0;
    pub const MOD: u32 = 1;
    pub const OPAQUE_MOD: u32 = 2;
    pub const OPAQUE_MOD2X: u32 = 3;
    pub const OPAQUE_MOD2X_NA: u32 = 4;
    pub const OPAQUE_OPAQUE: u32 = 5;
    pub const MOD_MOD: u32 = 6;
    pub const MOD_MOD2X: u32 = 7;
    pub const MOD_ADD: u32 = 8;
    pub const MOD_MOD2X_NA: u32 = 9;
    pub const MOD_ADD_NA: u32 = 10;
    pub const MOD_OPAQUE: u32 = 11;
    pub const OPAQUE_MOD2X_NA_ALPHA: u32 = 12;
    pub const OPAQUE_ADD_ALPHA: u32 = 13;
    pub const OPAQUE_ADD_ALPHA_ALPHA: u32 = 14;
    pub const OPAQUE_MOD2X_NA_ALPHA_ADD: u32 = 15;
    pub const MOD_ADD_ALPHA: u32 = 16;
    pub const MOD_ADD_ALPHA_ALPHA: u32 = 17;
    pub const OPAQUE_ALPHA_ALPHA: u32 = 18;
    pub const OPAQUE_MOD2X_NA_ALPHA_3S: u32 = 19;
    pub const OPAQUE_ADD_ALPHA_WGT: u32 = 20;
    pub const MOD_ADD_ALPHA_ALPHA2: u32 = 21;
    pub const OPAQUE_MOD_NA_ALPHA: u32 = 22;
    pub const MOD_ADD_ALPHA_WGT: u32 = 23;
    pub const OPAQUE_MOD_ADD_WGT: u32 = 24;
    pub const OPAQUE_MOD2X_NA_ALPHA_UNSH_ALPHA: u32 = 25;
    pub const MOD_DUAL_CROSSFADE: u32 = 26;
    pub const OPAQUE_MOD2X_NA_ALPHA_ALPHA: u32 = 27;
    pub const MOD_MASKED_DUAL_CROSSFADE: u32 = 28;
    pub const OPAQUE_ALPHA: u32 = 29;
    pub const GUILD: u32 = 30;
    pub const GUILD_NO_BORDER: u32 = 31;
    pub const GUILD_OPAQUE: u32 = 32;
    pub const MOD_DEPTH: u32 = 33;
    pub const ILLUM: u32 = 34;
    pub const MOD_MOD_MOD_CONST: u32 = 35;
    pub const MOD_MOD_DEPTH: u32 = 36;
}

/// The shaders of IDs with the high bit set (`0x8000 | index`): pixel,
/// vertex shader, and the textures they take.
const TABLE: [(u32, u32, usize); 36] = {
    use ps::*;
    use vs::*;
    [
        (OPAQUE_MOD2X_NA_ALPHA, T1_ENV, 2),
        (OPAQUE_ADD_ALPHA, T1_ENV, 2),
        (OPAQUE_ADD_ALPHA_ALPHA, T1_ENV, 2),
        (OPAQUE_MOD2X_NA_ALPHA_ADD, T1_ENV_T1, 3),
        (MOD_ADD_ALPHA, T1_ENV, 2),
        (OPAQUE_ADD_ALPHA, T1_T1, 2),
        (MOD_ADD_ALPHA, T1_T1, 2),
        (MOD_ADD_ALPHA_ALPHA, T1_ENV, 2),
        (OPAQUE_ALPHA_ALPHA, T1_ENV, 2),
        (OPAQUE_MOD2X_NA_ALPHA_3S, T1_ENV_T1, 3),
        (OPAQUE_ADD_ALPHA_WGT, T1_T1, 2),
        (MOD_ADD_ALPHA_ALPHA2, T1_ENV, 2),
        (OPAQUE_MOD_NA_ALPHA, T1_ENV, 2),
        (MOD_ADD_ALPHA_WGT, T1_ENV, 2),
        (MOD_ADD_ALPHA_WGT, T1_T1, 2),
        (OPAQUE_ADD_ALPHA_WGT, T1_T2, 2),
        (OPAQUE_MOD_ADD_WGT, T1_ENV, 2),
        (OPAQUE_MOD2X_NA_ALPHA_UNSH_ALPHA, T1_ENV_T1, 3),
        (MOD_DUAL_CROSSFADE, T1, 1),
        (MOD_DEPTH, EDGE_FADE_T1, 1),
        (OPAQUE_MOD2X_NA_ALPHA_ALPHA, T1_ENV_T2, 3),
        (MOD_MOD, EDGE_FADE_T1_T2, 2),
        (MOD_MASKED_DUAL_CROSSFADE, T1_T2, 2),
        (OPAQUE_ALPHA, T1_T1, 2),
        (OPAQUE_MOD2X_NA_ALPHA_UNSH_ALPHA, T1_ENV_T2, 3),
        (MOD_DEPTH, EDGE_FADE_ENV, 1),
        (GUILD, T1_T2_T1, 3),
        (GUILD_NO_BORDER, T1_T2, 2),
        (GUILD_OPAQUE, T1_T2_T1, 3),
        (ILLUM, T1_T1, 2),
        (MOD_MOD_MOD_CONST, T1_T2_T3, 3),
        (MOD_MOD_MOD_CONST, COLOR_T1_T2_T3, 3),
        (OPAQUE, T1, 1),
        (MOD_MOD2X, EDGE_FADE_T1_T2, 2),
        (MOD, EDGE_FADE_T1, 1),
        (MOD_MOD_DEPTH, EDGE_FADE_T1_T2, 2),
    ]
};

/// A batch's shaders: (pixel, vertex), by its shader ID and texture count.
pub(crate) fn shaders(shader: u16, textures: u16) -> (u32, u32) {
    if shader & 0x8000 != 0 {
        return match TABLE.get((shader & 0x7fff) as usize) {
            Some(&(p, v, _)) => (p, v),
            None => (ps::OPAQUE, vs::T1),
        };
    }
    let vertex = if textures <= 1 {
        if shader & 0x80 != 0 {
            vs::ENV
        } else if shader & 0x4000 != 0 {
            vs::T2
        } else {
            vs::T1
        }
    } else if shader & 0x80 != 0 {
        if shader & 0x8 != 0 {
            vs::ENV_ENV
        } else {
            vs::ENV_T1
        }
    } else if shader & 0x8 != 0 {
        vs::T1_ENV
    } else if shader & 0x4000 != 0 {
        vs::T1_T2
    } else {
        vs::T1_T1
    };
    let pixel = if textures <= 1 {
        if shader & 0x70 != 0 {
            ps::MOD
        } else {
            ps::OPAQUE
        }
    } else if shader & 0x70 != 0 {
        match shader & 7 {
            3 => ps::MOD_ADD,
            4 => ps::MOD_MOD2X,
            6 => ps::MOD_MOD2X_NA,
            7 => ps::MOD_ADD_NA,
            _ => ps::MOD_MOD,
        }
    } else {
        match shader & 7 {
            0 => ps::OPAQUE_OPAQUE,
            3 | 7 => ps::OPAQUE_ADD_ALPHA,
            4 => ps::OPAQUE_MOD2X,
            6 => ps::OPAQUE_MOD2X_NA,
            _ => ps::OPAQUE_MOD,
        }
    };
    (pixel, vertex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_ids_pick_their_combiners() {
        assert_eq!(shaders(0, 1), (ps::OPAQUE, vs::T1));
        assert_eq!(shaders(0x10, 1), (ps::MOD, vs::T1));
        assert_eq!(shaders(0x8000, 2), (ps::OPAQUE_MOD2X_NA_ALPHA, vs::T1_ENV));
        assert_eq!(shaders(0x8000 | 34, 1), (ps::MOD, vs::EDGE_FADE_T1));
        assert_eq!(
            shaders(0x0008 | 0x0003, 2),
            (ps::OPAQUE_ADD_ALPHA, vs::T1_ENV)
        );
    }
}
