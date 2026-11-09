//! What a profiled test run measures besides the profile (decisions T610,
//! T612): the game frames each test ran, and the memory each addon holds.

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::{Value as Json, json};
use wowlua_vm::Vm;

use crate::Suite;

/// A test's game frames, by number (`first` to `last`; none when
/// `first > last`).
#[derive(Clone, Debug, PartialEq)]
pub struct TestFrames {
    pub name: String,
    pub first: u64,
    pub last: u64,
}

thread_local! {
    /// The running file's tests so far (a file runs on one thread).
    static TESTS: RefCell<Vec<TestFrames>> = const { RefCell::new(Vec::new()) };
}

fn frame_count(vm: &Vm) -> u64 {
    vm.profiler().map_or(0, |p| p.frame_count())
}

/// Records the frames each test of a profiled run runs (its hooks'
/// included), for [`take_tests`].
pub fn track_tests(suite: &mut Suite) {
    let before = suite.before_test.take();
    suite.before_test = Some(Arc::new(move |vm: &mut Vm, name: &str| {
        if let Some(f) = &before {
            f(vm, name);
        }
        let next = frame_count(vm) + 1;
        TESTS.with(|t| {
            t.borrow_mut().push(TestFrames {
                name: name.to_string(),
                first: next,
                last: next - 1,
            })
        });
    }));
    let after = suite.after_test.take();
    suite.after_test = Some(Arc::new(move |vm: &mut Vm, name: &str| {
        let last = frame_count(vm);
        TESTS.with(|t| {
            if let Some(test) = t.borrow_mut().last_mut() {
                test.last = last;
            }
        });
        if let Some(f) = &after {
            f(vm, name);
        }
    }));
}

/// A file starts: what an earlier file on this thread left (one that did
/// not load) goes.
pub(crate) fn reset() {
    TESTS.with(|t| t.borrow_mut().clear());
}

/// The tests [`track_tests`] recorded on this thread since the last call.
pub fn take_tests() -> Vec<TestFrames> {
    TESTS.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

/// The tests' frames as a summary lists them.
pub fn tests_json(tests: &[TestFrames]) -> Json {
    json!(
        tests
            .iter()
            .map(|t| json!({ "name": t.name, "first": t.first, "last": t.last }))
            .collect::<Vec<_>>()
    )
}

/// The memory after a full collection: the Lua state's (as
/// `collectgarbage("count")` says) and each addon's (as
/// `GetAddOnMemoryUsage` says, decision T610), in KiB.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Memory {
    pub total_kb: f64,
    /// By addon, in name order; an addon whose code made nothing live is
    /// left out.
    pub addons: Vec<(String, f64)>,
}

impl Memory {
    pub fn to_json(&self) -> Json {
        let mut addons = serde_json::Map::new();
        for (name, kb) in &self.addons {
            addons.insert(name.clone(), json!(round(*kb)));
        }
        json!({ "total": round(self.total_kb), "addons": addons })
    }
}

fn round(kb: f64) -> f64 {
    (kb * 10.0).round() / 10.0
}

/// Measures the memory now, after a full collection.
pub fn memory(vm: &mut Vm) -> Memory {
    let _ = vm.collect();
    let names: Vec<String> = wowlua_engine::engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .map(|a| a.name.to_lowercase())
        .collect();
    Memory {
        total_kb: vm.heap.estimate_bytes() as f64 / 1024.0,
        addons: vm
            .memory_by_taint()
            .into_iter()
            .filter(|(name, _)| names.contains(&name.to_lowercase()))
            .map(|(name, bytes)| (name, bytes as f64 / 1024.0))
            .collect(),
    }
}
