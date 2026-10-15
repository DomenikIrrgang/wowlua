//! `rename-locals` (spec optimization §15.4.1): locals, parameters and
//! upvalues get the shortest free names, the most used first. Globals, table
//! fields and the implicit `self` keep their names, and so does `arg`: a Lua
//! 5.1 built with `LUA_COMPAT_VARARG` (not WoW's) declares it in every vararg
//! function, so keeping it keeps the code's meaning on any build.

use crate::ir::{Chunk, Pos};
use crate::scope::{rename, resolve_with_conflicts};
use std::collections::{HashMap, HashSet};

pub(super) const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// The `i`-th short name: `a`…`z`, `A`…`Z`, `_`, then two characters, ….
pub fn short_name(mut i: usize) -> String {
    const FIRST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_";
    const REST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789";
    let mut len = 1;
    let mut count = FIRST.len();
    while i >= count {
        i -= count;
        len += 1;
        count *= REST.len();
    }
    let mut out = vec![0u8; len];
    for slot in out.iter_mut().skip(1).rev() {
        *slot = REST[i % REST.len()];
        i /= REST.len();
    }
    out[0] = FIRST[i];
    String::from_utf8(out).expect("ASCII")
}

/// Returns the original names of the renamed identifiers by position.
pub fn apply(chunk: &mut Chunk) -> HashMap<Pos, String> {
    let res = resolve_with_conflicts(chunk);
    let mut order: Vec<usize> = (0..res.bindings.len()).collect();
    order.sort_by_key(|&b| std::cmp::Reverse(res.bindings[b].uses));
    let mut names: HashMap<usize, String> = HashMap::new();
    let fixed = |b: usize| res.bindings[b].implicit || res.bindings[b].name == "arg";
    for (b, binding) in res.bindings.iter().enumerate() {
        if fixed(b) {
            names.insert(b, binding.name.clone());
        }
    }
    for b in order {
        if fixed(b) {
            continue;
        }
        let taken: HashSet<&str> = res.conflicts[b]
            .iter()
            .filter_map(|c| names.get(c).map(String::as_str))
            .collect();
        let name = (0..)
            .map(short_name)
            .find(|n| {
                !KEYWORDS.contains(&n.as_str())
                    && n != "arg"
                    && !res.globals.contains(n)
                    && !taken.contains(n.as_str())
            })
            .expect("names are unbounded");
        names.insert(b, name);
    }
    rename(chunk, &res, &names)
}

#[cfg(test)]
mod tests {
    use super::short_name;

    #[test]
    fn short_names_grow() {
        assert_eq!(short_name(0), "a");
        assert_eq!(short_name(52), "_");
        assert_eq!(short_name(53), "aa");
        assert_eq!(short_name(53 + 63), "ba");
        assert_eq!(short_name(53 + 53 * 63), "aaa");
    }
}
