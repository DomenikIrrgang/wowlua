//! Searches over sorted position tables (token starts, line starts).

/// `slice.partition_point(pred)`, searched outward from `hint` (any value;
/// past the end counts as the end). `pred` must hold on a prefix of `slice`
/// and nowhere after it, so the point is unique and the hint only decides how
/// fast it is found: a few probes next to the hint instead of a binary search
/// over a whole large file.
pub fn partition_point_near<T>(slice: &[T], hint: usize, pred: impl Fn(&T) -> bool) -> usize {
    let hint = hint.min(slice.len());
    // The point lies in `lo..=hi`.
    let (mut lo, mut hi) = (0, slice.len());
    let mut step = 1;
    if hint == 0 || pred(&slice[hint - 1]) {
        // At `hint` or after it: probe `hint`, `hint + 1`, `hint + 3`, …
        lo = hint;
        loop {
            let probe = hint + step - 1;
            if probe >= hi {
                break;
            }
            if !pred(&slice[probe]) {
                hi = probe;
                break;
            }
            lo = probe + 1;
            step *= 2;
        }
    } else {
        // Before `hint - 1`: probe `hint - 2`, `hint - 3`, `hint - 5`, …
        hi = hint - 1;
        while step < hint {
            let probe = hint - 1 - step;
            if pred(&slice[probe]) {
                lo = probe + 1;
                break;
            }
            hi = probe;
            step *= 2;
        }
    }
    lo + slice[lo..hi].partition_point(&pred)
}

#[cfg(test)]
mod tests {
    use super::partition_point_near;

    #[test]
    fn partition_point_near_finds_the_point_from_every_hint() {
        let long: Vec<u32> = (0..200).map(|i| i / 3 * 2).collect();
        let slices: [&[u32]; 6] = [
            &[],
            &[0],
            &[4],
            &[0, 1, 2],
            &[0, 2, 2, 2, 5, 7, 7, 9, 12, 12, 12, 12, 20],
            &long,
        ];
        for slice in slices {
            let max = slice.last().copied().unwrap_or(0);
            for key in 0..=max + 2 {
                let below = slice.partition_point(|&s| s < key);
                let at_or_below = slice.partition_point(|&s| s <= key);
                for hint in (0..=slice.len() + 1).chain([usize::MAX]) {
                    assert_eq!(
                        partition_point_near(slice, hint, |&s| s < key),
                        below,
                        "{slice:?} < {key} from {hint}"
                    );
                    assert_eq!(
                        partition_point_near(slice, hint, |&s| s <= key),
                        at_or_below,
                        "{slice:?} <= {key} from {hint}"
                    );
                }
            }
        }
    }
}
