//! Profile reports (spec debugger §18.8, decision T308): the functions
//! table, the game frames, and the call tree as speedscope JSON, which
//! keeps the whole profile for wowlua's readers (decision T495).

use serde_json::{Value as Json, json};
use wowlua_vm::profile::{Cost, FRAME_TREES, Key, Profiler};

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

/// A profile as a file keeps it (decision T495): its functions and
/// dispatches, the call tree with each node's calls and own cost, and the
/// game frames with the trees of the newest ([`FRAME_TREES`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub name: String,
    pub keys: Vec<SavedKey>,
    /// The tree, the root first; a parent before its children.
    pub nodes: Vec<SavedNode>,
    pub frames: Vec<SavedFrame>,
}

/// A function or a dispatch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedKey {
    pub name: String,
    pub file: Option<String>,
    pub line: u32,
    /// A dispatch's kind (`event`, `script`, `update`, `timer`, `frame`);
    /// none for a function.
    pub kind: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SavedNode {
    pub key: u32,
    pub parent: u32,
    pub calls: u64,
    pub self_cost: Cost,
}

/// A game frame: its cost, and the nodes that ran in it (empty when its
/// tree was not kept).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedFrame {
    pub name: String,
    pub cost: Cost,
    pub nodes: Vec<SavedFrameNode>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SavedFrameNode {
    pub node: u32,
    pub calls: u64,
    pub self_cost: Cost,
}

impl Saved {
    /// A profile collected by the VM: every frame's cost, the newest
    /// frames' trees.
    pub fn of(p: &Profiler, map: &dyn Fn(&[u8]) -> Option<String>, name: &str) -> Saved {
        let keys = (0..p.keys.len() as u32)
            .map(|k| {
                let (name, file, line) = place(p, k, map);
                let kind = match &p.keys[k as usize] {
                    Key::Dispatch(kind, _) => Some(kind.label().to_string()),
                    _ => None,
                };
                SavedKey {
                    name,
                    file,
                    line,
                    kind,
                }
            })
            .collect();
        let nodes = p
            .nodes
            .iter()
            .map(|n| SavedNode {
                key: n.key,
                parent: n.parent,
                calls: n.calls,
                self_cost: n.self_cost,
            })
            .collect();
        let from = p.frames.len().saturating_sub(FRAME_TREES);
        let frames = p.frames[from..]
            .iter()
            .map(|f| SavedFrame {
                name: f.name.to_string(),
                cost: f.cost,
                nodes: f
                    .nodes
                    .iter()
                    .map(|n| SavedFrameNode {
                        node: n.node,
                        calls: n.calls,
                        self_cost: n.self_cost,
                    })
                    .collect(),
            })
            .collect();
        Saved {
            name: name.to_string(),
            keys,
            nodes,
            frames,
        }
    }

    /// A frame's call tree: each node that ran in it with its part and its
    /// total there, a parent before its children.
    pub fn frame_tree(&self, frame: usize) -> Vec<(SavedFrameNode, Cost)> {
        let Some(f) = self.frames.get(frame) else {
            return Vec::new();
        };
        let mut tree: Vec<(SavedFrameNode, Cost)> =
            f.nodes.iter().map(|n| (*n, n.self_cost)).collect();
        for i in (0..tree.len()).rev() {
            let Some(parent) = self.nodes.get(tree[i].0.node as usize).map(|n| n.parent) else {
                continue;
            };
            if tree[i].0.node == parent {
                continue;
            }
            if let Ok(at) = tree.binary_search_by_key(&parent, |(n, _)| n.node) {
                let t = tree[i].1;
                add(&mut tree[at].1, t);
            }
        }
        tree
    }

    /// The file: speedscope's, one sampled profile per measure (each
    /// node's self cost a sample of its path), with the whole profile
    /// under `wowlua` for wowlua's readers.
    pub fn to_speedscope(&self) -> Json {
        let frames: Vec<Json> = self
            .keys
            .iter()
            .map(|k| {
                let mut f = json!({ "name": k.name });
                if let Some(file) = &k.file {
                    f["file"] = json!(file);
                    f["line"] = json!(k.line);
                }
                f
            })
            .collect();
        // Each node's path of keys from the root (the root left out).
        let mut paths: Vec<Vec<u32>> = vec![Vec::new(); self.nodes.len()];
        for i in 1..self.nodes.len() {
            let parent = self.nodes[i].parent as usize;
            let mut path = paths[parent].clone();
            path.push(self.nodes[i].key);
            paths[i] = path;
        }
        let profiles: Vec<Json> = MEASURES
            .iter()
            .map(|(measure, unit, get)| {
                let mut samples = Vec::new();
                let mut weights = Vec::new();
                let mut total = 0u64;
                for (i, n) in self.nodes.iter().enumerate().skip(1) {
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
        let c = |c: &Cost| json!([c.instructions, c.bytes, c.nanos]);
        let wowlua = json!({
            "version": 1,
            "kinds": self.keys.iter().map(|k| json!(k.kind)).collect::<Vec<_>>(),
            "nodes": self
                .nodes
                .iter()
                .map(|n| json!([n.key, n.parent, n.calls, c(&n.self_cost)]))
                .collect::<Vec<_>>(),
            "frames": self
                .frames
                .iter()
                .map(|f| {
                    json!({
                        "name": f.name,
                        "cost": c(&f.cost),
                        "nodes": f
                            .nodes
                            .iter()
                            .map(|n| json!([n.node, n.calls, c(&n.self_cost)]))
                            .collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>(),
        });
        json!({
            "$schema": "https://www.speedscope.app/file-format-schema.json",
            "name": self.name,
            "exporter": "wowlua",
            "activeProfileIndex": 0,
            "shared": { "frames": frames },
            "profiles": profiles,
            "wowlua": wowlua,
        })
    }

    /// A speedscope file read back: wowlua's whole profile when it has
    /// one; else its first sampled profile as a tree of instructions (or
    /// its own unit), without calls or frames. None for what isn't one.
    pub fn from_speedscope(file: &Json) -> Option<Saved> {
        let keys: Vec<SavedKey> = file["shared"]["frames"]
            .as_array()?
            .iter()
            .map(|f| SavedKey {
                name: f["name"].as_str().unwrap_or("?").to_string(),
                file: f["file"].as_str().map(str::to_string),
                line: f["line"].as_u64().unwrap_or(0) as u32,
                kind: None,
            })
            .collect();
        let name = file["name"].as_str().unwrap_or("profile").to_string();
        let cost = |v: &Json| Cost {
            instructions: v[0].as_u64().unwrap_or(0),
            bytes: v[1].as_u64().unwrap_or(0),
            nanos: v[2].as_u64().unwrap_or(0),
        };
        let w = &file["wowlua"];
        if w["version"].as_u64() == Some(1) {
            let mut keys = keys;
            for (k, kind) in keys.iter_mut().zip(w["kinds"].as_array()?) {
                k.kind = kind.as_str().map(str::to_string);
            }
            let nodes = w["nodes"]
                .as_array()?
                .iter()
                .map(|n| SavedNode {
                    key: n[0].as_u64().unwrap_or(0) as u32,
                    parent: n[1].as_u64().unwrap_or(0) as u32,
                    calls: n[2].as_u64().unwrap_or(0),
                    self_cost: cost(&n[3]),
                })
                .collect();
            let frames = w["frames"]
                .as_array()?
                .iter()
                .map(|f| SavedFrame {
                    name: f["name"].as_str().unwrap_or("").to_string(),
                    cost: cost(&f["cost"]),
                    nodes: f["nodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|n| SavedFrameNode {
                            node: n[0].as_u64().unwrap_or(0) as u32,
                            calls: n[1].as_u64().unwrap_or(0),
                            self_cost: cost(&n[2]),
                        })
                        .collect(),
                })
                .collect();
            return Some(Saved {
                name,
                keys,
                nodes,
                frames,
            });
        }
        // Another tool's file: its first sampled profile's paths as a
        // tree, the weights as instructions.
        let profile = file["profiles"]
            .as_array()?
            .iter()
            .find(|p| p["type"] == "sampled")?;
        let mut nodes = vec![SavedNode::default()];
        let mut children: std::collections::HashMap<(u32, u32), u32> =
            std::collections::HashMap::new();
        let samples = profile["samples"].as_array()?;
        let weights = profile["weights"].as_array()?;
        for (sample, weight) in samples.iter().zip(weights) {
            let mut at = 0u32;
            for k in sample.as_array().into_iter().flatten() {
                let key = k.as_u64()? as u32;
                if key as usize >= keys.len() {
                    return None;
                }
                at = *children.entry((at, key)).or_insert_with(|| {
                    nodes.push(SavedNode {
                        key,
                        parent: at,
                        calls: 0,
                        self_cost: Cost::default(),
                    });
                    nodes.len() as u32 - 1
                });
            }
            nodes[at as usize].self_cost.instructions += weight.as_u64().unwrap_or(0);
        }
        Some(Saved {
            name,
            keys,
            nodes,
            frames: Vec::new(),
        })
    }
}

fn add(to: &mut Cost, c: Cost) {
    to.instructions += c.instructions;
    to.bytes += c.bytes;
    to.nanos += c.nanos;
}

type Measure = (&'static str, &'static str, fn(&Cost) -> u64);

const MEASURES: [Measure; 3] = [
    ("instructions", "none", |c| c.instructions),
    ("bytes allocated", "bytes", |c| c.bytes),
    ("host time", "nanoseconds", |c| c.nanos),
];

/// The profile as a speedscope file ([`Saved::to_speedscope`]).
pub fn speedscope(p: &Profiler, map: &dyn Fn(&[u8]) -> Option<String>, name: &str) -> Json {
    Saved::of(p, map, name).to_speedscope()
}
