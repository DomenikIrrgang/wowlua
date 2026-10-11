//! Deterministic topological ordering (spec §5.4.4–§5.4.5): among the files
//! that are ready, the preferred position (from `build.load-order` and
//! source paths) decides.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Orders `0..n` so that every edge `(a, b)` puts `a` before `b`, keeping
/// the preferred order (`rank[i]`, lower first) wherever the edges allow.
/// On a cycle, returns the nodes of one cycle in order.
pub fn stable_topo(
    n: usize,
    rank: &[usize],
    edges: &[(usize, usize)],
) -> Result<Vec<usize>, Vec<usize>> {
    let mut indegree = vec![0usize; n];
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in edges {
        if a != b {
            next[a].push(b);
            indegree[b] += 1;
        }
    }
    let mut ready: BinaryHeap<Reverse<(usize, usize)>> = (0..n)
        .filter(|&i| indegree[i] == 0)
        .map(|i| Reverse((rank[i], i)))
        .collect();
    let mut order = Vec::with_capacity(n);
    while let Some(Reverse((_, i))) = ready.pop() {
        order.push(i);
        for &j in &next[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                ready.push(Reverse((rank[j], j)));
            }
        }
    }
    if order.len() == n {
        return Ok(order);
    }
    Err(find_cycle(n, &next, &indegree))
}

/// One cycle among the nodes left with incoming edges. Every such node has
/// a predecessor that is also left, so walking predecessors finds a cycle.
fn find_cycle(n: usize, next: &[Vec<usize>], indegree: &[usize]) -> Vec<usize> {
    let remaining: Vec<bool> = (0..n).map(|i| indegree[i] > 0).collect();
    let mut prev: Vec<Option<usize>> = vec![None; n];
    for (a, targets) in next.iter().enumerate() {
        for &b in targets {
            if remaining[a] && remaining[b] && prev[b].is_none() {
                prev[b] = Some(a);
            }
        }
    }
    let Some(start) = (0..n).find(|&i| remaining[i]) else {
        return Vec::new();
    };
    let mut seen = vec![false; n];
    let mut current = start;
    while !seen[current] {
        seen[current] = true;
        match prev[current] {
            Some(p) => current = p,
            None => return vec![current],
        }
    }
    // `current` is on the cycle; collect it backwards, then reverse.
    let mut cycle = vec![current];
    let mut p = prev[current].expect("cycle");
    while p != current {
        cycle.push(p);
        p = prev[p].expect("cycle");
    }
    cycle.reverse();
    cycle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_preferred_order_where_edges_allow() {
        // Preferred 0,1,2,3; edge 3 -> 1 moves 3 before 1.
        assert_eq!(
            stable_topo(4, &[0, 1, 2, 3], &[(3, 1)]),
            Ok(vec![0, 2, 3, 1])
        );
        assert_eq!(stable_topo(3, &[2, 1, 0], &[]), Ok(vec![2, 1, 0]));
        let cycle = stable_topo(3, &[0, 1, 2], &[(0, 1), (1, 2), (2, 1)]).unwrap_err();
        assert_eq!(cycle.len(), 2);
        // A node downstream of the cycle is not reported as one.
        let cycle = stable_topo(4, &[0, 1, 2, 3], &[(1, 0), (2, 1), (1, 2)]).unwrap_err();
        assert_eq!(cycle.len(), 2);
        assert!(!cycle.contains(&0));
    }
}
