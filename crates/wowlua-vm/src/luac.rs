//! Verification only: decodes Lua 5.1 bytecode (`string.dump`, `luac`) and
//! compares it with the VM's compiled prototypes, instruction for
//! instruction, so the compiler can be checked against Lua's own on real
//! code (`cargo xtask corpus vm`).

use crate::heap::Heap;
use crate::proto::{ArithOp, Op, Proto, Rk};
use crate::value::Value;

/// A constant as dumped.
#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Nil,
    Bool(bool),
    Num(u64),
    Str(Vec<u8>),
}

/// One function of a dump.
#[derive(Debug, Default)]
pub struct DumpProto {
    pub code: Vec<u32>,
    pub constants: Vec<Const>,
    pub protos: Vec<DumpProto>,
    pub lines: Vec<u32>,
    pub nups: u8,
    pub num_params: u8,
    pub is_vararg: u8,
    pub max_stack: u8,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let v = *self.b.get(self.i)?;
        self.i += 1;
        Some(v)
    }

    fn int(&mut self) -> Option<u32> {
        let s = self.b.get(self.i..self.i + 4)?;
        self.i += 4;
        Some(u32::from_le_bytes(s.try_into().ok()?))
    }

    fn size(&mut self) -> Option<u64> {
        let s = self.b.get(self.i..self.i + 8)?;
        self.i += 8;
        Some(u64::from_le_bytes(s.try_into().ok()?))
    }

    fn string(&mut self) -> Option<Vec<u8>> {
        let n = self.size()? as usize;
        if n == 0 {
            return Some(Vec::new());
        }
        let s = self.b.get(self.i..self.i + n - 1)?.to_vec();
        self.i += n;
        Some(s)
    }

    fn proto(&mut self) -> Option<DumpProto> {
        let mut p = DumpProto::default();
        self.string()?; // source
        self.int()?; // linedefined
        self.int()?; // lastlinedefined
        p.nups = self.byte()?;
        p.num_params = self.byte()?;
        p.is_vararg = self.byte()?;
        p.max_stack = self.byte()?;
        let n = self.int()?;
        for _ in 0..n {
            p.code.push(self.int()?);
        }
        let n = self.int()?;
        for _ in 0..n {
            p.constants.push(match self.byte()? {
                0 => Const::Nil,
                1 => Const::Bool(self.byte()? != 0),
                3 => {
                    let s = self.b.get(self.i..self.i + 8)?;
                    self.i += 8;
                    Const::Num(u64::from_le_bytes(s.try_into().ok()?))
                }
                4 => Const::Str(self.string()?),
                _ => return None,
            });
        }
        let n = self.int()?;
        for _ in 0..n {
            p.protos.push(self.proto()?);
        }
        let n = self.int()?;
        for _ in 0..n {
            p.lines.push(self.int()?);
        }
        let n = self.int()?;
        for _ in 0..n {
            self.string()?;
            self.int()?;
            self.int()?;
        }
        let n = self.int()?;
        for _ in 0..n {
            self.string()?;
        }
        Some(p)
    }
}

/// Decodes a Lua 5.1 dump (little-endian, 4-byte ints, 8-byte sizes and
/// numbers, as on the host).
pub fn decode(dump: &[u8]) -> Option<DumpProto> {
    if dump.get(..4)? != b"\x1bLua" || dump.len() < 12 {
        return None;
    }
    Reader { b: dump, i: 12 }.proto()
}

/// An instruction as Lua encodes it: opcode and A, B, C (or Bx/sBx in B).
pub type Ins = (u8, i64, i64, i64);

fn decode_ins(i: u32) -> Ins {
    let op = (i & 0x3f) as u8;
    let a = i64::from((i >> 6) & 0xff);
    let c = i64::from((i >> 14) & 0x1ff);
    let b = i64::from((i >> 23) & 0x1ff);
    let bx = i64::from((i >> 14) & 0x3ffff);
    match op {
        1 | 5 | 7 | 36 => (op, a, bx, 0),
        22 | 31 | 32 => (op, a, bx - 131071, 0),
        _ => (op, a, b, c),
    }
}

fn fb2int(x: i64) -> i64 {
    let e = (x >> 3) & 31;
    if e == 0 { x } else { ((x & 7) + 8) << (e - 1) }
}

fn rk(r: Rk) -> i64 {
    match r {
        Rk::R(r) => i64::from(r),
        Rk::K(k) => 256 + i64::from(k),
    }
}

/// The VM's instruction in Lua's encoding.
pub fn encode_ours(op: &Op) -> Ins {
    let b = |x: bool| i64::from(x);
    let u = |x: u8| i64::from(x);
    match *op {
        Op::Move { a, b: bb } => (0, u(a), u(bb), 0),
        Op::LoadK { a, k } => (1, u(a), i64::from(k), 0),
        Op::LoadBool { a, value, skip } => (2, u(a), b(value), b(skip)),
        Op::LoadNil { a, b: bb } => (3, u(a), u(bb), 0),
        Op::GetUpval { a, u: uu } => (4, u(a), u(uu), 0),
        Op::GetGlobal { a, k } => (5, u(a), i64::from(k), 0),
        Op::GetTable { a, b: bb, c } => (6, u(a), u(bb), rk(c)),
        Op::SetGlobal { a, k } => (7, u(a), i64::from(k), 0),
        Op::SetUpval { a, u: uu } => (8, u(a), u(uu), 0),
        Op::SetTable { a, b: bb, c } => (9, u(a), rk(bb), rk(c)),
        Op::NewTable { a, narr, nhash } => (10, u(a), i64::from(narr), i64::from(nhash)),
        Op::SelfOp { a, b: bb, c } => (11, u(a), u(bb), rk(c)),
        Op::Arith { op, a, b: bb, c } => {
            let code = match op {
                ArithOp::Add => 12,
                ArithOp::Sub => 13,
                ArithOp::Mul => 14,
                ArithOp::Div => 15,
                ArithOp::Mod => 16,
                ArithOp::Pow => 17,
            };
            (code, u(a), rk(bb), rk(c))
        }
        Op::Unm { a, b: bb } => (18, u(a), u(bb), 0),
        Op::Not { a, b: bb } => (19, u(a), u(bb), 0),
        Op::Len { a, b: bb } => (20, u(a), u(bb), 0),
        Op::Concat { a, b: bb, c } => (21, u(a), u(bb), u(c)),
        Op::Jmp { sbx } => (22, 0, i64::from(sbx), 0),
        Op::Eq { a, b: bb, c } => (23, b(a), rk(bb), rk(c)),
        Op::Lt { a, b: bb, c } => (24, b(a), rk(bb), rk(c)),
        Op::Le { a, b: bb, c } => (25, b(a), rk(bb), rk(c)),
        Op::Test { a, c } => (26, u(a), 0, b(c)),
        Op::TestSet { a, b: bb, c } => (27, u(a), u(bb), b(c)),
        Op::Call { a, b: bb, c } => (28, u(a), u(bb), u(c)),
        Op::TailCall { a, b: bb } => (29, u(a), u(bb), 0),
        Op::Return { a, b: bb } => (30, u(a), u(bb), 0),
        Op::ForLoop { a, sbx } => (31, u(a), i64::from(sbx), 0),
        Op::ForPrep { a, sbx } => (32, u(a), i64::from(sbx), 0),
        Op::TForLoop { a, c } => (33, u(a), 0, u(c)),
        Op::SetList { a, b: bb, c } => (34, u(a), u(bb), i64::from(c)),
        Op::Close { a } => (35, u(a), 0, 0),
        Op::Closure { a, proto } => (36, u(a), i64::from(proto), 0),
        Op::Vararg { a, b: bb } => (37, u(a), u(bb), 0),
    }
}

/// Lua's code without the closure and long-SETLIST pseudo-instructions,
/// with jumps retargeted to the remaining instructions.
fn normalize(p: &DumpProto) -> Vec<Ins> {
    let mut real: Vec<(usize, Ins)> = Vec::new();
    let mut map = vec![0usize; p.code.len() + 1];
    let mut i = 0;
    while i < p.code.len() {
        map[i] = real.len();
        let ins = decode_ins(p.code[i]);
        real.push((i, ins));
        let skip = match ins.0 {
            36 => p.protos.get(ins.2 as usize).map_or(0, |c| c.nups as usize),
            34 if ins.3 == 0 => 1,
            _ => 0,
        };
        for k in 1..=skip {
            if i + k < map.len() {
                map[i + k] = real.len();
            }
        }
        i += 1 + skip;
    }
    map[p.code.len()] = real.len();
    real.into_iter()
        .enumerate()
        .map(|(ours, (pc, mut ins))| {
            match ins.0 {
                22 | 31 | 32 => {
                    let target = (pc as i64 + 1 + ins.2) as usize;
                    ins.2 = map[target.min(p.code.len())] as i64 - (ours as i64 + 1);
                }
                34 if ins.3 == 0 => ins.3 = i64::from(p.code[pc + 1]),
                10 => {
                    ins.2 = fb2int(ins.2);
                    ins.3 = fb2int(ins.3);
                }
                _ => {}
            }
            ins
        })
        .collect()
}

fn same_const(heap: &Heap, ours: Value, theirs: &Const) -> bool {
    match (ours, theirs) {
        (Value::Nil, Const::Nil) => true,
        (Value::Bool(a), Const::Bool(b)) => a == *b,
        (Value::Num(a), Const::Num(b)) => a.to_bits() == *b,
        (Value::Str(s), Const::Str(b)) => heap.str_bytes(s) == b.as_slice(),
        _ => false,
    }
}

/// The first difference between our prototype and Lua's, if any (`path`
/// names nested functions, e.g. `main/2/0`).
pub fn compare(heap: &Heap, ours: &Proto, theirs: &DumpProto, path: &str) -> Option<String> {
    let lua = normalize(theirs);
    let mine: Vec<Ins> = ours.code.iter().map(encode_ours).collect();
    if let Some(i) = (0..lua.len().max(mine.len())).find(|&i| lua.get(i) != mine.get(i)) {
        let line = ours.lines.get(i).copied().unwrap_or(0);
        return Some(format!(
            "{path}: instruction {i} (line {line}): ours {:?}, Lua {:?}",
            mine.get(i),
            lua.get(i)
        ));
    }
    if ours.max_stack != theirs.max_stack {
        return Some(format!(
            "{path}: max stack {} vs {}",
            ours.max_stack, theirs.max_stack
        ));
    }
    if ours.num_params != theirs.num_params || ours.is_vararg != (theirs.is_vararg & 2 != 0) {
        return Some(format!("{path}: parameters differ"));
    }
    if ours.upvals.len() != theirs.nups as usize {
        return Some(format!(
            "{path}: {} upvalues vs {}",
            ours.upvals.len(),
            theirs.nups
        ));
    }
    if ours.constants.len() != theirs.constants.len() {
        return Some(format!(
            "{path}: {} constants vs {}",
            ours.constants.len(),
            theirs.constants.len()
        ));
    }
    if let Some(k) = (0..ours.constants.len())
        .find(|&k| !same_const(heap, ours.constants[k], &theirs.constants[k]))
    {
        return Some(format!("{path}: constant {k} differs"));
    }
    if ours.protos.len() != theirs.protos.len() {
        return Some(format!(
            "{path}: {} functions vs {}",
            ours.protos.len(),
            theirs.protos.len()
        ));
    }
    for (i, (a, b)) in ours.protos.iter().zip(&theirs.protos).enumerate() {
        if let Some(d) = compare(heap, a, b, &format!("{path}/{i}")) {
            return Some(d);
        }
    }
    None
}

/// Instructions whose line differs from Lua's (debug information only).
pub fn line_differences(ours: &Proto, theirs: &DumpProto) -> usize {
    let lua_lines: Vec<u32> = {
        let mut out = Vec::new();
        let mut i = 0;
        while i < theirs.code.len() {
            out.push(theirs.lines.get(i).copied().unwrap_or(0));
            let ins = decode_ins(theirs.code[i]);
            let skip = match ins.0 {
                36 => theirs
                    .protos
                    .get(ins.2 as usize)
                    .map_or(0, |c| c.nups as usize),
                34 if ins.3 == 0 => 1,
                _ => 0,
            };
            i += 1 + skip;
        }
        out
    };
    let own = ours
        .lines
        .iter()
        .zip(&lua_lines)
        .filter(|(a, b)| a != b)
        .count();
    own + ours
        .protos
        .iter()
        .zip(&theirs.protos)
        .map(|(a, b)| line_differences(a, b))
        .sum::<usize>()
}
