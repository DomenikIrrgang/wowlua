//! `cargo xtask parse-stats <dir>` (#19): parses every `.lua` and `.wowlua`
//! file below a directory, keeps every tree alive, and reports what the
//! trees hold: tokens and trivia by kind, and the heap they take, by array
//! of the flat trees and in total (from a counting allocator, which also
//! counts the allocations).

use anyhow::{Result, bail};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Counts live bytes and allocations; everything else is the system's.
pub struct Counting;

static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call forwards to `System` with the same arguments; the
// counters are plain atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let live =
                LIVE.fetch_add(layout.size() as u64, Ordering::Relaxed) + layout.size() as u64;
            PEAK.fetch_max(live, Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            let live = LIVE.fetch_add(new_size as u64, Ordering::Relaxed) + new_size as u64
                - layout.size() as u64;
            LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
            PEAK.fetch_max(live, Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
}

pub fn run(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let mut files: Vec<_> = walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("lua" | "wowlua")
            ) && !p
                .components()
                .any(|c| matches!(c.as_os_str().to_str(), Some("dist" | "target")))
        })
        .collect();
    files.sort();
    let sources: Vec<Vec<u8>> = files.iter().map(std::fs::read).collect::<Result<_, _>>()?;
    let source_bytes: u64 = sources.iter().map(|s| s.len() as u64).sum();
    let mut db = wowlua_span::SourceDb::new();
    let ids: Vec<_> = files.iter().map(|p| db.add(p.as_path(), "")).collect();

    // Parse with a large stack (deep chains), keeping every tree.
    let (parses, live, allocs, peak, secs) = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let live0 = LIVE.load(Ordering::Relaxed);
            let allocs0 = ALLOCS.load(Ordering::Relaxed);
            PEAK.store(live0, Ordering::Relaxed);
            let started = std::time::Instant::now();
            let parses: Vec<wowlua_syntax::Parse> = sources
                .iter()
                .zip(&ids)
                .map(|(src, file)| wowlua_syntax::parse(src, *file))
                .collect();
            let secs = started.elapsed().as_secs_f64();
            let live = LIVE.load(Ordering::Relaxed) - live0;
            let allocs = ALLOCS.load(Ordering::Relaxed) - allocs0;
            let peak = PEAK.load(Ordering::Relaxed) - live0;
            (parses, live, allocs, peak, secs)
        })?
        .join()
        .expect("parsing");

    let mut bytes: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
    let mut add = |what: &'static str, b: usize, n: usize| {
        let e = bytes.entry(what).or_default();
        e.0 += b as u64;
        e.1 += n as u64;
    };
    let mut tokens: BTreeMap<String, u64> = BTreeMap::new();
    let mut trivia: BTreeMap<String, u64> = BTreeMap::new();
    let mut errors = 0;
    for p in &parses {
        errors += usize::from(p.has_errors());
        add("tokens", p.lexed.tokens.heap_bytes(), p.lexed.tokens.len());
        add(
            "trivia",
            std::mem::size_of_val(&p.lexed.trivia[..]),
            p.lexed.trivia.len(),
        );
        add(
            "line starts",
            std::mem::size_of_val(&p.lexed.line_starts[..]),
            p.lexed.line_starts.len(),
        );
        for (what, b, n) in p.tree_heap() {
            add(what, b, n);
        }
        for t in p.lexed.tokens.iter() {
            *tokens.entry(format!("{:?}", t.kind)).or_default() += 1;
        }
        for t in &p.lexed.trivia {
            *trivia.entry(format!("{:?}", t.kind)).or_default() += 1;
        }
    }

    let mb = |b: u64| b as f64 / 1_048_576.0;
    println!(
        "{} files, {:.1} MB of source, {} with errors; parsed in {:.2} s",
        parses.len(),
        mb(source_bytes),
        errors,
        secs
    );
    println!(
        "heap held by the trees (counting allocator): {:.1} MB in {} allocations ({:.1} bytes per source byte); peak while parsing {:.1} MB",
        mb(live),
        allocs,
        live as f64 / source_bytes.max(1) as f64,
        mb(peak)
    );
    let total: u64 = bytes.values().map(|(b, _)| b).sum();
    println!(
        "\nHEAP BY ARRAY (exact sizes, without allocator slack): {:.1} MB",
        mb(total)
    );
    let mut arrays: Vec<_> = bytes.iter().filter(|(_, (b, _))| *b > 0).collect();
    arrays.sort_by_key(|(_, (b, _))| std::cmp::Reverse(*b));
    for (k, (b, n)) in arrays {
        println!(
            "  {:>9.1} MB {:5.1}%  {:>11} items  {k}",
            mb(*b),
            100.0 * *b as f64 / total.max(1) as f64,
            n
        );
    }
    println!("\nTOKENS BY KIND ({} in all)", tokens.values().sum::<u64>());
    let mut t: Vec<_> = tokens.into_iter().collect();
    t.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
    for (k, v) in t.iter().take(20) {
        println!("  {v:>12}  {k}");
    }
    println!("\nTRIVIA BY KIND");
    for (k, v) in &trivia {
        println!("  {v:>12}  {k}");
    }
    Ok(())
}
