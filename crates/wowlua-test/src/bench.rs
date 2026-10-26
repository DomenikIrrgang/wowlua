//! `wowlua bench` (spec Tooling §16.1.7, decision T221): the `@bench`
//! functions of the test files, run on the embedded Lua 5.1 in the build
//! the profile optimizes and in the same build with every optimization
//! off, to compare them.
//!
//! Each test file loads in both builds (two VMs, the file's `@beforeAll`
//! hooks run in each). A benchmark is called in batches, as many calls a
//! batch as make it last the batch time in the unoptimized build; the two
//! builds take turns batch by batch, each after a full collection, so a
//! machine that slows down slows both. A benchmark's time is the median of
//! its batches, per call, with the median distance of the batches from it
//! as its spread.

use super::*;
use std::time::Duration;

/// How benchmarks are measured.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Batches per build.
    pub samples: usize,
    /// How long a batch should take.
    pub batch: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            samples: 10,
            batch: Duration::from_millis(20),
        }
    }
}

/// A benchmark's time in one build.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timing {
    /// The median time of a call, in nanoseconds.
    pub nanos: f64,
    /// The median distance of the batches from it, relative.
    pub spread: f64,
}

#[derive(Clone, Debug)]
pub struct BenchResult {
    pub name: String,
    /// Not run: why (`@skip`'s reason, or the error it raised).
    pub skipped: Option<String>,
    pub error: Option<String>,
    /// Calls per batch.
    pub calls: u64,
    pub unoptimized: Timing,
    pub optimized: Timing,
}

impl BenchResult {
    /// How many times faster the optimized build runs it.
    pub fn speedup(&self) -> f64 {
        if self.optimized.nanos > 0.0 {
            self.unoptimized.nanos / self.optimized.nanos
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct BenchFile {
    pub file: String,
    pub flavor: String,
    pub benches: Vec<BenchResult>,
    /// The file could not run.
    pub error: Option<String>,
}

/// One benchmark of a loaded file.
struct Entry {
    index: usize,
    name: String,
    skip: Option<String>,
}

fn benches(
    vm: &mut Vm,
    tests: TableRef,
    flavor: &str,
    file: &str,
    filters: &[String],
) -> Vec<Entry> {
    let count = get(vm, tests, "benchCount");
    let n = match vm.call(count, &[]).ok().and_then(|r| r.first().copied()) {
        Some(Value::Num(n)) => n as usize,
        _ => 0,
    };
    let info = get(vm, tests, "benchInfo");
    let flavor_v = vm.str(flavor);
    let mut out = Vec::new();
    for i in 1..=n {
        let Ok(r) = vm.call(info, &[Value::Num(i as f64), flavor_v]) else {
            continue;
        };
        let name = r.first().map(|&v| text(vm, v)).unwrap_or_default();
        let skip = match r.get(3) {
            Some(&v @ Value::Str(_)) => Some(text(vm, v)),
            _ => None,
        };
        let runs = matches!(r.get(4), Some(Value::Bool(true)));
        if !runs {
            continue;
        }
        if !filters.is_empty() && !filters.iter().any(|f| selects(f, file, &name, &name)) {
            continue;
        }
        out.push(Entry {
            index: i,
            name,
            skip,
        });
    }
    out
}

/// A loaded test file, its `@beforeAll` hooks run.
fn open(suite: &Suite, file: &TestFile) -> Result<Loaded, String> {
    natives::reset();
    let mut state = load(suite, file, None).map_err(|e| to_sources(suite, &e))?;
    let tests = state.tests;
    let job = start(&mut state.vm, tests, "beforeAll", None, None)?;
    if let Outcome::Fail { message, .. } = drive(&mut state.vm, tests, job) {
        return Err(to_sources(suite, &format!("@beforeAll failed: {message}")));
    }
    Ok(state)
}

/// Runs the file's `@beforeEach` or `@afterEach` hooks.
fn hooks(state: &mut Loaded, suite: &Suite, kind: &str) -> Result<(), String> {
    let tests = state.tests;
    let job = start(&mut state.vm, tests, kind, None, None)?;
    match drive(&mut state.vm, tests, job) {
        Outcome::Fail { message, .. } => {
            Err(to_sources(suite, &format!("@{kind} failed: {message}")))
        }
        _ => Ok(()),
    }
}

/// One batch: `calls` calls of benchmark `index`, after a collection.
fn batch(state: &mut Loaded, suite: &Suite, index: usize, calls: u64) -> Result<Duration, String> {
    state
        .vm
        .collect()
        .map_err(|e| error_text(&mut state.vm, &e))?;
    let tests = state.tests;
    let run = get(&mut state.vm, tests, "benchRun");
    let args = [Value::Num(index as f64), Value::Num(calls as f64)];
    let watch = wowlua_host::Stopwatch::start();
    let r = state.vm.call(run, &args);
    let took = watch.elapsed();
    // (what it printed would only pile up)
    let _ = natives::take_printed();
    match r {
        Ok(_) => Ok(took),
        Err(e) => {
            let message = error_text(&mut state.vm, &e);
            Err(to_sources(suite, &message))
        }
    }
}

fn timing(mut batches: Vec<Duration>, calls: u64) -> Timing {
    let per_call: Vec<f64> = {
        batches.sort();
        batches
            .iter()
            .map(|d| d.as_nanos() as f64 / calls.max(1) as f64)
            .collect()
    };
    let median = |v: &[f64]| -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        let mut v = v.to_vec();
        v.sort_by(|a, b| a.total_cmp(b));
        let m = v.len() / 2;
        if v.len().is_multiple_of(2) {
            (v[m - 1] + v[m]) / 2.0
        } else {
            v[m]
        }
    };
    let nanos = median(&per_call);
    let deviations: Vec<f64> = per_call.iter().map(|x| (x - nanos).abs()).collect();
    let spread = if nanos > 0.0 {
        median(&deviations) / nanos
    } else {
        0.0
    };
    Timing { nanos, spread }
}

/// Measures one benchmark in both builds.
fn measure(
    unoptimized: (&mut Loaded, &Suite),
    optimized: (&mut Loaded, &Suite),
    entry: &Entry,
    options: &Options,
) -> Result<(u64, Timing, Timing), String> {
    let (plain, plain_suite) = unoptimized;
    let (fast, fast_suite) = optimized;
    hooks(plain, plain_suite, "beforeEach")?;
    hooks(fast, fast_suite, "beforeEach")?;
    // Calls per batch: doubled until a batch of the unoptimized build
    // lasts the batch time (a warm-up for both).
    let mut calls: u64 = 1;
    loop {
        let took = batch(plain, plain_suite, entry.index, calls)?;
        batch(fast, fast_suite, entry.index, calls)?;
        if took >= options.batch || calls >= 1 << 30 {
            break;
        }
        let grow = if took.is_zero() {
            16.0
        } else {
            (options.batch.as_secs_f64() / took.as_secs_f64()).clamp(1.5, 16.0)
        };
        calls = ((calls as f64) * grow).ceil() as u64;
    }
    let mut a = Vec::with_capacity(options.samples);
    let mut b = Vec::with_capacity(options.samples);
    for s in 0..options.samples.max(1) {
        // In turns, first one build, then the other.
        if s.is_multiple_of(2) {
            a.push(batch(plain, plain_suite, entry.index, calls)?);
            b.push(batch(fast, fast_suite, entry.index, calls)?);
        } else {
            b.push(batch(fast, fast_suite, entry.index, calls)?);
            a.push(batch(plain, plain_suite, entry.index, calls)?);
        }
    }
    hooks(plain, plain_suite, "afterEach")?;
    hooks(fast, fast_suite, "afterEach")?;
    Ok((calls, timing(a, calls), timing(b, calls)))
}

/// Runs the benchmarks of the test files of `unoptimized` (the same files
/// are in `optimized`) whose path or names contain one of `filters`, one
/// file at a time, one benchmark at a time.
pub fn run(
    unoptimized: &Suite,
    optimized: &Suite,
    filters: &[String],
    options: &Options,
) -> Vec<BenchFile> {
    let mut out = Vec::new();
    for file in unoptimized
        .build
        .tests
        .iter()
        .filter(|t| t.role == TestRole::Test)
        .filter(|t| selectable(unoptimized, t, filters))
    {
        let shown = wowlua_driver::examples::source_of(&file.rel);
        let mut result = BenchFile {
            file: shown.clone(),
            flavor: unoptimized.flavor.clone(),
            benches: Vec::new(),
            error: None,
        };
        let Some(other) = optimized.build.tests.iter().find(|t| t.rel == file.rel) else {
            continue;
        };
        let loaded = open(unoptimized, file).and_then(|a| open(optimized, other).map(|b| (a, b)));
        let (mut plain, mut fast) = match loaded {
            Ok(both) => both,
            Err(e) => {
                result.error = Some(e);
                out.push(result);
                continue;
            }
        };
        let entries = benches(
            &mut plain.vm,
            plain.tests,
            &unoptimized.flavor,
            &shown,
            filters,
        );
        if entries.is_empty() {
            continue;
        }
        for entry in &entries {
            let mut r = BenchResult {
                name: entry.name.clone(),
                skipped: entry.skip.clone(),
                error: None,
                calls: 0,
                unoptimized: Timing::default(),
                optimized: Timing::default(),
            };
            if r.skipped.is_none() {
                match measure(
                    (&mut plain, unoptimized),
                    (&mut fast, optimized),
                    entry,
                    options,
                ) {
                    Ok((calls, a, b)) => {
                        r.calls = calls;
                        r.unoptimized = a;
                        r.optimized = b;
                    }
                    Err(e) => r.error = Some(e),
                }
            }
            result.benches.push(r);
        }
        for (state, suite) in [(&mut plain, unoptimized), (&mut fast, optimized)] {
            let tests = state.tests;
            if let Ok(job) = start(&mut state.vm, tests, "afterAll", None, None)
                && let Outcome::Fail { message, .. } = drive(&mut state.vm, tests, job)
            {
                result.error = Some(to_sources(suite, &format!("@afterAll failed: {message}")));
            }
        }
        out.push(result);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timings_are_medians_with_their_spread() {
        let ms = Duration::from_millis;
        let t = timing(vec![ms(10), ms(12), ms(11), ms(30), ms(11)], 1000);
        assert_eq!(t.nanos, 11_000.0);
        // Distances 1, 1, 0, 19, 0 ms: their median is 1 ms.
        assert!((t.spread - 1.0 / 11.0).abs() < 1e-9, "{t:?}");
    }
}
