//! How deep the deepest expression chain is that IR this process built may hold (lowering notes `wowlua_syntax::deep`'s): the parser builds
//! operator and suffix chains (`a + b + …`, `a.b.c…`) in a loop, so its
//! nesting limit does not bound how deep they are, and a pass that recurses
//! over one needs stack in proportion. Passes start through [`run`], which
//! costs nothing until a deep chain exists.

use std::sync::atomic::{AtomicUsize, Ordering};

static LINKS: AtomicUsize = AtomicUsize::new(0);

/// Links of a chain from which it is deep.
pub const DEEP: usize = 1000;

/// Stack a recursive pass may take per link (debug builds included).
const PER_LINK: usize = 4096;

/// Notes a chain of `links` links.
pub fn note(links: usize) {
    if links >= DEEP {
        LINKS.fetch_max(links, Ordering::Relaxed);
    }
}

/// The links of the deepest chain noted so far, if it is deep.
pub fn links() -> usize {
    LINKS.load(Ordering::Relaxed)
}

/// Runs `f` with enough stack to recurse over the deepest chain noted.
pub fn run<R>(f: impl FnOnce() -> R) -> R {
    let links = links();
    if links < DEEP {
        return f();
    }
    let need = links.saturating_mul(PER_LINK).saturating_add(1 << 20);
    stacker::maybe_grow(need, need, f)
}
