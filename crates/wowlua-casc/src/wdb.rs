//! The client's caches of what the server told it (`Cache/WDB/<locale>/`,
//! spec debugger §18.10, decision T322): creatures' display IDs, from
//! `creaturecache.wdb`, for creatures the client's own tables lack.

use std::collections::HashMap;

/// A cache's records: (ID, data).
fn records(cache: &[u8]) -> Vec<(u32, &[u8])> {
    let u = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(cache.get(at..at + 4)?.try_into().ok()?))
    };
    let mut out = Vec::new();
    if cache.get(..4) != Some(b"BOMW") {
        return out;
    }
    let mut pos = 24;
    while let (Some(id), Some(size)) = (u(pos), u(pos + 4)) {
        let size = size as usize;
        let Some(data) = cache.get(pos + 8..pos + 8 + size) else {
            break;
        };
        if size == 0 {
            break;
        }
        out.push((id, data));
        pos += 8 + size;
    }
    out
}

/// A creature record's displays: after its names, a count, the total
/// probability, then each display's ID, scale and probability.
fn displays(rec: &[u8]) -> Option<Vec<u32>> {
    let u = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(rec.get(at..at + 4)?.try_into().ok()?))
    };
    let f = |at: usize| u(at).map(f32::from_bits);
    for o in 0..rec.len().saturating_sub(16) {
        let (Some(count), Some(total)) = (u(o), f(o + 4)) else {
            break;
        };
        let count = count as usize;
        if !(1..=16).contains(&count) || !(total > 0.0 && total <= 100.01 * count as f32) {
            continue;
        }
        let ids: Option<Vec<u32>> = (0..count)
            .map(|k| {
                let at = o + 8 + 12 * k;
                let (id, scale, p) = (u(at)?, f(at + 4)?, f(at + 8)?);
                (id > 0
                    && id < 10_000_000
                    && scale > 0.01
                    && scale < 50.0
                    && (0.0..=100.01).contains(&p))
                .then_some(id)
            })
            .collect();
        if let Some(ids) = ids {
            return Some(ids);
        }
    }
    None
}

/// The creatures of a creature cache: their display IDs.
pub fn creature_displays(cache: &[u8]) -> HashMap<u32, Vec<u32>> {
    records(cache)
        .into_iter()
        .filter_map(|(id, rec)| Some((id, displays(rec)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_a_creatures_displays() {
        // A record: names, flags, then one display (12678, scale 1, 100%).
        let mut rec = vec![2, 0x80, 0, 0, 12, 0, 0, 0];
        rec.extend_from_slice(b"Raider Bork\0\0\0\0");
        rec.extend_from_slice(&1u32.to_le_bytes());
        rec.extend_from_slice(&100.0f32.to_le_bytes());
        rec.extend_from_slice(&12678u32.to_le_bytes());
        rec.extend_from_slice(&1.0f32.to_le_bytes());
        rec.extend_from_slice(&100.0f32.to_le_bytes());
        rec.extend_from_slice(&[0; 8]);
        let mut cache = b"BOMW".to_vec();
        cache.extend_from_slice(&[0; 20]);
        cache.extend_from_slice(&12796u32.to_le_bytes());
        cache.extend_from_slice(&(rec.len() as u32).to_le_bytes());
        cache.extend_from_slice(&rec);
        let d = super::creature_displays(&cache);
        assert_eq!(d.get(&12796), Some(&vec![12678]));
    }
}
