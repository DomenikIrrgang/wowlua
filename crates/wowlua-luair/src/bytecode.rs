//! Lua 5.1 bytecode (`string.dump` output), for the superset round-trip
//! test (spec compiler §13.5.3): two chunks compile to "the same bytecode"
//! when their dumps are equal after stripping debug information.

/// What to remove from a dump.
#[derive(Clone, Copy, Debug)]
pub struct Strip {
    /// `linedefined` / `lastlinedefined` of every function.
    pub line_defined: bool,
    /// Line info, local names and upvalue names.
    pub debug: bool,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    int_size: usize,
    size_t: usize,
    little: bool,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(slice)
    }

    fn uint(&mut self, size: usize) -> Option<u64> {
        let b = self.bytes(size)?;
        let mut v = 0u64;
        for i in 0..size {
            let byte = if self.little { b[size - 1 - i] } else { b[i] };
            v = (v << 8) | u64::from(byte);
        }
        Some(v)
    }

    fn int(&mut self) -> Option<usize> {
        usize::try_from(self.uint(self.int_size)?).ok()
    }

    fn string(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.uint(self.size_t)?).ok()?;
        self.bytes(len)
    }
}

struct Writer {
    out: Vec<u8>,
    int_size: usize,
    size_t: usize,
    little: bool,
}

impl Writer {
    fn uint(&mut self, v: u64, size: usize) {
        let mut bytes = vec![0u8; size];
        for (i, b) in bytes.iter_mut().enumerate() {
            let shift = if self.little { i } else { size - 1 - i } * 8;
            *b = (v >> shift) as u8;
        }
        self.out.extend(bytes);
    }

    fn int(&mut self, v: usize) {
        self.uint(v as u64, self.int_size);
    }

    fn string(&mut self, s: &[u8]) {
        self.uint(s.len() as u64, self.size_t);
        self.out.extend_from_slice(s);
    }
}

/// Re-encodes a Lua 5.1 dump without the parts selected by `strip`.
/// Returns `None` if `dump` is not a Lua 5.1 dump.
pub fn strip(dump: &[u8], strip: Strip) -> Option<Vec<u8>> {
    let header = dump.get(..12)?;
    if &header[..5] != b"\x1bLua\x51" {
        return None;
    }
    let mut r = Reader {
        data: dump,
        pos: 12,
        little: header[6] == 1,
        int_size: usize::from(header[7]),
        size_t: usize::from(header[8]),
    };
    let number_size = usize::from(header[10]);
    let mut w = Writer {
        out: header.to_vec(),
        int_size: r.int_size,
        size_t: r.size_t,
        little: r.little,
    };
    function(&mut r, &mut w, number_size, strip, 0)?;
    (r.pos == dump.len()).then_some(w.out)
}

fn function(
    r: &mut Reader,
    w: &mut Writer,
    number_size: usize,
    strip: Strip,
    depth: u32,
) -> Option<()> {
    if depth > 250 {
        return None;
    }
    let source = r.string()?;
    w.string(if strip.debug { b"" } else { source });
    let line_defined = r.int()?;
    let last_line = r.int()?;
    w.int(if strip.line_defined { 0 } else { line_defined });
    w.int(if strip.line_defined { 0 } else { last_line });
    w.out.extend_from_slice(r.bytes(4)?); // nups, numparams, is_vararg, maxstacksize
    let code = r.int()?;
    w.int(code);
    w.out.extend_from_slice(r.bytes(code.checked_mul(4)?)?);
    let constants = r.int()?;
    w.int(constants);
    for _ in 0..constants {
        let tag = r.bytes(1)?[0];
        w.out.push(tag);
        match tag {
            0 => {}
            1 => w.out.push(r.bytes(1)?[0]),
            3 => w.out.extend_from_slice(r.bytes(number_size)?),
            4 => {
                let s = r.string()?;
                w.string(s);
            }
            _ => return None,
        }
    }
    let protos = r.int()?;
    w.int(protos);
    for _ in 0..protos {
        function(r, w, number_size, strip, depth + 1)?;
    }
    // Debug information.
    let lines = r.int()?;
    let line_info = r.bytes(lines.checked_mul(r.int_size)?)?;
    let locals = r.int()?;
    let mut local_bytes = Vec::new();
    for _ in 0..locals {
        let name = r.string()?;
        let start = r.int()?;
        let end = r.int()?;
        local_bytes.push((name, start, end));
    }
    let upvalues = r.int()?;
    let mut upvalue_names = Vec::new();
    for _ in 0..upvalues {
        upvalue_names.push(r.string()?);
    }
    if strip.debug {
        w.int(0);
        w.int(0);
        w.int(0);
    } else {
        w.int(lines);
        w.out.extend_from_slice(line_info);
        w.int(locals);
        for (name, start, end) in local_bytes {
            w.string(name);
            w.int(start);
            w.int(end);
        }
        w.int(upvalues);
        for name in upvalue_names {
            w.string(name);
        }
    }
    Some(())
}

/// The line info of every function in `dump`, in dump order (for
/// diagnosing line-preservation differences).
pub fn line_info(dump: &[u8]) -> Option<Vec<Vec<u32>>> {
    let header = dump.get(..12)?;
    let mut r = Reader {
        data: dump,
        pos: 12,
        little: header[6] == 1,
        int_size: usize::from(header[7]),
        size_t: usize::from(header[8]),
    };
    let number_size = usize::from(header[10]);
    let mut out = Vec::new();
    lines_of(&mut r, number_size, &mut out, 0)?;
    Some(out)
}

fn lines_of(r: &mut Reader, number_size: usize, out: &mut Vec<Vec<u32>>, depth: u32) -> Option<()> {
    if depth > 250 {
        return None;
    }
    r.string()?;
    r.int()?;
    r.int()?;
    r.bytes(4)?;
    let code = r.int()?;
    r.bytes(code.checked_mul(4)?)?;
    let constants = r.int()?;
    for _ in 0..constants {
        match r.bytes(1)?[0] {
            0 => {}
            1 => {
                r.bytes(1)?;
            }
            3 => {
                r.bytes(number_size)?;
            }
            4 => {
                r.string()?;
            }
            _ => return None,
        }
    }
    let index = out.len();
    out.push(Vec::new());
    let protos = r.int()?;
    for _ in 0..protos {
        lines_of(r, number_size, out, depth + 1)?;
    }
    let lines = r.int()?;
    let mut info = Vec::with_capacity(lines);
    for _ in 0..lines {
        info.push(u32::try_from(r.int()?).ok()?);
    }
    out[index] = info;
    let locals = r.int()?;
    for _ in 0..locals {
        r.string()?;
        r.int()?;
        r.int()?;
    }
    let upvalues = r.int()?;
    for _ in 0..upvalues {
        r.string()?;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(src: &str) -> Vec<u8> {
        let lua = mlua::Lua::new();
        let f = lua.load(src).into_function().unwrap();
        f.dump(false)
    }

    #[test]
    fn equal_code_compares_equal_without_debug_info() {
        let s = Strip {
            line_defined: true,
            debug: true,
        };
        let a = strip(&dump("local x = 1\nreturn x + 2"), s).unwrap();
        let b = strip(&dump("local   y=1 return y+2"), s).unwrap();
        assert_eq!(a, b);
        let c = strip(&dump("local x = 1 return x + 3"), s).unwrap();
        assert_ne!(a, c);
        let full = Strip {
            line_defined: false,
            debug: false,
        };
        assert_eq!(strip(&dump("return 1"), full).unwrap(), dump("return 1"));
    }
}
