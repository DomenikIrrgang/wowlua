//! Bob Jenkins' `hashlittle2` (lookup3.c): the root file's name hashes.

#[inline]
fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *a = a.wrapping_sub(*c);
    *a ^= c.rotate_left(4);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= a.rotate_left(6);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= b.rotate_left(8);
    *b = b.wrapping_add(*a);
    *a = a.wrapping_sub(*c);
    *a ^= c.rotate_left(16);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= a.rotate_left(19);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= b.rotate_left(4);
    *b = b.wrapping_add(*a);
}

#[inline]
fn finish(a: &mut u32, b: &mut u32, c: &mut u32) {
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(14));
    *a ^= *c;
    *a = a.wrapping_sub(c.rotate_left(11));
    *b ^= *a;
    *b = b.wrapping_sub(a.rotate_left(25));
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(16));
    *a ^= *c;
    *a = a.wrapping_sub(c.rotate_left(4));
    *b ^= *a;
    *b = b.wrapping_sub(a.rotate_left(14));
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(24));
}

/// `hashlittle2(key, &pc, &pb)` with both seeds 0: `(pc, pb)`.
pub fn hashlittle2(key: &[u8]) -> (u32, u32) {
    let mut a: u32 = 0xdead_beef_u32.wrapping_add(key.len() as u32);
    let mut b = a;
    let mut c = a;
    let mut k = key;
    while k.len() > 12 {
        a = a.wrapping_add(u32::from_le_bytes([k[0], k[1], k[2], k[3]]));
        b = b.wrapping_add(u32::from_le_bytes([k[4], k[5], k[6], k[7]]));
        c = c.wrapping_add(u32::from_le_bytes([k[8], k[9], k[10], k[11]]));
        mix(&mut a, &mut b, &mut c);
        k = &k[12..];
    }
    if k.is_empty() {
        return (c, b);
    }
    let mut tail = [0u8; 12];
    tail[..k.len()].copy_from_slice(k);
    a = a.wrapping_add(u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]));
    b = b.wrapping_add(u32::from_le_bytes([tail[4], tail[5], tail[6], tail[7]]));
    c = c.wrapping_add(u32::from_le_bytes([tail[8], tail[9], tail[10], tail[11]]));
    finish(&mut a, &mut b, &mut c);
    (c, b)
}

/// The root file's hash of a path: upper case, backslashes.
pub fn name_hash(path: &str) -> u64 {
    let norm: Vec<u8> = path
        .bytes()
        .map(|b| match b {
            b'/' => b'\\',
            b => b.to_ascii_uppercase(),
        })
        .collect();
    let (pc, pb) = hashlittle2(&norm);
    (u64::from(pc) << 32) | u64::from(pb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_lookup3s_published_values() {
        // lookup3.c's driver5: "" → deadbeef deadbeef; "Four score and
        // seven years ago" → 17770551 ce7226e6 (c, b with seeds 0).
        assert_eq!(hashlittle2(b""), (0xdeadbeef, 0xdeadbeef));
        assert_eq!(
            hashlittle2(b"Four score and seven years ago"),
            (0x17770551, 0xce7226e6)
        );
    }
}
