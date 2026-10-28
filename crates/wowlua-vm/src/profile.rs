//! The profiler (spec debugger §18.8, decision T307): a call tree of
//! functions and the engine's dispatches, each node with its calls and
//! the instructions, bytes and host time spent in it (self). Totals are
//! sums over the tree, so a coroutine's time is not counted while it
//! waits.

use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::debug::DispatchKind;
use crate::heap::ThreadRef;
use crate::proto::Proto;

/// What a node is: a function or a dispatch.
#[derive(Clone, Debug)]
pub enum Key {
    Root,
    Lua(Rc<Proto>),
    Native(&'static str),
    Dispatch(DispatchKind, Rc<str>),
}

/// A key as it is looked up: functions by identity.
pub(crate) enum RawKey {
    Lua(Rc<Proto>),
    Native(&'static str),
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum KeyId {
    Lua(usize),
    Native(usize, usize),
    Dispatch(DispatchKind, Rc<str>),
}

/// What is measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    pub instructions: u64,
    pub bytes: u64,
    pub nanos: u64,
}

impl Cost {
    fn add(&mut self, o: Cost) {
        self.instructions += o.instructions;
        self.bytes += o.bytes;
        self.nanos += o.nanos;
    }
}

impl std::ops::Sub for Cost {
    type Output = Cost;
    fn sub(self, o: Cost) -> Cost {
        Cost {
            instructions: self.instructions.saturating_sub(o.instructions),
            bytes: self.bytes.saturating_sub(o.bytes),
            nanos: self.nanos.saturating_sub(o.nanos),
        }
    }
}

/// One node of the call tree.
#[derive(Clone, Debug)]
pub struct Node {
    /// Index into [`Profile::keys`].
    pub key: u32,
    pub parent: u32,
    pub children: Vec<u32>,
    pub calls: u64,
    /// Spent in the node itself, not in its children.
    pub self_cost: Cost,
}

/// One game frame's cost.
#[derive(Clone, Debug)]
pub struct FrameSample {
    pub name: Rc<str>,
    pub cost: Cost,
    /// The nodes that ran in the frame, by index (a parent before its
    /// children); empty for a frame older than the newest [`FRAME_TREES`].
    pub nodes: Vec<FrameNode>,
}

/// A node's part in one game frame: its calls and its own cost there.
#[derive(Clone, Copy, Debug)]
pub struct FrameNode {
    pub node: u32,
    pub calls: u64,
    pub self_cost: Cost,
}

#[derive(Clone, Copy)]
struct Entry {
    node: u32,
    depth: usize,
    dispatch: bool,
    /// The counters when it was entered (game frames' totals).
    start: Cost,
}

/// Deepest call path kept: deeper calls count toward the node at this
/// depth.
pub const MAX_DEPTH: usize = 256;

/// Most game frames kept (the newest).
const MAX_FRAMES: usize = 100_000;

/// Game frames whose call trees are kept (the newest; decision T495).
pub const FRAME_TREES: usize = 600;

/// The profiler's state, and the profile it collects.
pub struct Profiler {
    pub keys: Vec<Key>,
    key_ids: FxHashMap<KeyId, u32>,
    pub nodes: Vec<Node>,
    stacks: FxHashMap<ThreadRef, Vec<Entry>>,
    last: Cost,
    last_thread: Option<ThreadRef>,
    clock: Box<dyn Fn() -> u64>,
    pub frames: Vec<FrameSample>,
    frame_count: u64,
    /// While a game frame runs: each node's calls and cost in it.
    frame_nodes: Option<FxHashMap<u32, (u64, Cost)>>,
}

impl Profiler {
    /// A profiler whose host time comes from `clock` (nanoseconds).
    pub fn new(clock: Box<dyn Fn() -> u64>) -> Profiler {
        Profiler {
            keys: vec![Key::Root],
            key_ids: FxHashMap::default(),
            nodes: vec![Node {
                key: 0,
                parent: 0,
                children: Vec::new(),
                calls: 0,
                self_cost: Cost::default(),
            }],
            stacks: FxHashMap::default(),
            last: Cost::default(),
            last_thread: None,
            clock,
            frames: Vec::new(),
            frame_count: 0,
            frame_nodes: None,
        }
    }

    fn now(&self, instructions: u64, bytes: u64) -> Cost {
        Cost {
            instructions,
            bytes,
            nanos: (self.clock)(),
        }
    }

    /// Charges what was spent since the last event to the node running.
    fn charge(&mut self, now: Cost, thread: ThreadRef) {
        let spent = now - self.last;
        self.last = now;
        let running = self.last_thread.unwrap_or(thread);
        let node = self
            .stacks
            .get(&running)
            .and_then(|s| s.last())
            .map_or(0, |e| e.node);
        self.nodes[node as usize].self_cost.add(spent);
        if let Some(frame) = &mut self.frame_nodes {
            frame.entry(node).or_default().1.add(spent);
        }
        self.last_thread = Some(thread);
    }

    fn key(&mut self, id: KeyId, key: impl FnOnce() -> Key) -> u32 {
        if let Some(&k) = self.key_ids.get(&id) {
            return k;
        }
        let k = self.keys.len() as u32;
        self.keys.push(key());
        self.key_ids.insert(id, k);
        k
    }

    fn child(&mut self, parent: u32, key: u32) -> u32 {
        for &c in &self.nodes[parent as usize].children {
            if self.nodes[c as usize].key == key {
                return c;
            }
        }
        let n = self.nodes.len() as u32;
        self.nodes.push(Node {
            key,
            parent,
            children: Vec::new(),
            calls: 0,
            self_cost: Cost::default(),
        });
        self.nodes[parent as usize].children.push(n);
        n
    }

    /// The node a new entry of `thread` goes under: its top, or for a
    /// coroutine's first entry the top of the thread that resumed it.
    fn parent(&self, thread: ThreadRef, resumers: &[ThreadRef]) -> u32 {
        if let Some(e) = self.stacks.get(&thread).and_then(|s| s.last()) {
            return e.node;
        }
        for r in resumers.iter().rev() {
            if let Some(e) = self.stacks.get(r).and_then(|s| s.last()) {
                return e.node;
            }
        }
        0
    }

    fn push(
        &mut self,
        key: u32,
        depth: usize,
        thread: ThreadRef,
        resumers: &[ThreadRef],
        dispatch: bool,
        start: Cost,
    ) {
        let parent = self.parent(thread, resumers);
        let stack_len = self.stacks.get(&thread).map_or(0, Vec::len);
        let node = if stack_len >= MAX_DEPTH {
            parent
        } else {
            self.child(parent, key)
        };
        self.nodes[node as usize].calls += 1;
        if let Some(frame) = &mut self.frame_nodes {
            frame.entry(node).or_default().0 += 1;
        }
        self.stacks.entry(thread).or_default().push(Entry {
            node,
            depth,
            dispatch,
            start,
        });
    }

    pub(crate) fn enter(
        &mut self,
        key: RawKey,
        depth: usize,
        thread: ThreadRef,
        resumers: &[ThreadRef],
        instructions: u64,
        bytes: u64,
    ) {
        let now = self.now(instructions, bytes);
        self.charge(now, thread);
        let k = match key {
            RawKey::Lua(p) => {
                let id = KeyId::Lua(Rc::as_ptr(&p) as usize);
                self.key(id, || Key::Lua(p))
            }
            RawKey::Native(name) => {
                let id = KeyId::Native(name.as_ptr() as usize, name.len());
                self.key(id, || Key::Native(name))
            }
        };
        self.push(k, depth, thread, resumers, false, now);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enter_dispatch(
        &mut self,
        kind: DispatchKind,
        name: Rc<str>,
        depth: usize,
        thread: ThreadRef,
        resumers: &[ThreadRef],
        instructions: u64,
        bytes: u64,
    ) {
        let now = self.now(instructions, bytes);
        self.charge(now, thread);
        // Game frames are one node (each frame's own cost goes to `frames`).
        let name: Rc<str> = if kind == DispatchKind::Frame {
            "game frame".into()
        } else {
            name
        };
        let k = self.key(KeyId::Dispatch(kind, name.clone()), || {
            Key::Dispatch(kind, name)
        });
        if kind == DispatchKind::Frame {
            self.frame_nodes = Some(FxHashMap::default());
        }
        self.push(k, depth, thread, resumers, true, now);
    }

    /// Calls deeper than `depth` returned (an error unwound them).
    pub(crate) fn exit_to(
        &mut self,
        depth: usize,
        thread: ThreadRef,
        instructions: u64,
        bytes: u64,
    ) {
        let now = self.now(instructions, bytes);
        self.charge(now, thread);
        let mut frame_left = false;
        if let Some(s) = self.stacks.get_mut(&thread) {
            while let Some(e) = s.last().copied().filter(|e| e.depth > depth) {
                s.pop();
                frame_left |= e.dispatch
                    && matches!(
                        self.keys[self.nodes[e.node as usize].key as usize],
                        Key::Dispatch(DispatchKind::Frame, _)
                    );
            }
        }
        // A frame an error left is not one of the frames.
        if frame_left {
            self.frame_nodes = None;
        }
    }

    /// A dispatch at `depth` ended (and whatever it left running).
    pub(crate) fn exit_dispatch(
        &mut self,
        depth: usize,
        thread: ThreadRef,
        instructions: u64,
        bytes: u64,
    ) {
        let now = self.now(instructions, bytes);
        self.charge(now, thread);
        let Some(s) = self.stacks.get_mut(&thread) else {
            return;
        };
        while let Some(e) = s.last().copied() {
            if e.depth < depth || (e.depth == depth && !e.dispatch) {
                break;
            }
            s.pop();
            if e.dispatch && e.depth == depth {
                if let Key::Dispatch(DispatchKind::Frame, _) =
                    &self.keys[self.nodes[e.node as usize].key as usize]
                {
                    if self.frames.len() >= MAX_FRAMES {
                        self.frames.remove(0);
                    }
                    self.frame_count += 1;
                    let mut nodes: Vec<FrameNode> = self
                        .frame_nodes
                        .take()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(node, (calls, self_cost))| FrameNode {
                            node,
                            calls,
                            self_cost,
                        })
                        .collect();
                    nodes.sort_unstable_by_key(|n| n.node);
                    self.frames.push(FrameSample {
                        name: format!("frame {}", self.frame_count).into(),
                        cost: now - e.start,
                        nodes,
                    });
                    // Older frames keep their cost, not their tree.
                    if let Some(old) = self.frames.len().checked_sub(FRAME_TREES + 1) {
                        self.frames[old].nodes = Vec::new();
                    }
                }
                break;
            }
        }
    }

    /// A finished coroutine's stack goes.
    pub(crate) fn drop_thread(&mut self, thread: ThreadRef) {
        self.stacks.remove(&thread);
    }

    /// Each node's total: itself and everything below it.
    pub fn totals(&self) -> Vec<Cost> {
        let mut totals: Vec<Cost> = self.nodes.iter().map(|n| n.self_cost).collect();
        // Children always come after their parent.
        for i in (1..self.nodes.len()).rev() {
            let p = self.nodes[i].parent as usize;
            let t = totals[i];
            totals[p].add(t);
        }
        totals
    }

    /// A kept frame's call tree (`frames[frame]`): each node that ran in it
    /// with its part, and its total there (itself and its children in the
    /// frame), a parent before its children; empty for an older frame.
    pub fn frame_tree(&self, frame: usize) -> Vec<(FrameNode, Cost)> {
        let Some(sample) = self.frames.get(frame) else {
            return Vec::new();
        };
        let mut tree: Vec<(FrameNode, Cost)> =
            sample.nodes.iter().map(|n| (*n, n.self_cost)).collect();
        // Children always come after their parent.
        for i in (0..tree.len()).rev() {
            let parent = self.nodes[tree[i].0.node as usize].parent;
            if tree[i].0.node == parent {
                continue;
            }
            if let Ok(p) = tree.binary_search_by_key(&parent, |(n, _)| n.node) {
                let t = tree[i].1;
                tree[p].1.add(t);
            }
        }
        tree
    }

    /// Per key: calls, self and total cost (a recursive call's total is
    /// counted once, at its outermost node).
    pub fn flat(&self) -> Vec<FlatEntry> {
        let totals = self.totals();
        let mut out: Vec<FlatEntry> = (0..self.keys.len())
            .map(|k| FlatEntry {
                key: k as u32,
                calls: 0,
                self_cost: Cost::default(),
                total: Cost::default(),
            })
            .collect();
        // Walk the tree with the keys on the path.
        let mut path: Vec<u32> = Vec::new();
        let mut stack: Vec<(u32, bool)> = vec![(0, false)];
        while let Some((n, done)) = stack.pop() {
            let node = &self.nodes[n as usize];
            if done {
                path.pop();
                continue;
            }
            let e = &mut out[node.key as usize];
            e.calls += node.calls;
            e.self_cost.add(node.self_cost);
            if !path.contains(&node.key) {
                e.total.add(totals[n as usize]);
            }
            path.push(node.key);
            stack.push((n, true));
            for &c in node.children.iter().rev() {
                stack.push((c, false));
            }
        }
        out.retain(|e| e.key != 0 && e.calls > 0);
        out
    }

    /// A key's display name: `name (source:line)`, `[C] name`, `event X`.
    pub fn key_name(&self, key: u32) -> String {
        match &self.keys[key as usize] {
            Key::Root => "(root)".to_string(),
            Key::Lua(p) => {
                let src = crate::vm::chunkid(&p.source);
                if p.line_defined == 0 {
                    format!("main chunk ({})", String::from_utf8_lossy(&src))
                } else {
                    format!(
                        "function ({}:{})",
                        String::from_utf8_lossy(&src),
                        p.line_defined
                    )
                }
            }
            Key::Native(n) => format!("[C] {n}"),
            Key::Dispatch(k, n) => format!("{} {n}", k.label()),
        }
    }
}

/// One function's or dispatch's sums.
#[derive(Clone, Debug)]
pub struct FlatEntry {
    pub key: u32,
    pub calls: u64,
    pub self_cost: Cost,
    pub total: Cost,
}

impl crate::vm::Vm {
    /// Starts profiling (a fresh profile); host time from `clock`
    /// (nanoseconds, monotonic).
    pub fn start_profiling(&mut self, clock: Box<dyn Fn() -> u64>) {
        let mut p = Profiler::new(clock);
        p.last = Cost {
            instructions: self.instructions,
            bytes: self.heap.allocated(),
            nanos: (p.clock)(),
        };
        self.profiler = Some(Box::new(p));
        self.hook_base |= crate::debug::HOOK_COUNT;
        self.hooks_changed();
    }

    /// Stops profiling: the profile collected.
    pub fn stop_profiling(&mut self) -> Option<Box<Profiler>> {
        let mut p = self.profiler.take()?;
        let now = p.now(self.instructions, self.heap.allocated());
        p.charge(now, self.current);
        self.hook_base &= !crate::debug::HOOK_COUNT;
        self.hooks_changed();
        Some(p)
    }

    pub fn profiler(&self) -> Option<&Profiler> {
        self.profiler.as_deref()
    }

    /// Instructions run while hooks were on (the profiler's measure).
    pub fn instructions(&self) -> u64 {
        self.instructions
    }
}
