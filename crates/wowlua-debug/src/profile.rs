//! Profile reports (spec debugger §18.8, decision T308): the functions
//! table, the game frames, and the call tree as speedscope JSON.

use serde_json::{Value as Json, json};
use wowlua_vm::profile::{Cost, Key, Profiler};

/// A key's name, source and line, as reports show it.
fn place(
    p: &Profiler,
    key: u32,
    map: &dyn Fn(&[u8]) -> Option<String>,
) -> (String, Option<String>, u32) {
    match &p.keys[key as usize] {
        Key::Lua(proto) => {
            let source = map(&proto.source).unwrap_or_else(|| {
                String::from_utf8_lossy(&wowlua_vm::vm::chunkid(&proto.source)).into_owned()
            });
            let name = if proto.line_defined == 0 {
                format!("main chunk of {source}")
            } else {
                format!("function {source}:{}", proto.line_defined)
            };
            (name, Some(source), proto.line_defined)
        }
        _ => (p.key_name(key), None, 0),
    }
}

fn cost(c: Cost) -> Json {
    json!({ "instructions": c.instructions, "bytes": c.bytes, "nanos": c.nanos })
}

/// The report: `functions` (by total instructions, the first 500),
/// `frames` (the last 600 game frames), `speedscope` (the whole tree).
pub fn report(p: &Profiler, map: &dyn Fn(&[u8]) -> Option<String>) -> Json {
    let mut flat = p.flat();
    flat.sort_by_key(|e| std::cmp::Reverse(e.total.instructions));
    let functions: Vec<Json> = flat
        .iter()
        .take(500)
        .map(|e| {
            let (name, source, line) = place(p, e.key, map);
            json!({
                "name": name,
                "source": source,
                "line": line,
                "calls": e.calls,
                "self": cost(e.self_cost),
                "total": cost(e.total),
            })
        })
        .collect();
    let from = p.frames.len().saturating_sub(600);
    let frames: Vec<Json> = p.frames[from..]
        .iter()
        .map(|f| json!({ "name": f.name.as_ref(), "cost": cost(f.cost) }))
        .collect();
    json!({
        "functions": functions,
        "frames": frames,
        "speedscope": speedscope(p, map, "wowlua profile"),
    })
}

/// The call tree as a speedscope file: one sampled profile per measure,
/// each node's self cost a sample of its path.
pub fn speedscope(p: &Profiler, map: &dyn Fn(&[u8]) -> Option<String>, name: &str) -> Json {
    let frames: Vec<Json> = (0..p.keys.len() as u32)
        .map(|k| {
            let (name, file, line) = place(p, k, map);
            let mut f = json!({ "name": name });
            if let Some(file) = file {
                f["file"] = json!(file);
                f["line"] = json!(line);
            }
            f
        })
        .collect();
    // Each node's path of keys from the root (the root left out).
    let mut paths: Vec<Vec<u32>> = vec![Vec::new(); p.nodes.len()];
    for i in 1..p.nodes.len() {
        let parent = p.nodes[i].parent as usize;
        let mut path = paths[parent].clone();
        path.push(p.nodes[i].key);
        paths[i] = path;
    }
    type Measure = (&'static str, &'static str, fn(&Cost) -> u64);
    let measures: [Measure; 3] = [
        ("instructions", "none", |c| c.instructions),
        ("bytes allocated", "bytes", |c| c.bytes),
        ("host time", "nanoseconds", |c| c.nanos),
    ];
    let profiles: Vec<Json> = measures
        .iter()
        .map(|(measure, unit, get)| {
            let mut samples = Vec::new();
            let mut weights = Vec::new();
            let mut total = 0u64;
            for (i, n) in p.nodes.iter().enumerate().skip(1) {
                let w = get(&n.self_cost);
                if w > 0 {
                    samples.push(json!(paths[i]));
                    weights.push(json!(w));
                    total += w;
                }
            }
            json!({
                "type": "sampled",
                "name": measure,
                "unit": unit,
                "startValue": 0,
                "endValue": total,
                "samples": samples,
                "weights": weights,
            })
        })
        .collect();
    json!({
        "$schema": "https://www.speedscope.app/file-format-schema.json",
        "name": name,
        "exporter": "wowlua",
        "activeProfileIndex": 0,
        "shared": { "frames": frames },
        "profiles": profiles,
    })
}
