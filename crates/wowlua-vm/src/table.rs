//! Lua tables: a port of Lua 5.1's `ltable.c`, so that array sizing, the
//! length operator and traversal order behave as in the client. Strings,
//! numbers and booleans hash as in Lua; objects hash by a deterministic
//! pseudo-address.

use crate::heap::{Heap, TableRef};
use crate::taint::Slot;
use crate::value::Value;

const MAXBITS: usize = 26;
const MAXASIZE: i64 = 1 << MAXBITS;

/// Why a value cannot be a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    Nil,
    NaN,
    /// `table.freeze` (WoW): the table takes no more writes.
    Frozen,
}

impl KeyError {
    pub fn message(self) -> &'static str {
        match self {
            KeyError::Nil => "table index is nil",
            KeyError::NaN => "table index is NaN",
            KeyError::Frozen => "attempted to perform indexed assignment on a frozen table",
        }
    }
}

/// `next` was given a key that is not in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidKey;

/// Checks that `v` can be a table key.
pub fn check_key(v: Value) -> Result<(), KeyError> {
    match v {
        Value::Nil => Err(KeyError::Nil),
        Value::Num(n) if n.is_nan() => Err(KeyError::NaN),
        _ => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Node {
    /// Nil: a free node. Keys carry no taint: the client stores new keys
    /// secure.
    key: Value,
    val: Slot,
    next: Option<u32>,
}

#[derive(Default)]
pub struct Table {
    arr: Vec<Slot>,
    /// Empty: Lua's shared dummy node.
    nodes: Vec<Node>,
    lastfree: usize,
    pub meta: Option<TableRef>,
    /// Frozen by `table.freeze`: raw writes fail.
    pub frozen: bool,
}

/// `lua_number2int` then the integer check (`arrayindex`).
fn arrayindex(key: Value) -> i64 {
    if let Value::Num(n) = key {
        let k = n as i32;
        if f64::from(k) == n {
            return i64::from(k);
        }
    }
    -1
}

fn ceillog2(x: usize) -> usize {
    // ceil(log2(x)) for x >= 1
    let mut l = 0;
    let mut v = x - 1;
    while v > 0 {
        l += 1;
        v >>= 1;
    }
    l
}

fn raw_eq(a: Value, b: Value) -> bool {
    a.raw_eq(b)
}

impl Table {
    pub fn new(narr: usize, nhash: usize) -> Self {
        let mut t = Table {
            arr: vec![Slot::NIL; narr],
            ..Table::default()
        };
        t.set_node_vector(nhash);
        t
    }

    pub fn array_len(&self) -> usize {
        self.arr.len()
    }

    /// The allocated sizes of the array and hash parts.
    pub fn part_sizes(&self) -> (usize, usize) {
        (self.arr.len(), self.nodes.len())
    }

    pub(crate) fn capacity_bytes(&self) -> usize {
        self.arr.len() * 24 + self.nodes.len() * 48
    }

    fn sizenode(&self) -> usize {
        self.nodes.len().max(1)
    }

    fn set_node_vector(&mut self, size: usize) {
        if size == 0 {
            self.nodes = Vec::new();
            self.lastfree = 0;
            return;
        }
        let lsize = ceillog2(size);
        let size = 1usize << lsize;
        self.nodes = vec![Node::default(); size];
        self.lastfree = size;
    }

    fn hashmod(&self, n: u32) -> usize {
        (n % ((self.sizenode() as u32 - 1) | 1)) as usize
    }

    fn mainposition(&self, key: Value) -> usize {
        let size = self.sizenode();
        match key {
            Value::Num(n) => {
                if n == 0.0 {
                    return 0;
                }
                let bits = n.to_bits();
                let a = (bits as u32).wrapping_add((bits >> 32) as u32);
                self.hashmod(a)
            }
            Value::Str(s) => (s.hash as usize) & (size - 1),
            Value::Bool(b) => usize::from(b) & (size - 1),
            other => {
                let addr = Heap::address(other).unwrap_or(0) as u32;
                self.hashmod(addr)
            }
        }
    }

    fn find_node(&self, key: Value) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut n = self.mainposition(key);
        loop {
            if raw_eq(self.nodes[n].key, key) {
                return Some(n);
            }
            n = self.nodes[n].next? as usize;
        }
    }

    pub fn get_num(&self, n: f64) -> Value {
        self.get_num_slot(n).v
    }

    fn get_num_slot(&self, n: f64) -> Slot {
        let k = n as i32;
        if f64::from(k) == n {
            return self.get_int_i(k);
        }
        if n.is_nan() {
            return Slot::NIL;
        }
        self.find_node(Value::Num(n))
            .map_or(Slot::NIL, |i| self.nodes[i].val)
    }

    fn get_int_i(&self, key: i32) -> Slot {
        if (key as i64 - 1) >= 0 && ((key as i64 - 1) as usize) < self.arr.len() {
            return self.arr[(key - 1) as usize];
        }
        self.find_node(Value::Num(f64::from(key)))
            .map_or(Slot::NIL, |i| self.nodes[i].val)
    }

    pub fn get_int(&self, i: usize) -> Value {
        if i >= 1 && i <= self.arr.len() {
            return self.arr[i - 1].v;
        }
        self.get_num(i as f64)
    }

    pub fn get_str(&self, s: crate::heap::StrRef) -> Value {
        self.get_str_slot(s).v
    }

    pub fn get_str_slot(&self, s: crate::heap::StrRef) -> Slot {
        self.find_node(Value::Str(s))
            .map_or(Slot::NIL, |i| self.nodes[i].val)
    }

    pub fn get(&self, key: Value) -> Value {
        self.get_slot(key).v
    }

    /// The value and taint stored for `key` (absent keys: a secure nil).
    pub fn get_slot(&self, key: Value) -> Slot {
        match key {
            Value::Nil => Slot::NIL,
            Value::Str(s) => self.get_str_slot(s),
            Value::Num(n) => self.get_num_slot(n),
            _ => self.find_node(key).map_or(Slot::NIL, |i| self.nodes[i].val),
        }
    }

    /// The slot for `key`, creating it (`luaH_set`). `key` must be valid.
    fn pos(&mut self, key: Value) -> Pos {
        if let Value::Num(n) = key {
            let k = n as i32;
            if f64::from(k) == n && k >= 1 && (k as usize) <= self.arr.len() {
                return Pos::Arr(k as usize - 1);
            }
        }
        if let Some(i) = self.find_node(key) {
            return Pos::Node(i);
        }
        self.newkey(key)
    }

    /// Sets `t[key] = value` (raw), secure.
    pub fn set(&mut self, key: Value, value: Value) -> Result<(), KeyError> {
        self.set_slot(key, Slot::secure(value))
    }

    /// Sets `t[key]` to a value with its taint (raw).
    pub fn set_slot(&mut self, key: Value, value: Slot) -> Result<(), KeyError> {
        // As in Lua, even a nil value creates the key's slot.
        check_key(key)?;
        if self.frozen {
            return Err(KeyError::Frozen);
        }
        match self.pos(key) {
            Pos::Arr(i) => self.arr[i] = value,
            Pos::Node(i) => self.nodes[i].val = value,
        }
        Ok(())
    }

    pub fn set_int(&mut self, i: usize, value: Slot) {
        if i >= 1 && i <= self.arr.len() {
            self.arr[i - 1] = value;
            return;
        }
        let _ = self.set_slot(Value::Num(i as f64), value);
    }

    /// Grows the array part to `nasize` (`luaH_resizearray`).
    pub fn resize_array(&mut self, nasize: usize) {
        let nsize = if self.nodes.is_empty() {
            0
        } else {
            self.nodes.len()
        };
        self.resize(nasize, nsize);
    }

    pub fn set_str(&mut self, s: crate::heap::StrRef, value: Value) {
        let _ = self.set(Value::Str(s), value);
    }

    fn getfreepos(&mut self) -> Option<usize> {
        while self.lastfree > 0 {
            self.lastfree -= 1;
            if self.nodes[self.lastfree].key.is_nil() {
                return Some(self.lastfree);
            }
        }
        None
    }

    /// Inserts a new key (`newkey`), rehashing when no node is free.
    fn newkey(&mut self, key: Value) -> Pos {
        if self.nodes.is_empty() {
            self.rehash(key);
            return self.pos(key);
        }
        let mut mp = self.mainposition(key);
        if !self.nodes[mp].val.v.is_nil() {
            let Some(n) = self.getfreepos() else {
                self.rehash(key);
                return self.pos(key);
            };
            let othern_start = self.mainposition(self.nodes[mp].key);
            if othern_start != mp {
                // The colliding node is out of its main position: move it.
                let mut othern = othern_start;
                while self.nodes[othern].next != Some(mp as u32) {
                    othern = self.nodes[othern].next.expect("chain") as usize;
                }
                self.nodes[othern].next = Some(n as u32);
                self.nodes[n] = self.nodes[mp];
                self.nodes[mp].next = None;
                self.nodes[mp].val = Slot::NIL;
            } else {
                // The colliding node is in its own main position.
                self.nodes[n].next = self.nodes[mp].next;
                self.nodes[mp].next = Some(n as u32);
                mp = n;
            }
        }
        self.nodes[mp].key = key;
        Pos::Node(mp)
    }

    fn countint(key: Value, nums: &mut [usize; MAXBITS + 1]) -> usize {
        let k = arrayindex(key);
        if 0 < k && k <= MAXASIZE {
            nums[ceillog2(k as usize)] += 1;
            1
        } else {
            0
        }
    }

    fn numusearray(&self, nums: &mut [usize; MAXBITS + 1]) -> usize {
        let mut ause = 0;
        let mut i = 1;
        let mut ttlg = 1usize;
        for slot in nums.iter_mut() {
            let mut lc = 0;
            let mut lim = ttlg;
            if lim > self.arr.len() {
                lim = self.arr.len();
                if i > lim {
                    break;
                }
            }
            while i <= lim {
                if !self.arr[i - 1].v.is_nil() {
                    lc += 1;
                }
                i += 1;
            }
            *slot += lc;
            ause += lc;
            ttlg *= 2;
        }
        ause
    }

    fn computesizes(nums: &[usize; MAXBITS + 1], narray: &mut usize) -> usize {
        let mut a = 0;
        let mut na = 0;
        let mut n = 0;
        let mut twotoi = 1usize;
        let mut i = 0;
        while twotoi / 2 < *narray {
            if nums[i] > 0 {
                a += nums[i];
                if a > twotoi / 2 {
                    n = twotoi;
                    na = a;
                }
            }
            if a == *narray {
                break;
            }
            i += 1;
            twotoi *= 2;
        }
        *narray = n;
        na
    }

    fn rehash(&mut self, extra: Value) {
        let mut nums = [0usize; MAXBITS + 1];
        let mut nasize = self.numusearray(&mut nums);
        let mut totaluse = nasize;
        // numusehash
        let mut ause = 0;
        for n in self.nodes.iter().rev() {
            if !n.val.v.is_nil() {
                ause += Self::countint(n.key, &mut nums);
                totaluse += 1;
            }
        }
        nasize += ause;
        nasize += Self::countint(extra, &mut nums);
        totaluse += 1;
        let na = Self::computesizes(&nums, &mut nasize);
        self.resize(nasize, totaluse - na);
    }

    fn resize(&mut self, nasize: usize, nhsize: usize) {
        let oldasize = self.arr.len();
        let old_nodes = std::mem::take(&mut self.nodes);
        if nasize > oldasize {
            self.arr.resize(nasize, Slot::NIL);
        }
        self.set_node_vector(nhsize);
        if nasize < oldasize {
            let vanishing: Vec<(usize, Slot)> = self.arr[nasize..]
                .iter()
                .enumerate()
                .filter(|(_, v)| !v.v.is_nil())
                .map(|(i, v)| (nasize + i + 1, *v))
                .collect();
            self.arr.truncate(nasize);
            for (k, v) in vanishing {
                match self.pos(Value::Num(k as f64)) {
                    Pos::Arr(i) => self.arr[i] = v,
                    Pos::Node(i) => self.nodes[i].val = v,
                }
            }
        }
        for old in old_nodes.iter().rev() {
            if !old.val.v.is_nil() {
                match self.pos(old.key) {
                    Pos::Arr(i) => self.arr[i] = old.val,
                    Pos::Node(i) => self.nodes[i].val = old.val,
                }
            }
        }
    }

    /// The length operator on a table without `__len` (`luaH_getn`).
    pub fn border(&self) -> usize {
        let mut j = self.arr.len();
        if j > 0 && self.arr[j - 1].v.is_nil() {
            let mut i = 0;
            while j - i > 1 {
                let m = (i + j) / 2;
                if self.arr[m - 1].v.is_nil() {
                    j = m;
                } else {
                    i = m;
                }
            }
            return i;
        }
        if self.nodes.is_empty() {
            return j;
        }
        let mut i = j;
        j += 1;
        while !self.get_int(j).is_nil() {
            i = j;
            j *= 2;
            if j > i32::MAX as usize {
                let mut k = 1;
                while !self.get_int(k).is_nil() {
                    k += 1;
                }
                return k - 1;
            }
        }
        while j - i > 1 {
            let m = (i + j) / 2;
            if self.get_int(m).is_nil() {
                j = m;
            } else {
                i = m;
            }
        }
        i
    }

    /// The traversal index of `key` (`findindex`); `Err` if not present.
    fn findindex(&self, key: Value) -> Result<Option<usize>, InvalidKey> {
        if key.is_nil() {
            return Ok(None);
        }
        let i = arrayindex(key);
        if 0 < i && (i as usize) <= self.arr.len() {
            return Ok(Some(i as usize - 1));
        }
        // A key whose value is nil (even a collected one) is still found.
        self.find_node(key)
            .map(|n| Some(n + self.arr.len()))
            .ok_or(InvalidKey)
    }

    /// The entry after `key` (`luaH_next`); `Err` for a key not in the
    /// table.
    pub fn next(&self, key: Value) -> Result<Option<(Value, Value)>, InvalidKey> {
        Ok(self.next_slot(key)?.map(|(k, v)| (k, v.v)))
    }

    /// `next` with the value's taint.
    pub fn next_slot(&self, key: Value) -> Result<Option<(Value, Slot)>, InvalidKey> {
        let start = match self.findindex(key)? {
            None => 0,
            Some(i) => i + 1,
        };
        for i in start..self.arr.len() {
            if !self.arr[i].v.is_nil() {
                return Ok(Some((Value::Num((i + 1) as f64), self.arr[i])));
            }
        }
        let hstart = start.saturating_sub(self.arr.len());
        for i in hstart..self.nodes.len() {
            let n = &self.nodes[i];
            if !n.val.v.is_nil() {
                return Ok(Some((n.key, n.val)));
            }
        }
        Ok(None)
    }

    /// Every non-nil entry, in traversal order.
    pub fn entries(&self) -> impl Iterator<Item = (Value, Value)> + '_ {
        self.arr
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.v.is_nil())
            .map(|(i, v)| (Value::Num((i + 1) as f64), v.v))
            .chain(
                self.nodes
                    .iter()
                    .filter(|n| !n.val.v.is_nil())
                    .map(|n| (n.key, n.val.v)),
            )
    }

    /// Clears entries the collector found dead (weak tables): the value
    /// becomes nil, the key stays as a dead key (as in Lua).
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(Value, Value) -> bool) {
        for (i, v) in self.arr.iter_mut().enumerate() {
            if !v.v.is_nil() && !keep(Value::Num((i + 1) as f64), v.v) {
                *v = Slot::NIL;
            }
        }
        for n in self.nodes.iter_mut() {
            if !n.val.v.is_nil() && !keep(n.key, n.val.v) {
                n.val = Slot::NIL;
            }
        }
    }
}

/// Where a key's value lives.
enum Pos {
    Arr(usize),
    Node(usize),
}
