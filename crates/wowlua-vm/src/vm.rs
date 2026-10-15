//! The virtual machine: loading, calls, the interpreter loop, metamethods
//! and errors. Lua-to-Lua calls never recurse on the Rust stack; a native
//! function that calls Lua (pcall, sort comparators, metamethods) runs a
//! nested loop, which is where Lua 5.1 draws its C-call boundary too.

use std::rc::Rc;

use crate::compiler;
use crate::error::LuaError;
use crate::function::{Function, LuaFunction, NativeCall, NativeFn, NativeFunction, Upval};
use crate::heap::{FuncRef, Heap, StrRef, TableRef, ThreadRef, UpvalRef};
use crate::number::{fmt_num, str2num};
use crate::proto::{ArithOp, Op, Proto, Rk};
use crate::table::Table;
use crate::thread::{Frame, Status, Thread, ThreadState};
use crate::value::Value;

/// Lua-to-Lua call depth (`LUAI_MAXCALLS`).
const MAXCALLS: usize = 20000;
/// Nested native-to-Lua calls (`LUAI_MAXCCALLS`).
const MAXCCALLS: usize = 200;
/// `__index`/`__newindex` chains (`MAXTAGLOOP`).
const MAXTAGLOOP: usize = 100;
const LFIELDS_PER_FLUSH: usize = 50;

macro_rules! names {
    ($($field:ident = $text:literal),* $(,)?) => {
        /// Interned metamethod and library names.
        #[derive(Clone, Copy)]
        pub struct Names { $(pub $field: StrRef),* }
        impl Names {
            fn new(heap: &mut Heap) -> Names {
                Names { $($field: heap.intern($text.as_bytes())),* }
            }
            pub fn all(&self) -> Vec<StrRef> {
                vec![$(self.$field),*]
            }
        }
    };
}

names! {
    index = "__index",
    newindex = "__newindex",
    call = "__call",
    add = "__add",
    sub = "__sub",
    mul = "__mul",
    div = "__div",
    mod_ = "__mod",
    pow = "__pow",
    unm = "__unm",
    len = "__len",
    lt = "__lt",
    le = "__le",
    eq = "__eq",
    concat = "__concat",
    gc = "__gc",
    mode = "__mode",
    metatable = "__metatable",
    tostring = "__tostring",
    tostring_fn = "tostring",
    n = "n",
}

/// A native function on the call stack (for levels and messages).
#[derive(Clone, Copy)]
pub(crate) struct CEntry {
    /// Lua frames below it.
    pub at_frames: usize,
    pub func: FuncRef,
    /// Called directly by a Lua instruction (`CALL`, `TFORLOOP`).
    pub from_lua: bool,
}

/// One entry of the combined Lua and native call stack.
pub(crate) enum StackEntry {
    Lua(usize),
    Native(usize),
}

pub struct Vm {
    pub heap: Heap,
    pub(crate) cur: ThreadState,
    pub(crate) current: ThreadRef,
    pub(crate) main: ThreadRef,
    pub globals: TableRef,
    pub registry: TableRef,
    pub string_meta: Option<TableRef>,
    pub names: Names,
    pub(crate) c_depth: usize,
    pub(crate) cstack: Vec<CEntry>,
    pub(crate) errfunc: Option<Value>,
    in_errfunc: bool,
    output: Output,
    /// Values kept alive outside the heap (host references).
    pub(crate) pinned: Vec<Value>,
    pub(crate) gc_threshold: usize,
    pub(crate) gc_stopped: bool,
    /// `collectgarbage("setpause")`, percent.
    pub(crate) gc_pause: usize,
    /// Userdata whose `__gc` already ran.
    pub(crate) finalized: std::collections::HashSet<crate::heap::UdRef>,
}

/// Where `print` writes.
type Output = Box<dyn FnMut(&[u8])>;

impl Default for Vm {
    fn default() -> Self {
        Vm::new()
    }
}

impl Vm {
    /// A VM with Lua 5.1's base library and the standard libraries.
    pub fn new() -> Vm {
        let mut heap = Heap::default();
        let names = Names::new(&mut heap);
        let globals = heap.alloc_table(Table::new(0, 64));
        let registry = heap.alloc_table(Table::new(0, 8));
        let main = heap.alloc_thread(Thread {
            status: Status::Running,
            state: ThreadState::default(),
            start: None,
            resumed_from: None,
        });
        let mut vm = Vm {
            heap,
            cur: ThreadState::default(),
            current: main,
            main,
            globals,
            registry,
            string_meta: None,
            names,
            c_depth: 0,
            cstack: Vec::new(),
            errfunc: None,
            in_errfunc: false,
            output: Box::new(|bytes| {
                use std::io::Write;
                let _ = std::io::stdout().write_all(bytes);
            }),
            pinned: Vec::new(),
            gc_threshold: 4 << 20,
            gc_stopped: false,
            gc_pause: 200,
            finalized: std::collections::HashSet::new(),
        };
        vm.cur.stack.resize(64, Value::Nil);
        crate::stdlib::open(&mut vm);
        vm
    }

    /// Where `print` writes.
    pub fn set_output(&mut self, out: impl FnMut(&[u8]) + 'static) {
        self.output = Box::new(out);
    }

    pub(crate) fn write_output(&mut self, bytes: &[u8]) {
        (self.output)(bytes);
    }

    // ------------------------------------------------------------- strings

    pub fn str(&mut self, s: &str) -> Value {
        Value::Str(self.heap.intern(s.as_bytes()))
    }

    pub fn bytes(&mut self, b: &[u8]) -> Value {
        Value::Str(self.heap.intern(b))
    }

    pub fn str_bytes(&self, s: StrRef) -> &[u8] {
        self.heap.str_bytes(s)
    }

    // ----------------------------------------------------------- functions

    pub fn native(&mut self, name: &'static str, f: NativeFn) -> Value {
        self.native_with(name, f, Vec::new())
    }

    pub fn native_with(&mut self, name: &'static str, f: NativeFn, upvals: Vec<Value>) -> Value {
        let env = self.globals;
        Value::Func(self.heap.alloc_func(Function::Native(NativeFunction {
            f,
            name,
            upvals: upvals.into(),
            env,
        })))
    }

    pub fn set_global(&mut self, name: &str, v: Value) {
        let k = self.heap.intern(name.as_bytes());
        let g = self.globals;
        self.heap.table_mut(g).set_str(k, v);
    }

    pub fn get_global(&mut self, name: &str) -> Value {
        let k = self.heap.intern(name.as_bytes());
        self.heap.table(self.globals).get_str(k)
    }

    pub fn new_table(&mut self) -> TableRef {
        self.heap.alloc_table(Table::default())
    }

    /// Compiles a chunk. `chunkname` follows Lua: `@file`, `=name`, or the
    /// source itself.
    pub fn load(&mut self, src: &[u8], chunkname: &str) -> Result<Value, String> {
        let chunkid = chunkid(chunkname);
        // Lua 5.1 skips a first line starting with `#` in files only;
        // `loadstring` does not, so neither do we.
        let mut db = wowlua_span::SourceDb::new();
        let file = db.add(chunkname, String::from_utf8_lossy(src).into_owned());
        let parse = wowlua_syntax::parse(src, file);
        if let Some(d) = parse.diagnostics.iter().find(|d| d.is_error()) {
            let line = d
                .labels
                .first()
                .map(|l| line_of(src, l.span.start as usize))
                .unwrap_or(1);
            return Err(format!("{chunkid}:{line}: {}", d.message));
        }
        let proto = compiler::compile(&mut self.heap, &parse.chunk, src, Rc::from(chunkname))
            .map_err(|e| format!("{chunkid}:{}: {}", e.line, e.message))?;
        let env = self.globals;
        Ok(Value::Func(self.heap.alloc_func(Function::Lua(
            LuaFunction {
                proto,
                upvals: Box::new([]),
                env,
            },
        ))))
    }

    // --------------------------------------------------------------- calls

    fn ensure_stack(&mut self, size: usize) {
        if self.cur.stack.len() < size {
            let new = (size + 64).max(self.cur.stack.len() * 2);
            self.cur.stack.resize(new, Value::Nil);
        }
    }

    /// Calls `f` with `args`; returns all results. Errors unwind the call.
    pub fn call(&mut self, f: Value, args: &[Value]) -> Result<Vec<Value>, LuaError> {
        let func = self.cur.top.max(self.frame_top());
        self.ensure_stack(func + args.len() + 1);
        self.cur.stack[func] = f;
        self.cur.stack[func + 1..func + 1 + args.len()].copy_from_slice(args);
        let saved_frames = self.cur.frames.len();
        let saved_c = self.cstack.len();
        let saved_depth = self.c_depth;
        let saved_top = self.cur.top;
        match self.call_at(func, args.len(), -1, false) {
            Ok(()) => {
                let results = self.cur.stack[func..self.cur.top].to_vec();
                self.cur.top = saved_top;
                Ok(results)
            }
            Err(e) => {
                self.unwind(saved_frames, saved_c, func);
                self.c_depth = saved_depth;
                self.cur.top = saved_top;
                Err(e)
            }
        }
    }

    /// Like [`call`](Self::call), but errors are returned without running
    /// the error handler of an enclosing `xpcall` (Lua's `lua_pcall` with no
    /// handler).
    pub fn pcall(&mut self, f: Value, args: &[Value]) -> Result<Vec<Value>, LuaError> {
        let saved = self.errfunc.take();
        let r = self.call(f, args);
        self.errfunc = saved;
        r
    }

    /// Restores the stack after an error: frames, native entries and open
    /// upvalues above `level`.
    pub(crate) fn unwind(&mut self, frames: usize, cstack: usize, level: usize) {
        self.cur.frames.truncate(frames);
        self.cstack.truncate(cstack);
        self.close_upvals(level);
    }

    /// The first free slot above the running frame.
    fn frame_top(&self) -> usize {
        match self.cur.frames.last() {
            Some(f) => f.base + f.proto.max_stack as usize,
            None => 0,
        }
    }

    /// Calls the function at `func` with `nargs` arguments above it, leaving
    /// the results at `func` (`nresults` of them, or all with -1 and `top`
    /// set). A nested run: counts toward the C-call limit.
    pub(crate) fn call_at(
        &mut self,
        func: usize,
        nargs: usize,
        nresults: i32,
        from_lua: bool,
    ) -> Result<(), LuaError> {
        self.c_depth += 1;
        if self.c_depth >= MAXCCALLS {
            if self.c_depth == MAXCCALLS {
                self.c_depth -= 1;
                return Err(self.rt_error("C stack overflow"));
            }
            if self.c_depth >= MAXCCALLS + (MAXCCALLS >> 3) {
                self.c_depth -= 1;
                let v = self.str("error in error handling");
                return Err(LuaError::new(v));
            }
        }
        let r = match self.precall(func, nargs, nresults, from_lua) {
            Ok(true) => {
                self.cur.frames.last_mut().expect("frame").entry = true;
                self.execute()
            }
            Ok(false) => Ok(()),
            Err(e) => Err(e),
        };
        self.c_depth -= 1;
        r
    }

    /// Prepares a call. Lua functions get a frame (true); natives run now.
    fn precall(
        &mut self,
        func: usize,
        nargs: usize,
        nresults: i32,
        from_lua: bool,
    ) -> Result<bool, LuaError> {
        let mut nargs = nargs;
        let f = match self.cur.stack[func] {
            Value::Func(f) => f,
            other => {
                // `__call`
                let tm = self.get_tm(other, self.names.call);
                let Value::Func(f) = tm else {
                    return Err(self.call_error(func));
                };
                self.ensure_stack(func + nargs + 2);
                for i in (func..=func + nargs).rev() {
                    self.cur.stack[i + 1] = self.cur.stack[i];
                }
                self.cur.stack[func] = tm;
                nargs += 1;
                f
            }
        };
        match self.heap.func(f) {
            Function::Lua(lf) => {
                let proto = lf.proto.clone();
                if self.cur.frames.len() >= MAXCALLS {
                    return Err(self.rt_error("stack overflow"));
                }
                let np = proto.num_params as usize;
                let (base, nvarargs) = if proto.is_vararg {
                    // Fixed parameters move above the actual arguments; the
                    // extra arguments stay below the frame (adjust_varargs).
                    let top = func + 1 + nargs;
                    self.ensure_stack(top + np + proto.max_stack as usize + 1);
                    let mut actual = nargs;
                    while actual < np {
                        self.cur.stack[func + 1 + actual] = Value::Nil;
                        actual += 1;
                    }
                    let fixed = func + 1;
                    let base = func + 1 + actual;
                    for i in 0..np {
                        self.cur.stack[base + i] = self.cur.stack[fixed + i];
                        self.cur.stack[fixed + i] = Value::Nil;
                    }
                    (base, actual - np)
                } else {
                    (func + 1, 0)
                };
                let top = base + proto.max_stack as usize;
                self.ensure_stack(top + 1);
                // Missing parameters and the other registers start nil.
                let first_nil = if proto.is_vararg {
                    base + np
                } else {
                    base + nargs.min(np)
                };
                for slot in &mut self.cur.stack[first_nil..top] {
                    *slot = Value::Nil;
                }
                self.cur.frames.push(Frame {
                    closure: f,
                    proto,
                    func,
                    base,
                    pc: 0,
                    nresults,
                    nvarargs,
                    tail: false,
                    entry: false,
                });
                Ok(true)
            }
            Function::Native(nf) => {
                let (native, upvals) = (nf.f, nf.upvals.clone());
                let args = self.cur.stack[func + 1..func + 1 + nargs].to_vec();
                // Nested calls from the native go above its arguments.
                self.cur.top = func + 1 + nargs;
                self.cstack.push(CEntry {
                    at_frames: self.cur.frames.len(),
                    func: f,
                    from_lua,
                });
                let r = native(self, NativeCall { args, upvals });
                self.cstack.pop();
                let results = r?;
                self.place_results(func, &results, nresults);
                Ok(false)
            }
        }
    }

    fn place_results(&mut self, func: usize, results: &[Value], nresults: i32) {
        let n = if nresults < 0 {
            results.len()
        } else {
            nresults as usize
        };
        self.ensure_stack(func + n + 1);
        for i in 0..n {
            self.cur.stack[func + i] = results.get(i).copied().unwrap_or_default();
        }
        if nresults < 0 {
            self.cur.top = func + n;
        }
    }

    /// Returns from the top frame with the `n` values at `from`; true if it
    /// was the entry frame of the current `execute`.
    fn poscall(&mut self, from: usize, n: usize) -> bool {
        let frame = self.cur.frames.pop().expect("frame");
        let dest = frame.func;
        let wanted = frame.nresults;
        let count = if wanted < 0 { n } else { wanted as usize };
        self.ensure_stack(dest + count + 1);
        for i in 0..count {
            self.cur.stack[dest + i] = if i < n {
                self.cur.stack[from + i]
            } else {
                Value::Nil
            };
        }
        if wanted < 0 {
            self.cur.top = dest + n;
        }
        frame.entry
    }

    // ------------------------------------------------------------ upvalues

    fn find_upval(&mut self, idx: usize) -> UpvalRef {
        let open = &self.cur.open_upvals;
        match open.binary_search_by_key(&idx, |&(i, _)| i) {
            Ok(i) => open[i].1,
            Err(at) => {
                let u = self.heap.alloc_upval(Upval::Open {
                    thread: self.current,
                    idx,
                });
                self.cur.open_upvals.insert(at, (idx, u));
                u
            }
        }
    }

    pub(crate) fn close_upvals(&mut self, level: usize) {
        while let Some(&(idx, u)) = self.cur.open_upvals.last() {
            if idx < level {
                break;
            }
            let v = self.cur.stack[idx];
            *self.heap.upval_mut(u) = Upval::Closed(v);
            self.cur.open_upvals.pop();
        }
    }

    pub(crate) fn upval_get(&self, u: UpvalRef) -> Value {
        match *self.heap.upval(u) {
            Upval::Closed(v) => v,
            Upval::Open { thread, idx } => {
                if thread == self.current {
                    self.cur.stack[idx]
                } else {
                    self.heap.thread(thread).state.stack[idx]
                }
            }
        }
    }

    fn upval_set(&mut self, u: UpvalRef, v: Value) {
        match *self.heap.upval(u) {
            Upval::Closed(_) => *self.heap.upval_mut(u) = Upval::Closed(v),
            Upval::Open { thread, idx } => {
                if thread == self.current {
                    self.cur.stack[idx] = v;
                } else {
                    self.heap.thread_mut(thread).state.stack[idx] = v;
                }
            }
        }
    }

    // ---------------------------------------------------------- the loop

    fn rk(&self, base: usize, k: &[Value], rk: Rk) -> Value {
        match rk {
            Rk::R(r) => self.cur.stack[base + r as usize],
            Rk::K(i) => k[i as usize],
        }
    }

    /// Runs frames until the entry frame returns.
    pub(crate) fn execute(&mut self) -> Result<(), LuaError> {
        'frame: loop {
            let fi = self.cur.frames.len() - 1;
            let (closure, proto, base) = {
                let f = &self.cur.frames[fi];
                (f.closure, f.proto.clone(), f.base)
            };
            let k = &proto.constants[..];
            loop {
                let pc = self.cur.frames[fi].pc;
                let op = proto.code[pc];
                self.cur.frames[fi].pc = pc + 1;
                match op {
                    Op::Move { a, b } => {
                        self.cur.stack[base + a as usize] = self.cur.stack[base + b as usize];
                    }
                    Op::LoadK { a, k: i } => {
                        self.cur.stack[base + a as usize] = k[i as usize];
                    }
                    Op::LoadBool { a, value, skip } => {
                        self.cur.stack[base + a as usize] = Value::Bool(value);
                        if skip {
                            self.cur.frames[fi].pc += 1;
                        }
                    }
                    Op::LoadNil { a, b } => {
                        for r in a..=b {
                            self.cur.stack[base + r as usize] = Value::Nil;
                        }
                    }
                    Op::GetUpval { a, u } => {
                        let uv = self.closure_upval(closure, u);
                        self.cur.stack[base + a as usize] = self.upval_get(uv);
                    }
                    Op::GetGlobal { a, k: i } => {
                        let env = Value::Table(self.heap.func(closure).env());
                        let v = self.index(env, k[i as usize], None)?;
                        self.cur.stack[base + a as usize] = v;
                    }
                    Op::GetTable { a, b, c } => {
                        let t = self.cur.stack[base + b as usize];
                        let key = self.rk(base, k, c);
                        let v = match t {
                            Value::Table(tr) => {
                                let v = self.heap.table(tr).get(key);
                                if v.is_nil() && self.heap.table(tr).meta.is_some() {
                                    self.index(t, key, Some(b))?
                                } else {
                                    v
                                }
                            }
                            _ => self.index(t, key, Some(b))?,
                        };
                        self.cur.stack[base + a as usize] = v;
                    }
                    Op::SetGlobal { a, k: i } => {
                        let env = Value::Table(self.heap.func(closure).env());
                        let v = self.cur.stack[base + a as usize];
                        self.set_index(env, k[i as usize], v, None)?;
                    }
                    Op::SetUpval { a, u } => {
                        let uv = self.closure_upval(closure, u);
                        let v = self.cur.stack[base + a as usize];
                        self.upval_set(uv, v);
                    }
                    Op::SetTable { a, b, c } => {
                        let t = self.cur.stack[base + a as usize];
                        let key = self.rk(base, k, b);
                        let v = self.rk(base, k, c);
                        self.set_index(t, key, v, Some(a))?;
                    }
                    Op::NewTable { a, narr, nhash } => {
                        let t = self
                            .heap
                            .alloc_table(Table::new(narr as usize, nhash as usize));
                        self.cur.stack[base + a as usize] = Value::Table(t);
                        self.gc_step();
                    }
                    Op::SelfOp { a, b, c } => {
                        let obj = self.cur.stack[base + b as usize];
                        let key = self.rk(base, k, c);
                        self.cur.stack[base + a as usize + 1] = obj;
                        let v = self.index(obj, key, Some(b))?;
                        self.cur.stack[base + a as usize] = v;
                    }
                    Op::Arith { op, a, b, c } => {
                        let x = self.rk(base, k, b);
                        let y = self.rk(base, k, c);
                        let v = match (x, y) {
                            (Value::Num(x), Value::Num(y)) => Value::Num(arith(op, x, y)),
                            _ => self.arith_slow(op, x, y, b, c)?,
                        };
                        self.cur.stack[base + a as usize] = v;
                    }
                    Op::Unm { a, b } => {
                        let x = self.cur.stack[base + b as usize];
                        let v = match x {
                            Value::Num(n) => Value::Num(-n),
                            _ => self.unm_slow(x, b)?,
                        };
                        self.cur.stack[base + a as usize] = v;
                    }
                    Op::Not { a, b } => {
                        let x = self.cur.stack[base + b as usize];
                        self.cur.stack[base + a as usize] = Value::Bool(!x.truthy());
                    }
                    Op::Len { a, b } => {
                        let x = self.cur.stack[base + b as usize];
                        let v = self.len(x, Some(b))?;
                        self.cur.stack[base + a as usize] = v;
                    }
                    Op::Concat { a, b, c } => {
                        let v = self.concat_range(base, b, c)?;
                        self.cur.stack[base + a as usize] = v;
                        self.gc_step();
                    }
                    Op::Jmp { sbx } => {
                        self.jump(fi, sbx);
                    }
                    Op::Eq { a, b, c } => {
                        let x = self.rk(base, k, b);
                        let y = self.rk(base, k, c);
                        let r = self.equals(x, y)?;
                        self.cond_jump(fi, &proto, r == a);
                    }
                    Op::Lt { a, b, c } => {
                        let x = self.rk(base, k, b);
                        let y = self.rk(base, k, c);
                        let r = self.less_than(x, y)?;
                        self.cond_jump(fi, &proto, r == a);
                    }
                    Op::Le { a, b, c } => {
                        let x = self.rk(base, k, b);
                        let y = self.rk(base, k, c);
                        let r = self.less_equal(x, y)?;
                        self.cond_jump(fi, &proto, r == a);
                    }
                    Op::Test { a, c } => {
                        let t = self.cur.stack[base + a as usize].truthy();
                        self.cond_jump(fi, &proto, t == c);
                    }
                    Op::TestSet { a, b, c } => {
                        let v = self.cur.stack[base + b as usize];
                        if v.truthy() == c {
                            self.cur.stack[base + a as usize] = v;
                            self.cond_jump(fi, &proto, true);
                        } else {
                            self.cond_jump(fi, &proto, false);
                        }
                    }
                    Op::Call { a, b, c } => {
                        let func = base + a as usize;
                        let nargs = if b == 0 {
                            self.cur.top - func - 1
                        } else {
                            b as usize - 1
                        };
                        let nresults = c as i32 - 1;
                        if self.precall(func, nargs, nresults, true)? {
                            continue 'frame;
                        }
                    }
                    Op::TailCall { a, b } => {
                        let func = base + a as usize;
                        let nargs = if b == 0 {
                            self.cur.top - func - 1
                        } else {
                            b as usize - 1
                        };
                        self.close_upvals(base);
                        let cur_func = self.cur.frames[fi].func;
                        let nresults = self.cur.frames[fi].nresults;
                        let entry = self.cur.frames[fi].entry;
                        // Move the callee and arguments down to this frame.
                        for i in 0..=nargs {
                            self.cur.stack[cur_func + i] = self.cur.stack[func + i];
                        }
                        if matches!(self.callee_kind(cur_func), Some(true)) {
                            self.cur.frames.pop();
                            self.precall(cur_func, nargs, nresults, true)?;
                            let f = self.cur.frames.last_mut().expect("frame");
                            f.tail = true;
                            f.entry = entry;
                            continue 'frame;
                        }
                        // A native (or `__call`) callee: call it here; the
                        // following RETURN returns its results.
                        if self.precall(func, nargs, -1, true)? {
                            continue 'frame;
                        }
                    }
                    Op::Return { a, b } => {
                        let from = base + a as usize;
                        let n = if b == 0 {
                            self.cur.top - from
                        } else {
                            b as usize - 1
                        };
                        self.close_upvals(base);
                        if self.poscall(from, n) {
                            return Ok(());
                        }
                        continue 'frame;
                    }
                    Op::ForLoop { a, sbx } => {
                        let ra = base + a as usize;
                        let (Value::Num(idx), Value::Num(limit), Value::Num(step)) = (
                            self.cur.stack[ra],
                            self.cur.stack[ra + 1],
                            self.cur.stack[ra + 2],
                        ) else {
                            unreachable!("for loop prepared")
                        };
                        let idx = idx + step;
                        if if step > 0.0 {
                            idx <= limit
                        } else {
                            limit <= idx
                        } {
                            self.jump(fi, sbx);
                            self.cur.stack[ra] = Value::Num(idx);
                            self.cur.stack[ra + 3] = Value::Num(idx);
                        }
                    }
                    Op::ForPrep { a, sbx } => {
                        let ra = base + a as usize;
                        let init = self.tonumber(self.cur.stack[ra]);
                        let limit = self.tonumber(self.cur.stack[ra + 1]);
                        let step = self.tonumber(self.cur.stack[ra + 2]);
                        let Some(init) = init else {
                            return Err(self.rt_error("'for' initial value must be a number"));
                        };
                        let Some(limit) = limit else {
                            return Err(self.rt_error("'for' limit must be a number"));
                        };
                        let Some(step) = step else {
                            return Err(self.rt_error("'for' step must be a number"));
                        };
                        self.cur.stack[ra] = Value::Num(init - step);
                        self.cur.stack[ra + 1] = Value::Num(limit);
                        self.cur.stack[ra + 2] = Value::Num(step);
                        self.jump(fi, sbx);
                    }
                    Op::TForLoop { a, c } => {
                        let ra = base + a as usize;
                        let cb = ra + 3;
                        self.cur.stack[cb + 2] = self.cur.stack[ra + 2];
                        self.cur.stack[cb + 1] = self.cur.stack[ra + 1];
                        self.cur.stack[cb] = self.cur.stack[ra];
                        self.call_at(cb, 2, c as i32, true)?;
                        let v = self.cur.stack[cb];
                        if !v.is_nil() {
                            self.cur.stack[cb - 1] = v;
                        } else {
                            self.cur.frames[fi].pc += 1;
                        }
                    }
                    Op::SetList { a, b, c } => {
                        let ra = base + a as usize;
                        let n = if b == 0 {
                            self.cur.top - ra - 1
                        } else {
                            b as usize
                        };
                        let Value::Table(t) = self.cur.stack[ra] else {
                            unreachable!("SETLIST on a table")
                        };
                        let first = (c as usize - 1) * LFIELDS_PER_FLUSH;
                        let last = first + n;
                        let table = self.heap.table_mut(t);
                        if last > table.array_len() {
                            table.resize_array(last);
                        }
                        for i in (1..=n).rev() {
                            let v = self.cur.stack[ra + i];
                            self.heap.table_mut(t).set_int(first + i, v);
                        }
                    }
                    Op::Close { a } => {
                        self.close_upvals(base + a as usize);
                    }
                    Op::Closure { a, proto: pi } => {
                        let p = proto.protos[pi as usize].clone();
                        let mut upvals = Vec::with_capacity(p.upvals.len());
                        for d in &p.upvals {
                            upvals.push(match *d {
                                crate::proto::UpvalDesc::Local(r) => {
                                    self.find_upval(base + r as usize)
                                }
                                crate::proto::UpvalDesc::Upval(u) => self.closure_upval(closure, u),
                            });
                        }
                        let env = self.heap.func(closure).env();
                        let f = self.heap.alloc_func(Function::Lua(LuaFunction {
                            proto: p,
                            upvals: upvals.into_boxed_slice(),
                            env,
                        }));
                        self.cur.stack[base + a as usize] = Value::Func(f);
                        self.gc_step();
                    }
                    Op::Vararg { a, b } => {
                        let n = self.cur.frames[fi].nvarargs;
                        let ra = base + a as usize;
                        let wanted = if b == 0 {
                            self.ensure_stack(ra + n + 1);
                            self.cur.top = ra + n;
                            n
                        } else {
                            b as usize - 1
                        };
                        for j in 0..wanted {
                            self.cur.stack[ra + j] = if j < n {
                                self.cur.stack[base - n + j]
                            } else {
                                Value::Nil
                            };
                        }
                    }
                }
            }
        }
    }

    fn jump(&mut self, fi: usize, sbx: i32) {
        let f = &mut self.cur.frames[fi];
        f.pc = (f.pc as i64 + i64::from(sbx)) as usize;
    }

    /// After a test: take the following jump if `taken`, else skip it.
    fn cond_jump(&mut self, fi: usize, proto: &Proto, taken: bool) {
        let pc = self.cur.frames[fi].pc;
        if taken {
            let Op::Jmp { sbx } = proto.code[pc] else {
                unreachable!("a test is followed by a jump")
            };
            self.cur.frames[fi].pc = (pc as i64 + 1 + i64::from(sbx)) as usize;
        } else {
            self.cur.frames[fi].pc = pc + 1;
        }
    }

    fn closure_upval(&self, closure: FuncRef, u: u8) -> UpvalRef {
        match self.heap.func(closure) {
            Function::Lua(f) => f.upvals[u as usize],
            Function::Native(_) => unreachable!("native closures have no upvalue cells"),
        }
    }

    /// Whether the value at `slot` is a Lua function (true), a native
    /// (false), or not a function (None).
    fn callee_kind(&self, slot: usize) -> Option<bool> {
        match self.cur.stack[slot] {
            Value::Func(f) => Some(matches!(self.heap.func(f), Function::Lua(_))),
            _ => None,
        }
    }

    // ---------------------------------------------------------- metatables

    pub fn get_metatable(&self, v: Value) -> Option<TableRef> {
        match v {
            Value::Table(t) => self.heap.table(t).meta,
            Value::Userdata(u) => self.heap.userdata(u).meta,
            Value::Str(_) => self.string_meta,
            _ => None,
        }
    }

    pub fn get_tm(&self, v: Value, event: StrRef) -> Value {
        match self.get_metatable(v) {
            Some(m) => self.heap.table(m).get_str(event),
            None => Value::Nil,
        }
    }

    /// Calls a metamethod with arguments; returns its first result.
    pub(crate) fn call_tm(&mut self, tm: Value, args: &[Value]) -> Result<Value, LuaError> {
        let func = self.cur.top.max(self.frame_top());
        self.ensure_stack(func + args.len() + 2);
        self.cur.stack[func] = tm;
        self.cur.stack[func + 1..func + 1 + args.len()].copy_from_slice(args);
        self.call_at(func, args.len(), 1, false)?;
        Ok(self.cur.stack[func])
    }

    /// `t[key]` with `__index` (`luaV_gettable`). `reg` names the operand in
    /// errors (the register of `t` in the running Lua frame).
    pub fn index(&mut self, t: Value, key: Value, reg: Option<u8>) -> Result<Value, LuaError> {
        let mut t = t;
        let mut reg = reg;
        for _ in 0..MAXTAGLOOP {
            let tm = if let Value::Table(tr) = t {
                let table = self.heap.table(tr);
                let v = table.get(key);
                if !v.is_nil() {
                    return Ok(v);
                }
                match table.meta {
                    None => return Ok(Value::Nil),
                    Some(m) => {
                        let tm = self.heap.table(m).get_str(self.names.index);
                        if tm.is_nil() {
                            return Ok(Value::Nil);
                        }
                        tm
                    }
                }
            } else {
                let tm = self.get_tm(t, self.names.index);
                if tm.is_nil() {
                    return Err(self.type_error(t, "index", reg));
                }
                tm
            };
            if matches!(tm, Value::Func(_)) {
                return self.call_tm(tm, &[t, key]);
            }
            t = tm;
            reg = None;
        }
        Err(self.rt_error("loop in gettable"))
    }

    /// `t[key] = v` with `__newindex` (`luaV_settable`).
    pub fn set_index(
        &mut self,
        t: Value,
        key: Value,
        v: Value,
        reg: Option<u8>,
    ) -> Result<(), LuaError> {
        let mut t = t;
        let mut reg = reg;
        for _ in 0..MAXTAGLOOP {
            let tm = if let Value::Table(tr) = t {
                // The key is checked first, even with `__newindex`.
                if let Err(e) = crate::table::check_key(key) {
                    return Err(self.rt_error(e.message()));
                }
                let table = self.heap.table(tr);
                let existing = table.get(key);
                let tm = match table.meta {
                    Some(m) if existing.is_nil() => self.heap.table(m).get_str(self.names.newindex),
                    _ => Value::Nil,
                };
                if tm.is_nil() {
                    return self
                        .heap
                        .table_mut(tr)
                        .set(key, v)
                        .map_err(|e| self.rt_error(e.message()));
                }
                tm
            } else {
                let tm = self.get_tm(t, self.names.newindex);
                if tm.is_nil() {
                    return Err(self.type_error(t, "index", reg));
                }
                tm
            };
            if matches!(tm, Value::Func(_)) {
                let func = self.cur.top.max(self.frame_top());
                self.ensure_stack(func + 4);
                self.cur.stack[func] = tm;
                self.cur.stack[func + 1] = t;
                self.cur.stack[func + 2] = key;
                self.cur.stack[func + 3] = v;
                return self.call_at(func, 3, 0, false);
            }
            t = tm;
            reg = None;
        }
        Err(self.rt_error("loop in settable"))
    }

    // ---------------------------------------------------------- arithmetic

    pub fn tonumber(&self, v: Value) -> Option<f64> {
        match v {
            Value::Num(n) => Some(n),
            Value::Str(s) => str2num(self.heap.str_bytes(s)),
            _ => None,
        }
    }

    fn arith_tm(&self, op: ArithOp) -> StrRef {
        match op {
            ArithOp::Add => self.names.add,
            ArithOp::Sub => self.names.sub,
            ArithOp::Mul => self.names.mul,
            ArithOp::Div => self.names.div,
            ArithOp::Mod => self.names.mod_,
            ArithOp::Pow => self.names.pow,
        }
    }

    fn arith_slow(
        &mut self,
        op: ArithOp,
        x: Value,
        y: Value,
        rb: Rk,
        rc: Rk,
    ) -> Result<Value, LuaError> {
        if let (Some(a), Some(b)) = (self.tonumber(x), self.tonumber(y)) {
            return self.arith_num(op, a, b);
        }
        let event = self.arith_tm(op);
        let mut tm = self.get_tm(x, event);
        if tm.is_nil() {
            tm = self.get_tm(y, event);
        }
        if tm.is_nil() {
            // The first operand that is not a number is blamed.
            let (bad, reg) = if self.tonumber(x).is_none() {
                (x, rb)
            } else {
                (y, rc)
            };
            let reg = match reg {
                Rk::R(r) => Some(r),
                Rk::K(_) => None,
            };
            return Err(self.type_error(bad, "perform arithmetic on", reg));
        }
        self.call_tm(tm, &[x, y])
    }

    /// Arithmetic on numbers, where WoW's rules apply (S2 hooks them in).
    pub(crate) fn arith_num(&mut self, op: ArithOp, a: f64, b: f64) -> Result<Value, LuaError> {
        Ok(Value::Num(arith(op, a, b)))
    }

    /// Arithmetic from native code (no operand names).
    pub fn arith_values(&mut self, op: ArithOp, x: Value, y: Value) -> Result<Value, LuaError> {
        if let (Value::Num(a), Value::Num(b)) = (x, y) {
            return self.arith_num(op, a, b);
        }
        self.arith_slow(op, x, y, Rk::K(0), Rk::K(0))
    }

    fn unm_slow(&mut self, x: Value, reg: u8) -> Result<Value, LuaError> {
        if let Some(n) = self.tonumber(x) {
            return Ok(Value::Num(-n));
        }
        let tm = self.get_tm(x, self.names.unm);
        if tm.is_nil() {
            return Err(self.type_error(x, "perform arithmetic on", Some(reg)));
        }
        self.call_tm(tm, &[x, x])
    }

    /// `#v` (Lua 5.1: `__len` only for userdata).
    pub fn len(&mut self, v: Value, reg: Option<u8>) -> Result<Value, LuaError> {
        match v {
            Value::Table(t) => Ok(Value::Num(self.heap.table(t).border() as f64)),
            Value::Str(s) => Ok(Value::Num(self.heap.str_bytes(s).len() as f64)),
            _ => {
                let tm = self.get_tm(v, self.names.len);
                if tm.is_nil() {
                    return Err(self.type_error(v, "get length of", reg));
                }
                self.call_tm(tm, &[v, Value::Nil])
            }
        }
    }

    /// `R(b) .. … .. R(c)`, right to left, with `__concat` (`luaV_concat`).
    fn concat_range(&mut self, base: usize, b: u8, c: u8) -> Result<Value, LuaError> {
        let first = base + b as usize;
        let mut top = base + c as usize + 1;
        let mut total = c as usize - b as usize + 1;
        while total > 1 {
            let x = self.cur.stack[top - 2];
            let y = self.cur.stack[top - 1];
            let xs = self.is_strlike(x);
            let ys = self.is_strlike(y);
            if !xs || !ys {
                let mut tm = self.get_tm(x, self.names.concat);
                if tm.is_nil() {
                    tm = self.get_tm(y, self.names.concat);
                }
                if tm.is_nil() {
                    let (bad, slot) = if xs { (y, top - 1) } else { (x, top - 2) };
                    let reg = (slot - base) as u8;
                    return Err(self.type_error(bad, "concatenate", Some(reg)));
                }
                let r = self.call_tm(tm, &[x, y])?;
                self.cur.stack[top - 2] = r;
                top -= 1;
                total -= 1;
                continue;
            }
            // Collect as many string-like values as possible.
            let mut n = 1;
            while n < total && self.is_strlike(self.cur.stack[top - n - 1]) {
                n += 1;
            }
            let mut buf = Vec::new();
            for i in (top - n)..top {
                let v = self.cur.stack[i];
                self.append_strlike(&mut buf, v);
            }
            let s = self.heap.intern(&buf);
            self.cur.stack[top - n] = Value::Str(s);
            top -= n - 1;
            total -= n - 1;
        }
        let _ = first;
        Ok(self.cur.stack[top - 1])
    }

    fn is_strlike(&self, v: Value) -> bool {
        matches!(v, Value::Str(_) | Value::Num(_))
    }

    pub(crate) fn append_strlike(&self, buf: &mut Vec<u8>, v: Value) {
        match v {
            Value::Str(s) => buf.extend_from_slice(self.heap.str_bytes(s)),
            Value::Num(n) => buf.extend_from_slice(fmt_num(n).as_bytes()),
            _ => unreachable!("string-like"),
        }
    }

    /// Concatenates two values from native code.
    pub fn concat_values(&mut self, x: Value, y: Value) -> Result<Value, LuaError> {
        if self.is_strlike(x) && self.is_strlike(y) {
            let mut buf = Vec::new();
            self.append_strlike(&mut buf, x);
            self.append_strlike(&mut buf, y);
            return Ok(Value::Str(self.heap.intern(&buf)));
        }
        let mut tm = self.get_tm(x, self.names.concat);
        if tm.is_nil() {
            tm = self.get_tm(y, self.names.concat);
        }
        if tm.is_nil() {
            let bad = if self.is_strlike(x) { y } else { x };
            return Err(self.type_error(bad, "concatenate", None));
        }
        self.call_tm(tm, &[x, y])
    }

    // --------------------------------------------------------- comparison

    /// `__eq` applies to two tables or two userdata with the same handler.
    pub fn equals(&mut self, x: Value, y: Value) -> Result<bool, LuaError> {
        if x.raw_eq(y) {
            return Ok(true);
        }
        let same_kind = matches!(
            (x, y),
            (Value::Table(_), Value::Table(_)) | (Value::Userdata(_), Value::Userdata(_))
        );
        if !same_kind {
            return Ok(false);
        }
        let tm = self.comp_tm(x, y, self.names.eq);
        if tm.is_nil() {
            return Ok(false);
        }
        let r = self.call_tm(tm, &[x, y])?;
        Ok(r.truthy())
    }

    /// A comparison handler both operands share (`get_compTM`).
    fn comp_tm(&self, x: Value, y: Value, event: StrRef) -> Value {
        let (Some(m1), m2) = (self.get_metatable(x), self.get_metatable(y)) else {
            return Value::Nil;
        };
        let tm1 = self.heap.table(m1).get_str(event);
        if tm1.is_nil() {
            return Value::Nil;
        }
        if m2 == Some(m1) {
            return tm1;
        }
        let Some(m2) = m2 else { return Value::Nil };
        let tm2 = self.heap.table(m2).get_str(event);
        if tm1.raw_eq(tm2) { tm1 } else { Value::Nil }
    }

    fn order_tm(&mut self, x: Value, y: Value, event: StrRef) -> Result<Option<bool>, LuaError> {
        let tm1 = self.get_tm(x, event);
        if tm1.is_nil() {
            return Ok(None);
        }
        let tm2 = self.get_tm(y, event);
        if !tm1.raw_eq(tm2) {
            return Ok(None);
        }
        let r = self.call_tm(tm1, &[x, y])?;
        Ok(Some(r.truthy()))
    }

    fn order_error(&mut self, x: Value, y: Value) -> LuaError {
        let (t1, t2) = (x.type_name(), y.type_name());
        if t1 == t2 {
            self.rt_error(&format!("attempt to compare two {t1} values"))
        } else {
            self.rt_error(&format!("attempt to compare {t1} with {t2}"))
        }
    }

    pub fn less_than(&mut self, x: Value, y: Value) -> Result<bool, LuaError> {
        match (x, y) {
            (Value::Num(a), Value::Num(b)) => Ok(a < b),
            (Value::Str(a), Value::Str(b)) => Ok(self.heap.str_bytes(a) < self.heap.str_bytes(b)),
            _ if x.type_name() == y.type_name() => match self.order_tm(x, y, self.names.lt)? {
                Some(r) => Ok(r),
                None => Err(self.order_error(x, y)),
            },
            _ => Err(self.order_error(x, y)),
        }
    }

    pub fn less_equal(&mut self, x: Value, y: Value) -> Result<bool, LuaError> {
        match (x, y) {
            (Value::Num(a), Value::Num(b)) => Ok(a <= b),
            (Value::Str(a), Value::Str(b)) => Ok(self.heap.str_bytes(a) <= self.heap.str_bytes(b)),
            _ if x.type_name() == y.type_name() => {
                if let Some(r) = self.order_tm(x, y, self.names.le)? {
                    return Ok(r);
                }
                // Lua 5.1: `a <= b` as `not (b < a)`.
                match self.order_tm(y, x, self.names.lt)? {
                    Some(r) => Ok(!r),
                    None => Err(self.order_error(x, y)),
                }
            }
            _ => Err(self.order_error(x, y)),
        }
    }

    // ------------------------------------------------------------ strings

    /// `tostring(v)` without `__tostring`.
    pub fn raw_tostring(&mut self, v: Value) -> Vec<u8> {
        match v {
            Value::Nil => b"nil".to_vec(),
            Value::Bool(b) => {
                if b {
                    b"true".to_vec()
                } else {
                    b"false".to_vec()
                }
            }
            Value::Num(n) => fmt_num(n).into_bytes(),
            Value::Str(s) => self.heap.str_bytes(s).to_vec(),
            other => {
                let addr = Heap::address(other).unwrap_or(0);
                format!("{}: 0x{addr:08x}", other.type_name()).into_bytes()
            }
        }
    }

    /// `tostring(v)` with `__tostring`.
    pub fn tostring(&mut self, v: Value) -> Result<Value, LuaError> {
        let tm = self.get_tm(v, self.names.tostring);
        if !tm.is_nil() {
            return self.call_tm(tm, &[v]);
        }
        if let Value::Str(_) = v {
            return Ok(v);
        }
        let b = self.raw_tostring(v);
        Ok(self.bytes(&b))
    }

    // ------------------------------------------------------------- errors

    /// Raises `v` as an error: runs the active `xpcall` handler here, where
    /// the stack still shows the error.
    pub fn raise(&mut self, v: Value) -> LuaError {
        let Some(handler) = self.errfunc else {
            return LuaError::new(v);
        };
        if self.in_errfunc {
            let msg = self.str("error in error handling");
            return LuaError::new(msg);
        }
        self.in_errfunc = true;
        let r = self.call(handler, &[v]);
        self.in_errfunc = false;
        match r {
            Ok(results) => LuaError::new(results.first().copied().unwrap_or_default()),
            Err(_) => {
                let msg = self.str("error in error handling");
                LuaError::new(msg)
            }
        }
    }

    /// A runtime error with the position of the running Lua function.
    pub fn rt_error(&mut self, msg: &str) -> LuaError {
        let mut text = self.where_(0).into_bytes();
        text.extend_from_slice(msg.as_bytes());
        let v = self.bytes(&text);
        self.raise(v)
    }

    /// An error from native code with the position of its caller
    /// (`luaL_error`).
    pub fn error(&mut self, msg: &str) -> LuaError {
        let mut text = self.where_(1).into_bytes();
        text.extend_from_slice(msg.as_bytes());
        let v = self.bytes(&text);
        self.raise(v)
    }

    fn type_error(&mut self, v: Value, op: &str, reg: Option<u8>) -> LuaError {
        let t = v.type_name();
        let named = reg.and_then(|r| self.reg_name(r));
        match named {
            Some((kind, name)) => {
                self.rt_error(&format!("attempt to {op} {kind} '{name}' (a {t} value)"))
            }
            None => self.rt_error(&format!("attempt to {op} a {t} value")),
        }
    }

    fn call_error(&mut self, func: usize) -> LuaError {
        let v = self.cur.stack[func];
        // The register of the callee in the running frame, if it is one.
        let reg = self.cur.frames.last().and_then(|f| {
            if func >= f.base && func < f.base + f.proto.max_stack as usize {
                Some((func - f.base) as u8)
            } else {
                None
            }
        });
        let reg = if self.running_native() { None } else { reg };
        self.type_error(v, "call", reg)
    }

    /// Whether a native function is running above the top Lua frame.
    pub(crate) fn running_native(&self) -> bool {
        self.cstack
            .last()
            .is_some_and(|c| c.at_frames == self.cur.frames.len())
    }

    /// The running functions, innermost first.
    pub(crate) fn stack_entries(&self) -> Vec<StackEntry> {
        let mut out = Vec::new();
        let mut ci = self.cstack.len();
        let mut fi = self.cur.frames.len();
        loop {
            // Natives above Lua frame `fi - 1` come first (innermost).
            if ci > 0 && self.cstack[ci - 1].at_frames == fi {
                ci -= 1;
                out.push(StackEntry::Native(ci));
                continue;
            }
            if fi == 0 {
                break;
            }
            fi -= 1;
            out.push(StackEntry::Lua(fi));
            if self.cur.frames[fi].entry {
                // Natives that called into this nested run sit below it.
                continue;
            }
        }
        out
    }

    /// `chunk:line: ` of the function at `level` (0: the running one), or
    /// nothing for a native function (`luaL_where`).
    pub fn where_(&self, level: usize) -> String {
        match self.stack_entries().get(level) {
            Some(StackEntry::Lua(fi)) => {
                let f = &self.cur.frames[*fi];
                let line = f
                    .proto
                    .lines
                    .get(f.pc.saturating_sub(1))
                    .copied()
                    .unwrap_or(0);
                format!("{}:{line}: ", chunkid(&f.proto.source))
            }
            _ => String::new(),
        }
    }

    /// The kind and name of register `reg` in the running Lua frame
    /// (`getobjname`).
    fn reg_name(&self, reg: u8) -> Option<(&'static str, String)> {
        let f = self.cur.frames.last()?;
        obj_name(&self.heap, &f.proto, f.pc.saturating_sub(1), reg)
    }

    /// The name the running native function was called by, and whether it
    /// was called as a method (`getfuncname`).
    pub(crate) fn native_call_name(&self) -> Option<(&'static str, String)> {
        let c = self.cstack.last()?;
        if !c.from_lua || c.at_frames == 0 {
            return None;
        }
        let f = &self.cur.frames[c.at_frames - 1];
        let pc = f.pc.saturating_sub(1);
        match f.proto.code[pc] {
            Op::Call { a, .. } | Op::TailCall { a, .. } | Op::TForLoop { a, .. } => {
                obj_name(&self.heap, &f.proto, pc, a)
            }
            _ => None,
        }
    }

    /// `bad argument #n to 'name' (msg)` (`luaL_argerror`).
    pub fn arg_error(&mut self, narg: usize, msg: &str) -> LuaError {
        let mut narg = narg;
        let named = self.native_call_name();
        if let Some(("method", name)) = &named {
            narg -= 1;
            if narg == 0 {
                return self.error(&format!("calling '{name}' on bad self ({msg})"));
            }
        }
        let name = named.map(|(_, n)| n).unwrap_or_else(|| "?".to_string());
        self.error(&format!("bad argument #{narg} to '{name}' ({msg})"))
    }

    pub fn type_arg_error(&mut self, narg: usize, expected: &str, got: Option<Value>) -> LuaError {
        let got = match got {
            None => "no value",
            Some(v) => v.type_name(),
        };
        self.arg_error(narg, &format!("{expected} expected, got {got}"))
    }

    /// `msg` with the stack (`debug.traceback` / `debugstack`).
    pub fn traceback(&self, level: usize) -> String {
        let mut out = String::from("stack traceback:");
        for e in self.stack_entries().iter().skip(level) {
            match e {
                StackEntry::Lua(fi) => {
                    let f = &self.cur.frames[*fi];
                    let line = f
                        .proto
                        .lines
                        .get(f.pc.saturating_sub(1))
                        .copied()
                        .unwrap_or(0);
                    let what = if f.proto.line_defined == 0 {
                        "in main chunk".to_string()
                    } else {
                        format!(
                            "in function <{}:{}>",
                            chunkid(&f.proto.source),
                            f.proto.line_defined
                        )
                    };
                    out.push_str(&format!("\n\t{}:{line}: {what}", chunkid(&f.proto.source)));
                    if f.tail {
                        out.push_str("\n\t(tail call): ?");
                    }
                }
                StackEntry::Native(ci) => {
                    let name = match self.heap.func(self.cstack[*ci].func) {
                        Function::Native(n) => n.name,
                        Function::Lua(_) => "?",
                    };
                    out.push_str(&format!("\n\t[C]: in function '{name}'"));
                }
            }
        }
        out
    }
}

/// Number arithmetic as Lua 5.1 defines it (`luai_num*`).
pub fn arith(op: ArithOp, a: f64, b: f64) -> f64 {
    match op {
        ArithOp::Add => a + b,
        ArithOp::Sub => a - b,
        ArithOp::Mul => a * b,
        ArithOp::Div => a / b,
        ArithOp::Mod => a - (a / b).floor() * b,
        ArithOp::Pow => a.powf(b),
    }
}

/// The line of a byte offset (1-based).
fn line_of(src: &[u8], offset: usize) -> usize {
    src[..offset.min(src.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// Lua 5.1's `luaO_chunkid` (`LUA_IDSIZE` 60).
pub fn chunkid(source: &str) -> String {
    const IDSIZE: usize = 60;
    if let Some(rest) = source.strip_prefix('=') {
        return rest.chars().take(IDSIZE - 1).collect();
    }
    if let Some(rest) = source.strip_prefix('@') {
        let max = IDSIZE - " '...' ".len() - 1;
        if rest.len() > max {
            let cut = rest.len() - max;
            let mut i = cut;
            while !rest.is_char_boundary(i) {
                i += 1;
            }
            return format!("...{}", &rest[i..]);
        }
        return rest.to_string();
    }
    let first_line_len = source.find(['\n', '\r']).unwrap_or(source.len());
    let max = IDSIZE - " [string \"...\"] ".len() - 1;
    let len = first_line_len.min(max);
    let mut end = len;
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    if len < source.len() {
        format!("[string \"{}...\"]", &source[..end])
    } else {
        format!("[string \"{source}\"]")
    }
}

/// The name of register `reg` at instruction `pc` (`getobjname`).
pub(crate) fn obj_name(
    heap: &Heap,
    p: &Proto,
    pc: usize,
    reg: u8,
) -> Option<(&'static str, String)> {
    if let Some(name) = p.local_name(reg, pc) {
        return Some(("local", name.to_string()));
    }
    // A constant string key's text, else "?" (`kname`).
    let kname = |c: Rk| match c {
        Rk::K(k) => match p.constants[k as usize] {
            Value::Str(s) => String::from_utf8_lossy(heap.str_bytes(s)).into_owned(),
            _ => "?".to_string(),
        },
        Rk::R(_) => "?".to_string(),
    };
    match symbexec(p, pc, reg)? {
        Op::GetGlobal { k, .. } => Some(("global", kname(Rk::K(k)))),
        Op::Move { a, b } if b < a => obj_name(heap, p, pc, b),
        Op::GetTable { c, .. } => Some(("field", kname(c))),
        Op::GetUpval { u, .. } => Some((
            "upvalue",
            p.upval_names
                .get(u as usize)
                .map_or_else(|| "?".to_string(), |n| n.to_string()),
        )),
        Op::SelfOp { c, .. } => Some(("method", kname(c))),
        _ => None,
    }
}

/// The last instruction before `lastpc` that set `reg` (`symbexec`).
fn symbexec(p: &Proto, lastpc: usize, reg: u8) -> Option<Op> {
    let mut last: Option<usize> = None;
    let mut pc = 0;
    while pc < lastpc {
        let op = p.code[pc];
        match op {
            Op::LoadNil { a, b } => {
                if a <= reg && reg <= b {
                    last = Some(pc);
                }
            }
            Op::SelfOp { a, .. } => {
                if reg == a || reg == a + 1 {
                    last = Some(pc);
                }
            }
            Op::TForLoop { a, .. } => {
                if reg >= a + 2 {
                    last = Some(pc);
                }
            }
            Op::Call { a, .. } | Op::TailCall { a, .. } => {
                if reg >= a {
                    last = Some(pc);
                }
            }
            Op::Jmp { sbx } | Op::ForLoop { sbx, .. } | Op::ForPrep { sbx, .. } => {
                if let Op::ForLoop { a, .. } | Op::ForPrep { a, .. } = op
                    && a == reg
                {
                    last = Some(pc);
                }
                let dest = pc as i64 + 1 + i64::from(sbx);
                if (pc as i64) < dest && dest <= lastpc as i64 {
                    pc = dest as usize;
                    continue;
                }
            }
            Op::Move { a, .. }
            | Op::LoadK { a, .. }
            | Op::LoadBool { a, .. }
            | Op::GetUpval { a, .. }
            | Op::GetGlobal { a, .. }
            | Op::GetTable { a, .. }
            | Op::NewTable { a, .. }
            | Op::Arith { a, .. }
            | Op::Unm { a, .. }
            | Op::Not { a, .. }
            | Op::Len { a, .. }
            | Op::Concat { a, .. }
            | Op::TestSet { a, .. }
            | Op::Closure { a, .. }
            | Op::Vararg { a, .. }
                if a == reg =>
            {
                last = Some(pc);
            }
            _ => {}
        }
        pc += 1;
    }
    last.map(|pc| p.code[pc])
}
