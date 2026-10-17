//! Coverage (spec Tooling §16.1.6, decision T188): with coverage on, the
//! VM counts the instructions each function of the chosen chunks runs and
//! the outcomes of each conditional jump; the compiled code is not
//! changed. Chunks are registered when they load, with every function in
//! them, so code that never ran counts as not covered.

use std::collections::HashMap;
use std::rc::Rc;

use crate::proto::{Op, Proto};

/// One function's counts.
pub struct ProtoCounts {
    pub proto: Rc<Proto>,
    /// Times each instruction ran.
    pub hits: Vec<u32>,
    /// For each conditional instruction (by pc): how often its jump was
    /// taken, and how often not.
    pub branches: HashMap<usize, [u32; 2]>,
    /// The function the chunk is (its top level).
    pub main: bool,
}

/// Which chunks (by name) coverage counts.
pub type ChunkFilter = Box<dyn Fn(&[u8]) -> bool>;

pub struct Coverage {
    pub protos: Vec<ProtoCounts>,
    by_ptr: HashMap<usize, usize>,
    filter: ChunkFilter,
}

/// Whether an instruction is a test followed by a jump.
pub fn is_conditional(op: &Op) -> bool {
    matches!(
        op,
        Op::Eq { .. } | Op::Lt { .. } | Op::Le { .. } | Op::Test { .. } | Op::TestSet { .. }
    )
}

impl Coverage {
    pub fn new(filter: ChunkFilter) -> Coverage {
        Coverage {
            protos: Vec::new(),
            by_ptr: HashMap::new(),
            filter,
        }
    }

    /// Registers a loaded chunk and every function in it.
    pub(crate) fn register(&mut self, proto: &Rc<Proto>) {
        if !(self.filter)(&proto.source) {
            return;
        }
        self.add(proto, true);
    }

    fn add(&mut self, proto: &Rc<Proto>, main: bool) {
        let ptr = Rc::as_ptr(proto) as usize;
        if self.by_ptr.contains_key(&ptr) {
            return;
        }
        self.by_ptr.insert(ptr, self.protos.len());
        self.protos.push(ProtoCounts {
            proto: proto.clone(),
            hits: vec![0; proto.code.len()],
            branches: HashMap::new(),
            main,
        });
        for p in &proto.protos {
            self.add(p, false);
        }
    }

    /// The counts' index of a function, if it is counted.
    pub(crate) fn index_of(&self, proto: &Rc<Proto>) -> Option<usize> {
        self.by_ptr.get(&(Rc::as_ptr(proto) as usize)).copied()
    }

    #[inline]
    pub(crate) fn hit(&mut self, i: usize, pc: usize) {
        if let Some(h) = self.protos[i].hits.get_mut(pc) {
            *h = h.saturating_add(1);
        }
    }

    pub(crate) fn branch(&mut self, i: usize, pc: usize, taken: bool) {
        let b = self.protos[i].branches.entry(pc).or_insert([0, 0]);
        let slot = &mut b[usize::from(!taken)];
        *slot = slot.saturating_add(1);
    }
}
