//! Lua 5.1 pattern matching, ported from `lstrlib.c` (same semantics and
//! error messages; character classes are C's in the "C" locale).

pub const MAXCAPTURES: usize = 32;
const L_ESC: u8 = b'%';
pub const SPECIALS: &[u8] = b"^$*+?.([%-";

/// A capture: a byte range, or a position (`()`).
#[derive(Clone, Copy, Debug)]
pub enum Capture {
    Unfinished(usize),
    Range(usize, usize),
    Position(usize),
}

pub struct MatchState<'a> {
    pub src: &'a [u8],
    pub pat: &'a [u8],
    pub level: usize,
    pub capture: [Capture; MAXCAPTURES],
}

pub type MResult<T> = Result<T, String>;

impl<'a> MatchState<'a> {
    pub fn new(src: &'a [u8], pat: &'a [u8]) -> Self {
        // The pattern is a C string: it ends at the first NUL.
        let pat = match pat.iter().position(|&b| b == 0) {
            Some(i) => &pat[..i],
            None => pat,
        };
        MatchState {
            src,
            pat,
            level: 0,
            capture: [Capture::Position(0); MAXCAPTURES],
        }
    }

    fn pat_at(&self, p: usize) -> u8 {
        // The C string ends with a NUL.
        self.pat.get(p).copied().unwrap_or(0)
    }

    fn check_capture(&self, l: u8) -> MResult<usize> {
        let l = l.wrapping_sub(b'1') as usize;
        if l >= self.level || matches!(self.capture[l], Capture::Unfinished(_)) {
            return Err("invalid capture index".into());
        }
        Ok(l)
    }

    fn capture_to_close(&self) -> MResult<usize> {
        let mut level = self.level;
        while level > 0 {
            level -= 1;
            if matches!(self.capture[level], Capture::Unfinished(_)) {
                return Ok(level);
            }
        }
        Err("invalid pattern capture".into())
    }

    fn classend(&self, mut p: usize) -> MResult<usize> {
        let c = self.pat_at(p);
        p += 1;
        if c == L_ESC {
            if p >= self.pat.len() {
                return Err("malformed pattern (ends with '%')".into());
            }
            return Ok(p + 1);
        }
        if c == b'[' {
            if self.pat_at(p) == b'^' {
                p += 1;
            }
            loop {
                // Look for a `]`.
                if p >= self.pat.len() {
                    return Err("malformed pattern (missing ']')".into());
                }
                let cc = self.pat[p];
                p += 1;
                if cc == L_ESC && p < self.pat.len() {
                    p += 1; // skip escapes (e.g. `%]`)
                }
                if self.pat_at(p) == b']' {
                    break;
                }
            }
            return Ok(p + 1);
        }
        Ok(p)
    }

    fn matchbracketclass(&self, c: u8, p: usize, ec: usize) -> bool {
        let mut p = p;
        let mut sig = true;
        if self.pat_at(p + 1) == b'^' {
            sig = false;
            p += 1;
        }
        p += 1;
        while p < ec {
            if self.pat[p] == L_ESC {
                p += 1;
                if match_class(c, self.pat_at(p)) {
                    return sig;
                }
            } else if self.pat_at(p + 1) == b'-' && p + 2 < ec {
                p += 2;
                if self.pat[p - 2] <= c && c <= self.pat[p] {
                    return sig;
                }
            } else if self.pat[p] == c {
                return sig;
            }
            p += 1;
        }
        !sig
    }

    fn singlematch(&self, s: usize, p: usize, ep: usize) -> bool {
        if s >= self.src.len() {
            return false;
        }
        let c = self.src[s];
        match self.pat_at(p) {
            b'.' => true,
            L_ESC => match_class(c, self.pat_at(p + 1)),
            b'[' => self.matchbracketclass(c, p, ep - 1),
            pc => pc == c,
        }
    }

    fn matchbalance(&self, s: usize, p: usize) -> MResult<Option<usize>> {
        if p + 1 >= self.pat.len() {
            return Err("unbalanced pattern".into());
        }
        if s >= self.src.len() || self.src[s] != self.pat[p] {
            return Ok(None);
        }
        let b = self.pat[p];
        let e = self.pat[p + 1];
        let mut cont = 1;
        let mut s = s + 1;
        while s < self.src.len() {
            let c = self.src[s];
            if c == e {
                cont -= 1;
                if cont == 0 {
                    return Ok(Some(s + 1));
                }
            } else if c == b {
                cont += 1;
            }
            s += 1;
        }
        Ok(None)
    }

    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> MResult<Option<usize>> {
        let mut i = 0;
        while self.singlematch(s + i, p, ep) {
            i += 1;
        }
        // Try with the maximum repetitions, then fewer.
        loop {
            if let Some(r) = self.do_match(s + i, ep + 1)? {
                return Ok(Some(r));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    fn min_expand(&mut self, mut s: usize, p: usize, ep: usize) -> MResult<Option<usize>> {
        loop {
            if let Some(r) = self.do_match(s, ep + 1)? {
                return Ok(Some(r));
            }
            if self.singlematch(s, p, ep) {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(&mut self, s: usize, p: usize, position: bool) -> MResult<Option<usize>> {
        if self.level >= MAXCAPTURES {
            return Err("too many captures".into());
        }
        self.capture[self.level] = if position {
            Capture::Position(s)
        } else {
            Capture::Unfinished(s)
        };
        self.level += 1;
        let r = self.do_match(s, p)?;
        if r.is_none() {
            self.level -= 1;
        }
        Ok(r)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> MResult<Option<usize>> {
        let l = self.capture_to_close()?;
        let Capture::Unfinished(start) = self.capture[l] else {
            unreachable!()
        };
        self.capture[l] = Capture::Range(start, s);
        let r = self.do_match(s, p)?;
        if r.is_none() {
            self.capture[l] = Capture::Unfinished(start);
        }
        Ok(r)
    }

    fn match_capture(&self, s: usize, l: u8) -> MResult<Option<usize>> {
        let l = self.check_capture(l)?;
        let (start, end) = match self.capture[l] {
            Capture::Range(a, b) => (a, b),
            // A position capture's length is negative in C: never matches.
            Capture::Position(_) | Capture::Unfinished(_) => return Ok(None),
        };
        let len = end - start;
        if self.src.len() - s >= len && self.src[start..end] == self.src[s..s + len] {
            Ok(Some(s + len))
        } else {
            Ok(None)
        }
    }

    /// Matches the pattern from `p` at source position `s`; the end of the
    /// match.
    pub fn do_match(&mut self, s: usize, p: usize) -> MResult<Option<usize>> {
        let mut s = s;
        let mut p = p;
        loop {
            if p >= self.pat.len() {
                return Ok(Some(s));
            }
            match self.pat[p] {
                b'(' => {
                    return if self.pat_at(p + 1) == b')' {
                        self.start_capture(s, p + 2, true)
                    } else {
                        self.start_capture(s, p + 1, false)
                    };
                }
                b')' => return self.end_capture(s, p + 1),
                L_ESC => match self.pat_at(p + 1) {
                    b'b' => match self.matchbalance(s, p + 2)? {
                        Some(e) => {
                            s = e;
                            p += 4;
                            continue;
                        }
                        None => return Ok(None),
                    },
                    b'f' => {
                        p += 2;
                        if self.pat_at(p) != b'[' {
                            return Err("missing '[' after '%f' in pattern".into());
                        }
                        let ep = self.classend(p)?;
                        let prev = if s == 0 { 0 } else { self.src[s - 1] };
                        let cur = self.src.get(s).copied().unwrap_or(0);
                        if !self.matchbracketclass(prev, p, ep - 1)
                            && self.matchbracketclass(cur, p, ep - 1)
                        {
                            p = ep;
                            continue;
                        }
                        return Ok(None);
                    }
                    d if d.is_ascii_digit() => match self.match_capture(s, d)? {
                        Some(e) => {
                            s = e;
                            p += 2;
                            continue;
                        }
                        None => return Ok(None),
                    },
                    _ => {}
                },
                b'$' if p + 1 == self.pat.len() => {
                    return Ok(if s == self.src.len() { Some(s) } else { None });
                }
                _ => {}
            }
            // default: a single character class and its repetition.
            let ep = self.classend(p)?;
            let m = self.singlematch(s, p, ep);
            match self.pat_at(ep) {
                b'?' => {
                    if m && let Some(r) = self.do_match(s + 1, ep + 1)? {
                        return Ok(Some(r));
                    }
                    p = ep + 1;
                    continue;
                }
                b'*' => return self.max_expand(s, p, ep),
                b'+' => {
                    return if m {
                        self.max_expand(s + 1, p, ep)
                    } else {
                        Ok(None)
                    };
                }
                b'-' => return self.min_expand(s, p, ep),
                _ => {
                    if !m {
                        return Ok(None);
                    }
                    s += 1;
                    p = ep;
                }
            }
        }
    }
}

/// `match_class`: `%a`, `%d`, … (upper case: the complement); other
/// characters match themselves.
fn match_class(c: u8, cl: u8) -> bool {
    let res = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        b'z' => c == 0,
        _ => return cl == c,
    };
    if cl.is_ascii_uppercase() { !res } else { res }
}
