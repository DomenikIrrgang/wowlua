//! Salsa20 (20 rounds, a 16-byte key): what the storage's encrypted chunks
//! are encrypted with.

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[b] ^= s[a].wrapping_add(s[d]).rotate_left(7);
    s[c] ^= s[b].wrapping_add(s[a]).rotate_left(9);
    s[d] ^= s[c].wrapping_add(s[b]).rotate_left(13);
    s[a] ^= s[d].wrapping_add(s[c]).rotate_left(18);
}

fn block(input: &[u32; 16]) -> [u8; 64] {
    let mut s = *input;
    for _ in 0..10 {
        quarter(&mut s, 0, 4, 8, 12);
        quarter(&mut s, 5, 9, 13, 1);
        quarter(&mut s, 10, 14, 2, 6);
        quarter(&mut s, 15, 3, 7, 11);
        quarter(&mut s, 0, 1, 2, 3);
        quarter(&mut s, 5, 6, 7, 4);
        quarter(&mut s, 10, 11, 8, 9);
        quarter(&mut s, 15, 12, 13, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&s[i].wrapping_add(input[i]).to_le_bytes());
    }
    out
}

/// Encrypts or decrypts `data` in place.
pub fn apply(key: &[u8; 16], nonce: &[u8; 8], data: &mut [u8]) {
    let w = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    // "expand 16-byte k"
    let tau = [0x61707865, 0x3120646e, 0x79622d36, 0x6b206574];
    let mut state = [
        tau[0],
        w(&key[0..]),
        w(&key[4..]),
        w(&key[8..]),
        w(&key[12..]),
        tau[1],
        w(&nonce[0..]),
        w(&nonce[4..]),
        0,
        0,
        tau[2],
        w(&key[0..]),
        w(&key[4..]),
        w(&key[8..]),
        w(&key[12..]),
        tau[3],
    ];
    for chunk in data.chunks_mut(64) {
        let ks = block(&state);
        for (b, k) in chunk.iter_mut().zip(ks) {
            *b ^= k;
        }
        state[8] = state[8].wrapping_add(1);
        if state[8] == 0 {
            state[9] = state[9].wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn matches_the_reference_vector() {
        // The ECRYPT test vector set 1, vector 0 (128-bit key 80 00 … 00,
        // IV 0): the first 64 bytes of the stream.
        let mut key = [0u8; 16];
        key[0] = 0x80;
        let mut data = [0u8; 64];
        super::apply(&key, &[0; 8], &mut data);
        let hex: String = data[..16].iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(hex, "4DFA5E481DA23EA09A31022050859936");
    }
}
