//! Taint (spec emulator §17.4, decision T193), exact as the client tracks
//! it (elune's `lsec.h`): every stored value carries the taint of the code
//! that wrote it, and each thread the taint of its execution.
//!
//! - Reading a tainted value into a register taints the execution (unless a
//!   tainted closure fixes it), and the register keeps the value's taint.
//! - A value written while the execution is tainted takes that taint.
//! - Table fields and upvalues keep the taint of the register stored.
//! - Calling a tainted closure taints the execution and fixes it there.

use crate::value::Value;

/// Who wrote a value: an addon's name (an index into the VM's names), or
/// none (secure).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Taint(pub(crate) u32);

impl Taint {
    pub const SECURE: Taint = Taint(0);

    pub fn is_secure(self) -> bool {
        self.0 == 0
    }
}

/// Why a value or an execution is tainted (decision T196): a record of the
/// VM's (`Vm::origin_record`), or none known: the code that made it is
/// itself tainted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Origin(pub(crate) u32);

impl Origin {
    pub const NONE: Origin = Origin(0);
}

/// A stored value: a stack slot, a table field or a closed upvalue.
#[derive(Clone, Copy, Debug, Default)]
pub struct Slot {
    pub v: Value,
    pub t: Taint,
    /// Where its taint came from (in the slot's padding: no extra size).
    pub o: Origin,
}

impl Slot {
    pub const NIL: Slot = Slot {
        v: Value::Nil,
        t: Taint::SECURE,
        o: Origin::NONE,
    };

    pub fn secure(v: Value) -> Slot {
        Slot {
            v,
            t: Taint::SECURE,
            o: Origin::NONE,
        }
    }
}

/// Reads taint the execution (`LUA_TAINTFLAG_RD`).
pub const READ: u8 = 1;
/// Writes take the execution's taint (`LUA_TAINTFLAG_WR`).
pub const WRITE: u8 = 2;

/// A thread's taint (elune's `TaintState` plus the fixed taint).
#[derive(Clone, Copy, Debug, Default)]
pub struct TaintState {
    /// [`READ`] | [`WRITE`]; 0 in stock Lua.
    pub mode: u8,
    /// The execution's taint (`stacktaint`).
    pub stack: Taint,
    /// The running Lua closure's own taint: while set, reads do not change
    /// the execution's taint (`fixedtaint`).
    pub fixed: Taint,
    /// Taint for new objects (`newgctaint`) and new closures (`newcltaint`).
    pub new_object: Taint,
    pub new_closure: Taint,
    /// Why the execution is tainted: the read that tainted it.
    pub origin: Origin,
    /// A read just tainted the execution: the value's origin, waiting for
    /// the VM to record where (`Vm::settle_origin`).
    pub tainted_by: Option<Origin>,
}

impl TaintState {
    /// The taint values written now take (`writetaint`).
    pub fn write(&self) -> Taint {
        if self.mode & WRITE != 0 {
            self.stack
        } else {
            Taint::SECURE
        }
    }

    /// Reading a value with `t` (`luaR_taintstack`).
    pub fn read(&mut self, t: Taint) {
        self.read_from(t, Origin::NONE);
    }

    /// Reading a value with `t` whose taint came from `o`.
    pub fn read_from(&mut self, t: Taint, o: Origin) {
        if !t.is_secure() && self.fixed.is_secure() && self.mode & READ != 0 {
            if self.stack != t {
                self.tainted_by = Some(o);
            }
            self.stack = t;
        }
    }

    /// The origin values made now take: the execution's, while tainted.
    fn fresh_origin(&self) -> Origin {
        if self.write().is_secure() {
            Origin::NONE
        } else {
            self.origin
        }
    }

    /// The register a copied value lands in (`setobj2s`): its own taint, or
    /// the write taint; a tainted value taints the execution.
    pub fn copy(&mut self, s: Slot) -> Slot {
        if s.t.is_secure() {
            Slot {
                v: s.v,
                t: self.write(),
                o: self.fresh_origin(),
            }
        } else {
            self.read_from(s.t, s.o);
            s
        }
    }

    /// A value made now (`setnvalue` and the like).
    pub fn fresh(&self, v: Value) -> Slot {
        Slot {
            v,
            t: self.write(),
            o: self.fresh_origin(),
        }
    }

    /// What `lua_savetaint` keeps (the fixed taint is not part of it).
    pub fn saved(&self) -> TaintState {
        TaintState {
            fixed: Taint::SECURE,
            ..*self
        }
    }

    /// `lua_restoretaint`: everything but the fixed taint.
    pub fn restore(&mut self, saved: TaintState) {
        let fixed = self.fixed;
        *self = TaintState { fixed, ..saved };
    }

    /// The taint a new object takes (`luaR_taintalloc`).
    pub fn alloc(&self, is_function: bool) -> Taint {
        if !self.new_object.is_secure() {
            self.new_object
        } else if !self.write().is_secure() {
            self.write()
        } else if is_function {
            self.new_closure
        } else {
            Taint::SECURE
        }
    }
}
