//! The heap: one arena per kind of collectable object, addressed by handles
//! (decision T191). A handle is an index plus the slot's generation, so a
//! handle to a freed slot is detected instead of reaching a new object.
//! Strings are interned: equal byte strings share one handle.

use std::collections::HashMap;
use std::rc::Rc;

use crate::function::{Function, Upval};
use crate::table::Table;
use crate::thread::Thread;
use crate::value::Value;

macro_rules! handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name {
            pub(crate) idx: u32,
            pub(crate) generation: u32,
        }
    };
}

/// An interned string, with its Lua 5.1 hash (`luaS_hash`), which table
/// lookups use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StrRef {
    pub(crate) idx: u32,
    pub(crate) generation: u32,
    pub(crate) hash: u32,
}

/// Lua 5.1's string hash.
pub fn lua_hash(bytes: &[u8]) -> u32 {
    let l = bytes.len();
    let mut h = l as u32;
    let step = (l >> 5) + 1;
    let mut l1 = l;
    while l1 >= step {
        h ^= (h << 5)
            .wrapping_add(h >> 2)
            .wrapping_add(u32::from(bytes[l1 - 1]));
        l1 -= step;
    }
    h
}
handle!(
    /// A table.
    TableRef
);
handle!(
    /// A Lua or native function.
    FuncRef
);
handle!(
    /// An upvalue cell.
    UpvalRef
);
handle!(
    /// A coroutine.
    ThreadRef
);
handle!(
    /// A full userdata.
    UdRef
);
handle!(
    /// A secret value (`secretwrap`): it holds the value it hides.
    SecretRef
);

impl SecretRef {
    /// A secret `nil`: falsy like `nil`, so it needs no slot.
    pub const NIL: SecretRef = SecretRef {
        idx: u32::MAX,
        generation: 0,
    };
}

struct Entry<T> {
    value: T,
    marked: bool,
}

/// Slots of one object kind.
pub(crate) struct Arena<T> {
    slots: Vec<Option<Entry<T>>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Arena {
            slots: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }
}

impl<T> Arena<T> {
    fn alloc(&mut self, value: T) -> (u32, u32) {
        self.live += 1;
        let entry = Some(Entry {
            value,
            marked: false,
        });
        if let Some(idx) = self.free.pop() {
            self.slots[idx as usize] = entry;
            (idx, self.generations[idx as usize])
        } else {
            let idx = u32::try_from(self.slots.len()).expect("heap exhausted");
            self.slots.push(entry);
            self.generations.push(0);
            (idx, 0)
        }
    }

    fn entry(&self, idx: u32, generation: u32) -> &Entry<T> {
        match self.slots.get(idx as usize) {
            Some(Some(e)) if self.generations[idx as usize] == generation => e,
            _ => panic!("stale handle {idx}/{generation}"),
        }
    }

    fn entry_mut(&mut self, idx: u32, generation: u32) -> &mut Entry<T> {
        match self.slots.get_mut(idx as usize) {
            Some(Some(e)) if self.generations[idx as usize] == generation => e,
            _ => panic!("stale handle {idx}/{generation}"),
        }
    }

    pub(crate) fn get(&self, idx: u32, generation: u32) -> &T {
        &self.entry(idx, generation).value
    }

    pub(crate) fn get_mut(&mut self, idx: u32, generation: u32) -> &mut T {
        &mut self.entry_mut(idx, generation).value
    }

    /// Marks the object; returns whether it was unmarked before.
    pub(crate) fn mark(&mut self, idx: u32, generation: u32) -> bool {
        let e = self.entry_mut(idx, generation);
        let first = !e.marked;
        e.marked = true;
        first
    }

    pub(crate) fn is_marked(&self, idx: u32, generation: u32) -> bool {
        self.entry(idx, generation).marked
    }

    /// Frees every unmarked object (calling `freed` with it first) and
    /// clears the marks of the rest.
    pub(crate) fn sweep(&mut self, mut freed: impl FnMut(&T)) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            match slot {
                Some(e) if e.marked => e.marked = false,
                Some(e) => {
                    freed(&e.value);
                    *slot = None;
                    self.generations[i] = self.generations[i].wrapping_add(1);
                    self.free.push(i as u32);
                    self.live -= 1;
                }
                None => {}
            }
        }
    }

    /// Like [`sweep`](Self::sweep), reporting each freed slot.
    pub(crate) fn sweep_with(&mut self, mut freed: impl FnMut(u32, u32)) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            match slot {
                Some(e) if e.marked => e.marked = false,
                Some(_) => {
                    freed(i as u32, self.generations[i]);
                    *slot = None;
                    self.generations[i] = self.generations[i].wrapping_add(1);
                    self.free.push(i as u32);
                    self.live -= 1;
                }
                None => {}
            }
        }
    }

    pub(crate) fn live(&self) -> usize {
        self.live
    }

    /// Every live object, mutably.
    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots
            .iter_mut()
            .filter_map(|s| s.as_mut().map(|e| &mut e.value))
    }

    /// Every live object, with its index and generation.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u32, u32, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.as_ref()
                .map(|e| (i as u32, self.generations[i], &e.value))
        })
    }
}

/// A Lua string: immutable bytes.
pub struct LuaStr {
    pub bytes: Rc<[u8]>,
}

/// A full userdata: an optional metatable and environment, and a payload the
/// host defines.
pub struct Userdata {
    /// Creation order (set by the heap): finalizers run newest first.
    pub serial: u64,
    pub meta: Option<TableRef>,
    pub env: Option<TableRef>,
    pub data: Box<dyn std::any::Any>,
}

/// Every collectable object.
#[derive(Default)]
pub struct Heap {
    pub(crate) strings: Arena<LuaStr>,
    intern: HashMap<Rc<[u8]>, StrRef>,
    pub(crate) tables: Arena<Table>,
    pub(crate) funcs: Arena<Function>,
    pub(crate) upvals: Arena<Upval>,
    pub(crate) threads: Arena<Thread>,
    pub(crate) userdata: Arena<Userdata>,
    pub(crate) secrets: Arena<Value>,
    /// Approximate bytes allocated since the last collection.
    pub(crate) debt: usize,
    /// The last userdata's creation serial.
    ud_serial: u64,
}

impl Heap {
    /// The interned string for `bytes`.
    pub fn intern(&mut self, bytes: &[u8]) -> StrRef {
        if let Some(&s) = self.intern.get(bytes) {
            return s;
        }
        self.debt += bytes.len() + 32;
        let rc: Rc<[u8]> = Rc::from(bytes);
        let (idx, generation) = self.strings.alloc(LuaStr { bytes: rc.clone() });
        let s = StrRef {
            idx,
            generation,
            hash: lua_hash(bytes),
        };
        self.intern.insert(rc, s);
        s
    }

    pub fn str_bytes(&self, s: StrRef) -> &[u8] {
        &self.strings.get(s.idx, s.generation).bytes
    }

    /// The bytes, shared (for holding across heap mutation).
    pub fn str_rc(&self, s: StrRef) -> Rc<[u8]> {
        self.strings.get(s.idx, s.generation).bytes.clone()
    }

    pub fn alloc_table(&mut self, t: Table) -> TableRef {
        self.debt += 64 + t.capacity_bytes();
        let (idx, generation) = self.tables.alloc(t);
        TableRef { idx, generation }
    }

    pub fn table(&self, t: TableRef) -> &Table {
        self.tables.get(t.idx, t.generation)
    }

    pub fn table_mut(&mut self, t: TableRef) -> &mut Table {
        self.tables.get_mut(t.idx, t.generation)
    }

    pub fn alloc_func(&mut self, f: Function) -> FuncRef {
        self.debt += 64;
        let (idx, generation) = self.funcs.alloc(f);
        FuncRef { idx, generation }
    }

    pub fn func(&self, f: FuncRef) -> &Function {
        self.funcs.get(f.idx, f.generation)
    }

    pub fn func_mut(&mut self, f: FuncRef) -> &mut Function {
        self.funcs.get_mut(f.idx, f.generation)
    }

    pub fn alloc_upval(&mut self, u: Upval) -> UpvalRef {
        self.debt += 32;
        let (idx, generation) = self.upvals.alloc(u);
        UpvalRef { idx, generation }
    }

    pub fn upval(&self, u: UpvalRef) -> &Upval {
        self.upvals.get(u.idx, u.generation)
    }

    pub fn upval_mut(&mut self, u: UpvalRef) -> &mut Upval {
        self.upvals.get_mut(u.idx, u.generation)
    }

    pub fn alloc_thread(&mut self, t: Thread) -> ThreadRef {
        self.debt += 256;
        let (idx, generation) = self.threads.alloc(t);
        ThreadRef { idx, generation }
    }

    pub fn thread(&self, t: ThreadRef) -> &Thread {
        self.threads.get(t.idx, t.generation)
    }

    pub fn thread_mut(&mut self, t: ThreadRef) -> &mut Thread {
        self.threads.get_mut(t.idx, t.generation)
    }

    pub fn alloc_userdata(&mut self, mut u: Userdata) -> UdRef {
        self.debt += 64;
        self.ud_serial += 1;
        u.serial = self.ud_serial;
        let (idx, generation) = self.userdata.alloc(u);
        UdRef { idx, generation }
    }

    pub fn alloc_secret(&mut self, v: Value) -> SecretRef {
        if v.is_nil() {
            return SecretRef::NIL;
        }
        self.debt += 32;
        let (idx, generation) = self.secrets.alloc(v);
        SecretRef { idx, generation }
    }

    /// The value a secret hides.
    pub fn secret(&self, s: SecretRef) -> Value {
        if s == SecretRef::NIL {
            return Value::Nil;
        }
        *self.secrets.get(s.idx, s.generation)
    }

    pub fn userdata(&self, u: UdRef) -> &Userdata {
        self.userdata.get(u.idx, u.generation)
    }

    pub fn userdata_mut(&mut self, u: UdRef) -> &mut Userdata {
        self.userdata.get_mut(u.idx, u.generation)
    }

    /// Frees unmarked strings, dropping them from the intern table.
    pub(crate) fn sweep_strings(&mut self) {
        let intern = &mut self.intern;
        self.strings.sweep(|s| {
            intern.remove(&s.bytes);
        });
    }

    /// Live objects of every kind (for `collectgarbage("count")`-style
    /// estimates and tests).
    pub fn live_objects(&self) -> usize {
        self.strings.live()
            + self.tables.live()
            + self.funcs.live()
            + self.upvals.live()
            + self.threads.live()
            + self.userdata.live()
    }

    /// An approximate size in bytes of everything live.
    pub fn estimate_bytes(&self) -> usize {
        let strings: usize = self
            .strings
            .iter()
            .map(|(_, _, s)| s.bytes.len() + 32)
            .sum();
        let tables: usize = self
            .tables
            .iter()
            .map(|(_, _, t)| 64 + t.capacity_bytes())
            .sum();
        strings
            + tables
            + self.funcs.live() * 64
            + self.upvals.live() * 32
            + self.threads.live() * 256
            + self.userdata.live() * 64
            + self.secrets.live() * 32
    }

    /// The value's object identity, for `tostring` and table keys.
    pub fn address(v: Value) -> Option<u64> {
        let (kind, idx) = match v {
            Value::Table(t) => (1u64, t.idx),
            Value::Func(f) => (2, f.idx),
            Value::Thread(t) => (3, t.idx),
            Value::Userdata(u) => (4, u.idx),
            _ => return None,
        };
        Some((kind << 28) | (u64::from(idx) * 8 + 0x1000_0000))
    }
}
