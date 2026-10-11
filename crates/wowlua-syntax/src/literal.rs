//! Values of string literals. The lexer has validated them already, so these
//! functions never fail; malformed input decodes as far as possible.

/// The value of a quoted string (`raw` includes the quotes), following Lua
/// 5.1's `read_string`: unknown escapes yield the character itself.
pub fn short_string_value(raw: &[u8]) -> Vec<u8> {
    let inner = match raw {
        [q, inner @ .., last] if q == last && raw.len() >= 2 => inner,
        [_, inner @ ..] => inner,
        [] => raw,
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        let c = inner[i];
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&e) = inner.get(i) else { break };
        match e {
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'\n' | b'\r' => {
                out.push(b'\n');
                if let Some(&n) = inner.get(i + 1)
                    && (n == b'\n' || n == b'\r')
                    && n != e
                {
                    i += 1;
                }
            }
            b'0'..=b'9' => {
                let mut value = 0u32;
                let mut n = 0;
                while n < 3 && inner.get(i).is_some_and(u8::is_ascii_digit) {
                    value = value * 10 + u32::from(inner[i] - b'0');
                    i += 1;
                    n += 1;
                }
                out.push(value.min(255) as u8);
                continue;
            }
            other => out.push(other),
        }
        i += 1;
    }
    out
}

/// The value of a long string (`raw` includes the brackets): the first line
/// break after the opening bracket is skipped and line breaks become `\n`.
pub fn long_string_value(raw: &[u8]) -> Vec<u8> {
    let level = raw.iter().skip(1).take_while(|&&c| c == b'=').count();
    let open = level + 2;
    let close = (level + 2).min(raw.len().saturating_sub(open));
    let body = raw.get(open..raw.len() - close).unwrap_or_default();
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    let mut first = true;
    while i < body.len() {
        let c = body[i];
        if c == b'\n' || c == b'\r' {
            if let Some(&n) = body.get(i + 1)
                && (n == b'\n' || n == b'\r')
                && n != c
            {
                i += 1;
            }
            if !first {
                out.push(b'\n');
            }
        } else {
            out.push(c);
        }
        first = false;
        i += 1;
    }
    out
}

/// The value of a text part of a backtick string (spec §1.2.4).
pub fn interp_text_value(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c == b'\n' || c == b'\r' {
            if let Some(&n) = raw.get(i + 1)
                && (n == b'\n' || n == b'\r')
                && n != c
            {
                i += 1;
            }
            out.push(b'\n');
            i += 1;
            continue;
        }
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&e) = raw.get(i) else { break };
        match e {
            b'x' => {
                let hex = raw
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok());
                if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                } else {
                    i += 1;
                }
                continue;
            }
            b'u' => {
                let close = raw[i..].iter().position(|&c| c == b'}').map(|p| p + i);
                let code = close.and_then(|end| {
                    std::str::from_utf8(raw.get(i + 2..end)?)
                        .ok()
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                });
                if let (Some(end), Some(code)) = (close, code) {
                    push_utf8(&mut out, code);
                    i = end + 1;
                } else {
                    i += 1;
                }
                continue;
            }
            b'z' => {
                i += 1;
                while raw.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                    i += 1;
                }
                continue;
            }
            b'0'..=b'9' => {
                let mut value = 0u32;
                let mut n = 0;
                while n < 3 && raw.get(i).is_some_and(u8::is_ascii_digit) {
                    value = value * 10 + u32::from(raw[i] - b'0');
                    i += 1;
                    n += 1;
                }
                out.push(value.min(255) as u8);
                continue;
            }
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'\n' | b'\r' => {
                out.push(b'\n');
                if let Some(&n) = raw.get(i + 1)
                    && (n == b'\n' || n == b'\r')
                    && n != e
                {
                    i += 1;
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    out
}

/// UTF-8 encoding of `code` (up to 31 bits, like Lua 5.3's `\u{…}`).
fn push_utf8(out: &mut Vec<u8>, code: u32) {
    if code < 0x80 {
        out.push(code as u8);
        return;
    }
    let mut bytes = Vec::new();
    let mut code = code;
    let mut limit = 0x3F;
    while code > limit {
        bytes.push(0x80 | (code & 0x3F) as u8);
        code >>= 6;
        limit >>= 1;
    }
    let lead = ((!limit << 1) & 0xFF) as u8 | code as u8;
    out.push(lead);
    out.extend(bytes.iter().rev());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_lua_51_escapes() {
        assert_eq!(short_string_value(br#""a\tb\65\q""#), b"a\tbAq");
        assert_eq!(short_string_value(b"'x\\\r\ny'"), b"x\ny");
        assert_eq!(long_string_value(b"[==[\nline\r\nnext]==]"), b"line\nnext");
        assert_eq!(
            interp_text_value(br"\x41\u{20AC}\{"),
            "A\u{20AC}{".as_bytes()
        );
    }
}
