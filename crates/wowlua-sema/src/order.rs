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

/// The strongly connected components of the graph over `0..n`, ordered so
/// that every edge `(a, b)` between two components puts `a`'s first. Ties
/// go to the component whose lowest node is lower, and each component
/// lists its nodes in order: without cycles, this is `stable_topo` with
/// `rank[i] = i`.
pub fn components_in_order(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let sccs = strongly_connected(n, edges.iter().copied());
    let mut component = vec![0; n];
    for (c, members) in sccs.iter().enumerate() {
        for &m in members {
            component[m] = c;
        }
    }
    let between: Vec<(usize, usize)> = edges
        .iter()
        .map(|&(a, b)| (component[a], component[b]))
        .filter(|(a, b)| a != b)
        .collect();
    let rank: Vec<usize> = sccs.iter().map(|m| m[0]).collect();
    let order = stable_topo(sccs.len(), &rank, &between).expect("components form no cycle");
    let mut sccs: Vec<Option<Vec<usize>>> = sccs.into_iter().map(Some).collect();
    order
        .into_iter()
        .map(|c| sccs[c].take().expect("each component once"))
        .collect()
}

/// The strongly connected components of the graph over `0..n`, each
/// sorted, in the reverse of a topological order.
pub fn strongly_connected(
    n: usize,
    edges: impl IntoIterator<Item = (usize, usize)>,
) -> Vec<Vec<usize>> {
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, to) in edges {
        next[from].push(to);
    }
    // Iterative Tarjan.
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut counter = 0;
    let mut out = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut i)) = call.last_mut() {
            if *i < next[v].len() {
                let w = next[v][*i];
                *i += 1;
                if index[w] == usize::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut members = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        members.push(w);
                        if w == v {
                            break;
                        }
                    }
                    members.sort();
                    out.push(members);
                }
            }
        }
    }
    out
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

    #[test]
    fn orders_components_and_keeps_cycles_together() {
        // 3 and 1 import each other; 2 imports nothing, 0 imports 1.
        assert_eq!(
            components_in_order(4, &[(3, 1), (1, 3), (1, 0)]),
            vec![vec![1, 3], vec![0], vec![2]]
        );
        // Without cycles, the same order as `stable_topo`.
        let edges = [(3, 1), (2, 0)];
        let flat: Vec<usize> = components_in_order(4, &edges).concat();
        assert_eq!(Ok(flat), stable_topo(4, &[0, 1, 2, 3], &edges));
        // A module the cycle imports comes first.
        assert_eq!(
            components_in_order(3, &[(0, 1), (1, 0), (2, 0)]),
            vec![vec![2], vec![0, 1]]
        );
    }
}
