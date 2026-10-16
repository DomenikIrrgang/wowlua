//! Per-file work on worker threads (#8). A region computes every item with
//! a pure job and merges the results on the calling thread, in item order,
//! so diagnostics and outputs are what running the items in order gives.
//! Work in flight is capped by a weight budget (source bytes standing for
//! the memory a job takes); items start largest first.

use std::cell::{Cell, RefCell};
use std::sync::{Condvar, Mutex};

/// The stack of a worker thread: the main thread's on macOS and Linux, and
/// then some (it only reserves address space). Deep inputs get more through
/// `wowlua_syntax::deep`.
const STACK: usize = 16 << 20;

thread_local! {
    static IN_JOB: Cell<bool> = const { Cell::new(false) };
    static PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Whether the current thread runs a job: a panic there is caught, and the
/// region runs again in order to report the panic as a sequential build
/// would.
pub fn in_job() -> bool {
    IN_JOB.with(Cell::get)
}

/// Keeps the rendered message of a panic in a job (the CLI's panic hook
/// calls it instead of reporting the panic).
pub fn record_panic(message: String) {
    PANIC.with(|p| *p.borrow_mut() = Some(message));
}

/// Why a region did not run in parallel.
#[derive(Debug)]
pub(crate) enum Failed {
    /// A job panicked (its message, for the nondeterminism check).
    Panicked(String),
    /// A job ran out of memory in the embedded Lua.
    OutOfMemory,
    /// No worker thread could be spawned; nothing ran.
    NoWorkers,
}

/// Runs `job` on every item on worker threads and returns the results in
/// item order. Items start largest first; a free worker takes the first
/// pending item whose weight keeps the weights in flight within `budget`
/// (one larger than the budget runs alone). After a job fails, no new job
/// starts; the running ones finish.
pub(crate) fn map<T: Send, R: Send>(
    threads: usize,
    budget: u64,
    items: Vec<T>,
    weight: impl Fn(&T) -> u64,
    job: impl Fn(T) -> R + Sync,
) -> Result<Vec<R>, Failed> {
    struct Queue {
        /// Item indices and weights, the heaviest first.
        pending: Vec<(usize, u64)>,
        in_flight: u64,
        running: usize,
        failed: Option<Failed>,
    }
    let mut pending: Vec<(usize, u64)> = items
        .iter()
        .enumerate()
        .map(|(i, t)| (i, weight(t)))
        .collect();
    pending.sort_by_key(|&(i, w)| (std::cmp::Reverse(w), i));
    let count = items.len();
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
    let results: Vec<Mutex<Option<R>>> = (0..count).map(|_| Mutex::new(None)).collect();
    let queue = Mutex::new(Queue {
        pending,
        in_flight: 0,
        running: 0,
        failed: None,
    });
    let ready = Condvar::new();
    let worker = || {
        loop {
            let (i, w) = {
                let mut q = queue.lock().expect("queue");
                loop {
                    if q.pending.is_empty() || q.failed.is_some() {
                        return;
                    }
                    let fits = q.pending.iter().position(|&(_, w)| {
                        q.running == 0 || q.in_flight.saturating_add(w) <= budget
                    });
                    if let Some(at) = fits {
                        let next = q.pending.remove(at);
                        q.in_flight += next.1;
                        q.running += 1;
                        break next;
                    }
                    q = ready.wait(q).expect("queue");
                }
            };
            let item = slots[i].lock().expect("slot").take().expect("taken once");
            IN_JOB.with(|f| f.set(true));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(item)));
            IN_JOB.with(|f| f.set(false));
            let failure = match &result {
                Ok(_) => None,
                Err(payload) if payload.is::<crate::LuaOutOfMemory>() => Some(Failed::OutOfMemory),
                Err(payload) => Some(Failed::Panicked(
                    PANIC.with(|p| p.borrow_mut().take()).unwrap_or_else(|| {
                        payload
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "a job panicked".into())
                    }),
                )),
            };
            {
                let mut q = queue.lock().expect("queue");
                q.in_flight -= w;
                q.running -= 1;
                if let Some(f) = failure
                    && q.failed.is_none()
                {
                    q.failed = Some(f);
                }
            }
            ready.notify_all();
            if let Ok(r) = result {
                *results[i].lock().expect("result") = Some(r);
            }
        }
    };
    let spawned = std::thread::scope(|scope| {
        let mut spawned = 0;
        for _ in 0..threads.min(count) {
            let started = std::thread::Builder::new()
                .name("wowlua-worker".into())
                .stack_size(STACK)
                .spawn_scoped(scope, worker);
            if started.is_ok() {
                spawned += 1;
            }
        }
        spawned
    });
    if spawned == 0 && count > 0 {
        return Err(Failed::NoWorkers);
    }
    if let Some(failed) = queue.into_inner().expect("queue").failed {
        return Err(failed);
    }
    Ok(results
        .into_iter()
        .map(|r| r.into_inner().expect("result").expect("every item ran"))
        .collect())
}

/// How many threads and how much memory parallel regions may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Jobs {
    /// Worker threads; `1` runs every region in order on the calling thread.
    pub threads: std::num::NonZeroUsize,
    /// Peak memory the work in flight may add, in bytes.
    pub memory: u64,
    /// Set explicitly (flag, environment or config): regions run in
    /// parallel even for small inputs.
    pub explicit: bool,
}

impl Default for Jobs {
    /// Up to 8 threads (the largest file caps what more could give, #8)
    /// and 1 GiB for the work in flight; small inputs stay on one thread.
    fn default() -> Jobs {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        Jobs {
            threads: std::num::NonZeroUsize::new(cores.min(8))
                .unwrap_or(std::num::NonZeroUsize::MIN),
            memory: 1 << 30,
            explicit: false,
        }
    }
}

impl Jobs {
    /// The jobs setting: `threads` (`--jobs`), else `WOWLUA_JOBS`, else
    /// `build.jobs` in `$WOWLUA_HOME/config.json`, else the default; the
    /// allowance from `WOWLUA_JOBS_MEMORY`, else `build.jobs-memory` (MiB),
    /// else 1 GiB. Any of them set makes it explicit.
    pub fn resolve(threads: Option<usize>) -> Jobs {
        let config: Option<serde_json::Value> = crate::install::wowlua_home()
            .and_then(|home| std::fs::read_to_string(home.join("config.json")).ok())
            .and_then(|text| serde_json::from_str(&text).ok());
        let config_number = |key: &str| config.as_ref()?.pointer(key)?.as_u64();
        let env_number = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
        };
        let mut jobs = Jobs::default();
        let threads = threads
            .map(|t| t as u64)
            .or_else(|| env_number("WOWLUA_JOBS"))
            .or_else(|| config_number("/build/jobs"));
        if let Some(t) = threads {
            jobs.threads =
                std::num::NonZeroUsize::new(t as usize).unwrap_or(std::num::NonZeroUsize::MIN);
            jobs.explicit = true;
        }
        if let Some(mib) =
            env_number("WOWLUA_JOBS_MEMORY").or_else(|| config_number("/build/jobs-memory"))
        {
            jobs.memory = mib.saturating_mul(1 << 20);
            jobs.explicit = true;
        }
        jobs
    }

    /// One thread: today's sequential code.
    pub fn one() -> Jobs {
        Jobs {
            threads: std::num::NonZeroUsize::MIN,
            memory: 0,
            explicit: true,
        }
    }

    /// Runs a region: `compute` every item and `merge` each result in item
    /// order. With one thread, or a total weight below `small` when not
    /// explicit, every item is computed and merged in turn on the calling
    /// thread (one result alive at a time). Otherwise the items are computed
    /// on workers within the memory allowance (`per_byte`: the memory a job
    /// takes per unit of weight), and merged after; if a job fails there,
    /// the region runs again in order, which reports a panic as a
    /// sequential build would.
    pub(crate) fn region<T: Send + Clone, R: Send>(
        &self,
        items: Vec<T>,
        weight: impl Fn(&T) -> u64,
        per_byte: u64,
        compute: impl Fn(T) -> R + Sync,
        mut merge: impl FnMut(R),
    ) {
        let total: u64 = items.iter().map(&weight).sum();
        let small = 256 * 1024;
        if self.threads.get() == 1 || items.len() < 2 || (!self.explicit && total < small) {
            for item in items {
                merge(compute(item));
            }
            return;
        }
        let budget = (self.memory / per_byte.max(1)).max(1);
        match map(self.threads.get(), budget, items.clone(), weight, &compute) {
            Ok(results) => results.into_iter().for_each(merge),
            Err(failed) => {
                let mut results = Vec::with_capacity(items.len());
                for item in items {
                    results.push(compute(item));
                }
                if let Failed::Panicked(message) = failed {
                    panic!(
                        "a parallel job panicked, but running the same work in order did not: {message}"
                    );
                }
                results.into_iter().for_each(merge);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    fn weights(n: usize) -> Vec<u64> {
        let mut x: u64 = 12345;
        (0..n)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                1 + (x >> 33) % 100
            })
            .collect()
    }

    /// Results come back in item order; the weights in flight stay within
    /// the budget unless one job runs alone.
    #[test]
    fn results_in_order_within_the_budget() {
        let items = weights(200);
        let budget = 150;
        let in_flight = AtomicU64::new(0);
        let running = AtomicUsize::new(0);
        let over = AtomicUsize::new(0);
        let out = map(
            8,
            budget,
            items.iter().copied().enumerate().collect(),
            |(_, w)| *w,
            |(i, w)| {
                let now = in_flight.fetch_add(w, Ordering::SeqCst) + w;
                let jobs = running.fetch_add(1, Ordering::SeqCst) + 1;
                if now > budget && jobs > 1 {
                    over.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_micros(w * 20));
                running.fetch_sub(1, Ordering::SeqCst);
                in_flight.fetch_sub(w, Ordering::SeqCst);
                i
            },
        )
        .unwrap();
        assert_eq!(out, (0..200).collect::<Vec<_>>());
        assert_eq!(over.load(Ordering::SeqCst), 0);
    }

    /// An item heavier than the budget runs alone.
    #[test]
    fn a_heavy_item_runs_alone() {
        let running = AtomicUsize::new(0);
        let alone = AtomicUsize::new(0);
        map(
            4,
            10,
            vec![100u64, 1, 1, 1, 1],
            |w| *w,
            |w| {
                let jobs = running.fetch_add(1, Ordering::SeqCst) + 1;
                if w == 100 && jobs == 1 {
                    alone.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                running.fetch_sub(1, Ordering::SeqCst);
            },
        )
        .unwrap();
        assert_eq!(alone.load(Ordering::SeqCst), 1);
    }

    /// A panicking job fails the region, and no job starts after it.
    #[test]
    fn a_panic_fails_the_region() {
        let started = AtomicUsize::new(0);
        let failed = map(
            1,
            1000,
            (0..10).collect(),
            |_| 1,
            |i: u32| {
                started.fetch_add(1, Ordering::SeqCst);
                if i == 3 {
                    panic!("job {i}");
                }
            },
        );
        assert!(
            matches!(&failed, Err(Failed::Panicked(m)) if m.contains("job 3")),
            "{failed:?}"
        );
        assert_eq!(
            started.load(Ordering::SeqCst),
            4,
            "jobs 0-3 ran, none after"
        );
    }

    /// Jobs run on workers, never on the calling thread.
    #[test]
    fn jobs_run_on_workers() {
        assert!(!in_job());
        let inside = map(2, 10, vec![1, 2, 3], |_| 1, |_| in_job()).unwrap();
        assert_eq!(inside, [true, true, true]);
        assert!(!in_job());
    }

    fn four() -> Jobs {
        Jobs {
            threads: std::num::NonZeroUsize::new(4).unwrap(),
            memory: 1 << 20,
            explicit: true,
        }
    }

    /// A region merges in item order, whatever order the jobs finish in.
    #[test]
    fn a_region_merges_in_order() {
        let mut merged = Vec::new();
        four().region(
            (0..50u64).collect(),
            |&i| 50 - i,
            1,
            |i| {
                std::thread::sleep(std::time::Duration::from_micros(i * 37 % 300));
                i * 2
            },
            |r| merged.push(r),
        );
        assert_eq!(merged, (0..50).map(|i| i * 2).collect::<Vec<_>>());
    }

    fn panic_message(f: impl FnOnce()) -> String {
        let payload =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err("panics");
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    /// A panic in a job is reported by running the region again in order:
    /// as a sequential build reports it.
    #[test]
    fn a_job_panic_reruns_in_order() {
        let message = panic_message(|| {
            four().region(
                (0..8).collect(),
                |_| 1,
                1,
                |i: u32| if i == 5 { panic!("file {i}") } else { i },
                |_| {},
            )
        });
        assert_eq!(message, "file 5");
        // One that only a worker hits is a bug of its own.
        let message = panic_message(|| {
            four().region(
                (0..8).collect(),
                |_| 1,
                1,
                |i: u32| {
                    if in_job() && i == 2 {
                        panic!("only parallel")
                    } else {
                        i
                    }
                },
                |_| {},
            )
        });
        assert!(
            message
                .starts_with("a parallel job panicked, but running the same work in order did not"),
            "{message}"
        );
    }

    /// Out of memory in a job: the region runs in order and keeps that.
    #[test]
    fn out_of_memory_reruns_in_order() {
        let mut merged = Vec::new();
        four().region(
            (0..8).collect(),
            |_| 1,
            1,
            |i: u32| {
                if in_job() && i == 3 {
                    std::panic::panic_any(crate::LuaOutOfMemory);
                }
                i
            },
            |r| merged.push(r),
        );
        assert_eq!(merged, (0..8).collect::<Vec<_>>());
    }

    /// A job can recurse deeper than 2 MiB of stack allows.
    #[test]
    fn jobs_have_a_large_stack() {
        #[inline(never)]
        fn deep(n: u32) -> u32 {
            let pad = std::hint::black_box([n; 64]);
            if n == 0 {
                pad[0]
            } else {
                deep(n - 1) + pad[1] - pad[1]
            }
        }
        let out = map(2, 10, vec![20_000u32, 20_000], |_| 1, deep).unwrap();
        assert_eq!(out, [0, 0]);
    }
}
