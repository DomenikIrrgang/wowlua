//! `shorten-literals` (spec optimization §15.4.1): the shortest numeral that
//! Lua 5.1 reads as the same number, and the shortest quoting of a string.
//! Both leave the chunk's constants unchanged.

/// The bytes a Lua 5.1 string literal stands for (`"…"`, `'…'` or a long
/// bracket), or `None` if it is not one.
pub fn decode_string(raw: &[u8]) -> Option<Vec<u8>> {
    match raw.first()? {
        b'"' | b'\'' => decode_short(raw),
        b'[' => decode_long(raw),
        _ => None,
    }
}

fn decode_short(raw: &[u8]) -> Option<Vec<u8>> {
    let quote = raw[0];
    if raw.len() < 2 || *raw.last()? != quote {
        return None;
    }
    let body = &raw[1..raw.len() - 1];
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let c = body[i];
        if c != b'\\' {
            if c == b'\n' || c == b'\r' || c == quote {
                return None;
            }
            out.push(c);
            i += 1;
            continue;
        }
        let e = *body.get(i + 1)?;
        i += 2;
        match e {
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            // An escaped line break is a `\n`; `\r\n` and `\n\r` count once.
            b'\n' | b'\r' => {
                out.push(b'\n');
                if let Some(&n) = body.get(i)
                    && (n == b'\n' || n == b'\r')
                    && n != e
                {
                    i += 1;
                }
            }
            b'0'..=b'9' => {
                let mut value = u32::from(e - b'0');
                let mut digits = 1;
                while digits < 3
                    && let Some(&d) = body.get(i)
                    && d.is_ascii_digit()
                {
                    value = value * 10 + u32::from(d - b'0');
                    i += 1;
                    digits += 1;
                }
                out.push(u8::try_from(value).ok()?);
            }
            other => out.push(other),
        }
    }
    Some(out)
}

fn decode_long(raw: &[u8]) -> Option<Vec<u8>> {
    let level = raw[1..].iter().take_while(|&&c| c == b'=').count();
    if raw.get(1 + level) != Some(&b'[') {
        return None;
    }
    let close_len = level + 2;
    let start = level + 2;
    if raw.len() < start + close_len {
        return None;
    }
    let end = raw.len() - close_len;
    let closing = &raw[end..];
    if closing[0] != b']'
        || closing[close_len - 1] != b']'
        || closing[1..close_len - 1].iter().any(|&c| c != b'=')
    {
        return None;
    }
    let body = &raw[start..end];
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    // The first line break is skipped.
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
    Some(out)
}

/// The shortest quoted literal for `bytes`; `prefer` breaks ties between
/// the two quote characters.
pub fn encode_string(bytes: &[u8], prefer: u8) -> Vec<u8> {
    let doubles = bytes.iter().filter(|&&c| c == b'"').count();
    let singles = bytes.iter().filter(|&&c| c == b'\'').count();
    let quote = match doubles.cmp(&singles) {
        std::cmp::Ordering::Less => b'"',
        std::cmp::Ordering::Greater => b'\'',
        std::cmp::Ordering::Equal if prefer == b'\'' => b'\'',
        std::cmp::Ordering::Equal => b'"',
    };
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(quote);
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.push(b'\t'),
            7 => out.extend_from_slice(b"\\a"),
            8 => out.extend_from_slice(b"\\b"),
            11 => out.extend_from_slice(b"\\v"),
            12 => out.extend_from_slice(b"\\f"),
            c if c == quote => {
                out.push(b'\\');
                out.push(c);
            }
            c if c < 32 || c == 127 => {
                // `\1` followed by a digit would read as one escape.
                let next_digit = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
                let digits = if next_digit {
                    format!("{c:03}")
                } else {
                    c.to_string()
                };
                out.push(b'\\');
                out.extend_from_slice(digits.as_bytes());
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// The shortest form of string literal `raw`, or `None` to keep it.
pub fn shorten_string(raw: &[u8]) -> Option<Vec<u8>> {
    let bytes = decode_string(raw)?;
    let prefer = if raw[0] == b'\'' { b'\'' } else { b'"' };
    let short = encode_string(&bytes, prefer);
    (short.len() < raw.len()).then_some(short)
}

/// The value Lua 5.1 reads from numeral `text`, when it is certain on every
/// client (`strtod`, or `strtoul` for hexadecimal up to 32 bits).
fn numeral_value(text: &str) -> Option<f64> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        let value = u32::from_str_radix(hex, 16).ok()?;
        return Some(f64::from(value));
    }
    if !text
        .bytes()
        .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return None;
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The shortest numeral for the value of `text`, or `None` to keep it.
pub fn shorten_number(text: &str) -> Option<String> {
    let value = numeral_value(text)?;
    // Shortest round-trip digits and their exponent: value = digits × 10^exp.
    let sci = format!("{value:e}");
    let (mantissa, exp) = sci.split_once('e')?;
    let exp: i64 = exp.parse().ok()?;
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exp = exp - (digits.len() as i64 - 1);
    let mut candidates: Vec<String> = Vec::new();
    if digits == "0" {
        candidates.push("0".into());
    } else {
        if exp != 0 {
            candidates.push(format!("{digits}e{exp}"));
        }
        if exp >= 0 {
            candidates.push(format!("{digits}{}", "0".repeat(exp as usize)));
        } else {
            let point = digits.len() as i64 + exp;
            if point > 0 {
                let (a, b) = digits.split_at(point as usize);
                candidates.push(format!("{a}.{b}"));
            } else {
                candidates.push(format!(".{}{digits}", "0".repeat((-point) as usize)));
            }
        }
    }
    let best = candidates.into_iter().min_by_key(String::len)?;
    (best.len() < text.len() && numeral_value(&best) == Some(value)).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_lua_strings() {
        assert_eq!(decode_string(br#""a\tb\\\"""#).unwrap(), b"a\tb\\\"");
        assert_eq!(decode_string(br"'\65\0662'").unwrap(), b"AB2");
        assert_eq!(decode_string(b"\"a\\\r\nb\"").unwrap(), b"a\nb");
        assert_eq!(decode_string(b"[==[\nx]]\r\ny]==]").unwrap(), b"x]]\ny");
        assert_eq!(decode_string(b"[[]]").unwrap(), b"");
        assert_eq!(decode_string(br#""\256""#), None);
    }

    #[test]
    fn encodes_the_shortest_quoting() {
        assert_eq!(encode_string(b"it's", b'\''), br#""it's""#);
        assert_eq!(encode_string(br#"say "hi""#, b'"'), br#"'say "hi"'"#);
        assert_eq!(encode_string(b"\x01\x02", b'"'), br#""\1\2""#);
        assert_eq!(encode_string(b"\x011", b'"'), br#""\0011""#);
        assert_eq!(shorten_string(b"[[abc]]").unwrap(), b"\"abc\"");
        assert_eq!(shorten_string(b"\"abc\""), None);
        assert_eq!(shorten_string(br#""\097""#).unwrap(), b"\"a\"");
    }

    #[test]
    fn shortens_numerals() {
        assert_eq!(shorten_number("1000000").as_deref(), Some("1e6"));
        assert_eq!(shorten_number("0.5").as_deref(), Some(".5"));
        assert_eq!(shorten_number("0.000001").as_deref(), Some("1e-6"));
        assert_eq!(shorten_number("1.50").as_deref(), Some("1.5"));
        assert_eq!(shorten_number("0x10").as_deref(), Some("16"));
        assert_eq!(shorten_number("1500000").as_deref(), Some("15e5"));
        assert_eq!(shorten_number("0.0").as_deref(), Some("0"));
        assert_eq!(shorten_number("42"), None);
        assert_eq!(shorten_number("0x1p4"), None);
        assert_eq!(shorten_number("0x100000000"), None);
        assert_eq!(shorten_number("123.456"), None);
    }
}
