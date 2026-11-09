//! What a profiled test run measures besides the profile (decisions T610,
//! T612): the game frames each test ran, and the memory each addon holds;
//! and the perf scenarios' phases, results and baseline (`wowlua perf`,
//! decision T613).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value as Json, json};
use wowlua_vm::Vm;
use wowlua_vm::profile::{FrameSample, Key, NATIVE_NOTE, Profiler};

/// The baseline file, in the package (`--baseline` names another).
pub const BASELINE: &str = "tests/perf-baseline.json";

/// A change past this many percent is better or worse (`--threshold`).
pub const THRESHOLD: f64 = 2.0;

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
    MEMORY.with(|m| m.borrow_mut().clear());
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

// ------------------------------------------------------------ perf scenarios

/// Whether a test file is a perf scenario (decision T613): it calls
/// `config({ perf = true })` at its top level, read from its syntax
/// before anything loads.
pub fn is_scenario(source: &[u8]) -> bool {
    use wowlua_syntax::view::*;
    let file = wowlua_span::SourceDb::new().add("", "");
    let parse = wowlua_syntax::parse(source, file);
    parse.view().block().stmts().iter().any(|s| {
        let StmtKind::Call(call) = s.kind() else {
            return false;
        };
        let ExprKind::Call { callee, args } = call.kind() else {
            return false;
        };
        if !matches!(callee.kind(), ExprKind::Name(n) if n.text() == "config") {
            return false;
        }
        let Some(ExprKind::Table(fields)) = args.exprs().first().map(|e| e.kind()) else {
            return false;
        };
        fields.iter().any(|f| {
            matches!(f, TableField::Named { name, value, .. }
                if name.text() == "perf" && matches!(value.kind(), ExprKind::True))
        })
    })
}

/// A phase's measures by name (`instructions.mean`, `memory.MyAddon`, …):
/// counts, the same on every run (decision T613).
pub type Metrics = BTreeMap<String, f64>;

/// One phase of a scenario: a test of its file.
#[derive(Clone, Debug, PartialEq)]
pub struct Phase {
    pub name: String,
    /// The game frames it ran.
    pub frames: u64,
    pub metrics: Metrics,
    /// The frame where the addon cost the most (instructions and the
    /// native estimate): its number and its costliest functions.
    pub worst: Option<Worst>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Worst {
    pub frame: u64,
    /// Each function's name and its own instructions and native estimate
    /// in that frame.
    pub top: Vec<(String, u64)>,
}

/// A scenario file's phases, for one flavor.
#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    pub file: String,
    pub flavor: String,
    pub phases: Vec<Phase>,
}

/// Every scenario's phases: a run's results, or a baseline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Results {
    pub scenarios: Vec<Scenario>,
}

thread_local! {
    /// The running scenario's memory after each phase.
    static MEMORY: RefCell<Vec<Memory>> = const { RefCell::new(Vec::new()) };
}

/// Tracks a perf run: each test's frames ([`track_tests`]) and the memory
/// after it (after a full collection), for [`take_phases`].
pub fn track_phases(suite: &mut Suite) {
    track_tests(suite);
    let after = suite.after_test.take();
    suite.after_test = Some(Arc::new(move |vm: &mut Vm, name: &str| {
        if let Some(f) = &after {
            f(vm, name);
        }
        let m = memory(vm);
        MEMORY.with(|list| list.borrow_mut().push(m));
    }));
}

/// The phases of the scenario that just ran on this thread, from its
/// profile: `own` is the addon measured.
pub fn take_phases(p: &Profiler, own: &str, map: &dyn Fn(&[u8]) -> Option<String>) -> Vec<Phase> {
    let tests = take_tests();
    let memory = MEMORY.with(|m| std::mem::take(&mut *m.borrow_mut()));
    let group = p.groups.iter().position(|g| g.eq_ignore_ascii_case(own));
    tests
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let frames: Vec<&FrameSample> = p
                .frames
                .iter()
                .filter(|f| f.number >= t.first && f.number <= t.last)
                .collect();
            let part = |f: &FrameSample| {
                group
                    .and_then(|g| f.groups.get(g))
                    .copied()
                    .unwrap_or_default()
            };
            let mut metrics = Metrics::new();
            if !frames.is_empty() {
                let series = |get: &dyn Fn(&FrameSample) -> u64| -> Vec<u64> {
                    frames.iter().map(|f| get(f)).collect()
                };
                for (name, values) in [
                    ("instructions", series(&|f| part(f).instructions)),
                    ("native", series(&|f| part(f).native)),
                    ("bytes", series(&|f| part(f).bytes)),
                ] {
                    let (mean, p95, max) = spread(&values);
                    metrics.insert(format!("{name}.mean"), mean);
                    metrics.insert(format!("{name}.p95"), p95);
                    metrics.insert(format!("{name}.max"), max);
                }
                let all = series(&|f| f.cost.instructions);
                metrics.insert("all.instructions.mean".into(), spread(&all).0);
                metrics.insert("gc".into(), frames.iter().map(|f| f.gc).sum::<u64>() as f64);
            }
            if let Some(m) = memory.get(i) {
                for (addon, kb) in &m.addons {
                    metrics.insert(format!("memory.{addon}"), round(*kb));
                }
            }
            let worst = frames
                .iter()
                .max_by_key(|f| {
                    let c = part(f);
                    // The earliest of equals.
                    (c.instructions + c.native, std::cmp::Reverse(f.number))
                })
                .map(|f| Worst {
                    frame: f.number,
                    top: f
                        .top
                        .iter()
                        .take(3)
                        .map(|t| {
                            (
                                key_label(p, t.key, map),
                                t.self_cost.instructions + t.self_cost.native,
                            )
                        })
                        .collect(),
                });
            Phase {
                name: t.name.clone(),
                frames: frames.len() as u64,
                metrics,
                worst,
            }
        })
        .collect()
}

/// The mean (rounded), 95th percentile and maximum of per-frame values.
fn spread(values: &[u64]) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let mean = (sorted.iter().sum::<u64>() as f64 / n as f64).round();
    let p95 = sorted[(n * 95 / 100).min(n - 1)] as f64;
    (mean, p95, sorted[n - 1] as f64)
}

/// A function as a phase's worst frame names it: its source and line, a
/// native's name.
fn key_label(p: &Profiler, key: u32, map: &dyn Fn(&[u8]) -> Option<String>) -> String {
    match &p.keys[key as usize] {
        Key::Lua(proto) => {
            let source = map(&proto.source).unwrap_or_else(|| {
                String::from_utf8_lossy(&wowlua_vm::vm::chunkid(&proto.source)).into_owned()
            });
            if proto.line_defined == 0 {
                format!("main chunk of {source}")
            } else {
                format!("{source}:{}", proto.line_defined)
            }
        }
        _ => p.key_name(key),
    }
}

/// A number as JSON: a whole one without a fraction.
fn number(v: f64) -> Json {
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

impl Results {
    /// The results file (`version` 1).
    pub fn to_json(&self) -> Json {
        let scenarios: Vec<Json> = self
            .scenarios
            .iter()
            .map(|s| {
                let phases: Vec<Json> = s
                    .phases
                    .iter()
                    .map(|p| {
                        let mut metrics = serde_json::Map::new();
                        for (k, v) in &p.metrics {
                            metrics.insert(k.clone(), number(*v));
                        }
                        let mut phase = json!({
                            "name": p.name,
                            "frames": p.frames,
                            "metrics": metrics,
                        });
                        if let Some(w) = &p.worst {
                            phase["worst"] = json!({
                                "frame": w.frame,
                                "top": w.top.iter().map(|(n, c)| json!([n, c])).collect::<Vec<_>>(),
                            });
                        }
                        phase
                    })
                    .collect();
                json!({ "file": s.file, "flavor": s.flavor, "phases": phases })
            })
            .collect();
        json!({
            "version": 1,
            "native": NATIVE_NOTE,
            "scenarios": scenarios,
        })
    }

    /// A results file read back; none for what isn't one.
    pub fn from_json(v: &Json) -> Option<Results> {
        if v["version"].as_u64() != Some(1) {
            return None;
        }
        let scenarios = v["scenarios"]
            .as_array()?
            .iter()
            .map(|s| Scenario {
                file: s["file"].as_str().unwrap_or("").to_string(),
                flavor: s["flavor"].as_str().unwrap_or("").to_string(),
                phases: s["phases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| Phase {
                        name: p["name"].as_str().unwrap_or("").to_string(),
                        frames: p["frames"].as_u64().unwrap_or(0),
                        metrics: p["metrics"]
                            .as_object()
                            .into_iter()
                            .flatten()
                            .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                            .collect(),
                        worst: None,
                    })
                    .collect(),
            })
            .collect();
        Some(Results { scenarios })
    }

    /// The phase `name` of `file` for `flavor`.
    pub fn phase(&self, file: &str, flavor: &str, name: &str) -> Option<&Phase> {
        self.scenarios
            .iter()
            .find(|s| s.file == file && s.flavor == flavor)?
            .phases
            .iter()
            .find(|p| p.name == name)
    }

    /// `other`'s scenarios added, replacing those of the same file and
    /// flavor (a baseline written from a run of some scenarios keeps the
    /// rest).
    pub fn merge(&mut self, other: &Results) {
        for s in &other.scenarios {
            match self
                .scenarios
                .iter_mut()
                .find(|t| t.file == s.file && t.flavor == s.flavor)
            {
                Some(t) => *t = s.clone(),
                None => self.scenarios.push(s.clone()),
            }
        }
        self.scenarios
            .sort_by(|a, b| (&a.file, &a.flavor).cmp(&(&b.file, &b.flavor)));
    }
}

/// How a measure compares with the baseline's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    Same,
    Better,
    Worse,
}

/// A measure against the baseline's: the change in percent (infinite from
/// zero), and whether it is past `threshold` percent either way (every
/// measure is better lower).
pub fn verdict(base: f64, new: f64, threshold: f64) -> (f64, Verdict) {
    let change = if base == 0.0 {
        if new == 0.0 { 0.0 } else { f64::INFINITY }
    } else {
        (new - base) / base * 100.0
    };
    let v = if change > threshold {
        Verdict::Worse
    } else if change < -threshold {
        Verdict::Better
    } else {
        Verdict::Same
    };
    (change, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scenario_says_so_in_its_config() {
        assert!(is_scenario(b"config({ perf = true })"));
        assert!(is_scenario(
            b"import config from \"wowlua:test\"\nconfig({ profile = \"raid\", perf = true })"
        ));
        assert!(!is_scenario(b"config({ perf = false })"));
        assert!(!is_scenario(b"config({ profile = \"perf\" })"));
        // Only the file's own top-level call.
        assert!(!is_scenario(
            b"local function f() config({ perf = true }) end"
        ));
    }

    #[test]
    fn measures_compare_by_percent() {
        assert_eq!(verdict(100.0, 103.0, 2.0).1, Verdict::Worse);
        assert_eq!(verdict(100.0, 101.0, 2.0).1, Verdict::Same);
        assert_eq!(verdict(100.0, 90.0, 2.0), (-10.0, Verdict::Better));
        assert_eq!(verdict(0.0, 0.0, 2.0), (0.0, Verdict::Same));
        assert_eq!(verdict(0.0, 1.0, 2.0).1, Verdict::Worse);
        assert_eq!(spread(&[5, 1, 3, 100]), (27.0, 100.0, 100.0));
        assert_eq!(spread(&[]), (0.0, 0.0, 0.0));
    }

    #[test]
    fn results_read_back_and_merge() {
        let phase = |name: &str, v: f64| Phase {
            name: name.into(),
            frames: 2,
            metrics: [("instructions.mean".to_string(), v)].into_iter().collect(),
            worst: None,
        };
        let scenario = |file: &str, v: f64| Scenario {
            file: file.into(),
            flavor: "camelot".into(),
            phases: vec![phase("idle", v)],
        };
        let mut base = Results {
            scenarios: vec![
                scenario("tests/a.test.wowlua", 10.0),
                scenario("tests/b.test.wowlua", 20.5),
            ],
        };
        assert_eq!(Results::from_json(&base.to_json()), Some(base.clone()));
        base.merge(&Results {
            scenarios: vec![scenario("tests/b.test.wowlua", 7.0)],
        });
        assert_eq!(base.scenarios.len(), 2);
        let b = base
            .phase("tests/b.test.wowlua", "camelot", "idle")
            .unwrap();
        assert_eq!(b.metrics["instructions.mean"], 7.0);
        assert!(Results::from_json(&json!({ "version": 2 })).is_none());
    }
}
