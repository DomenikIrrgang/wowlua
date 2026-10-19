//! Compiles a Lua 5.1 syntax tree to the VM's bytecode. This is a close port
//! of Lua 5.1's `lparser.c`/`lcode.c` code generation (expression
//! descriptors, jump lists, register allocation), so that evaluation order,
//! multiple results, limits and error names match the client's.

use rustc_hash::FxHashMap;
use std::rc::Rc;

use wowlua_span::Span;
use wowlua_syntax::view::{
    Args, ArgsKind, BinOp, Block, Chunk, ElseIfs, Expr, ExprKind, Exprs, Func, FuncName, Stmt,
    StmtKind, TableField, TableFields, TypedName, TypedNames, UnOp,
};

use crate::heap::Heap;
use crate::proto::{ArithOp, LocVar, Op, Proto, Rk, UpvalDesc};
use crate::value::Value;

const NO_JUMP: i32 = -1;
const NO_REG: u8 = u8::MAX;
const MAXSTACK: usize = 250;
const MAXVARS: usize = 200;
const MAXUPVALUES: usize = 60;
const MAXCCALLS: usize = 200;
const LFIELDS_PER_FLUSH: usize = 50;

/// A compile error: the line and the message (without the chunk name).
#[derive(Debug, Clone)]
pub struct CompileError {
    pub line: u32,
    pub message: String,
}

/// Expression descriptor kinds (Lua's `expkind`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum EK {
    Void,
    Nil,
    True,
    False,
    K(u32),
    KNum(f64),
    Local(u8),
    Upval(u8),
    Global(u32),
    Indexed(u8, Rk),
    Jmp(usize),
    Reloc(usize),
    NonReloc(u8),
    Call(usize),
    Vararg(usize),
}

#[derive(Clone, Copy, Debug)]
struct Exp {
    k: EK,
    /// Patch list of "exit when true".
    t: i32,
    /// Patch list of "exit when false".
    f: i32,
}

impl Exp {
    fn new(k: EK) -> Exp {
        Exp {
            k,
            t: NO_JUMP,
            f: NO_JUMP,
        }
    }

    fn has_jumps(&self) -> bool {
        self.t != self.f
    }

    fn is_numeral(&self) -> bool {
        matches!(self.k, EK::KNum(_)) && self.t == NO_JUMP && self.f == NO_JUMP
    }

    fn has_multret(&self) -> bool {
        matches!(self.k, EK::Call(_) | EK::Vararg(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ConstKey {
    Nil,
    Bool(bool),
    Num(u64),
    Str(crate::heap::StrRef),
}

struct BlockCnt {
    breaklist: i32,
    nactvar: usize,
    upval: bool,
    breakable: bool,
}

struct FuncState {
    proto: Proto,
    blocks: Vec<BlockCnt>,
    /// The last jump target (-1: none yet, as in Lua's `open_func`).
    lasttarget: isize,
    jpc: i32,
    freereg: usize,
    nactvar: usize,
    /// Indices into `proto.locvars`: the active locals, then pending ones.
    actvar: Vec<usize>,
    constants: FxHashMap<ConstKey, u32>,
}

impl FuncState {
    fn pc(&self) -> usize {
        self.proto.code.len()
    }
}

enum VarKind {
    Local(u8),
    Upval(u8),
    Global,
}

/// Compiler settings.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Lua 5.1's `LUA_COMPAT_VARARG`: a vararg function declares a local
    /// `arg`. WoW's Lua has no `arg`; only the bytecode comparison with a
    /// stock Lua 5.1 build turns this on.
    pub compat_vararg: bool,
    /// The nested C calls when the parser runs: its syntax levels count on
    /// from there (`L->nCcalls`).
    pub c_depth: usize,
}

/// Where each line starts, by Lua's rule: `\n`, `\r`, `\r\n` and `\n\r`
/// each end one line.
pub fn line_starts(src: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        i += 1;
        if c == b'\n' || c == b'\r' {
            if i < src.len() && (src[i] == b'\n' || src[i] == b'\r') && src[i] != c {
                i += 1;
            }
            starts.push(i);
        }
    }
    starts
}

pub fn compile(
    heap: &mut Heap,
    chunk: Chunk<'_>,
    src: &[u8],
    source: Rc<[u8]>,
) -> Result<Rc<Proto>, CompileError> {
    compile_with(heap, chunk, src, source, Options::default())
}

pub fn compile_with(
    heap: &mut Heap,
    chunk: Chunk<'_>,
    src: &[u8],
    source: Rc<[u8]>,
    options: Options,
) -> Result<Rc<Proto>, CompileError> {
    let line_starts = line_starts(src);
    let mut c = Compiler {
        heap,
        src,
        line_starts,
        source,
        fs: Vec::new(),
        lastline: 1,
        depth: options.c_depth,
        options,
    };
    c.open_func(0);
    c.fs_mut().proto.is_vararg = true;
    c.chunk(chunk.block())?;
    let proto = c.close_func();
    Ok(Rc::new(proto))
}

struct Compiler<'a> {
    heap: &'a mut Heap,
    src: &'a [u8],
    line_starts: Vec<usize>,
    source: Rc<[u8]>,
    fs: Vec<FuncState>,
    lastline: u32,
    depth: usize,
    options: Options,
}

type R<T> = Result<T, CompileError>;

impl Compiler<'_> {
    // ---------------------------------------------------------------- utils

    fn fs(&self) -> &FuncState {
        self.fs.last().expect("a function")
    }

    fn fs_mut(&mut self) -> &mut FuncState {
        self.fs.last_mut().expect("a function")
    }

    fn line(&self, offset: u32) -> u32 {
        let off = offset as usize;
        match self.line_starts.binary_search(&off) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    /// The offset of the first byte that is not whitespace or a comment
    /// at or after `offset` (the next token).
    fn next_token(&self, offset: u32) -> u32 {
        let src = &self.src;
        let mut i = offset as usize;
        loop {
            while i < src.len() && src[i].is_ascii_whitespace() {
                i += 1;
            }
            if src[i..].starts_with(b"--") {
                // A long comment `--[[ … ]]` or `--[==[ … ]==]`, else a line.
                let rest = &src[i + 2..];
                let level = if rest.first() == Some(&b'[') {
                    let eq = rest[1..].iter().take_while(|&&b| b == b'=').count();
                    (rest.get(1 + eq) == Some(&b'[')).then_some(eq)
                } else {
                    None
                };
                match level {
                    Some(eq) => {
                        let close = format!("]{}]", "=".repeat(eq));
                        let start = i + 2 + 2 + eq;
                        match src[start..]
                            .windows(close.len())
                            .position(|w| w == close.as_bytes())
                        {
                            Some(p) => i = start + p + close.len(),
                            None => return src.len() as u32,
                        }
                    }
                    None => {
                        while i < src.len() && src[i] != b'\n' {
                            i += 1;
                        }
                    }
                }
                continue;
            }
            return i as u32;
        }
    }

    fn line_end(&self, span: Span) -> u32 {
        self.line(span.end.saturating_sub(1).max(span.start))
    }

    fn err<T>(&self, line: u32, message: impl Into<String>) -> R<T> {
        Err(CompileError {
            line,
            message: message.into(),
        })
    }

    fn not_lua<T>(&self, span: Span, what: &str) -> R<T> {
        self.err(
            self.line(span.start),
            format!("{what} is not Lua 5.1 (compile the file with wowlua first)"),
        )
    }

    fn enter_level(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAXCCALLS {
            return self.err(self.lastline, "chunk has too many syntax levels");
        }
        Ok(())
    }

    fn leave_level(&mut self) {
        self.depth -= 1;
    }

    /// A syntax error raised with the token at `at` as the lookahead: at
    /// that token's (last) line, `near` its text as Lua writes it.
    fn syntax_error<T>(&self, at: u32, message: &str) -> R<T> {
        use wowlua_syntax::token::TokenKind as T;
        let at = self.next_token(at);
        let rest = &self.src[at as usize..];
        let file = wowlua_span::SourceDb::new().add("", "");
        let lexed = wowlua_syntax::lexer::lex(rest, file);
        let tok = lexed.tokens.token(0);
        let near = match tok.kind {
            T::Eof => "'<eof>'".to_string(),
            T::String | T::LongString | T::Number | T::Name => {
                let text = &rest[tok.start as usize..tok.end as usize];
                format!("'{}'", String::from_utf8_lossy(text))
            }
            k => k.describe().to_string(),
        };
        let line = self.line(at + tok.end.max(tok.start + 1) - 1);
        self.err(line, format!("{message} near {near}"))
    }

    fn errorlimit(&self, limit: usize, what: &str) -> CompileError {
        let line = self.fs().proto.line_defined;
        let message = if line == 0 {
            format!("main function has more than {limit} {what}")
        } else {
            format!("function at line {line} has more than {limit} {what}")
        };
        CompileError {
            line: self.lastline,
            message,
        }
    }

    // ------------------------------------------------------------ emitting

    fn code(&mut self, op: Op) -> usize {
        self.dischargejpc();
        let line = self.lastline;
        let fs = self.fs_mut();
        fs.proto.code.push(op);
        fs.proto.lines.push(line);
        fs.pc() - 1
    }

    fn fixline(&mut self, line: u32) {
        let fs = self.fs_mut();
        // In Lua a closure with upvalues is followed by pseudo-instructions,
        // and those get the line: the closure keeps its own.
        if let Some(Op::Closure { proto, .. }) = fs.proto.code.last()
            && !fs.proto.protos[*proto as usize].upvals.is_empty()
        {
            return;
        }
        if let Some(l) = fs.proto.lines.last_mut() {
            *l = line;
        }
    }

    fn op(&self, pc: usize) -> Op {
        self.fs().proto.code[pc]
    }

    fn op_mut(&mut self, pc: usize) -> &mut Op {
        &mut self.fs_mut().proto.code[pc]
    }

    fn getjump(&self, pc: usize) -> i32 {
        let offset = match self.op(pc) {
            Op::Jmp { sbx } | Op::ForLoop { sbx, .. } | Op::ForPrep { sbx, .. } => sbx,
            other => unreachable!("not a jump: {other:?}"),
        };
        if offset == NO_JUMP {
            NO_JUMP
        } else {
            pc as i32 + 1 + offset
        }
    }

    fn fixjump(&mut self, pc: usize, dest: usize) {
        let offset = dest as i32 - (pc as i32 + 1);
        match self.op_mut(pc) {
            Op::Jmp { sbx } | Op::ForLoop { sbx, .. } | Op::ForPrep { sbx, .. } => *sbx = offset,
            other => unreachable!("not a jump: {other:?}"),
        }
    }

    fn getlabel(&mut self) -> usize {
        let pc = self.fs().pc();
        self.fs_mut().lasttarget = pc as isize;
        pc
    }

    fn jump(&mut self) -> i32 {
        let jpc = self.fs().jpc;
        self.fs_mut().jpc = NO_JUMP;
        let mut j = self.code(Op::Jmp { sbx: NO_JUMP }) as i32;
        self.concat(&mut j, jpc);
        j
    }

    fn ret(&mut self, first: usize, nret: i32) {
        self.code(Op::Return {
            a: first as u8,
            b: (nret + 1) as u8,
        });
    }

    fn condjump(&mut self, op: Op) -> i32 {
        self.code(op);
        self.jump()
    }

    fn jumpcontrol(&self, pc: usize) -> usize {
        if pc >= 1 && self.op(pc - 1).is_test() {
            pc - 1
        } else {
            pc
        }
    }

    fn need_value(&self, mut list: i32) -> bool {
        while list != NO_JUMP {
            let i = self.op(self.jumpcontrol(list as usize));
            if !matches!(i, Op::TestSet { .. }) {
                return true;
            }
            list = self.getjump(list as usize);
        }
        false
    }

    fn patchtestreg(&mut self, node: usize, reg: u8) -> bool {
        let at = self.jumpcontrol(node);
        let Op::TestSet { b, c, .. } = self.op(at) else {
            return false;
        };
        *self.op_mut(at) = if reg != NO_REG && reg != b {
            Op::TestSet { a: reg, b, c }
        } else {
            Op::Test { a: b, c }
        };
        true
    }

    fn removevalues(&mut self, mut list: i32) {
        while list != NO_JUMP {
            self.patchtestreg(list as usize, NO_REG);
            list = self.getjump(list as usize);
        }
    }

    fn patchlistaux(&mut self, mut list: i32, vtarget: usize, reg: u8, dtarget: usize) {
        while list != NO_JUMP {
            let next = self.getjump(list as usize);
            if self.patchtestreg(list as usize, reg) {
                self.fixjump(list as usize, vtarget);
            } else {
                self.fixjump(list as usize, dtarget);
            }
            list = next;
        }
    }

    fn dischargejpc(&mut self) {
        let jpc = self.fs().jpc;
        let pc = self.fs().pc();
        self.patchlistaux(jpc, pc, NO_REG, pc);
        self.fs_mut().jpc = NO_JUMP;
    }

    fn patchlist(&mut self, list: i32, target: usize) {
        if target == self.fs().pc() {
            self.patchtohere(list);
        } else {
            self.patchlistaux(list, target, NO_REG, target);
        }
    }

    fn patchtohere(&mut self, list: i32) {
        self.getlabel();
        let mut jpc = self.fs().jpc;
        self.concat(&mut jpc, list);
        self.fs_mut().jpc = jpc;
    }

    fn concat(&mut self, l1: &mut i32, l2: i32) {
        if l2 == NO_JUMP {
            return;
        }
        if *l1 == NO_JUMP {
            *l1 = l2;
            return;
        }
        let mut list = *l1 as usize;
        loop {
            let next = self.getjump(list);
            if next == NO_JUMP {
                break;
            }
            list = next as usize;
        }
        self.fixjump(list, l2 as usize);
    }

    // ----------------------------------------------------------- registers

    fn checkstack(&mut self, n: usize) -> R<()> {
        let newstack = self.fs().freereg + n;
        if newstack > self.fs().proto.max_stack as usize {
            if newstack >= MAXSTACK {
                return self.err(self.lastline, "function or expression too complex");
            }
            self.fs_mut().proto.max_stack = newstack as u8;
        }
        Ok(())
    }

    fn reserveregs(&mut self, n: usize) -> R<()> {
        self.checkstack(n)?;
        self.fs_mut().freereg += n;
        Ok(())
    }

    fn freereg(&mut self, reg: u8) {
        let fs = self.fs_mut();
        if reg != NO_REG && reg as usize >= fs.nactvar {
            fs.freereg -= 1;
            debug_assert_eq!(reg as usize, fs.freereg, "register freed out of order");
        }
    }

    fn freerk(&mut self, rk: Rk) {
        if let Rk::R(r) = rk {
            self.freereg(r);
        }
    }

    fn freeexp(&mut self, e: &Exp) {
        if let EK::NonReloc(r) = e.k {
            self.freereg(r);
        }
    }

    // ----------------------------------------------------------- constants

    fn addk(&mut self, key: ConstKey, value: Value) -> R<u32> {
        if let Some(&k) = self.fs().constants.get(&key) {
            return Ok(k);
        }
        let fs = self.fs_mut();
        let k = fs.proto.constants.len() as u32;
        fs.proto.constants.push(value);
        fs.constants.insert(key, k);
        Ok(k)
    }

    fn number_k(&mut self, n: f64) -> R<u32> {
        // `0` and `-0` are one constant, as in Lua 5.1.
        let bits = if n == 0.0 {
            0.0f64.to_bits()
        } else {
            n.to_bits()
        };
        self.addk(ConstKey::Num(bits), Value::Num(n))
    }

    fn string_k(&mut self, bytes: &[u8]) -> R<u32> {
        let s = self.heap.intern(bytes);
        self.addk(ConstKey::Str(s), Value::Str(s))
    }

    fn bool_k(&mut self, b: bool) -> R<u32> {
        self.addk(ConstKey::Bool(b), Value::Bool(b))
    }

    fn nil_k(&mut self) -> R<u32> {
        self.addk(ConstKey::Nil, Value::Nil)
    }

    // ------------------------------------------------------- expressions

    fn nil(&mut self, from: usize, n: usize) {
        let fs = self.fs();
        if fs.pc() as isize > fs.lasttarget {
            if fs.pc() == 0 {
                if from >= fs.nactvar {
                    return;
                }
            } else if let Op::LoadNil { a, b } = fs.proto.code[fs.pc() - 1] {
                let (pfrom, pto) = (a as usize, b as usize);
                if pfrom <= from && from <= pto + 1 {
                    if from + n - 1 > pto {
                        let pc = fs.pc() - 1;
                        *self.op_mut(pc) = Op::LoadNil {
                            a,
                            b: (from + n - 1) as u8,
                        };
                    }
                    return;
                }
            }
        }
        self.code(Op::LoadNil {
            a: from as u8,
            b: (from + n - 1) as u8,
        });
    }

    fn setreturns(&mut self, e: &mut Exp, nresults: i32) -> R<()> {
        match e.k {
            EK::Call(pc) => {
                if let Op::Call { c, .. } = self.op_mut(pc) {
                    *c = (nresults + 1) as u8;
                }
            }
            EK::Vararg(pc) => {
                let freereg = self.fs().freereg as u8;
                if let Op::Vararg { a, b } = self.op_mut(pc) {
                    *b = (nresults + 1) as u8;
                    *a = freereg;
                }
                self.reserveregs(1)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn setmultret(&mut self, e: &mut Exp) -> R<()> {
        self.setreturns(e, -1)
    }

    fn setoneret(&mut self, e: &mut Exp) {
        match e.k {
            EK::Call(pc) => {
                let Op::Call { a, .. } = self.op(pc) else {
                    unreachable!()
                };
                e.k = EK::NonReloc(a);
            }
            EK::Vararg(pc) => {
                if let Op::Vararg { b, .. } = self.op_mut(pc) {
                    *b = 2;
                }
                e.k = EK::Reloc(pc);
            }
            _ => {}
        }
    }

    fn dischargevars(&mut self, e: &mut Exp) {
        match e.k {
            EK::Local(r) => e.k = EK::NonReloc(r),
            EK::Upval(u) => e.k = EK::Reloc(self.code(Op::GetUpval { a: 0, u })),
            EK::Global(k) => e.k = EK::Reloc(self.code(Op::GetGlobal { a: 0, k })),
            EK::Indexed(t, key) => {
                self.freerk(key);
                self.freereg(t);
                e.k = EK::Reloc(self.code(Op::GetTable { a: 0, b: t, c: key }));
            }
            EK::Vararg(_) | EK::Call(_) => self.setoneret(e),
            _ => {}
        }
    }

    fn code_label(&mut self, a: u8, value: bool, skip: bool) -> usize {
        self.getlabel();
        self.code(Op::LoadBool { a, value, skip })
    }

    fn set_a(op: &mut Op, reg: u8) {
        match op {
            Op::GetUpval { a, .. }
            | Op::GetGlobal { a, .. }
            | Op::GetTable { a, .. }
            | Op::NewTable { a, .. }
            | Op::Arith { a, .. }
            | Op::Unm { a, .. }
            | Op::Not { a, .. }
            | Op::Len { a, .. }
            | Op::Concat { a, .. }
            | Op::Closure { a, .. }
            | Op::Vararg { a, .. }
            | Op::LoadK { a, .. }
            | Op::Move { a, .. } => *a = reg,
            other => unreachable!("not relocatable: {other:?}"),
        }
    }

    fn discharge2reg(&mut self, e: &mut Exp, reg: u8) -> R<()> {
        self.dischargevars(e);
        match e.k {
            EK::Nil => self.nil(reg as usize, 1),
            EK::False | EK::True => {
                self.code(Op::LoadBool {
                    a: reg,
                    value: e.k == EK::True,
                    skip: false,
                });
            }
            EK::K(k) => {
                self.code(Op::LoadK { a: reg, k });
            }
            EK::KNum(n) => {
                let k = self.number_k(n)?;
                self.code(Op::LoadK { a: reg, k });
            }
            EK::Reloc(pc) => Self::set_a(self.op_mut(pc), reg),
            EK::NonReloc(r) => {
                if r != reg {
                    self.code(Op::Move { a: reg, b: r });
                }
            }
            EK::Void | EK::Jmp(_) => return Ok(()),
            _ => unreachable!("undischarged expression"),
        }
        e.k = EK::NonReloc(reg);
        Ok(())
    }

    fn discharge2anyreg(&mut self, e: &mut Exp) -> R<()> {
        if !matches!(e.k, EK::NonReloc(_)) {
            self.reserveregs(1)?;
            let r = (self.fs().freereg - 1) as u8;
            self.discharge2reg(e, r)?;
        }
        Ok(())
    }

    fn exp2reg(&mut self, e: &mut Exp, reg: u8) -> R<()> {
        self.discharge2reg(e, reg)?;
        if let EK::Jmp(pc) = e.k {
            let mut t = e.t;
            self.concat(&mut t, pc as i32);
            e.t = t;
        }
        if e.has_jumps() {
            let mut p_f = NO_JUMP;
            let mut p_t = NO_JUMP;
            if self.need_value(e.t) || self.need_value(e.f) {
                let fj = if matches!(e.k, EK::Jmp(_)) {
                    NO_JUMP
                } else {
                    self.jump()
                };
                p_f = self.code_label(reg, false, true) as i32;
                p_t = self.code_label(reg, true, false) as i32;
                self.patchtohere(fj);
            }
            let final_ = self.getlabel();
            self.patchlistaux(e.f, final_, reg, p_f as usize);
            self.patchlistaux(e.t, final_, reg, p_t as usize);
        }
        e.f = NO_JUMP;
        e.t = NO_JUMP;
        e.k = EK::NonReloc(reg);
        Ok(())
    }

    fn exp2nextreg(&mut self, e: &mut Exp) -> R<()> {
        self.dischargevars(e);
        self.freeexp(e);
        self.reserveregs(1)?;
        let r = (self.fs().freereg - 1) as u8;
        self.exp2reg(e, r)
    }

    fn exp2anyreg(&mut self, e: &mut Exp) -> R<u8> {
        self.dischargevars(e);
        if let EK::NonReloc(r) = e.k {
            if !e.has_jumps() {
                return Ok(r);
            }
            if r as usize >= self.fs().nactvar {
                self.exp2reg(e, r)?;
                return Ok(r);
            }
        }
        self.exp2nextreg(e)?;
        let EK::NonReloc(r) = e.k else { unreachable!() };
        Ok(r)
    }

    fn exp2val(&mut self, e: &mut Exp) -> R<()> {
        if e.has_jumps() {
            self.exp2anyreg(e)?;
        } else {
            self.dischargevars(e);
        }
        Ok(())
    }

    fn exp2rk(&mut self, e: &mut Exp) -> R<Rk> {
        self.exp2val(e)?;
        // An operand holds constants 0..=255 (`MAXINDEXRK`); others go to a
        // register, as in Lua 5.1.
        const MAXINDEXRK: usize = 255;
        match e.k {
            EK::KNum(_) | EK::True | EK::False | EK::Nil => {
                if self.fs().proto.constants.len() <= MAXINDEXRK {
                    let k = match e.k {
                        EK::KNum(n) => self.number_k(n)?,
                        EK::Nil => self.nil_k()?,
                        other => self.bool_k(other == EK::True)?,
                    };
                    e.k = EK::K(k);
                    return Ok(Rk::K(k));
                }
            }
            EK::K(k) if k as usize <= MAXINDEXRK => return Ok(Rk::K(k)),
            _ => {}
        }
        Ok(Rk::R(self.exp2anyreg(e)?))
    }

    fn storevar(&mut self, var: &Exp, ex: &mut Exp) -> R<()> {
        match var.k {
            EK::Local(r) => {
                self.freeexp(ex);
                return self.exp2reg(ex, r);
            }
            EK::Upval(u) => {
                let e = self.exp2anyreg(ex)?;
                self.code(Op::SetUpval { a: e, u });
            }
            EK::Global(k) => {
                let e = self.exp2anyreg(ex)?;
                self.code(Op::SetGlobal { a: e, k });
            }
            EK::Indexed(t, key) => {
                let e = self.exp2rk(ex)?;
                self.code(Op::SetTable { a: t, b: key, c: e });
            }
            _ => unreachable!("not assignable"),
        }
        self.freeexp(ex);
        Ok(())
    }

    fn self_(&mut self, e: &mut Exp, key: &mut Exp) -> R<()> {
        self.exp2anyreg(e)?;
        self.freeexp(e);
        let func = self.fs().freereg as u8;
        self.reserveregs(2)?;
        let EK::NonReloc(b) = e.k else { unreachable!() };
        let c = self.exp2rk(key)?;
        self.code(Op::SelfOp { a: func, b, c });
        self.freeexp(key);
        e.k = EK::NonReloc(func);
        Ok(())
    }

    fn invertjump(&mut self, e: &Exp) {
        let EK::Jmp(pc) = e.k else { unreachable!() };
        let at = self.jumpcontrol(pc);
        match self.op_mut(at) {
            Op::Eq { a, .. } | Op::Lt { a, .. } | Op::Le { a, .. } => *a = !*a,
            other => unreachable!("not a comparison: {other:?}"),
        }
    }

    fn jumponcond(&mut self, e: &mut Exp, cond: bool) -> R<i32> {
        if let EK::Reloc(pc) = e.k
            && let Op::Not { b, .. } = self.op(pc)
        {
            // Remove the `not` and test its operand inverted.
            let fs = self.fs_mut();
            fs.proto.code.pop();
            fs.proto.lines.pop();
            return Ok(self.condjump(Op::Test { a: b, c: !cond }));
        }
        self.discharge2anyreg(e)?;
        self.freeexp(e);
        let EK::NonReloc(b) = e.k else { unreachable!() };
        Ok(self.condjump(Op::TestSet {
            a: NO_REG,
            b,
            c: cond,
        }))
    }

    fn goiftrue(&mut self, e: &mut Exp) -> R<()> {
        self.dischargevars(e);
        // Lua 5.1 has no "always false" case here (5.2 added one).
        let pc = match e.k {
            EK::K(_) | EK::KNum(_) | EK::True => NO_JUMP,
            EK::Jmp(pc) => {
                self.invertjump(e);
                pc as i32
            }
            _ => self.jumponcond(e, false)?,
        };
        let mut f = e.f;
        self.concat(&mut f, pc);
        e.f = f;
        self.patchtohere(e.t);
        e.t = NO_JUMP;
        Ok(())
    }

    fn goiffalse(&mut self, e: &mut Exp) -> R<()> {
        self.dischargevars(e);
        let pc = match e.k {
            EK::Nil | EK::False => NO_JUMP,
            EK::Jmp(pc) => pc as i32,
            _ => self.jumponcond(e, true)?,
        };
        let mut t = e.t;
        self.concat(&mut t, pc);
        e.t = t;
        self.patchtohere(e.f);
        e.f = NO_JUMP;
        Ok(())
    }

    fn codenot(&mut self, e: &mut Exp) -> R<()> {
        self.dischargevars(e);
        match e.k {
            EK::Nil | EK::False => e.k = EK::True,
            EK::K(_) | EK::KNum(_) | EK::True => e.k = EK::False,
            EK::Jmp(_) => self.invertjump(e),
            EK::Reloc(_) | EK::NonReloc(_) => {
                self.discharge2anyreg(e)?;
                self.freeexp(e);
                let EK::NonReloc(b) = e.k else { unreachable!() };
                e.k = EK::Reloc(self.code(Op::Not { a: 0, b }));
            }
            _ => unreachable!(),
        }
        std::mem::swap(&mut e.f, &mut e.t);
        self.removevalues(e.f);
        self.removevalues(e.t);
        Ok(())
    }

    fn indexed(&mut self, t: &mut Exp, key: &mut Exp) -> R<()> {
        let EK::NonReloc(reg) = t.k else {
            unreachable!("table not in a register")
        };
        let k = self.exp2rk(key)?;
        t.k = EK::Indexed(reg, k);
        Ok(())
    }

    fn constfolding(op: Option<ArithOp>, e1: &mut Exp, e2: &Exp) -> bool {
        if !e1.is_numeral() || !e2.is_numeral() {
            return false;
        }
        let (EK::KNum(v1), EK::KNum(v2)) = (e1.k, e2.k) else {
            return false;
        };
        let r = match op {
            Some(ArithOp::Add) => v1 + v2,
            Some(ArithOp::Sub) => v1 - v2,
            Some(ArithOp::Mul) => v1 * v2,
            Some(ArithOp::Div) => {
                if v2 == 0.0 {
                    return false;
                }
                v1 / v2
            }
            Some(ArithOp::Mod) => {
                if v2 == 0.0 {
                    return false;
                }
                v1 - (v1 / v2).floor() * v2
            }
            Some(ArithOp::Pow) => v1.powf(v2),
            None => -v1,
        };
        if r.is_nan() {
            return false;
        }
        e1.k = EK::KNum(r);
        true
    }

    /// Arithmetic: `op` None is unary minus; `len` is the length operator.
    fn codearith(&mut self, op: Option<ArithOp>, len: bool, e1: &mut Exp, e2: &mut Exp) -> R<()> {
        if !len && Self::constfolding(op, e1, e2) {
            return Ok(());
        }
        let unary = len || op.is_none();
        let o2 = if unary { None } else { Some(self.exp2rk(e2)?) };
        let o1 = self.exp2rk(e1)?;
        // Free the higher register first.
        let higher_first = match (o1, o2) {
            (Rk::K(_), _) => true,
            (Rk::R(_), None | Some(Rk::K(_))) => false,
            (Rk::R(a), Some(Rk::R(b))) => a > b,
        };
        if higher_first {
            self.freeexp(e1);
            self.freeexp(e2);
        } else {
            self.freeexp(e2);
            self.freeexp(e1);
        }
        let reg = |rk: Rk| match rk {
            Rk::R(r) => r,
            Rk::K(_) => unreachable!("unary operand in a register"),
        };
        let pc = if len {
            self.code(Op::Len { a: 0, b: reg(o1) })
        } else if let Some(op) = op {
            self.code(Op::Arith {
                op,
                a: 0,
                b: o1,
                c: o2.expect("binary"),
            })
        } else {
            self.code(Op::Unm { a: 0, b: reg(o1) })
        };
        e1.k = EK::Reloc(pc);
        Ok(())
    }

    fn codecomp(&mut self, kind: u8, cond: bool, e1: &mut Exp, e2: &mut Exp) -> R<()> {
        let mut o1 = self.exp2rk(e1)?;
        let mut o2 = self.exp2rk(e2)?;
        self.freeexp(e2);
        self.freeexp(e1);
        let mut cond = cond;
        if !cond && kind != 0 {
            std::mem::swap(&mut o1, &mut o2);
            cond = true;
        }
        let op = match kind {
            0 => Op::Eq {
                a: cond,
                b: o1,
                c: o2,
            },
            1 => Op::Lt {
                a: cond,
                b: o1,
                c: o2,
            },
            _ => Op::Le {
                a: cond,
                b: o1,
                c: o2,
            },
        };
        e1.k = EK::Jmp(self.condjump(op) as usize);
        Ok(())
    }

    fn infix(&mut self, op: BinOp, v: &mut Exp) -> R<()> {
        match op {
            BinOp::And => self.goiftrue(v)?,
            BinOp::Or => self.goiffalse(v)?,
            BinOp::Concat => self.exp2nextreg(v)?,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                if !v.is_numeral() {
                    self.exp2rk(v)?;
                }
            }
            _ => {
                self.exp2rk(v)?;
            }
        }
        Ok(())
    }

    fn posfix(&mut self, op: BinOp, e1: &mut Exp, e2: &mut Exp) -> R<()> {
        match op {
            BinOp::And => {
                self.dischargevars(e2);
                let mut f = e2.f;
                self.concat(&mut f, e1.f);
                e2.f = f;
                *e1 = *e2;
            }
            BinOp::Or => {
                self.dischargevars(e2);
                let mut t = e2.t;
                self.concat(&mut t, e1.t);
                e2.t = t;
                *e1 = *e2;
            }
            BinOp::Concat => {
                self.exp2val(e2)?;
                let EK::NonReloc(r1) = e1.k else {
                    unreachable!("concat operand in a register")
                };
                if let EK::Reloc(pc) = e2.k
                    && let Op::Concat { b, .. } = self.op(pc)
                {
                    debug_assert_eq!(r1 + 1, b);
                    self.freeexp(e1);
                    if let Op::Concat { b, .. } = self.op_mut(pc) {
                        *b = r1;
                    }
                    e1.k = EK::Reloc(pc);
                } else {
                    self.exp2nextreg(e2)?;
                    let EK::NonReloc(r2) = e2.k else {
                        unreachable!()
                    };
                    // codearith for CONCAT: both operands are registers.
                    self.freeexp(e2);
                    self.freeexp(e1);
                    let pc = self.code(Op::Concat { a: 0, b: r1, c: r2 });
                    e1.k = EK::Reloc(pc);
                }
            }
            BinOp::Add => self.codearith(Some(ArithOp::Add), false, e1, e2)?,
            BinOp::Sub => self.codearith(Some(ArithOp::Sub), false, e1, e2)?,
            BinOp::Mul => self.codearith(Some(ArithOp::Mul), false, e1, e2)?,
            BinOp::Div => self.codearith(Some(ArithOp::Div), false, e1, e2)?,
            BinOp::Mod => self.codearith(Some(ArithOp::Mod), false, e1, e2)?,
            BinOp::Pow => self.codearith(Some(ArithOp::Pow), false, e1, e2)?,
            BinOp::Eq => self.codecomp(0, true, e1, e2)?,
            BinOp::Ne => self.codecomp(0, false, e1, e2)?,
            BinOp::Lt => self.codecomp(1, true, e1, e2)?,
            BinOp::Le => self.codecomp(2, true, e1, e2)?,
            BinOp::Gt => self.codecomp(1, false, e1, e2)?,
            BinOp::Ge => self.codecomp(2, false, e1, e2)?,
            _ => unreachable!("checked before"),
        }
        Ok(())
    }

    fn setlist(&mut self, base: u8, nelems: usize, tostore: i32) -> R<()> {
        let c = ((nelems.max(1) - 1) / LFIELDS_PER_FLUSH + 1) as u32;
        let b = if tostore < 0 { 0 } else { tostore as u8 };
        self.code(Op::SetList { a: base, b, c });
        self.fs_mut().freereg = base as usize + 1;
        Ok(())
    }

    // ----------------------------------------------------------- variables

    fn open_func(&mut self, line_defined: u32) {
        let proto = Proto {
            source: self.source.clone(),
            line_defined,
            max_stack: 2,
            ..Proto::default()
        };
        self.fs.push(FuncState {
            proto,
            blocks: Vec::new(),
            lasttarget: -1,
            jpc: NO_JUMP,
            freereg: 0,
            nactvar: 0,
            actvar: Vec::new(),
            constants: FxHashMap::default(),
        });
    }

    fn close_func(&mut self) -> Proto {
        self.ret(0, 0);
        self.removevars(0);
        let fs = self.fs.pop().expect("a function");
        fs.proto
    }

    fn new_localvar(&mut self, name: &str, n: usize) -> R<()> {
        if self.fs().nactvar + n + 1 > MAXVARS {
            return Err(self.errorlimit(MAXVARS, "local variables"));
        }
        let fs = self.fs_mut();
        fs.proto.locvars.push(LocVar {
            name: Rc::from(name),
            start_pc: 0,
            end_pc: 0,
        });
        let idx = fs.proto.locvars.len() - 1;
        let at = fs.nactvar + n;
        if fs.actvar.len() <= at {
            fs.actvar.resize(at + 1, 0);
        }
        fs.actvar[at] = idx;
        Ok(())
    }

    fn adjustlocalvars(&mut self, nvars: usize) {
        let fs = self.fs_mut();
        let pc = fs.pc();
        for _ in 0..nvars {
            let idx = fs.actvar[fs.nactvar];
            fs.proto.locvars[idx].start_pc = pc;
            fs.nactvar += 1;
        }
    }

    fn removevars(&mut self, tolevel: usize) {
        let fs = self.fs_mut();
        let pc = fs.pc();
        while fs.nactvar > tolevel {
            fs.nactvar -= 1;
            let idx = fs.actvar[fs.nactvar];
            fs.proto.locvars[idx].end_pc = pc;
        }
        fs.actvar.truncate(fs.nactvar);
    }

    fn searchvar(&self, level: usize, name: &str) -> Option<u8> {
        let fs = &self.fs[level];
        (0..fs.nactvar)
            .rev()
            .find(|&i| &*fs.proto.locvars[fs.actvar[i]].name == name)
            .map(|i| i as u8)
    }

    fn markupval(&mut self, level: usize, reg: u8) {
        let fs = &mut self.fs[level];
        for bl in fs.blocks.iter_mut().rev() {
            if bl.nactvar <= reg as usize {
                bl.upval = true;
                return;
            }
        }
    }

    fn indexupvalue(&mut self, level: usize, name: &str, desc: UpvalDesc) -> R<u8> {
        if let Some(i) = self.fs[level].proto.upvals.iter().position(|&d| d == desc) {
            return Ok(i as u8);
        }
        if self.fs[level].proto.upvals.len() + 1 > MAXUPVALUES {
            let line = self.fs[level].proto.line_defined;
            let message = if line == 0 {
                format!("main function has more than {MAXUPVALUES} upvalues")
            } else {
                format!("function at line {line} has more than {MAXUPVALUES} upvalues")
            };
            return self.err(self.lastline, message);
        }
        let p = &mut self.fs[level].proto;
        p.upvals.push(desc);
        p.upval_names.push(Rc::from(name));
        Ok((p.upvals.len() - 1) as u8)
    }

    fn singlevaraux(&mut self, level: usize, name: &str, base: bool) -> R<VarKind> {
        if let Some(reg) = self.searchvar(level, name) {
            if !base {
                self.markupval(level, reg);
            }
            return Ok(VarKind::Local(reg));
        }
        if level == 0 {
            return Ok(VarKind::Global);
        }
        Ok(match self.singlevaraux(level - 1, name, false)? {
            VarKind::Global => VarKind::Global,
            VarKind::Local(r) => {
                VarKind::Upval(self.indexupvalue(level, name, UpvalDesc::Local(r))?)
            }
            VarKind::Upval(u) => {
                VarKind::Upval(self.indexupvalue(level, name, UpvalDesc::Upval(u))?)
            }
        })
    }

    fn singlevar(&mut self, name: &str) -> R<Exp> {
        let level = self.fs.len() - 1;
        Ok(Exp::new(match self.singlevaraux(level, name, true)? {
            VarKind::Local(r) => EK::Local(r),
            VarKind::Upval(u) => EK::Upval(u),
            VarKind::Global => EK::Global(self.string_k(name.as_bytes())?),
        }))
    }

    // ---------------------------------------------------------- statements

    fn enterblock(&mut self, breakable: bool) {
        let fs = self.fs_mut();
        let nactvar = fs.nactvar;
        fs.blocks.push(BlockCnt {
            breaklist: NO_JUMP,
            nactvar,
            upval: false,
            breakable,
        });
    }

    fn leaveblock(&mut self) {
        let bl = self.fs_mut().blocks.pop().expect("a block");
        self.removevars(bl.nactvar);
        if bl.upval {
            self.code(Op::Close {
                a: bl.nactvar as u8,
            });
        }
        let nactvar = self.fs().nactvar;
        self.fs_mut().freereg = nactvar;
        self.patchtohere(bl.breaklist);
    }

    fn chunk(&mut self, block: Block<'_>) -> R<()> {
        self.enter_level()?;
        for stmt in &block.stmts() {
            self.statement(stmt)?;
            // The statement's last token is the last one read.
            self.lastline = self.line_end(stmt.span());
            let fs = self.fs_mut();
            debug_assert!(fs.freereg >= fs.nactvar);
            fs.freereg = fs.nactvar;
        }
        self.leave_level();
        Ok(())
    }

    fn block(&mut self, block: Block<'_>) -> R<()> {
        self.enterblock(false);
        self.chunk(block)?;
        self.leaveblock();
        Ok(())
    }

    fn statement(&mut self, stmt: Stmt<'_>) -> R<()> {
        let line = self.line(stmt.span().start);
        self.lastline = line;
        if !stmt.decorators().is_empty() {
            return self.not_lua(stmt.span(), "a decorator");
        }
        match stmt.kind() {
            StmtKind::Assign { targets, values } => self.assignment(targets, values),
            StmtKind::Call(e) => {
                let v = self.expr(e)?;
                match v.k {
                    EK::Call(pc) => {
                        if let Op::Call { c, .. } = self.op_mut(pc) {
                            *c = 1;
                        }
                        Ok(())
                    }
                    _ => {
                        self.freeexp(&v);
                        self.err(line, "syntax error")
                    }
                }
            }
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => self.whilestat(cond, body),
            StmtKind::Repeat { body, cond } => self.repeatstat(body, cond),
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => self.ifstat(cond, then_block, elseifs, else_block),
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => {
                self.enterblock(true);
                self.fornum(var, start, limit, step, body, line)?;
                self.leaveblock();
                Ok(())
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                self.enterblock(true);
                self.forlist(vars, exprs, body, line)?;
                self.leaveblock();
                Ok(())
            }
            StmtKind::Function { name, func } => self.funcstat(name, func, line),
            StmtKind::LocalFunction { name, func } => {
                self.new_localvar(name.text(), 0)?;
                let reg = self.fs().freereg as u8;
                let v = Exp::new(EK::Local(reg));
                self.reserveregs(1)?;
                self.adjustlocalvars(1);
                let mut b = self.body(func, false, line)?;
                self.storevar(&v, &mut b)?;
                let fs = self.fs_mut();
                let idx = fs.actvar[fs.nactvar - 1];
                fs.proto.locvars[idx].start_pc = fs.pc();
                Ok(())
            }
            StmtKind::Local { names, values } => self.localstat(names, values),
            StmtKind::Return(values) => self.retstat(values),
            StmtKind::Break => self.breakstat(stmt.span().end),
            StmtKind::Compound { .. } => self.not_lua(stmt.span(), "a compound assignment"),
            StmtKind::Continue => self.not_lua(stmt.span(), "`continue`"),
            StmtKind::LocalDestructure { .. } => self.not_lua(stmt.span(), "destructuring"),
            StmtKind::Const { .. } => self.not_lua(stmt.span(), "`const`"),
            StmtKind::Class(_) => self.not_lua(stmt.span(), "a class"),
            StmtKind::Interface(_) => self.not_lua(stmt.span(), "an interface"),
            StmtKind::Enum(_) => self.not_lua(stmt.span(), "an enum"),
            StmtKind::TypeAlias(_) => self.not_lua(stmt.span(), "a type alias"),
            StmtKind::Declare(_) => self.not_lua(stmt.span(), "a declaration"),
            StmtKind::Import(_) => self.not_lua(stmt.span(), "an import"),
            StmtKind::Export(_) => self.not_lua(stmt.span(), "an export"),
            StmtKind::Error => self.err(line, "syntax error"),
        }
    }

    fn cond(&mut self, e: Expr<'_>) -> R<i32> {
        let mut v = self.expr(e)?;
        if v.k == EK::Nil {
            v.k = EK::False;
        }
        self.goiftrue(&mut v)?;
        Ok(v.f)
    }

    fn whilestat(&mut self, cond: Expr<'_>, body: Block<'_>) -> R<()> {
        let whileinit = self.getlabel();
        let condexit = self.cond(cond)?;
        self.enterblock(true);
        self.block(body)?;
        let j = self.jump();
        self.patchlist(j, whileinit);
        self.leaveblock();
        self.patchtohere(condexit);
        Ok(())
    }

    fn repeatstat(&mut self, body: Block<'_>, cond: Expr<'_>) -> R<()> {
        let repeat_init = self.getlabel();
        self.enterblock(true);
        self.enterblock(false);
        self.chunk(body)?;
        let condexit = self.cond(cond)?;
        let upval = self.fs().blocks.last().is_some_and(|b| b.upval);
        if !upval {
            self.leaveblock();
            self.patchlist(condexit, repeat_init);
        } else {
            // Inside the loop: never an error.
            self.breakstat(0)?;
            self.patchtohere(condexit);
            self.leaveblock();
            let j = self.jump();
            self.patchlist(j, repeat_init);
        }
        self.leaveblock();
        Ok(())
    }

    fn test_then_block(&mut self, cond: Expr<'_>, block: Block<'_>) -> R<i32> {
        let condexit = self.cond(cond)?;
        // `then` is read: an empty block's jumps take its line.
        self.lastline = self.line(self.next_token(cond.span().end));
        self.block(block)?;
        Ok(condexit)
    }

    fn ifstat(
        &mut self,
        cond: Expr<'_>,
        then_block: Block<'_>,
        elseifs: ElseIfs<'_>,
        else_block: Option<Block<'_>>,
    ) -> R<()> {
        let mut escapelist = NO_JUMP;
        let mut flist = self.test_then_block(cond, then_block)?;
        for (c, b) in elseifs {
            let j = self.jump();
            self.concat(&mut escapelist, j);
            self.patchtohere(flist);
            self.lastline = self.line(c.span().start);
            flist = self.test_then_block(c, b)?;
        }
        if let Some(b) = else_block {
            let j = self.jump();
            self.concat(&mut escapelist, j);
            self.patchtohere(flist);
            self.block(b)?;
        } else {
            self.concat(&mut escapelist, flist);
        }
        self.patchtohere(escapelist);
        Ok(())
    }

    fn exp1(&mut self, e: Expr<'_>) -> R<()> {
        let mut v = self.expr(e)?;
        self.exp2nextreg(&mut v)
    }

    fn forbody(
        &mut self,
        base: usize,
        line: u32,
        nvars: usize,
        isnum: bool,
        body: Block<'_>,
        do_at: u32,
    ) -> R<()> {
        self.adjustlocalvars(3);
        // The loop starts after `do` is read.
        self.lastline = self.line(do_at);
        let prep = if isnum {
            self.code(Op::ForPrep {
                a: base as u8,
                sbx: NO_JUMP,
            }) as i32
        } else {
            self.jump()
        };
        self.enterblock(false);
        self.adjustlocalvars(nvars);
        self.reserveregs(nvars)?;
        self.block(body)?;
        self.leaveblock();
        self.patchtohere(prep);
        let endfor = if isnum {
            self.code(Op::ForLoop {
                a: base as u8,
                sbx: NO_JUMP,
            }) as i32
        } else {
            self.code(Op::TForLoop {
                a: base as u8,
                c: nvars as u8,
            }) as i32
        };
        self.fixline(line);
        let list = if isnum { endfor } else { self.jump() };
        self.patchlist(list, prep as usize + 1);
        Ok(())
    }

    fn fornum(
        &mut self,
        var: TypedName<'_>,
        start: Expr<'_>,
        limit: Expr<'_>,
        step: Option<Expr<'_>>,
        body: Block<'_>,
        line: u32,
    ) -> R<()> {
        let base = self.fs().freereg;
        self.new_localvar("(for index)", 0)?;
        self.new_localvar("(for limit)", 1)?;
        self.new_localvar("(for step)", 2)?;
        self.new_localvar(var.name.text(), 3)?;
        self.exp1(start)?;
        self.exp1(limit)?;
        if let Some(s) = step {
            self.exp1(s)?;
        } else {
            let k = self.number_k(1.0)?;
            let reg = self.fs().freereg as u8;
            self.code(Op::LoadK { a: reg, k });
            self.reserveregs(1)?;
        }
        let last = step.unwrap_or(limit);
        let do_at = self.next_token(last.span().end);
        self.forbody(base, line, 1, true, body, do_at)
    }

    fn forlist(
        &mut self,
        vars: TypedNames<'_>,
        exprs: Exprs<'_>,
        body: Block<'_>,
        line: u32,
    ) -> R<()> {
        let base = self.fs().freereg;
        self.new_localvar("(for generator)", 0)?;
        self.new_localvar("(for state)", 1)?;
        self.new_localvar("(for control)", 2)?;
        for (i, v) in vars.iter().enumerate() {
            self.new_localvar(v.name.text(), 3 + i)?;
        }
        let (nexps, mut e) = self.explist(exprs)?;
        self.adjust_assign(3, nexps, &mut e)?;
        self.checkstack(3)?;
        let do_at = self.next_token(exprs.at(exprs.len() - 1).span().end);
        // Lua takes this loop's line after reading `in`: the next token's.
        let _ = line;
        let line = self.line(exprs.at(0).span().start);
        self.forbody(base, line, vars.len(), false, body, do_at)
    }

    fn breakstat(&mut self, end: u32) -> R<()> {
        let fs = self.fs();
        let mut upval = false;
        let mut found = None;
        for (i, bl) in fs.blocks.iter().enumerate().rev() {
            if bl.breakable {
                found = Some(i);
                break;
            }
            upval |= bl.upval;
        }
        let Some(i) = found else {
            return self.syntax_error(end, "no loop to break");
        };
        if upval {
            let a = self.fs().blocks[i].nactvar as u8;
            self.code(Op::Close { a });
        }
        let j = self.jump();
        let mut list = self.fs().blocks[i].breaklist;
        self.concat(&mut list, j);
        self.fs_mut().blocks[i].breaklist = list;
        Ok(())
    }

    fn funcstat(&mut self, name: FuncName<'_>, func: Func<'_>, line: u32) -> R<()> {
        let mut v = self.singlevar(name.path().at(0).text())?;
        for field in name.path().iter().skip(1).chain(name.method()) {
            self.exp2anyreg(&mut v)?;
            let k = self.string_k(field.text().as_bytes())?;
            let mut key = Exp::new(EK::K(k));
            self.indexed(&mut v, &mut key)?;
        }
        let mut b = self.body(func, name.method().is_some(), line)?;
        self.storevar(&v, &mut b)?;
        self.fixline(line);
        Ok(())
    }

    fn localstat(&mut self, names: TypedNames<'_>, values: Exprs<'_>) -> R<()> {
        for (i, n) in names.iter().enumerate() {
            self.new_localvar(n.name.text(), i)?;
        }
        // The names, then any `=`, are read before the values.
        let mut end = names.at(names.len() - 1).name.span().end;
        self.lastline = self.line(end - 1);
        let (nexps, mut e) = if values.is_empty() {
            (0, Exp::new(EK::Void))
        } else {
            end = self.next_token(end);
            self.lastline = self.line(end);
            self.explist(values)?
        };
        self.adjust_assign(names.len(), nexps, &mut e)?;
        self.adjustlocalvars(names.len());
        Ok(())
    }

    fn adjust_assign(&mut self, nvars: usize, nexps: usize, e: &mut Exp) -> R<()> {
        let mut extra = nvars as i32 - nexps as i32;
        if e.has_multret() {
            extra += 1;
            if extra < 0 {
                extra = 0;
            }
            self.setreturns(e, extra)?;
            if extra > 1 {
                self.reserveregs(extra as usize - 1)?;
            }
        } else {
            if e.k != EK::Void {
                self.exp2nextreg(e)?;
            }
            if extra > 0 {
                let reg = self.fs().freereg;
                self.reserveregs(extra as usize)?;
                self.nil(reg, extra as usize);
            }
        }
        Ok(())
    }

    /// Compiles `exprs`, all but the last into consecutive registers; returns
    /// the count and the last, still open, expression.
    fn explist(&mut self, exprs: Exprs<'_>) -> R<(usize, Exp)> {
        let mut v = self.expr(exprs.at(0))?;
        for (prev, e) in exprs.iter().zip(exprs.iter().skip(1)) {
            // The previous value is stored after the `,` is read.
            self.lastline = self.line(self.next_token(prev.span().end));
            self.exp2nextreg(&mut v)?;
            v = self.expr(e)?;
        }
        Ok((exprs.len(), v))
    }

    fn check_conflict(&mut self, lhs: &mut [Exp], v: &Exp) -> R<()> {
        let EK::Local(reg) = v.k else { return Ok(()) };
        let extra = self.fs().freereg as u8;
        let mut conflict = false;
        for lh in lhs.iter_mut() {
            if let EK::Indexed(t, key) = &mut lh.k {
                if *t == reg {
                    conflict = true;
                    *t = extra;
                }
                if *key == Rk::R(reg) {
                    conflict = true;
                    *key = Rk::R(extra);
                }
            }
        }
        if conflict {
            self.code(Op::Move { a: extra, b: reg });
            self.reserveregs(1)?;
        }
        Ok(())
    }

    fn assignment(&mut self, targets: Exprs<'_>, values: Exprs<'_>) -> R<()> {
        let mut lhs: Vec<Exp> = Vec::with_capacity(targets.len());
        for t in targets {
            let v = self.primary_target(t)?;
            if !lhs.is_empty() {
                self.check_conflict(&mut lhs, &v)?;
            }
            lhs.push(v);
        }
        if lhs.len() > MAXCCALLS {
            return Err(self.errorlimit(MAXCCALLS, "variables in assignment"));
        }
        let nvars = lhs.len();
        // The values follow the `=`.
        self.lastline = self.line(self.next_token(targets.at(nvars - 1).span().end));
        let (nexps, mut e) = self.explist(values)?;
        let mut rest = nvars;
        if nexps != nvars {
            self.adjust_assign(nvars, nexps, &mut e)?;
            if nexps > nvars {
                self.fs_mut().freereg -= nexps - nvars;
            }
        } else {
            self.setoneret(&mut e);
            let last = lhs[nvars - 1];
            self.storevar(&last, &mut e)?;
            rest = nvars - 1;
        }
        for i in (0..rest).rev() {
            let reg = (self.fs().freereg - 1) as u8;
            let mut e = Exp::new(EK::NonReloc(reg));
            let target = lhs[i];
            self.storevar(&target, &mut e)?;
        }
        Ok(())
    }

    /// An assignment target: a variable or an indexed expression.
    fn primary_target(&mut self, e: Expr<'_>) -> R<Exp> {
        match e.kind() {
            ExprKind::Name(n) => self.singlevar(n.text()),
            ExprKind::Index { obj, key } => {
                let mut v = self.expr(obj)?;
                self.exp2anyreg(&mut v)?;
                // The key follows the `[`.
                self.lastline = self.line(self.next_token(obj.span().end));
                let mut k = self.expr(key)?;
                self.exp2val(&mut k)?;
                self.indexed(&mut v, &mut k)?;
                Ok(v)
            }
            ExprKind::Field { obj, name } => {
                let mut v = self.expr(obj)?;
                self.exp2anyreg(&mut v)?;
                let kk = self.string_k(name.text().as_bytes())?;
                let mut k = Exp::new(EK::K(kk));
                self.indexed(&mut v, &mut k)?;
                Ok(v)
            }
            _ => self.err(self.line(e.span().start), "syntax error"),
        }
    }

    fn retstat(&mut self, values: Exprs<'_>) -> R<()> {
        let (first, nret) = if values.is_empty() {
            (0, 0)
        } else {
            let (n, mut e) = self.explist(values)?;
            if e.has_multret() {
                self.setmultret(&mut e)?;
                if let EK::Call(pc) = e.k
                    && n == 1
                    && let Op::Call { a, b, .. } = self.op(pc)
                {
                    *self.op_mut(pc) = Op::TailCall { a, b };
                }
                (self.fs().nactvar, -1)
            } else if n == 1 {
                (self.exp2anyreg(&mut e)? as usize, 1)
            } else {
                self.exp2nextreg(&mut e)?;
                (self.fs().nactvar, n as i32)
            }
        };
        self.ret(first, nret);
        Ok(())
    }

    // --------------------------------------------------------- expressions

    fn body(&mut self, func: Func<'_>, needself: bool, line: u32) -> R<Exp> {
        if !func.type_params().is_empty() {
            // Type parameters are erased; harmless, but not Lua 5.1 syntax.
            return self.not_lua(func.span(), "a generic function");
        }
        self.open_func(line);
        let mut n = 0;
        if needself {
            self.new_localvar("self", n)?;
            n += 1;
        }
        for p in &func.params() {
            if p.default().is_some() {
                return self.not_lua(func.span(), "a default parameter");
            }
            self.new_localvar(p.name().text(), n)?;
            n += 1;
        }
        self.fs_mut().proto.is_vararg = func.vararg().is_some();
        let mut hasarg = 0;
        if func.vararg().is_some() && self.options.compat_vararg {
            self.new_localvar("arg", n)?;
            n += 1;
            hasarg = 1;
        }
        self.adjustlocalvars(n);
        let nactvar = self.fs().nactvar;
        self.fs_mut().proto.num_params = (nactvar - hasarg) as u8;
        self.reserveregs(nactvar)?;
        self.chunk(func.body())?;
        self.lastline = self.line(func.end_span().start);
        self.fs_mut().proto.last_line_defined = self.lastline;
        let proto = self.close_func();
        let fs = self.fs_mut();
        let idx = fs.proto.protos.len() as u32;
        fs.proto.protos.push(Rc::new(proto));
        Ok(Exp::new(EK::Reloc(
            self.code(Op::Closure { a: 0, proto: idx }),
        )))
    }

    fn funcargs(&mut self, f: &mut Exp, args: Args<'_>) -> R<()> {
        let line = match args.kind() {
            ArgsKind::Parens { span, .. } => self.line(span.start),
            ArgsKind::Table(e) => self.line(e.span().start),
            // The string is the lookahead: the scanner is at its end.
            ArgsKind::String(e) => self.line_end(e.span()),
        };
        let mut a = match args.kind() {
            ArgsKind::Parens { args, span } => {
                if args.is_empty() {
                    Exp::new(EK::Void)
                } else {
                    let (_, mut e) = self.explist(args)?;
                    self.setmultret(&mut e)?;
                    // The last argument is closed after the `)`.
                    self.lastline = self.line_end(span);
                    e
                }
            }
            ArgsKind::Table(e) | ArgsKind::String(e) => self.expr(e)?,
        };
        let EK::NonReloc(base) = f.k else {
            unreachable!("function in a register")
        };
        let nparams: i32 = if a.has_multret() {
            -1
        } else {
            if a.k != EK::Void {
                self.exp2nextreg(&mut a)?;
            }
            self.fs().freereg as i32 - (base as i32 + 1)
        };
        f.k = EK::Call(self.code(Op::Call {
            a: base,
            b: (nparams + 1) as u8,
            c: 2,
        }));
        self.fixline(line);
        self.fs_mut().freereg = base as usize + 1;
        Ok(())
    }

    fn constructor(&mut self, fields: TableFields<'_>, span: Span) -> R<Exp> {
        // NEWTABLE is emitted before the `{` is read.
        let pc = self.code(Op::NewTable {
            a: 0,
            narr: 0,
            nhash: 0,
        });
        let mut t = Exp::new(EK::Reloc(pc));
        self.exp2nextreg(&mut t)?;
        // Then the `{` is read.
        self.lastline = self.line(span.start);
        let EK::NonReloc(treg) = t.k else {
            unreachable!()
        };
        let mut na = 0usize;
        let mut nh = 0usize;
        let mut tostore = 0usize;
        let mut v = Exp::new(EK::Void);
        let mut prev_end: Option<u32> = None;
        for field in fields {
            // Each field starts after the separator is read.
            if let Some(end) = prev_end {
                self.lastline = self.line(self.next_token(end));
            }
            prev_end = Some(field.span().end);
            // closelistfield
            if v.k != EK::Void {
                self.exp2nextreg(&mut v)?;
                v.k = EK::Void;
                if tostore == LFIELDS_PER_FLUSH {
                    self.setlist(treg, na, tostore as i32)?;
                    tostore = 0;
                }
            }
            match field {
                TableField::Positional(e) => {
                    v = self.expr(e)?;
                    na += 1;
                    tostore += 1;
                }
                TableField::Named { name, value, .. } => {
                    let reg = self.fs().freereg;
                    let k = self.string_k(name.text().as_bytes())?;
                    let mut key = Exp::new(EK::K(k));
                    nh += 1;
                    // The key is prepared after the `=`.
                    self.lastline = self.line(self.next_token(name.span().end));
                    let rkkey = self.exp2rk(&mut key)?;
                    let mut val = self.expr(value)?;
                    let rkval = self.exp2rk(&mut val)?;
                    self.code(Op::SetTable {
                        a: treg,
                        b: rkkey,
                        c: rkval,
                    });
                    self.fs_mut().freereg = reg;
                }
                TableField::Keyed { key, value, span } => {
                    let reg = self.fs().freereg;
                    // After the `[`.
                    self.lastline = self.line(span.start);
                    let mut k = self.expr(key)?;
                    self.exp2val(&mut k)?;
                    nh += 1;
                    // After the `]` and the `=`.
                    let close = self.next_token(key.span().end);
                    self.lastline = self.line(self.next_token(close + 1));
                    let rkkey = self.exp2rk(&mut k)?;
                    let mut val = self.expr(value)?;
                    let rkval = self.exp2rk(&mut val)?;
                    self.code(Op::SetTable {
                        a: treg,
                        b: rkkey,
                        c: rkval,
                    });
                    self.fs_mut().freereg = reg;
                }
            }
        }
        // lastlistfield, after the closing brace.
        self.lastline = self.line_end(span);
        if tostore > 0 {
            if v.has_multret() {
                self.setmultret(&mut v)?;
                self.setlist(treg, na, -1)?;
                na -= 1;
            } else {
                if v.k != EK::Void {
                    self.exp2nextreg(&mut v)?;
                }
                self.setlist(treg, na, tostore as i32)?;
            }
        }
        if let Op::NewTable { narr, nhash, .. } = self.op_mut(pc) {
            // Sizes are stored as Lua's "floating point bytes", which round
            // them up.
            *narr = fb2int(int2fb(na as u32));
            *nhash = fb2int(int2fb(nh as u32));
        }
        Ok(t)
    }

    fn expr(&mut self, e: Expr<'_>) -> R<Exp> {
        self.enter_level()?;
        let r = self.expr_inner(e);
        self.leave_level();
        let v = r?;
        self.lastline = self.line_end(e.span());
        Ok(v)
    }

    fn expr_inner(&mut self, e: Expr<'_>) -> R<Exp> {
        Ok(match e.kind() {
            ExprKind::Nil => Exp::new(EK::Nil),
            ExprKind::True => Exp::new(EK::True),
            ExprKind::False => Exp::new(EK::False),
            ExprKind::Number(n) => match n.value() {
                Some(v) => Exp::new(EK::KNum(v)),
                None => return self.err(self.line(e.span().start), "malformed number"),
            },
            ExprKind::String(s) => Exp::new(EK::K(self.string_k(&s.value())?)),
            ExprKind::Vararg => {
                if !self.fs().proto.is_vararg {
                    return self.err(
                        self.line(e.span().start),
                        "cannot use '...' outside a vararg function near '...'",
                    );
                }
                // Emitted before the `...` is read.
                Exp::new(EK::Vararg(self.code(Op::Vararg { a: 0, b: 1 })))
            }
            ExprKind::Function(f) => self.body(f, false, self.line(e.span().start))?,
            ExprKind::Table(fields) => self.constructor(fields, e.span())?,
            ExprKind::Name(n) => self.singlevar(n.text())?,
            ExprKind::Index { obj, key } => {
                let mut v = self.expr(obj)?;
                self.exp2anyreg(&mut v)?;
                // The key follows the `[`.
                self.lastline = self.line(self.next_token(obj.span().end));
                let mut k = self.expr(key)?;
                self.exp2val(&mut k)?;
                self.indexed(&mut v, &mut k)?;
                v
            }
            ExprKind::Field { obj, name } => {
                let mut v = self.expr(obj)?;
                self.exp2anyreg(&mut v)?;
                let kk = self.string_k(name.text().as_bytes())?;
                let mut k = Exp::new(EK::K(kk));
                self.indexed(&mut v, &mut k)?;
                v
            }
            ExprKind::Call { callee, args } => {
                let mut f = self.expr(callee)?;
                self.exp2nextreg(&mut f)?;
                self.funcargs(&mut f, args)?;
                f
            }
            ExprKind::Method { obj, name, args } => {
                let mut v = self.expr(obj)?;
                self.lastline = self.line(name.span().start);
                let kk = self.string_k(name.text().as_bytes())?;
                let mut k = Exp::new(EK::K(kk));
                self.self_(&mut v, &mut k)?;
                self.funcargs(&mut v, args)?;
                v
            }
            ExprKind::Paren(inner) => {
                let mut v = self.expr(inner)?;
                // Discharged after the `)`.
                self.lastline = self.line_end(e.span());
                self.dischargevars(&mut v);
                v
            }
            ExprKind::Binary { op, lhs, rhs } => {
                if !op.is_lua() {
                    return self.not_lua(e.span(), &format!("the `{}` operator", op.as_str()));
                }
                let mut v1 = self.expr(lhs)?;
                // `infix` runs after the operator is read.
                self.lastline = self.line(self.next_token(lhs.span().end));
                self.infix(op, &mut v1)?;
                let mut v2 = self.expr(rhs)?;
                self.posfix(op, &mut v1, &mut v2)?;
                v1
            }
            ExprKind::Unary { op, operand } => {
                let mut v = self.expr(operand)?;
                match op {
                    UnOp::Neg => {
                        if !v.is_numeral() {
                            self.exp2anyreg(&mut v)?;
                        }
                        let mut zero = Exp::new(EK::KNum(0.0));
                        self.codearith(None, false, &mut v, &mut zero)?;
                    }
                    UnOp::Not => self.codenot(&mut v)?,
                    UnOp::Len => {
                        self.exp2anyreg(&mut v)?;
                        let mut zero = Exp::new(EK::KNum(0.0));
                        self.codearith(None, true, &mut v, &mut zero)?;
                    }
                    UnOp::BNot => return self.not_lua(e.span(), "the `~` operator"),
                }
                v
            }
            ExprKind::Interp(_) => return self.not_lua(e.span(), "string interpolation"),
            ExprKind::OptField { .. }
            | ExprKind::OptIndex { .. }
            | ExprKind::OptMethod { .. }
            | ExprKind::OptCall { .. } => return self.not_lua(e.span(), "optional chaining"),
            ExprKind::Cast { .. } => return self.not_lua(e.span(), "a type assertion"),
            ExprKind::Error => return self.err(self.line(e.span().start), "syntax error"),
        })
    }
}

/// `luaO_int2fb`: `x` as a "floating point byte" (`eeeeexxx`).
fn int2fb(mut x: u32) -> u32 {
    let mut e = 0;
    while x >= 16 {
        x = (x + 1) >> 1;
        e += 1;
    }
    if x < 8 { x } else { ((e + 1) << 3) | (x - 8) }
}

/// `luaO_fb2int`.
fn fb2int(x: u32) -> u32 {
    let e = (x >> 3) & 31;
    if e == 0 { x } else { ((x & 7) + 8) << (e - 1) }
}
